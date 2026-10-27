//! Sending a prepared request.
//!
//! Every request opens its own connection (DNS → TCP → TLS → request), so each phase gets an
//! honest time, as in `curl -w`. HTTP/2 is used when the server offers it over TLS. Through a
//! proxy, DNS and connecting are to the proxy, and the tunnel counts as connecting.

pub(crate) mod body;
pub mod client_cert;
pub mod proxy;
mod tls;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::client::conn::{http1, http2};
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::{Serialize, Serializer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use url::{Host, Position, Url};

pub use body::ResponseBody;
pub use client_cert::{CertSpec, ClientCert};
pub use proxy::ProxyConfig;
use body::{Decoder, Sink};

use crate::cookies::{self, SharedJar};
use crate::error::chain;
use crate::prepare::Prepared;
use crate::{Error, Result};

pub const USER_AGENT: &str = concat!("TsubameDo/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct SendOptions {
    pub follow_redirects: bool,
    pub max_redirects: u32,
    /// The whole exchange, redirects included; `None` waits forever.
    pub timeout: Option<Duration>,
    pub connect_timeout: Duration,
    pub verify_tls: bool,
    /// Bodies longer than this (after decompression) go to a temporary file.
    pub spill_threshold: usize,
    pub spill_dir: PathBuf,
    pub decompress: bool,
    /// Cookies to send and to keep from the responses, on every hop of redirects too.
    pub cookies: Option<SharedJar>,
    pub proxy: ProxyConfig,
    /// Client certificates by host, for servers that ask for one.
    pub client_certs: Arc<Vec<ClientCert>>,
}

impl Default for SendOptions {
    fn default() -> Self {
        SendOptions {
            follow_redirects: true,
            max_redirects: 10,
            timeout: None,
            connect_timeout: Duration::from_secs(10),
            verify_tls: true,
            spill_threshold: 8 << 20,
            spill_dir: std::env::temp_dir().join("tsubamedo"),
            decompress: true,
            cookies: None,
            proxy: ProxyConfig::System,
            client_certs: Arc::new(Vec::new()),
        }
    }
}

impl SendOptions {
    /// Applies what the request file overrides.
    pub fn for_request(&self, prepared: &Prepared) -> SendOptions {
        let o = &prepared.options;
        SendOptions {
            follow_redirects: o.follow_redirects.unwrap_or(self.follow_redirects),
            timeout: o.timeout_ms.map(Duration::from_millis).or(self.timeout),
            verify_tls: o.verify_tls.unwrap_or(self.verify_tls),
            cookies: if o.cookies == Some(false) { None } else { self.cookies.clone() },
            ..self.clone()
        }
    }
}

/// Times of the last hop; `total` covers the whole exchange with redirects.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Timings {
    #[serde(serialize_with = "ms")]
    pub dns: Duration,
    #[serde(serialize_with = "ms")]
    pub connect: Duration,
    #[serde(serialize_with = "ms_opt")]
    pub tls: Option<Duration>,
    /// From the start of sending to the response headers: the server's thinking time.
    #[serde(serialize_with = "ms")]
    pub wait: Duration,
    #[serde(serialize_with = "ms")]
    pub download: Duration,
    #[serde(serialize_with = "ms")]
    pub total: Duration,
}

fn ms<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(d.as_secs_f64() * 1000.0)
}

