use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors are shown to the user as is, in the language of the process (see [`crate::lang`]).
#[derive(Debug)]
pub enum Error {
    UndefinedVars(Vec<String>),
    VarCycle(String),
    BadUrl { url: String, reason: String },
    Invalid(String),
    Script(String),
    OAuth(String),
    Dns { host: String, reason: String },
    Connect { target: String, reason: String },
    Proxy { proxy: String, reason: String },
    Tls { host: String, reason: String },
    Timeout(u64),
    Http(String),
    TooManyRedirects(u32),
    Body { encoding: String, reason: String },
    Io { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, message: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Error::UndefinedVars(names) => tr!("не заданы переменные: {}", "variables not set: {}", names.join(", ")),
            Error::VarCycle(name) => tr!("переменная {name} ссылается сама на себя", "variable {name} refers to itself"),
            Error::BadUrl { url, reason } => tr!("неверный адрес «{url}»: {reason}", "invalid address \"{url}\": {reason}"),
            Error::Invalid(text) | Error::Script(text) => text.clone(),
            Error::OAuth(text) => format!("OAuth 2.0: {text}"),
            Error::Dns { host, reason } => tr!("не удалось найти адрес {host}: {reason}", "could not resolve {host}: {reason}"),
            Error::Connect { target, reason } => tr!("не удалось подключиться к {target}: {reason}", "could not connect to {target}: {reason}"),
            Error::Proxy { proxy, reason } => tr!("прокси {proxy}: {reason}", "proxy {proxy}: {reason}"),
            Error::Tls { host, reason } => tr!("TLS с {host}: {reason}", "TLS with {host}: {reason}"),
            Error::Timeout(ms) => tr!("ответ не пришёл за {ms} мс", "no response in {ms} ms"),
            Error::Http(text) => tr!("ошибка HTTP: {text}", "HTTP error: {text}"),
            Error::TooManyRedirects(n) => tr!("больше {n} перенаправлений подряд", "more than {n} redirects in a row"),
            Error::Body { encoding, reason } => tr!("не удалось прочитать тело ответа ({encoding}): {reason}", "could not read the response body ({encoding}): {reason}"),
            Error::Io { path, source } => format!("{}: {source}", path.display()),
            Error::Parse { path, message } => format!("{}: {message}", path.display()),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
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
