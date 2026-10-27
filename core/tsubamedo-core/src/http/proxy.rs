//! Proxies: the system's (environment variables as curl reads them, then the macOS network
//! settings), one the user gives, or none. An HTTP proxy tunnels HTTPS with `CONNECT` and takes
//! plain HTTP in absolute form; SOCKS5 resolves names itself. This machine is always reached
//! directly: a local server behind a corporate proxy is never what anyone wants.

use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::{Host, Url};

/// How long the system's settings are trusted before they are read again (a VPN goes up).
const SYSTEM_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum ProxyConfig {
    /// `HTTPS_PROXY` and the like, else the system's network settings.
    #[default]
    System,
    None,
    Custom {
        /// `proxy.corp:3128`, `http://login:password@proxy:3128`, `socks5://proxy:1080`.
        url: String,
        /// Hosts reached directly: `.corp.local, 10.0.0.0/8, internal:8080`.
        #[serde(default)]
        no_proxy: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    pub socks: bool,
    pub host: String,
    pub port: u16,
    pub auth: Option<(String, String)>,
}

impl Proxy {
    pub fn parse(text: &str) -> Result<Proxy, String> {
        let text = text.trim();
        let with_scheme = if text.contains("://") { text.to_string() } else { format!("http://{text}") };
        let url = Url::parse(&with_scheme).map_err(|e| format!("адрес прокси «{text}»: {e}"))?;
        let socks = match url.scheme() {
            "http" => false,
            "socks5" | "socks5h" | "socks" => true,
            "https" => return Err(format!("прокси «{text}»: TLS до самого прокси (https://) не поддерживается, нужен http:// или socks5://")),
            other => return Err(format!("прокси «{text}»: схема {other} не поддерживается, нужен http:// или socks5://")),
        };
        let host = url.host_str().filter(|h| !h.is_empty()).ok_or_else(|| format!("прокси «{text}»: нет имени сервера"))?;
        let decode = |s: &str| percent_decode(s);
        let auth = (!url.username().is_empty()).then(|| (decode(url.username()), decode(url.password().unwrap_or_default())));
        Ok(Proxy {
            socks,
            host: host.trim_start_matches('[').trim_end_matches(']').to_string(),
            port: url.port().unwrap_or(if socks { 1080 } else { 80 }),
            auth,
        })
    }

    /// For the window and errors, without the password.
    pub fn label(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        format!("{}{host}:{}", if self.socks { "socks5://" } else { "" }, self.port)
    }

    fn basic(&self) -> Option<String> {
        self.auth.as_ref().map(|(u, p)| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"))))
    }

    /// `Proxy-Authorization` for plain HTTP sent through it.
    pub(crate) fn authorization(&self) -> Option<String> {
        if self.socks { None } else { self.basic() }
    }
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
}

impl ProxyConfig {
    /// The proxy for this address, if it goes through one.
    pub fn for_url(&self, url: &Url) -> Result<Option<Proxy>, String> {
        if is_loopback(url) {
            return Ok(None);
        }
        match self {
            ProxyConfig::None => Ok(None),
            ProxyConfig::Custom { url: proxy, no_proxy } => {
                if proxy.trim().is_empty() || bypassed(url, no_proxy.split(','), false) {
                    return Ok(None);
                }
                Proxy::parse(proxy).map(Some)
            }
            ProxyConfig::System => system().pick(url),
        }
    }
}

/// What the system says, for the settings window too.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemProxy {
    /// «переменные окружения», «настройки macOS», or empty when there is none.
    pub source: String,
    pub http: Option<String>,
    pub https: Option<String>,
    pub socks: Option<String>,
    pub bypass: Vec<String>,
    /// Names without dots go directly (macOS «Исключить простые имена хостов»).
    pub simple_direct: bool,
    /// A PAC script is set: it is not run, requests go as the rest says.
    pub pac: Option<String>,
}

impl SystemProxy {
    fn pick(&self, url: &Url) -> Result<Option<Proxy>, String> {
        if bypassed(url, self.bypass.iter().map(String::as_str), self.simple_direct) {
            return Ok(None);
        }
        let chosen = if url.scheme() == "https" { self.https.as_ref() } else { self.http.as_ref() };
        match chosen.or(self.socks.as_ref()) {
            Some(p) => Proxy::parse(p).map(Some),
            None => Ok(None),
        }
    }
}

/// The system's proxies, read again every few seconds.
pub fn system() -> SystemProxy {
    static CACHE: Mutex<Option<(Instant, SystemProxy)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, found)) = cache.as_ref()
        && at.elapsed() < SYSTEM_TTL
    {
        return found.clone();
    }
    let found = from_env().unwrap_or_else(from_os);
    *cache = Some((Instant::now(), found.clone()));
    found
}

