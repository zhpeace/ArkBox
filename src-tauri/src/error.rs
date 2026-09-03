use serde::Serialize;

/// 统一错误类型。命令统一返回 `Result<T, String>`，便于前端处理。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("压缩/解压错误: {0}")]
    Archive(String),

    #[error("不支持的格式: {0}")]
    UnsupportedFormat(String),

    #[error("密码错误或损坏的压缩包: {0}")]
    BadPassword(String),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

/// 命令返回的错误统一转成字符串。
pub type AppResult<T> = Result<T, AppError>;

/// 前端可反序列化的错误结构。
#[derive(Serialize)]
pub struct ErrorPayload {
    pub message: String,
}

pub fn to_string_err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