fn ms_opt<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
    match d {
        Some(d) => ms(d, s),
        None => s.serialize_none(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsInfo {
    pub version: String,
    pub alpn: Option<String>,
    /// The file of the client certificate offered to the server.
    pub client_cert: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Redirect {
    pub status: u16,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    /// `HTTP/1.1` or `HTTP/2`.
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: ResponseBody,
    /// Bytes that came over the network, before decompression.
    pub wire_size: u64,
    pub timings: Timings,
    /// The final address after redirects.
    pub url: String,
    pub remote_addr: SocketAddr,
    pub tls: Option<TlsInfo>,
    pub redirects: Vec<Redirect>,
    /// The `Cookie` header that went out with the request when the jar added to it.
    pub sent_cookie: Option<String>,
    /// The proxy of the last hop.
    pub via: Option<String>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn is_json(&self) -> bool {
        self.header("content-type").is_some_and(|c| {
            let mime = c.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            mime == "application/json" || mime.ends_with("+json")
        })
    }
}

pub async fn send(request: &Prepared, options: &SendOptions) -> Result<Response> {
    let exchange = exchange(request, options);
    match options.timeout {
        None => exchange.await,
        Some(limit) => tokio::time::timeout(limit, exchange)
            .await
            .map_err(|_| Error::Timeout(limit.as_millis() as u64))?,
    }
}

async fn exchange(request: &Prepared, options: &SendOptions) -> Result<Response> {
    let start = Instant::now();
    let mut method = request.method.clone();
    let mut url = request.url.clone();
    let mut headers = request.headers.clone();
    let mut body = request.body.clone();
    let mut redirects = Vec::new();
    // The user's own cookies go only to the server the request was written for.
    let own_cookie = request.header("cookie").map(str::to_string);
    let origin = url.origin();
    let mut sent_cookie = None;
    loop {
        if let Some(jar) = &options.cookies {
            let from_jar = jar.lock().unwrap().header(&url);
            let own = own_cookie.as_deref().filter(|_| url.origin() == origin);
            let cookie = cookies::merge(own, from_jar.as_deref());
            if redirects.is_empty() && cookie != own_cookie {
                sent_cookie = cookie.clone();
            }
            set_header(&mut headers, "Cookie", cookie);
        }
        let hop = hop(&method, &url, &headers, body.clone(), options).await?;
        if let Some(jar) = &options.cookies {
            let mut jar = jar.lock().unwrap();
            for value in hop.response.headers().get_all(hyper::header::SET_COOKIE) {
                jar.store(&url, &String::from_utf8_lossy(value.as_bytes()));
            }
        }
        let status = hop.response.status().as_u16();
        let location = hop.response.headers().get(hyper::header::LOCATION).and_then(|l| l.to_str().ok());
        if let (true, 301 | 302 | 303 | 307 | 308, Some(location)) = (options.follow_redirects, status, location) {
            if redirects.len() as u32 >= options.max_redirects {
                return Err(Error::TooManyRedirects(options.max_redirects));
            }
            let next = url.join(location).map_err(|e| Error::Http(format!("неверный Location «{location}»: {e}")))?;
            redirects.push(Redirect { status, url: url.to_string() });
            // What browsers do: 303 always, and 301/302 after POST, repeat as GET without a body.
            if (status == 303 && method != "HEAD") || (matches!(status, 301 | 302) && method == "POST") {
                method = "GET".into();
                body = Bytes::new();
                headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-type") && !n.eq_ignore_ascii_case("content-length"));
            }
            // Credentials stay with the server they were meant for.
            if next.origin() != url.origin() {
                headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization") && !n.eq_ignore_ascii_case("cookie"));
            }
            url = next;
            continue;
        }
        let mut response = read(hop, url, redirects, start, options).await?;
        response.sent_cookie = sent_cookie;
        return Ok(response);
    }
}

/// Puts the header in place of the first one with that name (or at the end) and drops the
/// others; `None` drops them all.
fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: Option<String>) {
    let at = headers.iter().position(|(n, _)| n.eq_ignore_ascii_case(name));
    headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    if let Some(value) = value {
        headers.insert(at.unwrap_or(headers.len()).min(headers.len()), (name.to_string(), value));
    }
}

struct Hop {
    response: hyper::Response<Incoming>,
    // Kept until the body is read: the connection lives as long as its sender.
    _sender: Sender,
    remote_addr: SocketAddr,
    tls: Option<TlsInfo>,
    timings: Timings,
    headers_at: Instant,
    via: Option<String>,
}

enum Sender {
    H1(http1::SendRequest<Full<Bytes>>),
    H2(http2::SendRequest<Full<Bytes>>),
}

async fn hop(method: &str, url: &Url, headers: &[(String, String)], body: Bytes, options: &SendOptions) -> Result<Hop> {
    let host = url.host().ok_or_else(|| Error::BadUrl { url: url.to_string(), reason: "нет имени сервера".into() })?;
    let host_name = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let https = url.scheme() == "https";
    let proxy = options.proxy.for_url(url).map_err(|reason| Error::Proxy { proxy: "из настроек".into(), reason })?;
    let proxy_error = |p: &proxy::Proxy, reason: String| Error::Proxy { proxy: p.label(), reason };
    let mut timings = Timings::default();

    // Where the connection goes: the server, or the proxy.
    let (dial, dial_port) = match &proxy {
        Some(p) => {
            let name = if p.host.contains(':') { format!("[{}]", p.host) } else { p.host.clone() };
            (Host::parse(&name).map_err(|e| proxy_error(p, e.to_string()))?, p.port)
        }
        None => (host.to_owned(), port),
    };
    let t = Instant::now();
    let addrs: Vec<SocketAddr> = match &dial {
        Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(*ip), dial_port)],
        Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(*ip), dial_port)],
        Host::Domain(domain) => tokio::net::lookup_host((domain.as_str(), dial_port))
            .await
            .map_err(|e| match &proxy {
                Some(p) => proxy_error(p, format!("адрес не найден: {e}")),
                None => Error::Dns { host: domain.to_string(), reason: e.to_string() },
            })?
            .collect(),
    };
    timings.dns = t.elapsed();

    let t = Instant::now();
    let (mut tcp, remote_addr) = connect(&addrs, options.connect_timeout).await.map_err(|reason| match &proxy {
        Some(p) => proxy_error(p, format!("не подключиться: {reason}")),
        None => Error::Connect { target: format!("{host_name}:{port}"), reason },
    })?;
    // Plain HTTP through an HTTP proxy: the whole address goes in the request line.
    let mut absolute = false;
    if let Some(p) = &proxy {
        let opened = if p.socks {
            proxy::socks5(&mut tcp, &host_name, port, p).await
        } else if https {
            proxy::tunnel(&mut tcp, &format!("{host_name}:{port}"), p).await
        } else {
            absolute = true;
            Ok(())
        };
        opened.map_err(|reason| proxy_error(p, reason))?;
    }
    timings.connect = t.elapsed();

    let (sender, tls) = if https {
        let t = Instant::now();
        let server_name = match &host {
            Host::Domain(d) => rustls::pki_types::ServerName::try_from(d.to_string())
                .map_err(|e| Error::Tls { host: host_name.clone(), reason: e.to_string() })?,
            Host::Ipv4(ip) => rustls::pki_types::ServerName::IpAddress(IpAddr::V4(*ip).into()),
            Host::Ipv6(ip) => rustls::pki_types::ServerName::IpAddress(IpAddr::V6(*ip).into()),
        };
        let cert = client_cert::find(&options.client_certs, &host_name, port);
        let identity = match cert.map(|c| c.identity.clone()) {
            Some(Ok(identity)) => Some(identity),
            Some(Err(reason)) => return Err(Error::Tls { host: host_name.clone(), reason: format!("клиентский сертификат: {reason}") }),
            None => None,
        };
        let config = tls::config(options.verify_tls, identity.as_deref()).map_err(|reason| Error::Tls { host: host_name.clone(), reason })?;
        let stream = tokio_rustls::TlsConnector::from(config)
            .connect(server_name, tcp)
            .await
            .map_err(|e| {
                let mut reason = chain(&e);
                // TLS 1.3 says so; TLS 1.2 servers often just break off.
                if identity.is_none() && (reason.contains("CertificateRequired") || reason.contains("HandshakeFailure")) {
                    reason.push_str(" — возможно, сервер ждёт клиентский сертификат (Настройки → Клиентские сертификаты, в CLI — --cert)");
                }
                Error::Tls { host: host_name.clone(), reason }
            })?;
        timings.tls = Some(t.elapsed());
        let session = stream.get_ref().1;
        let alpn = session.alpn_protocol().map(|p| String::from_utf8_lossy(p).into_owned());
        let version = match session.protocol_version() {
            Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3".to_string(),
            Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2".to_string(),
            other => format!("{other:?}"),
        };
        let h2 = alpn.as_deref() == Some("h2");
        let client_cert = identity.map(|i| i.label.clone());
        (handshake(stream, h2).await?, Some(TlsInfo { version, alpn, client_cert }))
    } else {
        (handshake(tcp, false).await?, None)
    };

    let h2 = matches!(sender, Sender::H2(_));
    let target = if h2 || absolute { &url[..Position::AfterQuery] } else { &url[Position::BeforePath..Position::AfterQuery] };
    let http_method = http::Method::from_bytes(method.as_bytes()).map_err(|_| Error::Invalid(format!("неверный метод «{method}»")))?;
    let mut builder = hyper::Request::builder().method(http_method).uri(target);
    let has = |name: &str| headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name));
    if !h2 && !has("host") {
        let host_header = match url.port() {
            Some(port) => format!("{host_name}:{port}"),
            None => host_name.clone(),
        };
        builder = builder.header(hyper::header::HOST, host_header);
    }
    for (name, value) in headers {
        // HTTP/2 forbids connection-level headers; the authority comes from the URI.
        if h2 && ["host", "connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade"].iter().any(|h| name.eq_ignore_ascii_case(h)) {
            continue;
        }
        let name = http::HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| Error::Invalid(format!("неверное имя заголовка «{name}»")))?;
        let value = http::HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| Error::Invalid(format!("неверное значение заголовка {name}")))?;
        builder = builder.header(name, value);
    }
    if absolute
        && !has("proxy-authorization")
        && let Some(auth) = proxy.as_ref().and_then(proxy::Proxy::authorization)
    {
        builder = builder.header(hyper::header::PROXY_AUTHORIZATION, auth);
    }
    for (name, value) in [("user-agent", USER_AGENT), ("accept", "*/*"), ("accept-encoding", "gzip, deflate, br")] {
        if !has(name) && (name != "accept-encoding" || options.decompress) {
            builder = builder.header(name, value);
        }
    }
    let request = builder.body(Full::new(body)).map_err(|e| Error::Invalid(e.to_string()))?;

    let t = Instant::now();
    let mut sender = sender;
    let response = match &mut sender {
        Sender::H1(s) => s.send_request(request).await,
        Sender::H2(s) => s.send_request(request).await,
    }
    .map_err(|e| Error::Http(chain(&e)))?;
    timings.wait = t.elapsed();
    let via = proxy.as_ref().map(proxy::Proxy::label);
    Ok(Hop { response, _sender: sender, remote_addr, tls, timings, headers_at: Instant::now(), via })
}

