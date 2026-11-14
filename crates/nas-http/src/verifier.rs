//! Certificate verifiers. NAS certificates are almost always self-signed,
//! so trust comes from the fingerprint the user confirmed, not from a CA.
//!
//! This is the only module that touches rustls' `dangerous()` API. Neither
//! verifier accepts an arbitrary certificate: [`PinVerifier`] accepts exactly
//! the pinned one, [`CaptureVerifier`] accepts none.

use std::sync::{Arc, Mutex, PoisonError};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error, SignatureScheme};

/// Where a verifier leaves the fingerprint it saw.
pub(crate) type Seen = Arc<Mutex<Option<crate::Fingerprint>>>;

/// The crypto provider of this crate: ring, no aws-lc.
pub(crate) fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Client config that only talks to the certificate with fingerprint `pin`.
pub(crate) fn pinned_config(pin: crate::Fingerprint, seen: Seen) -> Result<ClientConfig, Error> {
    let provider = provider();
    let verifier = PinVerifier {
        pin,
        seen,
        provider: provider.clone(),
    };
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

/// Client config for the first contact: records the fingerprint and always
/// aborts the handshake, so no application data can follow.
pub(crate) fn capture_config(seen: Seen) -> Result<ClientConfig, Error> {
    let provider = provider();
    let verifier = CaptureVerifier {
        seen,
        provider: provider.clone(),
    };
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth())
}

fn store(seen: &Seen, fp: crate::Fingerprint) {
    *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(fp);
}

/// Accepts exactly the certificate with the pinned fingerprint. Chain, name
/// and validity period are not checked: the user confirmed this one
/// certificate, and NAS certificates are often expired or issued to another
/// name. The handshake signatures are still checked, so the server must hold
/// the private key of the pinned certificate.
#[derive(Debug)]
struct PinVerifier {
    pin: crate::Fingerprint,
    seen: Seen,
    provider: Arc<CryptoProvider>,
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
        let fp = crate::Fingerprint::of_der(end_entity);
        if fp == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            store(&self.seen, fp);
            Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
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

/// Records the fingerprint of the presented certificate and rejects it.
#[derive(Debug)]
struct CaptureVerifier {
    seen: Seen,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for CaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        store(&self.seen, crate::Fingerprint::of_der(end_entity));
        Err(Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure,
        ))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure,
        ))
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::InvalidCertificate(
            CertificateError::ApplicationVerificationFailure,
        ))
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
