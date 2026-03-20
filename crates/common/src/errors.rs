use thiserror::Error;

/// Top-level error taxonomy for OpenTrade.
///
/// Domain-specific errors use `thiserror` for structured handling.
/// Transient or operational errors bubble up via `anyhow` at boundaries.
#[derive(Error, Debug)]
pub enum OtError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Exchange error: {0}")]
    Exchange(#[from] ExchangeError),

    #[error("Risk limit breached: {0}")]
    RiskBreach(String),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Data error: {0}")]
    Data(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Model error: {0}")]
    Model(String),

    #[error("Kill switch triggered: {0}")]
    KillSwitch(String),

    #[error("Execution error: {0}")]
    Execution(String),
}

#[derive(Error, Debug)]
pub enum ExchangeError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Order rejected: {0}")]
    OrderRejected(String),

    #[error("Insufficient balance: {0}")]
    InsufficientBalance(String),

    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("Connection lost: {0}")]
    ConnectionLost(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Parse error: {0}")]
    Parse(String),
}

pub type OtResult<T> = Result<T, OtError>;
