//! Shared helpers of the alert tests: a local SMTP sink, a local webhook sink, a fixed resolver and
//! the seeding of jobs (AC-MN-01, AC-MN-02, ADR-0036).

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::net::IpAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use restichub_cp::alerts::sender::Sender;
use restichub_cp::alerts::settings::{AlertsSettings, SmtpSecurity, SmtpSettings, WebhookSettings};
use restichub_cp::alerts::transport::{BoxIo, ResolveFuture, Resolver, tls_config_with_roots};
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, JobId, PlanId, RepositoryId, TenantId};
use time::OffsetDateTime;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{self, RootCertStore, ServerConfig};

pub const ORIGIN: &str = "https://backup.example.com";

/// One message that the SMTP sink accepted.
#[derive(Debug, Clone)]
pub struct Mail {
    pub from: String,
    pub to: Vec<String>,
    pub data: String,
}

impl Mail {
    pub fn header(&self, name: &str) -> Option<String> {
        let prefix = format!("{}:", name.to_ascii_lowercase());
        self.data
            .lines()
            .take_while(|line| !line.is_empty())
            .find(|line| line.to_ascii_lowercase().starts_with(&prefix))
            .map(|line| line[prefix.len()..].trim().to_owned())
    }

    pub fn text(&self) -> String {
        let body = self.data.split("\r\n\r\n").nth(1).unwrap_or_default();
        let joined: String = body.split("\r\n").collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(joined.trim())
            .expect("the body is base64");
        String::from_utf8(bytes).expect("the body is UTF-8")
    }
}

/// What the SMTP sink offers: TLS from the first byte, `STARTTLS`, and `AUTH PLAIN`.
#[derive(Clone, Default)]
pub struct SmtpOptions {
    pub implicit_tls: Option<Arc<ServerConfig>>,
    pub starttls: Option<Arc<ServerConfig>>,
    pub offer_auth: bool,
}

pub struct SmtpSink {
    pub port: u16,
    pub mails: Arc<Mutex<Vec<Mail>>>,
    /// The decoded `AUTH PLAIN` payloads that the sink saw.
    pub credentials: Arc<Mutex<Vec<String>>>,
}

impl SmtpSink {
    pub async fn start() -> Self {
        Self::start_with(SmtpOptions::default()).await
    }

    /// Accepts TLS from the first byte with the given server configuration.
    pub async fn start_tls(config: Arc<ServerConfig>) -> Self {
        Self::start_with(SmtpOptions {
            implicit_tls: Some(config),
            ..SmtpOptions::default()
        })
        .await
    }

    pub async fn start_with(options: SmtpOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let mails = Arc::new(Mutex::new(Vec::new()));
        let credentials = Arc::new(Mutex::new(Vec::new()));
        let (store, creds) = (mails.clone(), credentials.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let (store, creds, options) = (store.clone(), creds.clone(), options.clone());
                tokio::spawn(async move {
                    let stream: BoxIo = match &options.implicit_tls {
                        Some(config) => {
                            match TlsAcceptor::from(config.clone()).accept(stream).await {
                                Ok(stream) => Box::new(stream),
                                Err(_) => return,
                            }
                        }
                        None => Box::new(stream),
                    };
                    serve_smtp(stream, store, creds, options).await;
                });
            }
        });
        Self {
            port,
            mails,
            credentials,
        }
    }

    pub fn received(&self) -> Vec<Mail> {
        self.mails.lock().expect("lock").clone()
    }
}