fn from_env() -> Option<SystemProxy> {
    let var = |names: &[&str]| names.iter().find_map(|n| std::env::var(n).ok().filter(|v| !v.trim().is_empty()));
    let all = var(&["ALL_PROXY", "all_proxy"]);
    let https = var(&["HTTPS_PROXY", "https_proxy"]);
    // Lowercase first, as curl: `HTTP_PROXY` can be set by a request header in CGI.
    let http = var(&["http_proxy", "HTTP_PROXY"]);
    if all.is_none() && https.is_none() && http.is_none() {
        return None;
    }
    let bypass = var(&["NO_PROXY", "no_proxy"]).map(|n| n.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let socks = all.as_ref().filter(|a| a.starts_with("socks")).cloned();
    let all = all.filter(|a| !a.starts_with("socks"));
    Some(SystemProxy {
        source: "переменные окружения".into(),
        http: http.or(all.clone()),
        https: https.or(all),
        socks,
        bypass,
        simple_direct: false,
        pac: None,
    })
}

#[cfg(target_os = "macos")]
fn from_os() -> SystemProxy {
    let Ok(out) = std::process::Command::new("/usr/sbin/scutil").arg("--proxy").output() else { return SystemProxy::default() };
    parse_scutil(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(target_os = "macos"))]
fn from_os() -> SystemProxy {
    SystemProxy::default()
}

/// `scutil --proxy`: the settings of the network in use, as macOS applies them.
fn parse_scutil(text: &str) -> SystemProxy {
    let mut fields = std::collections::HashMap::new();
    let mut bypass = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("ExceptionsList") {
            in_exceptions = true;
            continue;
        }
        if in_exceptions {
            if line == "}" {
                in_exceptions = false;
            } else if let Some((_, value)) = line.split_once(" : ") {
                bypass.push(value.trim().to_string());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(" : ") {
            fields.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let on = |key: &str| fields.get(key).map(String::as_str) == Some("1");
    let proxy = |kind: &str, scheme: &str| {
        let host = fields.get(&format!("{kind}Proxy"))?;
        let port = fields.get(&format!("{kind}Port")).map(String::as_str).unwrap_or(if kind == "SOCKS" { "1080" } else { "80" });
        on(&format!("{kind}Enable")).then(|| format!("{scheme}{host}:{port}"))
    };
    let found = SystemProxy {
        source: String::new(),
        http: proxy("HTTP", ""),
        https: proxy("HTTPS", ""),
        socks: proxy("SOCKS", "socks5://"),
        bypass,
        simple_direct: on("ExcludeSimpleHostnames"),
        pac: on("ProxyAutoConfigEnable").then(|| fields.get("ProxyAutoConfigURLString").cloned().unwrap_or_default()),
    };
    let any = found.http.is_some() || found.https.is_some() || found.socks.is_some() || found.pac.is_some();
    SystemProxy { source: if any { "настройки macOS".into() } else { String::new() }, ..found }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Whether the host is in the list: `*`, a domain with its subdomains (`corp.local`,
/// `.corp.local`, `*.corp.local`), an address or network (`10.0.0.0/8`, macOS's `169.254/16`),
/// any of them with `:port`.
fn bypassed<'a>(url: &Url, list: impl Iterator<Item = &'a str>, simple_direct: bool) -> bool {
    let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let port = url.port_or_known_default();
    if simple_direct && matches!(url.host(), Some(Host::Domain(_))) && !host.contains('.') {
        return true;
    }
    list.map(str::trim).filter(|e| !e.is_empty()).any(|entry| {
        if entry == "*" {
            return true;
        }
        if entry.eq_ignore_ascii_case("<local>") {
            return !host.contains('.');
        }
        let (name, entry_port) = match entry.rsplit_once(':') {
            Some((n, p)) if !n.contains(':') && p.parse::<u16>().is_ok() => (n, p.parse::<u16>().ok()),
            _ => (entry, None),
        };
        if entry_port.is_some() && entry_port != port {
            return false;
        }
        let name = name.trim_start_matches('*').trim_start_matches('.').to_ascii_lowercase();
        if let Some((net, bits)) = name.split_once('/') {
            return in_network(&host, net, bits);
        }
        host == name || host.ends_with(&format!(".{name}"))
    })
}

fn in_network(host: &str, net: &str, bits: &str) -> bool {
    let (Ok(ip), Ok(bits)) = (host.parse::<Ipv4Addr>(), bits.parse::<u32>()) else { return false };
    // macOS writes `169.254/16`: the missing parts are zeros.
    let mut parts: Vec<u8> = net.split('.').filter_map(|p| p.parse().ok()).collect();
    parts.resize(4, 0);
    let net = u32::from_be_bytes([parts[0], parts[1], parts[2], parts[3]]);
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits.min(32)) };
    u32::from(ip) & mask == net & mask
}

/// Asks an HTTP proxy for a tunnel to `authority` (`host:port`).
pub(crate) async fn tunnel(stream: &mut TcpStream, authority: &str, proxy: &Proxy) -> Result<(), String> {
    let mut head = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: {}\r\n", super::USER_AGENT);
    if let Some(auth) = proxy.basic() {
        head.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    // Byte by byte: nothing after the head may be taken from the tunnel.
    while !answer.ends_with(b"\r\n\r\n") {
        if answer.len() > 16 << 10 || stream.read(&mut byte).await.map_err(|e| e.to_string())? == 0 {
            return Err("прокси закрыл соединение, не ответив на CONNECT".into());
        }
        answer.push(byte[0]);
    }
    let answer = String::from_utf8_lossy(&answer);
    let status_line = answer.lines().next().unwrap_or_default();
    let status: u16 = status_line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    match status {
        200..=299 => Ok(()),
        407 if proxy.auth.is_none() => Err("прокси требует вход (407): укажите логин и пароль — http://логин:пароль@прокси:порт".into()),
        407 => Err("прокси не принял логин и пароль (407)".into()),
        _ => Err(format!("прокси не открыл туннель к {authority}: {}", status_line.trim())),
    }
}

/// SOCKS5 (RFC 1928, login and password per RFC 1929); the proxy resolves the name.
pub(crate) async fn socks5(stream: &mut TcpStream, host: &str, port: u16, proxy: &Proxy) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let methods: &[u8] = if proxy.auth.is_some() { &[5, 2, 0, 2] } else { &[5, 1, 0] };
    stream.write_all(methods).await.map_err(io)?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).await.map_err(io)?;
    match (reply, &proxy.auth) {
        ([5, 0], _) => {}
        ([5, 2], Some((user, pass))) => {
            let mut hello = vec![1, user.len().min(255) as u8];
            hello.extend_from_slice(&user.as_bytes()[..user.len().min(255)]);
            hello.push(pass.len().min(255) as u8);
            hello.extend_from_slice(&pass.as_bytes()[..pass.len().min(255)]);
            stream.write_all(&hello).await.map_err(io)?;
            let mut ok = [0u8; 2];
            stream.read_exact(&mut ok).await.map_err(io)?;
            if ok[1] != 0 {
                return Err("SOCKS-прокси не принял логин и пароль".into());
            }
        }
        ([5, 0xff], _) | ([5, 2], None) => return Err("SOCKS-прокси требует вход: socks5://логин:пароль@прокси:порт".into()),
        _ => return Err("это не SOCKS5-прокси".into()),
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.extend_from_slice(&[3, host.len().min(255) as u8]);
            request.extend_from_slice(&host.as_bytes()[..host.len().min(255)]);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.map_err(io)?;
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(io)?;
    if head[1] != 0 {
        let why = match head[1] {
            2 => "правила прокси не разрешают",
            3 => "сеть недоступна",
            4 => "сервер недоступен",
            5 => "в соединении отказано",
            6 => "истёк срок",
            _ => "ошибка",
        };
        return Err(format!("SOCKS-прокси не подключился к {host}:{port}: {why}"));
    }
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(io)?;
            len[0] as usize + 2
        }
        _ => return Err("непонятный ответ SOCKS-прокси".into()),
    };
    let mut skip = vec![0u8; rest];
    stream.read_exact(&mut skip).await.map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn proxy_addresses() {
        assert_eq!(Proxy::parse("proxy.corp:3128").unwrap(), Proxy { socks: false, host: "proxy.corp".into(), port: 3128, auth: None });
        let p = Proxy::parse("http://ivan%40corp:p%3Ass@10.0.0.1:8080").unwrap();
        assert_eq!(p.auth, Some(("ivan@corp".into(), "p:ss".into())));
        assert_eq!(p.label(), "10.0.0.1:8080");
        assert!(Proxy::parse("socks5h://gw").unwrap().socks);
        assert!(Proxy::parse("https://proxy:443").is_err());
    }

    #[test]
    fn bypass_lists() {
        let list = ["*.corp.local", "10.0.0.0/8", "169.254/16", "internal:8080", ""];
        let by = |u: &str| bypassed(&url(u), list.iter().copied(), false);
        assert!(by("https://api.corp.local/x") && by("https://corp.local") && !by("https://notcorp.local"));
        assert!(by("http://10.1.2.3/") && by("http://169.254.1.1/") && !by("http://11.0.0.1/"));
        assert!(by("http://internal:8080/") && !by("http://internal/"));
        assert!(bypassed(&url("http://intranet/"), std::iter::empty(), true));

        let custom = ProxyConfig::Custom { url: "proxy:3128".into(), no_proxy: ".test".into() };
        assert_eq!(custom.for_url(&url("http://localhost:8080/")), Ok(None), "this machine directly");
        assert_eq!(custom.for_url(&url("https://api.test/")), Ok(None));
        assert_eq!(custom.for_url(&url("https://example.com/")).unwrap().unwrap().port, 3128);
        assert_eq!(ProxyConfig::None.for_url(&url("https://example.com/")), Ok(None));
    }

    #[test]
    fn macos_settings() {
        let text = "<dictionary> {\n  ExceptionsList : <array> {\n    0 : *.local\n    1 : 169.254/16\n  }\n  ExcludeSimpleHostnames : 1\n  FTPPassive : 1\n  HTTPEnable : 1\n  HTTPPort : 3128\n  HTTPProxy : proxy.corp\n  HTTPSEnable : 1\n  HTTPSPort : 3129\n  HTTPSProxy : proxy.corp\n  ProxyAutoConfigEnable : 0\n  SOCKSEnable : 0\n}\n";
        let s = parse_scutil(text);
        assert_eq!((s.http.as_deref(), s.https.as_deref(), s.socks.as_deref()), (Some("proxy.corp:3128"), Some("proxy.corp:3129"), None));
        assert_eq!(s.bypass, ["*.local", "169.254/16"]);
        assert_eq!(s.source, "настройки macOS");
        assert_eq!(s.pick(&url("https://api.example.com/")).unwrap().unwrap().port, 3129);
        assert_eq!(s.pick(&url("http://printer.local/")), Ok(None));
        assert_eq!(s.pick(&url("http://intranet/")), Ok(None));
        assert_eq!(parse_scutil("<dictionary> {\n  HTTPEnable : 0\n}\n").source, "");
    }
}
