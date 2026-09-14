/// 全 crate 唯一的错误类型。只加真的用到的变体。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("UI: {0}")]
    Ui(#[from] slint::PlatformError),
    #[error("event loop: {0}")]
    EventLoop(#[from] slint::EventLoopError),
    #[error("platform: {0}")]
    Platform(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("config format: {0}")]
    Json(#[from] serde_json::Error),
    #[error("timed out")]
    Timeout,
    #[error("not supported on this platform yet")]
    Unsupported,
}
