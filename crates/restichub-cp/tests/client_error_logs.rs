//! A client input error is a 4xx answer. The log holds it at the debug level only, and never as an
//! internal error.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::io;
use std::sync::{Arc, Mutex, Once, OnceLock};

use axum::http::StatusCode;
use serde_json::json;

use queue_support::{Env, test_queue};

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for BufferWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| io::Error::other("the log buffer is poisoned"))?;
        guard.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
    type Writer = BufferWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BufferWriter(self.0.clone())
    }
}

fn install_log_capture() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(BufferWriter(logs().clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("cannot install the subscriber");
    });
}

fn captured_logs() -> String {
    let guard = logs().lock().unwrap_or_else(|error| error.into_inner());
    String::from_utf8_lossy(&guard).into_owned()
}

#[tokio::test]
async fn a_malformed_client_input_is_not_an_internal_error() {
    let Some(db) = common::test_db().await else {
        return;
    };
    install_log_capture();
    let env = Env::start(db, test_queue()).await;

    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/agents/not-an-id/replace",
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = env
        .call(
            &env.admin,
            "GET",
            "/api/v1/audit?actor_id=00000000-0000-0000-0000-000000000000",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let logs = captured_logs();
    assert!(
        logs.contains("rejected request input"),
        "the capture must hold the rejection: {logs}"
    );
    for line in logs
        .lines()
        .filter(|line| line.contains("rejected request input"))
    {
        assert!(
            !line.contains("ERROR"),
            "a client error is logged as ERROR: {line}"
        );
    }
    assert!(
        !logs.contains("internal error"),
        "a client error is logged as internal: {logs}"
    );
    env.db.cleanup().await;
}
