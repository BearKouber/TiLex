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
    /// HTTP 失败只留分类和状态码，**不带响应体和地址**：两者都可能回显 API key（design §2.5）。
    #[error("{}", http_message(*status, *kind))]
    Http { status: Option<u16>, kind: HttpKind },
    /// 服务不支持这对语言（旧版 "Language not supported"）。
    #[error("language not supported by this service")]
    LanguageUnsupported,
    /// 服务缺必填配置，值是字段名（`api_key`、`base_url`、`model` …），界面据此提示去哪填。
    #[error("service is missing {0}")]
    NotConfigured(&'static str),
    #[error("wordbook: {0}")]
    Db(#[from] rusqlite::Error),
}

/// 界面按这个显示人话（B2 起在 .slint 的 Strings 里翻译）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpKind {
    Timeout,
    /// DNS、拒绝连接、TLS 握手、代理连不上、地址写错。
    Connect,
    /// 4xx
    Client,
    /// 5xx 和其他非 2xx
    Server,
    /// 2xx 但内容不是约定的形状（含空结果）。
    Format,
}

fn http_message(status: Option<u16>, kind: HttpKind) -> String {
    match (kind, status) {
        (HttpKind::Timeout, _) => "request timed out".into(),
        (HttpKind::Connect, _) => "cannot connect to the server".into(),
        (HttpKind::Format, _) => "unexpected response from the server".into(),
        (_, Some(code)) => format!("server returned HTTP {code}"),
        (_, None) => "request failed".into(),
    }
}
