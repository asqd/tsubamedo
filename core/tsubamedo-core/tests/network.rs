//! Proxies and client certificates against local servers: plain HTTP through an HTTP proxy,
//! HTTPS through a CONNECT tunnel to a server that wants a client certificate, SOCKS5, and
//! certificates in PEM, PKCS#12 and with an encrypted key (`tests/certs/make.sh`).
//!
//! `api.test` does not exist: only the proxies know where it is.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tsubamedo_core::http::{self, CertSpec, ClientCert, ProxyConfig, SendOptions};
use tsubamedo_core::prepare::prepare;
use tsubamedo_core::{Request, Vars};

fn certs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs")
}

/// HTTPS that wants a client certificate from the test CA, answering with its name.
async fn mtls_server() -> u16 {
    let ca = CertificateDer::from_pem_file(certs().join("ca.pem")).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone()).build().unwrap();
    let chain: Vec<CertificateDer> = CertificateDer::pem_file_iter(certs().join("server.pem")).unwrap().map(Result::unwrap).collect();
    let key = PrivateKeyDer::from_pem_file(certs().join("server.key")).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else { return };
                let mut buf = vec![0; 4096];
                let _ = tls.read(&mut buf).await;
                let body = if tls.get_ref().1.peer_certificates().is_some_and(|c| !c.is_empty()) { "hello client" } else { "who?" };
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = tls.write_all(reply.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    port
}

/// An HTTP proxy: tunnels `CONNECT api.test:443` to `tunnel_to`, answers plain HTTP itself
/// with what it got. With `login`, wants `Proxy-Authorization`.
async fn http_proxy(tunnel_to: u16, login: Option<&'static str>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && client.read(&mut byte).await.unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).to_string();
                log.lock().unwrap().push(head.lines().next().unwrap_or_default().to_string());
                if let Some(login) = login {
                    let want = format!("proxy-authorization: basic {}", base64_of(login)).to_ascii_lowercase();
                    if !head.to_ascii_lowercase().contains(&want) {
                        let _ = client.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n").await;
                        return;
                    }
                }
                if head.starts_with("CONNECT api.test:443 ") {
                    let mut server = TcpStream::connect(("127.0.0.1", tunnel_to)).await.unwrap();
                    client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await.unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                } else {
                    let body = head.replace("\r\n", "\n");
                    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = client.write_all(reply.as_bytes()).await;
                }
            });
        }
    });
    (port, seen)
}

/// A SOCKS5 proxy without login that sends every connection to `to`, remembering the names.
async fn socks_proxy(to: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (mut c, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::spawn(async move {
                let mut hello = [0u8; 2];
                c.read_exact(&mut hello).await.unwrap();
                let mut methods = vec![0u8; hello[1] as usize];
                c.read_exact(&mut methods).await.unwrap();
                c.write_all(&[5, 0]).await.unwrap();
                let mut req = [0u8; 5];
                c.read_exact(&mut req).await.unwrap();
                let mut name = vec![0u8; req[4] as usize + 2];
                c.read_exact(&mut name).await.unwrap();
                let port = u16::from_be_bytes([name[name.len() - 2], name[name.len() - 1]]);
                log.lock().unwrap().push(format!("{}:{port}", String::from_utf8_lossy(&name[..name.len() - 2])));
                let mut server = TcpStream::connect(("127.0.0.1", to)).await.unwrap();
                c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await.unwrap();
                let _ = tokio::io::copy_bidirectional(&mut c, &mut server).await;
            });
        }
    });
    (port, seen)
}

