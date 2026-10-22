use tracing_subscriber::EnvFilter;

use crate::config::{Config, LogFormat};

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("the log filter is not valid")]
    Filter(#[source] tracing_subscriber::filter::ParseError),
    #[error("cannot set the tracing subscriber")]
    Init(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Initializes the tracing subscriber from the configuration and routes panics through it.
///
/// The function uses the text format or the JSON format. The filter comes from `log_level`. The
/// panic hook keeps every log line in the chosen format (AC-NFR-11), where the default hook writes
/// plain text to stderr.
pub fn init(config: &Config) -> Result<(), TelemetryError> {
    let filter = EnvFilter::try_new(&config.log_level).map_err(TelemetryError::Filter)?;
    let result = match config.log_format {
        LogFormat::Text => tracing_subscriber::fmt().with_env_filter(filter).try_init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init(),
    };
    result.map_err(TelemetryError::Init)?;
    install_panic_hook();
    Ok(())
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), ToString::to_string);
        tracing::error!(
            %location,
            panic = crate::supervisor::panic_message(info.payload()),
            "a thread panicked"
        );
    }));
}

/// Initializes a text tracing subscriber for a short command such as `migrate`.
pub fn init_command() -> Result<(), TelemetryError> {
    let filter = EnvFilter::try_new("info").map_err(TelemetryError::Filter)?;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(TelemetryError::Init)
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn ac_nfr_11_a_panic_is_one_json_line_with_its_text_and_place() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();

        let previous = std::panic::take_hook();
        install_panic_hook();
        let caught = tracing::subscriber::with_default(subscriber, || {
            std::panic::catch_unwind(|| panic!("the pass broke"))
        });
        std::panic::set_hook(previous);
        assert!(caught.is_err());

        let output = String::from_utf8(captured.0.lock().expect("lock").clone()).expect("utf8");
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 1, "{output}");
        let line: serde_json::Value = serde_json::from_str(lines[0]).expect("a JSON line");
        assert_eq!(line["level"], "ERROR");
        assert_eq!(line["fields"]["panic"], "the pass broke");
        assert!(
            line["fields"]["location"]
                .as_str()
                .is_some_and(|location| location.contains("telemetry.rs"))
        );
    }
}
