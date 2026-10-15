//! A small SMTP client for the email channels (FR-MN-04, ADR-0036).
//!
//! The client sends one plain text message to a list of recipients. It supports implicit TLS,
//! `STARTTLS` and a plain connection, and `AUTH PLAIN`. It never sends the credentials over a
//! plain connection.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc2822;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::pki_types::ServerName;

use super::settings::{SmtpSecurity, SmtpSettings};
use super::transport::{BoxIo, SendError, because};

/// Limit for the whole send, so a stuck server cannot hold a pass.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest line of a server reply that the client reads.
const MAX_REPLY_LINE: usize = 1024;

/// Bounds a server that never ends its answer.
const MAX_REPLY_LINES: usize = 64;

/// One message. The caller fills it with scrubbed text.
#[derive(Debug, Clone)]
pub struct Mail {
    pub subject: String,
    pub text: String,
    pub message_id_domain: String,
    pub date: OffsetDateTime,
    pub id: String,
}

/// Sends the message to the recipients through the server of the installation. `Some` is the note
/// about recipients that the server refused while it accepted others.
pub async fn send(
    settings: &SmtpSettings,
    tls: &Arc<ClientConfig>,
    recipients: &[String],
    mail: &Mail,
) -> Result<Option<String>, SendError> {
    tokio::time::timeout(SEND_TIMEOUT, send_inner(settings, tls, recipients, mail))
        .await
        .map_err(because(SendError::Timeout))?
}