fn base64_of(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

async fn get(url: &str, options: &SendOptions) -> tsubamedo_core::Result<http::Response> {
    let request = Request { url: url.into(), ..Request::default() };
    http::send(&prepare(&request, &Vars::new(), Path::new(".")).unwrap(), options).await
}

fn body(r: &http::Response) -> String {
    String::from_utf8_lossy(r.body.in_memory().unwrap()).into_owned()
}

fn client_cert(cert: &str, key: &str, passphrase: &str) -> ClientCert {
    let spec = CertSpec { host: "api.test".into(), cert: cert.into(), key: key.into(), passphrase: passphrase.into() };
    ClientCert::load(&spec, &certs())
}

fn custom(port: u16, login: &str) -> ProxyConfig {
    ProxyConfig::Custom { url: format!("http://{login}127.0.0.1:{port}"), no_proxy: String::new() }
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_through_a_proxy_with_login() {
    let (port, seen) = http_proxy(0, Some("ivan:секрет")).await;
    let options = SendOptions { proxy: custom(port, "ivan:%D1%81%D0%B5%D0%BA%D1%80%D0%B5%D1%82@"), ..SendOptions::default() };
    let r = get("http://api.test/orders?id=1", &options).await.unwrap();
    assert!(body(&r).starts_with("GET http://api.test/orders?id=1 HTTP/1.1\n"), "{}", body(&r));
    assert_eq!(r.via.as_deref(), Some(format!("127.0.0.1:{port}").as_str()));
    assert_eq!(seen.lock().unwrap().len(), 1);

    let no_login = SendOptions { proxy: custom(port, ""), ..SendOptions::default() };
    assert_eq!(get("http://api.test/", &no_login).await.unwrap().status, 407, "plain HTTP: the proxy's own answer");
}

#[tokio::test(flavor = "multi_thread")]
async fn https_through_a_tunnel_with_a_client_certificate() {
    let server = mtls_server().await;
    let (port, seen) = http_proxy(server, None).await;
    let pem = SendOptions {
        proxy: custom(port, ""),
        verify_tls: false,
        client_certs: Arc::new(vec![client_cert("client.pem", "client.key", "")]),
        ..SendOptions::default()
    };
    let r = get("https://api.test/", &pem).await.unwrap();
    assert_eq!(body(&r), "hello client");
    assert_eq!(r.tls.as_ref().and_then(|t| t.client_cert.as_deref()), Some("client.pem"));
    assert_eq!(seen.lock().unwrap()[0], "CONNECT api.test:443 HTTP/1.1");

    for (cert, key) in [("client.p12", ""), ("client.pem", "client-encrypted.key")] {
        let options = SendOptions { client_certs: Arc::new(vec![client_cert(cert, key, "kot")]), ..pem.clone() };
        assert_eq!(body(&get("https://api.test/", &options).await.unwrap()), "hello client", "{cert} {key}");
    }

    // Without one the server breaks off the handshake; a wrong passphrase is said as such.
    let none = SendOptions { client_certs: Arc::new(Vec::new()), ..pem.clone() };
    assert!(matches!(get("https://api.test/", &none).await, Err(tsubamedo_core::Error::Tls { .. }) | Err(tsubamedo_core::Error::Http(_))));
    let wrong = SendOptions { client_certs: Arc::new(vec![client_cert("client.p12", "", "мяу")]), ..pem.clone() };
    assert_eq!(get("https://api.test/", &wrong).await.unwrap_err().to_string(), "TLS с api.test: клиентский сертификат: client.p12: неверный passphrase");

    let (guarded, _) = http_proxy(server, Some("a:b")).await;
    let refused = SendOptions { proxy: custom(guarded, ""), ..pem };
    assert_eq!(
        get("https://api.test/", &refused).await.unwrap_err().to_string(),
        format!("прокси 127.0.0.1:{guarded}: прокси требует вход (407): укажите логин и пароль — http://логин:пароль@прокси:порт")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn socks5_resolves_the_name_itself() {
    let (http_port, _) = http_proxy(0, None).await;
    let (port, seen) = socks_proxy(http_port).await;
    let options = SendOptions { proxy: ProxyConfig::Custom { url: format!("socks5://127.0.0.1:{port}"), no_proxy: String::new() }, ..SendOptions::default() };
    let r = get("http://api.test:8080/x", &options).await.unwrap();
    assert!(body(&r).starts_with("GET /x HTTP/1.1\n"), "through SOCKS the request is as to the server: {}", body(&r));
    assert_eq!(*seen.lock().unwrap(), ["api.test:8080"]);
    assert_eq!(r.via.as_deref(), Some(format!("socks5://127.0.0.1:{port}").as_str()));
}
