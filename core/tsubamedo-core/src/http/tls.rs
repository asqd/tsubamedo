//! rustls configs: system roots (they include corporate CAs) or no verification at all for
//! test stands with self-signed certificates; with a client certificate when the host wants one.

use std::sync::{Arc, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use super::client_cert::Identity;

pub(crate) fn config(verify: bool, identity: Option<&Identity>) -> Result<Arc<ClientConfig>, String> {
    static VERIFIED: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    static INSECURE: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(identity) = identity {
        // Rare and cheap once the roots are read: built for each connection.
        return build(verify, Some(identity)).map(Arc::new);
    }
    let cell = if verify { &VERIFIED } else { &INSECURE };
    Ok(cell.get_or_init(|| Arc::new(build(verify, None).expect("no client certificate, nothing to fail"))).clone())
}

fn build(verify: bool, identity: Option<&Identity>) -> Result<ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions");
    let builder = if verify {
        builder.with_root_certificates(roots())
    } else {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    };
    let mut config = match identity {
        None => builder.with_no_client_auth(),
        Some(id) => builder
            .with_client_auth_cert(id.chain.clone(), id.key.clone_key())
            .map_err(|e| tr!("клиентский сертификат {}: {e}", "client certificate {}: {e}", id.label))?,
    };
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Read once: the system's store takes tens of milliseconds.
fn roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = RootCertStore::empty();
            store.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            if store.is_empty() {
                store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
            Arc::new(store)
        })
        .clone()
}

#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
