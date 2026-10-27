//! Client certificates (mutual TLS), chosen by host:
//!
//! ```toml
//! # tsubamedo.toml
//! [[certificates]]
//! host = "api.bank.test"         # also "*.bank.test", "host:8443", "*"
//! cert = "certs/client.pem"      # certificate chain; the key may be in the same file
//! key = "certs/client.key"
//!
//! [[certificates]]
//! host = "*.partner.test"
//! cert = "~/certs/partner.p12"   # PKCS#12 (.p12, .pfx), as Keychain exports it
//! passphrase = "{{cert_password}}"
//! ```
//!
//! PEM is read here. PKCS#12 and keys under a passphrase go through the system's `openssl`
//! (macOS has it): their ciphers would cost the app dozens of crates.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// A certificate as written in the collection, `{{variables}}` already substituted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CertSpec {
    pub host: String,
    pub cert: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub passphrase: String,
}

#[derive(Debug)]
pub struct Identity {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    /// The file, for the window: which certificate went out.
    pub label: String,
}

/// A host's certificate, or why it could not be read: the error matters only when a request
/// goes to that host.
#[derive(Debug, Clone)]
pub struct ClientCert {
    pub host: String,
    pub identity: Result<Arc<Identity>, String>,
}

impl ClientCert {
    pub fn load(spec: &CertSpec, base: &Path) -> ClientCert {
        ClientCert { host: spec.host.trim().to_string(), identity: cached(spec, base) }
    }
}

/// The first certificate whose host matches.
pub fn find<'a>(certs: &'a [ClientCert], host: &str, port: u16) -> Option<&'a ClientCert> {
    certs.iter().find(|c| matches(&c.host, host, port))
}

/// `api.test`, `api.test:8443`, `*.test` (subdomains, not `test` itself), `*`.
fn matches(pattern: &str, host: &str, port: u16) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let (name, want_port) = match pattern.rsplit_once(':') {
        Some((n, p)) if !n.contains(':') => match p.parse::<u16>() {
            Ok(p) => (n.to_string(), Some(p)),
            Err(_) => (pattern.clone(), None),
        },
        _ => (pattern.clone(), None),
    };
    if want_port.is_some_and(|p| p != port) {
        return false;
    }
    match name.strip_prefix("*.") {
        _ if name == "*" => true,
        Some(domain) => host.ends_with(&format!(".{domain}")),
        None => host == name,
    }
}

fn resolve(path: &str, base: &Path) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => base.join(path),
    }
}

/// Read once while the files stay the same: a PKCS#12 file costs a run of `openssl`, and the
/// session reads the collection's certificates for every request.
fn cached(spec: &CertSpec, base: &Path) -> Result<Arc<Identity>, String> {
    static CACHE: Mutex<Vec<(String, Arc<Identity>)>> = Mutex::new(Vec::new());
    let stamp = |path: &str| {
        let path = resolve(path, base);
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        format!("{}@{modified:?}", path.display())
    };
    let secret = ring::digest::digest(&ring::digest::SHA256, spec.passphrase.as_bytes());
    let key = format!("{}|{}|{:?}", stamp(&spec.cert), if spec.key.trim().is_empty() { String::new() } else { stamp(&spec.key) }, secret.as_ref());
    if let Some((_, found)) = CACHE.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Ok(found.clone());
    }
    let identity = Arc::new(load(spec, base)?);
    let mut cache = CACHE.lock().unwrap();
    cache.push((key, identity.clone()));
    if cache.len() > 16 {
        cache.remove(0);
    }
    Ok(identity)
}

fn load(spec: &CertSpec, base: &Path) -> Result<Identity, String> {
    if spec.cert.trim().is_empty() {
        return Err("не указан файл сертификата".into());
    }
    let cert_path = resolve(&spec.cert, base);
    let label = cert_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = std::fs::read(&cert_path).map_err(|e| format!("{}: {e}", cert_path.display()))?;
    let ext = cert_path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let is_pem = bytes.trim_ascii_start().starts_with(b"-----");
    let pem = if ext == "p12" || ext == "pfx" || !is_pem {
        openssl(&["pkcs12", "-nodes", "-in"], &cert_path, &spec.passphrase).map_err(|e| format!("{label}: {e}"))?
    } else {
        bytes
    };
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem).filter_map(Result::ok).collect();
    if chain.is_empty() {
        return Err(format!("{label}: в файле нет сертификата (BEGIN CERTIFICATE)"));
    }
    let key_pem = if spec.key.trim().is_empty() {
        pem
    } else {
        let key_path = resolve(&spec.key, base);
        std::fs::read(&key_path).map_err(|e| format!("{}: {e}", key_path.display()))?
    };
    let key = match PrivateKeyDer::from_pem_slice(&key_pem) {
        Ok(key) => key,
        Err(_) if String::from_utf8_lossy(&key_pem).contains("ENCRYPTED") => {
            if spec.passphrase.is_empty() {
                return Err(format!("{label}: ключ зашифрован, нужен passphrase"));
            }
            let key_path = if spec.key.trim().is_empty() { cert_path.clone() } else { resolve(&spec.key, base) };
            let plain = openssl(&["pkey", "-in"], &key_path, &spec.passphrase).map_err(|e| format!("{label}: {e}"))?;
            PrivateKeyDer::from_pem_slice(&plain).map_err(|_| format!("{label}: openssl не отдал ключ"))?
        }
        Err(_) => return Err(format!("{label}: нет закрытого ключа — укажите key или файл, где есть и ключ")),
    };
    Ok(Identity { chain, key, label })
}

/// `openssl <args> <file>` with the passphrase from the environment (never on the command
/// line, where `ps` shows it); PEM out. OpenSSL 3 needs `-legacy` for old PKCS#12 files.
fn openssl(args: &[&str], file: &Path, passphrase: &str) -> Result<Vec<u8>, String> {
    let run = |legacy: bool| {
        let mut c = std::process::Command::new("openssl");
        c.args(args).arg(file).args(["-passin", "env:TSUBAMEDO_CERT_PASS"]).env("TSUBAMEDO_CERT_PASS", passphrase);
        if legacy {
            c.arg("-legacy");
        }
        c.output()
    };
    let out = run(false).map_err(|e| format!("нужен openssl, чтобы прочитать этот файл ({e})"))?;
    if out.status.success() && !out.stdout.is_empty() {
        return Ok(out.stdout);
    }
    if let Ok(again) = run(true)
        && again.status.success()
        && !again.stdout.is_empty()
    {
        return Ok(again.stdout);
    }
    let why = String::from_utf8_lossy(&out.stderr);
    if why.contains("mac verify failure") || why.contains("bad decrypt") || why.contains("invalid password") || why.contains("Invalid password") {
        return Err(if passphrase.is_empty() { "файл под паролем, нужен passphrase".into() } else { "неверный passphrase".into() });
    }
    Err(format!("openssl не прочитал файл: {}", why.lines().next().unwrap_or("неизвестная ошибка").trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert!(matches("api.bank.test", "API.bank.test", 443));
        assert!(matches("api.bank.test:8443", "api.bank.test", 8443) && !matches("api.bank.test:8443", "api.bank.test", 443));
        assert!(matches("*.bank.test", "api.bank.test", 443) && !matches("*.bank.test", "bank.test", 443));
        assert!(matches("*", "anything", 1));
        assert!(matches("127.0.0.1", "127.0.0.1", 9443));
    }
}