async fn send_inner(
    settings: &SmtpSettings,
    tls: &Arc<ClientConfig>,
    recipients: &[String],
    mail: &Mail,
) -> Result<Option<String>, SendError> {
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((settings.host.as_str(), settings.port)),
    )
    .await
    .map_err(because(SendError::Timeout))?
    .map_err(because(SendError::ConnectFailed))?;
    let connector = TlsConnector::from(tls.clone());
    let server_name =
        ServerName::try_from(settings.host.clone()).map_err(because(SendError::TlsFailed))?;

    let mut stream: BoxIo = match settings.security {
        SmtpSecurity::Tls => Box::new(
            connector
                .connect(server_name.clone(), tcp)
                .await
                .map_err(because(SendError::TlsFailed))?,
        ),
        _ => Box::new(tcp),
    };

    let mut session = Session::new(stream);
    session.expect(&[220]).await?;
    let mut capabilities = session.ehlo().await?;

    if settings.security == SmtpSecurity::StartTls {
        if !capabilities
            .iter()
            .any(|line| line.eq_ignore_ascii_case("STARTTLS"))
        {
            return Err(SendError::SmtpUnsupported("starttls"));
        }
        session.command("STARTTLS", &[220]).await?;
        stream = session.into_inner();
        stream = Box::new(
            connector
                .connect(server_name, stream)
                .await
                .map_err(because(SendError::TlsFailed))?,
        );
        session = Session::new(stream);
        capabilities = session.ehlo().await?;
    }

    if let (Some(user), Some(password)) = (&settings.username, &settings.password) {
        if settings.security == SmtpSecurity::None {
            return Err(SendError::SmtpUnsupported("auth_without_tls"));
        }
        if !capabilities.iter().any(|line| {
            line.to_ascii_uppercase().starts_with("AUTH")
                && line.to_ascii_uppercase().contains("PLAIN")
        }) {
            return Err(SendError::SmtpUnsupported("auth_plain"));
        }
        let token = STANDARD.encode(format!("\0{user}\0{}", password.expose_secret()));
        session
            .command(&format!("AUTH PLAIN {token}"), &[235])
            .await?;
    }

    session
        .command(&format!("MAIL FROM:<{}>", settings.from), &[250])
        .await?;
    let mut accepted = Vec::new();
    let mut refused = 0_usize;
    let mut last_rejection = None;
    for recipient in recipients {
        match session
            .command(&format!("RCPT TO:<{recipient}>"), &[250, 251])
            .await
        {
            Ok(_) => accepted.push(recipient.clone()),
            Err(error @ SendError::Smtp { .. }) => {
                refused += 1;
                last_rejection = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    if accepted.is_empty() {
        return Err(last_rejection.unwrap_or(SendError::BadResponse));
    }
    let note = last_rejection.map(|error| {
        format!(
            "partial_recipients: {refused} of {} refused, last {}",
            recipients.len(),
            error.code()
        )
    });
    session.command("DATA", &[354]).await?;
    session
        .write_raw(&render(&settings.from, recipients, mail))
        .await?;
    session.expect(&[250]).await?;
    // The server may close at once after QUIT; the message is already accepted.
    let _ = session.command("QUIT", &[221]).await;
    Ok(note)
}

/// Builds the message with CRLF line ends and dot stuffing, ending with the lone dot line.
fn render(from: &str, recipients: &[String], mail: &Mail) -> Vec<u8> {
    let date = mail.date.format(&Rfc2822).unwrap_or_default();
    let mut headers = vec![
        format!("From: <{from}>"),
        format!(
            "To: {}",
            recipients
                .iter()
                .map(|r| format!("<{r}>"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!("Subject: {}", encode_subject(&mail.subject)),
        format!("Date: {date}"),
        format!("Message-ID: <{}@{}>", mail.id, mail.message_id_domain),
        "MIME-Version: 1.0".to_owned(),
        "Content-Type: text/plain; charset=utf-8".to_owned(),
        "Content-Transfer-Encoding: base64".to_owned(),
        "Auto-Submitted: auto-generated".to_owned(),
    ];
    let encoded = STANDARD.encode(mail.text.as_bytes());
    let mut body = String::new();
    for chunk in encoded.as_bytes().chunks(76) {
        body.push_str(&String::from_utf8_lossy(chunk));
        body.push_str("\r\n");
    }
    headers.push(String::new());
    let mut out = headers.join("\r\n").into_bytes();
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body.as_bytes());
    // The base64 text has no line that starts with a dot, so no stuffing is needed for the body.
    out.extend_from_slice(b".\r\n");
    out
}

/// The most bytes of text in one encoded word: 45 bytes give 60 base64 characters, and with the
/// 12 characters of `=?UTF-8?B?` and `?=` the word is 72 characters, under the 75 of RFC 2047.
const ENCODED_WORD_BYTES: usize = 45;

/// An ASCII subject goes as it is; any other goes as RFC 2047 encoded words of at most 75
/// characters, folded on a space. No word splits a character.
fn encode_subject(subject: &str) -> String {
    let clean: String = subject.chars().filter(|c| !c.is_control()).collect();
    if clean.is_ascii() {
        return clean;
    }
    let mut words = Vec::new();
    let mut current = String::new();
    for character in clean.chars() {
        if current.len() + character.len_utf8() > ENCODED_WORD_BYTES {
            words.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .iter()
        .map(|word| format!("=?UTF-8?B?{}?=", STANDARD.encode(word.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

struct Session {
    reader: BufReader<BoxIo>,
}

impl Session {
    fn new(stream: BoxIo) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    fn into_inner(self) -> BoxIo {
        self.reader.into_inner()
    }

    async fn write_raw(&mut self, bytes: &[u8]) -> Result<(), SendError> {
        self.reader.get_mut().write_all(bytes).await?;
        self.reader.get_mut().flush().await?;
        Ok(())
    }

    /// Reads one reply and checks its code. Returns the text of every line.
    async fn expect(&mut self, codes: &[u16]) -> Result<Vec<String>, SendError> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let read = (&mut self.reader)
                .take(MAX_REPLY_LINE as u64)
                .read_line(&mut line)
                .await?;
            if read == 0 || !line.ends_with('\n') || lines.len() >= MAX_REPLY_LINES {
                return Err(SendError::BadResponse);
            }
            let line = line.trim_end();
            let code: u16 = line
                .get(..3)
                .and_then(|digits| digits.parse().ok())
                .ok_or(SendError::BadResponse)?;
            let text = line.get(4..).unwrap_or("").to_owned();
            let last = line.as_bytes().get(3) != Some(&b'-');
            lines.push(text);
            if last {
                return if codes.contains(&code) {
                    Ok(lines)
                } else {
                    Err(SendError::Smtp {
                        code,
                        text: lines.join(" "),
                    })
                };
            }
        }
    }

    async fn command(&mut self, command: &str, codes: &[u16]) -> Result<Vec<String>, SendError> {
        self.write_raw(format!("{command}\r\n").as_bytes()).await?;
        self.expect(codes).await
    }

    /// The capability lines follow the first line of the answer.
    async fn ehlo(&mut self) -> Result<Vec<String>, SendError> {
        let lines = self.command("EHLO restichub", &[250]).await?;
        Ok(lines.into_iter().skip(1).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-MN-04: an encoded word is at most 75 characters and holds whole characters.
    #[test]
    fn a_non_ascii_subject_goes_as_short_encoded_words() {
        let subject =
            "Резервная копия не выполнена на хосте очень-длинное-имя-сервера-для-проверки";
        let encoded = encode_subject(subject);
        let words: Vec<&str> = encoded.split("\r\n ").collect();
        assert!(words.len() > 1);
        let mut decoded = String::new();
        for word in words {
            assert!(word.len() <= 75, "{word}");
            let inner = word
                .strip_prefix("=?UTF-8?B?")
                .and_then(|rest| rest.strip_suffix("?="))
                .expect("an encoded word");
            let bytes = STANDARD.decode(inner).expect("base64");
            decoded.push_str(std::str::from_utf8(&bytes).expect("whole characters"));
        }
        assert_eq!(decoded, subject);
        assert_eq!(encode_subject("Plain subject"), "Plain subject");
    }
}