async fn connect(addrs: &[SocketAddr], limit: Duration) -> Result<(TcpStream, SocketAddr), String> {
    let mut last = String::from("сервер не найден");
    for addr in addrs {
        match tokio::time::timeout(limit, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok((stream, *addr));
            }
            Ok(Err(e)) => last = e.to_string(),
            Err(_) => last = format!("нет ответа за {} с", limit.as_secs()),
        }
    }
    Err(last)
}

async fn handshake<S>(io: S, h2: bool) -> Result<Sender>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(io);
    if h2 {
        let (sender, conn) = http2::handshake(TokioExecutor::new(), io).await.map_err(|e| Error::Http(chain(&e)))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(Sender::H2(sender))
    } else {
        let (sender, conn) = http1::handshake(io).await.map_err(|e| Error::Http(chain(&e)))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(Sender::H1(sender))
    }
}

async fn read(hop: Hop, url: Url, redirects: Vec<Redirect>, start: Instant, options: &SendOptions) -> Result<Response> {
    let Hop { response, _sender, remote_addr, tls, mut timings, headers_at, via } = hop;
    let (parts, mut incoming) = response.into_parts();
    let encoding = parts
        .headers
        .get(hyper::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let decode_as = if options.decompress { encoding.as_str() } else { "" };
    let mut decoder = Decoder::new(decode_as, Sink::new(options.spill_threshold, options.spill_dir.clone()));
    let body_error = |reason: String| Error::Body { encoding: if decode_as.is_empty() { "без сжатия".into() } else { decode_as.to_string() }, reason };
    let mut wire_size = 0u64;
    while let Some(frame) = incoming.frame().await {
        let frame = frame.map_err(|e| Error::Http(chain(&e)))?;
        if let Ok(data) = frame.into_data() {
            wire_size += data.len() as u64;
            decoder.write_all(&data).map_err(|e| body_error(e.to_string()))?;
        }
    }
    let body = decoder.finish().map_err(|e| body_error(e.to_string()))?;
    timings.download = headers_at.elapsed();
    timings.total = start.elapsed();

    let reason = parts
        .extensions
        .get::<hyper::ext::ReasonPhrase>()
        .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned())
        .or_else(|| parts.status.canonical_reason().map(str::to_string))
        .unwrap_or_default();
    let version = match parts.version {
        http::Version::HTTP_10 => "HTTP/1.0",
        http::Version::HTTP_2 => "HTTP/2",
        _ => "HTTP/1.1",
    };
    let headers = parts
        .headers
        .iter()
        .map(|(n, v)| (n.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    Ok(Response {
        status: parts.status.as_u16(),
        reason,
        version: version.into(),
        headers,
        body,
        wire_size,
        timings,
        url: url.to_string(),
        remote_addr,
        tls,
        redirects,
        sent_cookie: None,
        via,
    })
}
