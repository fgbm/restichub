//! The tracing setup of the control plane.

use tracing_subscriber::EnvFilter;

use crate::config::{Config, LogFormat};

/// The error of the tracing setup.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// The log filter is not valid.
    #[error("the log filter is not valid")]
    Filter(#[source] tracing_subscriber::filter::ParseError),
    /// The tracing subscriber is already set.
    #[error("cannot set the tracing subscriber")]
    Init(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Initializes the tracing subscriber from the configuration.
///
/// The function uses the text format or the JSON format. The filter comes from `log_level`.
pub fn init(config: &Config) -> Result<(), TelemetryError> {
    let filter = EnvFilter::try_new(&config.log_level).map_err(TelemetryError::Filter)?;
    let result = match config.log_format {
        LogFormat::Text => tracing_subscriber::fmt().with_env_filter(filter).try_init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init(),
    };
    result.map_err(TelemetryError::Init)
}
