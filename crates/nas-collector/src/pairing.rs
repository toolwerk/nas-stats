//! What the app needs to trust the collector: a self-signed certificate,
//! whose fingerprint the app pins like that of a NAS, and a key the app
//! sends as bearer token. Both are made at the first start in the data
//! directory (`tls/cert.der`, `tls/key.der`, `token`, readable only by the
//! owner) and reused afterwards; `nas-collector pairing` prints them.
//!
//! Deleting the files makes new ones at the next start; the apps then have
//! to be paired again.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use nas_http::Fingerprint;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// Name the certificate is issued for.
pub const CERT_NAME: &str = "nas-collector";

/// Certificate, private key and key for the apps.
pub struct Pairing {
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
    /// 32 random bytes as hex.
    pub token: String,
}

impl Pairing {
    /// Fingerprint of the certificate, as the app pins it.
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_der(&self.cert)
    }
}

/// Reads certificate, key and token from `dir`, making what is missing.
pub fn ensure(dir: &Path) -> std::io::Result<Pairing> {
    let tls = dir.join("tls");
    std::fs::create_dir_all(&tls)?;
    std::fs::set_permissions(&tls, std::fs::Permissions::from_mode(0o700))?;
    let cert_path = tls.join("cert.der");
    let key_path = tls.join("key.der");
    let (cert, key) = match (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        (Ok(cert), Ok(key)) if !cert.is_empty() && !key.is_empty() => (cert, key),
        _ => {
            let made = rcgen::generate_simple_self_signed(vec![CERT_NAME.to_owned()])
                .map_err(std::io::Error::other)?;
            let cert = made.cert.der().to_vec();
            let key = made.signing_key.serialize_der();
            write_private(&key_path, &key)?;
            write_private(&cert_path, &cert)?;
            (cert, key)
        }
    };

    let token_path = dir.join("token");
    let token = match std::fs::read_to_string(&token_path) {
        Ok(text) if !text.trim().is_empty() => text.trim().to_owned(),
        _ => {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
            let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            write_private(&token_path, format!("{token}\n").as_bytes())?;
            token
        }
    };

    Ok(Pairing {
        cert: CertificateDer::from(cert),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
        token,
    })
}

/// Writes a file only the owner may read, replacing an old one.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` applies only to a new file.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_start_makes_private_files_and_later_starts_reuse_them() {
        let dir =
            std::env::temp_dir().join(format!("nas-collector-pairing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = ensure(&dir).expect("pairing is made");
        assert_eq!(first.token.len(), 64);
        assert!(first.token.chars().all(|c| c.is_ascii_hexdigit()));
        for file in ["tls/cert.der", "tls/key.der", "token"] {
            let mode = std::fs::metadata(dir.join(file))
                .expect("file exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{file}");
        }

        let again = ensure(&dir).expect("pairing is read");
        assert_eq!(again.fingerprint(), first.fingerprint());
        assert_eq!(again.token, first.token);

        // A deleted token is made anew, the certificate stays.
        std::fs::remove_file(dir.join("token")).expect("removed");
        let new_token = ensure(&dir).expect("pairing is read");
        assert_ne!(new_token.token, first.token);
        assert_eq!(new_token.fingerprint(), first.fingerprint());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
