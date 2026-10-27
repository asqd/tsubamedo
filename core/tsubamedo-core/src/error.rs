use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors are shown to the user as is, so their text is in Russian.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("не заданы переменные: {}", .0.join(", "))]
    UndefinedVars(Vec<String>),
    #[error("переменная {0} ссылается сама на себя")]
    VarCycle(String),
    #[error("неверный адрес «{url}»: {reason}")]
    BadUrl { url: String, reason: String },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Script(String),
    #[error("не удалось найти адрес {host}: {reason}")]
    Dns { host: String, reason: String },
    #[error("не удалось подключиться к {target}: {reason}")]
    Connect { target: String, reason: String },
    #[error("TLS с {host}: {reason}")]
    Tls { host: String, reason: String },
    #[error("ответ не пришёл за {0} мс")]
    Timeout(u64),
    #[error("ошибка HTTP: {0}")]
    Http(String),
    #[error("больше {0} перенаправлений подряд")]
    TooManyRedirects(u32),
    #[error("не удалось прочитать тело ответа ({encoding}): {reason}")]
    Body { encoding: String, reason: String },
    #[error("{}: {source}", .path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{}: {message}", .path.display())]
    Parse { path: PathBuf, message: String },
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }
}

/// The error with all its causes: hyper and rustls keep the useful part in `source()`.
pub(crate) fn chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}
