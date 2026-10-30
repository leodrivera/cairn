//! Which certificate authorities outbound HTTPS trusts (CAPTCHA, broker, OIDC providers): the Mozilla list built into
//! the binary plus the system's. The system's list is where an operator adds their own CA, for an identity provider
//! on a home network: `/etc/ssl/certs`, or the files `SSL_CERT_FILE` and `SSL_CERT_DIR` name. Those variables replace
//! the system's list rather than add to it, so the built-in list stays underneath and public sites never drop out.

use std::sync::OnceLock;

use rustls_pki_types::CertificateDer;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// For every outbound agent. The roots are read once, on first use.
pub fn config() -> TlsConfig {
    static ROOTS: OnceLock<RootCerts> = OnceLock::new();
    let roots = ROOTS.get_or_init(|| RootCerts::new_with_certs(&load()));
    TlsConfig::builder().root_certs(roots.clone()).build()
}

fn load() -> Vec<Certificate<'static>> {
    let found = rustls_native_certs::load_native_certs();
    for error in &found.errors {
        tracing::warn!(event = "tls.system_roots_error", error = %error);
    }
    combine(found.certs)
}

/// The built-in list, then the system's certificates.
fn combine(system: Vec<CertificateDer<'static>>) -> Vec<Certificate<'static>> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .map(|c| Certificate::from_der(c.as_ref()).to_owned())
        .chain(
            system
                .iter()
                .map(|c| Certificate::from_der(c.as_ref()).to_owned()),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls_pki_types::pem::PemObject;

    fn test_ca() -> rustls_pki_types::CertificateDer<'static> {
        rustls_pki_types::CertificateDer::from_pem_file("tests/fixtures/test-ca.pem").unwrap()
    }

    #[test]
    fn system_roots_add_to_the_builtin_list() {
        let ca = test_ca();
        let all = combine(vec![ca.clone()]);
        assert_eq!(
            all.len(),
            webpki_root_certs::TLS_SERVER_ROOT_CERTS.len() + 1
        );
        assert!(all.iter().any(|c| c.der() == ca.as_ref()));
    }

    #[test]
    fn builtin_list_alone_without_system_roots() {
        assert_eq!(
            combine(Vec::new()).len(),
            webpki_root_certs::TLS_SERVER_ROOT_CERTS.len()
        );
    }
}