async fn serve_smtp(
    stream: BoxIo,
    store: Arc<Mutex<Vec<Mail>>>,
    credentials: Arc<Mutex<Vec<String>>>,
    options: SmtpOptions,
) {
    let mut reader = BufReader::new(stream);
    // Before the upgrade a server that offers `STARTTLS` offers no `AUTH`.
    let mut secure = options.starttls.is_none();
    if reader
        .get_mut()
        .write_all(b"220 sink ready\r\n")
        .await
        .is_err()
    {
        return;
    }
    let (mut from, mut to) = (String::new(), Vec::new());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let upper = line.to_ascii_uppercase();
        let reply: Vec<u8> = if upper.starts_with("EHLO") {
            let mut caps = String::from("250-sink\r\n");
            if !secure {
                caps.push_str("250-STARTTLS\r\n");
            } else if options.offer_auth {
                caps.push_str("250-AUTH PLAIN\r\n");
            }
            caps.push_str("250 8BITMIME\r\n");
            caps.into_bytes()
        } else if upper.starts_with("STARTTLS") {
            let Some(config) = &options.starttls else {
                return;
            };
            if reader
                .get_mut()
                .write_all(b"220 go ahead\r\n")
                .await
                .is_err()
            {
                return;
            }
            let inner = reader.into_inner();
            match TlsAcceptor::from(config.clone()).accept(inner).await {
                Ok(upgraded) => {
                    reader = BufReader::new(Box::new(upgraded));
                    secure = true;
                    continue;
                }
                Err(_) => return,
            }
        } else if upper.starts_with("AUTH PLAIN") {
            let token = line["AUTH PLAIN".len()..].trim();
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(token)
                .unwrap_or_default();
            credentials
                .lock()
                .expect("lock")
                .push(String::from_utf8_lossy(&decoded).replace('\0', "|"));
            b"235 welcome\r\n".to_vec()
        } else if upper.starts_with("MAIL FROM:") {
            from = between_angles(&line);
            b"250 ok\r\n".to_vec()
        } else if upper.starts_with("RCPT TO:") {
            let recipient = between_angles(&line);
            if recipient.starts_with("reject") {
                b"550 no such user\r\n".to_vec()
            } else {
                to.push(recipient);
                b"250 ok\r\n".to_vec()
            }
        } else if upper.starts_with("DATA") {
            if reader.get_mut().write_all(b"354 go\r\n").await.is_err() {
                return;
            }
            let mut data = String::new();
            loop {
                let mut part = String::new();
                if reader.read_line(&mut part).await.unwrap_or(0) == 0 {
                    return;
                }
                if part == ".\r\n" {
                    break;
                }
                data.push_str(&part);
            }
            store.lock().expect("lock").push(Mail {
                from: from.clone(),
                to: std::mem::take(&mut to),
                data,
            });
            b"250 queued\r\n".to_vec()
        } else if upper.starts_with("QUIT") {
            let _ = reader.get_mut().write_all(b"221 bye\r\n").await;
            return;
        } else {
            b"500 what\r\n".to_vec()
        };
        if reader.get_mut().write_all(&reply).await.is_err() {
            return;
        }
    }
}

fn between_angles(line: &str) -> String {
    let start = line.find('<').map_or(0, |i| i + 1);
    let end = line.find('>').unwrap_or(line.len());
    line[start..end].to_owned()
}

/// One request that the webhook sink received.
#[derive(Debug, Clone)]
pub struct HookRequest {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HookRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("the body is JSON")
    }
}

pub struct WebhookSink {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<HookRequest>>>,
    pub status: Arc<AtomicU16>,
}

impl WebhookSink {
    pub async fn start() -> Self {
        Self::start_with(None).await
    }

    pub async fn start_tls(config: Arc<ServerConfig>) -> Self {
        Self::start_with(Some(config)).await
    }

    async fn start_with(tls: Option<Arc<ServerConfig>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let status = Arc::new(AtomicU16::new(200));
        let (store, code) = (requests.clone(), status.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let (store, code, tls) = (store.clone(), code.clone(), tls.clone());
                tokio::spawn(async move {
                    match tls {
                        Some(config) => {
                            if let Ok(stream) = TlsAcceptor::from(config).accept(stream).await {
                                serve_http(stream, store, code).await;
                            }
                        }
                        None => serve_http(stream, store, code).await,
                    }
                });
            }
        });
        Self {
            port,
            requests,
            status,
        }
    }

    pub fn received(&self) -> Vec<HookRequest> {
        self.requests.lock().expect("lock").clone()
    }

    pub fn answer_with(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
    }
}

async fn serve_http<S>(stream: S, store: Arc<Mutex<Vec<HookRequest>>>, status: Arc<AtomicU16>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
        return;
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let length: usize = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    if reader.read_exact(&mut body).await.is_err() {
        return;
    }
    store.lock().expect("lock").push(HookRequest {
        request_line: request_line.trim_end().to_owned(),
        headers,
        body,
    });
    let code = status.load(Ordering::SeqCst);
    let answer = format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = reader.get_mut().write_all(answer.as_bytes()).await;
    let _ = reader.get_mut().shutdown().await;
}

