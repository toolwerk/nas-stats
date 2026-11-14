//! Certificate pinning. A QNAP usually serves a self-signed certificate,
//! so the usual chain check cannot work. Instead the tool trusts exactly
//! one certificate: the one whose SHA-256 fingerprint the user confirmed
//! (`--pin`). Without a pin, or with a different certificate, the
//! handshake fails before any request (and so before the password) is
//! sent. The fingerprint seen is kept so the caller can show it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};

/// Fingerprint seen during the last handshake, shared with the verifier.
pub type Seen = Arc<Mutex<Option<String>>>;

/// SHA-256 over the DER bytes, upper-case hex pairs joined by `:`, the
/// form browsers show.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Compares two fingerprints, ignoring `:`, whitespace and case.
pub fn same_fingerprint(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| *c != ':' && !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let a = norm(a);
    !a.is_empty() && a == norm(b)
}

#[derive(Debug)]
pub struct PinVerifier {
    pin: Option<String>,
    seen: Seen,
    provider: Arc<CryptoProvider>,
}

impl PinVerifier {
    pub fn new(pin: Option<&str>, provider: Arc<CryptoProvider>) -> Self {
        Self {
            pin: pin.map(str::to_owned),
            seen: Arc::new(Mutex::new(None)),
            provider,
        }
    }

    pub fn seen(&self) -> Seen {
        Arc::clone(&self.seen)
    }

    /// Records the fingerprint and accepts only the pinned one. No name
    /// check: the pin identifies the device.
    pub fn check(&self, end_entity: &[u8]) -> Result<(), Error> {
        let fp = fingerprint(end_entity);
        let ok = self
            .pin
            .as_deref()
            .is_some_and(|pin| same_fingerprint(pin, &fp));
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(fp);
        }
        if ok {
            Ok(())
        } else {
            Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.check(end_entity.as_ref())
            .map(|()| ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Blocking HTTP client that trusts only the pinned certificate, speaks
/// only HTTPS, follows no redirects, keeps cookies (Container Station)
/// and gives up after 20 seconds.
pub fn client(pin: Option<&str>) -> Result<(reqwest::blocking::Client, Seen), String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinVerifier::new(pin, Arc::clone(&provider));
    let seen = verifier.seen();
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS setup: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let client = reqwest::blocking::Client::builder()
        .tls_backend_preconfigured(config)
        .cookie_store(true)
        // No redirects: a 307 to http:// would resend password and sid
        // to a device nobody pinned.
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;
    Ok((client, seen))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verifier(pin: Option<&str>) -> PinVerifier {
        PinVerifier::new(pin, Arc::new(rustls::crypto::ring::default_provider()))
    }

    #[test]
    fn fingerprint_is_upper_hex_with_colons() {
        // SHA-256 of "abc".
        assert_eq!(
            fingerprint(b"abc"),
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
    }

    #[test]
    fn comparison_ignores_colons_and_case() {
        let fp = fingerprint(b"abc");
        assert!(same_fingerprint(&fp, &fp.replace(':', "").to_lowercase()));
        assert!(same_fingerprint(&fp.to_lowercase(), &fp));
        assert!(!same_fingerprint(&fp, &fingerprint(b"abd")));
        assert!(!same_fingerprint("", ""));
    }

    #[test]
    fn without_pin_rejects_and_records() {
        let v = verifier(None);
        assert!(v.check(b"abc").is_err());
        let seen = v.seen().lock().map(|s| s.clone()).ok().flatten();
        assert_eq!(seen, Some(fingerprint(b"abc")));
    }

    #[test]
    fn wrong_pin_rejects_and_records() {
        let v = verifier(Some(&fingerprint(b"other")));
        assert!(matches!(
            v.check(b"abc"),
            Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            ))
        ));
        let seen = v.seen().lock().map(|s| s.clone()).ok().flatten();
        assert_eq!(seen, Some(fingerprint(b"abc")));
    }

    #[test]
    fn matching_pin_accepts() {
        let pin = fingerprint(b"abc").replace(':', "").to_lowercase();
        assert!(verifier(Some(&pin)).check(b"abc").is_ok());
    }

    #[test]
    fn client_builds() {
        assert!(client(None).is_ok());
    }
}