/// Answers every lookup with one address.
pub struct FixedResolver(pub IpAddr);

impl Resolver for FixedResolver {
    fn resolve(&self, _host: &str, _port: u16) -> ResolveFuture {
        let address = self.0;
        Box::pin(async move { Ok(vec![address]) })
    }
}

/// Allows plain HTTP and loopback, so the sender reaches the sinks.
pub fn local_settings(smtp_port: Option<u16>, security: SmtpSecurity) -> AlertsSettings {
    AlertsSettings {
        public_origin: Some(ORIGIN.to_owned()),
        smtp: smtp_port.map(|port| SmtpSettings {
            host: "127.0.0.1".to_owned(),
            port,
            security,
            username: None,
            password: None,
            from: "restichub@backup.example.com".to_owned(),
        }),
        webhook: WebhookSettings {
            allow_http: true,
            allow_networks: restichub_cp::alerts::ssrf::networks(&["127.0.0.0/8"]),
        },
        language: restichub_cp::alerts::text::Language::default(),
    }
}

pub fn local_sender(smtp_port: Option<u16>) -> Sender {
    Sender::new(Arc::new(local_settings(smtp_port, SmtpSecurity::None))).expect("sender")
}

/// A self-signed certificate for the name, the server configuration and a client configuration
/// that trusts it.
pub fn tls_pair(name: &str) -> (Arc<ServerConfig>, Arc<rustls::ClientConfig>) {
    let certified = rcgen::generate_simple_self_signed(vec![name.to_owned()]).expect("cert");
    let der = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key)
        .expect("server config");
    let mut roots = RootCertStore::empty();
    roots.add(der).expect("root");
    (
        Arc::new(server),
        tls_config_with_roots(roots).expect("client config"),
    )
}

/// An owner-level transaction with the tenant scope, for a fixture the API does not offer.
pub async fn scoped(
    db: &TestDatabase,
    tenant: TenantId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = db.owner_pool().begin().await.expect("begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("scope");
    tx
}

/// Moves a job to a final status.
#[allow(clippy::too_many_arguments)]
pub async fn finish_job(
    db: &TestDatabase,
    tenant: TenantId,
    job: JobId,
    status: &str,
    result_code: &str,
    message: Option<&str>,
    finished_at: OffsetDateTime,
) {
    let mut tx = scoped(db, tenant).await;
    sqlx::query(
        "UPDATE jobs SET status = $2, result_code = $3, result_message = $4, \
                started_at = $5, finished_at = $5, updated_at = $5 WHERE id = $1",
    )
    .bind(job.as_uuid())
    .bind(status)
    .bind(result_code)
    .bind(message)
    .bind(finished_at)
    .execute(&mut *tx)
    .await
    .expect("cannot finish the job");
    tx.commit().await.expect("commit");
}

/// Seeds a scheduled backup job of a plan, one fire time apart from the others.
pub async fn backup_job(
    db: &TestDatabase,
    tenant: TenantId,
    plan: PlanId,
    agent: AgentId,
    repository: RepositoryId,
    fire_time: OffsetDateTime,
) -> JobId {
    crate::common::seed_scheduled_job(db.database(), tenant, plan, agent, repository, fire_time)
        .await
}

pub async fn set_last_seen(
    db: &TestDatabase,
    tenant: TenantId,
    agent: AgentId,
    seen: Option<OffsetDateTime>,
) {
    let mut tx = scoped(db, tenant).await;
    sqlx::query("UPDATE agents SET last_seen_at = $2, first_connected_at = COALESCE(first_connected_at, $2) WHERE id = $1")
        .bind(agent.as_uuid())
        .bind(seen)
        .execute(&mut *tx)
        .await
        .expect("cannot set last_seen_at");
    tx.commit().await.expect("commit");
}

pub async fn plan_created_at(db: &TestDatabase, tenant: TenantId, plan: PlanId) -> OffsetDateTime {
    let mut tx = scoped(db, tenant).await;
    let (created,): (OffsetDateTime,) =
        sqlx::query_as("SELECT created_at FROM plans WHERE id = $1")
            .bind(plan.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("plan");
    created
}
