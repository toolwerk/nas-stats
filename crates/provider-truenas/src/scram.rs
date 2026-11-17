//! SCRAM-SHA-512 (RFC 5802) client side as TrueNAS 26 expects it for API
//! keys, optionally bound to the TLS certificate (RFC 5929
//! `tls-server-end-point`). The key itself never goes over the wire.
//!
//! TrueNAS departs from the RFC in one point: the user name carries the key
//! id, `n=<user>:<id>`, so the server finds the right key of the user.

use std::num::NonZeroU32;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use nas_model::ProviderError;
use ring::rand::SecureRandom as _;
use ring::{digest, hmac, pbkdf2};

/// Lowest and highest iteration count the client accepts, as the TrueNAS
/// client library does. Fewer would make the key cheap to guess from a
/// recorded exchange, more would stall the app.
pub(crate) const MIN_ITERATIONS: u32 = 50_000;
pub(crate) const MAX_ITERATIONS: u32 = 5_000_000;
/// Longest salt the client accepts, in bytes.
const MAX_SALT: usize = 1024;
/// Length of the client nonce, in bytes.
pub(crate) const NONCE_LEN: usize = 32;
/// Name of the channel binding type in the GS2 header.
const BINDING_NAME: &str = "tls-server-end-point";

/// Keys derived from the secret for one salt and iteration count. They are
/// worth keeping: deriving them costs hundreds of thousands of rounds.
#[derive(Clone)]
pub(crate) struct ScramKeys {
    client_key: [u8; 64],
    stored_key: [u8; 64],
    server_key: [u8; 64],
}

impl ScramKeys {
    /// SaltedPassword = PBKDF2-HMAC-SHA512(secret, salt, i); ClientKey,
    /// StoredKey and ServerKey as in RFC 5802, section 3.
    pub(crate) fn derive(secret: &[u8], salt: &[u8], iterations: NonZeroU32) -> Self {
        let mut salted = [0u8; 64];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA512,
            iterations,
            salt,
            secret,
            &mut salted,
        );
        let salted_key = hmac::Key::new(hmac::HMAC_SHA512, &salted);
        let client_key = to_64(hmac::sign(&salted_key, b"Client Key").as_ref());
        let server_key = to_64(hmac::sign(&salted_key, b"Server Key").as_ref());
        let stored_key = to_64(digest::digest(&digest::SHA512, &client_key).as_ref());
        Self {
            client_key,
            stored_key,
            server_key,
        }
    }
}

fn to_64(bytes: &[u8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    out.copy_from_slice(bytes);
    out
}

/// RFC 5929 `tls-server-end-point` for a certificate in DER form: the hash
/// of the certificate with the hash of its signature algorithm, MD5 and
/// SHA-1 raised to SHA-256. Only RSA PKCS #1 and ECDSA with MD5, SHA-1,
/// SHA-256, SHA-384 or SHA-512 are known; for anything else (RSA-PSS,
/// EdDSA, SHA-224) there is no binding and the login goes without.
pub(crate) fn channel_binding(cert_der: &[u8]) -> Option<Vec<u8>> {
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der).ok()?;
    let algorithm = match cert.signature_algorithm.algorithm.to_id_string().as_str() {
        // md5, sha1 and sha256 with RSA; ecdsa with sha1 and sha256
        "1.2.840.113549.1.1.4"
        | "1.2.840.113549.1.1.5"
        | "1.2.840.113549.1.1.11"
        | "1.2.840.10045.4.1"
        | "1.2.840.10045.4.3.2" => &digest::SHA256,
        // sha384 with RSA and ECDSA
        "1.2.840.113549.1.1.12" | "1.2.840.10045.4.3.3" => &digest::SHA384,
        // sha512 with RSA and ECDSA
        "1.2.840.113549.1.1.13" | "1.2.840.10045.4.3.4" => &digest::SHA512,
        _ => return None,
    };
    Some(digest::digest(algorithm, cert_der).as_ref().to_vec())
}

/// A fresh random client nonce.
pub(crate) fn random_nonce() -> Result<[u8; NONCE_LEN], ProviderError> {
    let mut nonce = [0u8; NONCE_LEN];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| ProviderError::Parse("Zufallszahl für SCRAM fehlt".into()))?;
    Ok(nonce)
}

/// The first message of the client, kept for the rest of the exchange.
pub(crate) struct ClientFirst {
    gs2_header: String,
    bare: String,
    nonce: [u8; NONCE_LEN],
    binding: Option<Vec<u8>>,
}

impl ClientFirst {
    /// `n=<user>:<key id>,r=<nonce>` with the GS2 header
    /// `p=tls-server-end-point,,` when there is a binding, else `n,,`.
    pub(crate) fn new(
        username: &str,
        key_id: u64,
        nonce: [u8; NONCE_LEN],
        binding: Option<Vec<u8>>,
    ) -> Self {
        let gs2_header = match binding {
            Some(_) => format!("p={BINDING_NAME},,"),
            None => "n,,".to_owned(),
        };
        let bare = format!("n={username}:{key_id},r={}", B64.encode(nonce));
        Self {
            gs2_header,
            bare,
            nonce,
            binding,
        }
    }

    /// The message as sent (`rfc_str` of `CLIENT_FIRST_MESSAGE`).
    pub(crate) fn message(&self) -> String {
        format!("{}{}", self.gs2_header, self.bare)
    }

    /// Whether the exchange is bound to the certificate.
    pub(crate) fn is_bound(&self) -> bool {
        self.binding.is_some()
    }
}

/// The first answer of the server, checked.
pub(crate) struct ServerFirst {
    raw: String,
    nonce: String,
    pub(crate) salt: Vec<u8>,
    pub(crate) iterations: NonZeroU32,
}

impl ServerFirst {
    /// Reads `r=<nonce>,s=<salt>,i=<iterations>`. The nonce must start with
    /// the client nonce and add one of the server, the salt must have 1 to
    /// 1024 bytes and the count lie between [`MIN_ITERATIONS`] and
    /// [`MAX_ITERATIONS`]; anything else is [`ProviderError::Parse`].
    pub(crate) fn parse(raw: &str, client: &ClientFirst) -> Result<Self, ProviderError> {
        let bad = |what: &str| ProviderError::Parse(format!("SCRAM-Antwort des NAS: {what}"));
        let (mut nonce, mut salt, mut iterations) = (None, None, None);
        for part in raw.split(',') {
            match part.split_once('=') {
                Some(("r", v)) if nonce.is_none() => nonce = Some(v),
                Some(("s", v)) if salt.is_none() => salt = Some(v),
                Some(("i", v)) if iterations.is_none() => iterations = Some(v),
                _ => return Err(bad("unbekannter Teil")),
            }
        }
        let (Some(nonce), Some(salt), Some(iterations)) = (nonce, salt, iterations) else {
            return Err(bad("Teil fehlt"));
        };
        let nonce_bytes = B64.decode(nonce).map_err(|_| bad("Nonce kein Base64"))?;
        if nonce_bytes.len() <= NONCE_LEN || !nonce_bytes.starts_with(&client.nonce) {
            return Err(bad("Nonce passt nicht zur Anfrage"));
        }
        let salt = B64.decode(salt).map_err(|_| bad("Salt kein Base64"))?;
        if salt.is_empty() || salt.len() > MAX_SALT {
            return Err(bad("Salt hat eine unzulässige Länge"));
        }
        let iterations: u32 = iterations.parse().map_err(|_| bad("Iterationen"))?;
        let iterations = NonZeroU32::new(iterations)
            .filter(|i| (MIN_ITERATIONS..=MAX_ITERATIONS).contains(&i.get()))
            .ok_or_else(|| bad("Iterationen außerhalb von 50000 bis 5000000"))?;
        Ok(Self {
            raw: raw.to_owned(),
            nonce: nonce.to_owned(),
            salt,
            iterations,
        })
    }
}

/// The final message of the client and what checking the server needs.
pub(crate) struct ClientFinal {
    message: String,
    auth_message: String,
}

impl ClientFinal {
    /// `c=<base64 of GS2 header and binding>,r=<nonce>,p=<proof>` with
    /// ClientProof = ClientKey XOR HMAC(StoredKey, AuthMessage).
    pub(crate) fn new(first: &ClientFirst, server: &ServerFirst, keys: &ScramKeys) -> Self {
        let mut cbind_input = first.gs2_header.as_bytes().to_vec();
        if let Some(binding) = &first.binding {
            cbind_input.extend_from_slice(binding);
        }
        let without_proof = format!("c={},r={}", B64.encode(cbind_input), server.nonce);
        let auth_message = format!("{},{},{without_proof}", first.bare, server.raw);
        let stored = hmac::Key::new(hmac::HMAC_SHA512, &keys.stored_key);
        let signature = hmac::sign(&stored, auth_message.as_bytes());
        let proof: Vec<u8> = keys
            .client_key
            .iter()
            .zip(signature.as_ref())
            .map(|(k, s)| k ^ s)
            .collect();
        Self {
            message: format!("{without_proof},p={}", B64.encode(proof)),
            auth_message,
        }
    }

    /// The message as sent (`rfc_str` of `CLIENT_FINAL_MESSAGE`).
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// Checks `v=<signature>` of the server against
    /// HMAC(ServerKey, AuthMessage). A wrong, missing or unreadable
    /// signature, or an error `e=…`, is [`ProviderError::LoginFailed`]:
    /// whoever answered does not know the key.
    pub(crate) fn verify_server(&self, raw: &str, keys: &ScramKeys) -> Result<(), ProviderError> {
        let signature = raw
            .strip_prefix("v=")
            .filter(|v| !v.contains(','))
            .and_then(|v| B64.decode(v).ok())
            .ok_or(ProviderError::LoginFailed)?;
        let server = hmac::Key::new(hmac::HMAC_SHA512, &keys.server_key);
        hmac::verify(&server, self.auth_message.as_bytes(), &signature)
            .map_err(|_| ProviderError::LoginFailed)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// Messages of the TrueNAS client library for fixed nonces, salt and
    /// key; origin in `tests/fixtures/README.md`.
    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/scram_vectors.json"))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn text<'a>(v: &'a Value, key: &str) -> &'a str {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{key} fehlt"))
    }

    fn bytes(v: &Value, key: &str) -> Vec<u8> {
        B64.decode(text(v, key))
            .unwrap_or_else(|e| panic!("{key}: {e}"))
    }

    fn nonce(v: &Value) -> [u8; NONCE_LEN] {
        bytes(v, "client_nonce")
            .try_into()
            .unwrap_or_else(|_| panic!("Nonce mit 32 Byte"))
    }

    fn keys(v: &Value, server: &ServerFirst) -> ScramKeys {
        let secret = text(v, "api_key")
            .split_once('-')
            .map(|(_, s)| s)
            .unwrap_or_default();
        ScramKeys::derive(secret.as_bytes(), &server.salt, server.iterations)
    }

    fn check_case(v: &Value, case: &str, binding: Option<Vec<u8>>) {
        let c = v.get(case).unwrap_or_else(|| panic!("{case} fehlt"));
        let id: u64 = text(v, "api_key")
            .split_once('-')
            .and_then(|(id, _)| id.parse().ok())
            .unwrap_or_else(|| panic!("Key-Id"));
        let first = ClientFirst::new(text(v, "username"), id, nonce(v), binding);
        assert_eq!(first.message(), text(c, "client_first"), "{case}");
        let server = ServerFirst::parse(text(c, "server_first"), &first)
            .unwrap_or_else(|e| panic!("{case}: {e:?}"));
        let keys = keys(v, &server);
        let last = ClientFinal::new(&first, &server, &keys);
        assert_eq!(last.message(), text(c, "client_final"), "{case}");
        assert_eq!(last.verify_server(text(c, "server_final"), &keys), Ok(()));
        // One flipped bit in the signature fails.
        let mut wrong = bytes(c, "server_final_signature");
        wrong[0] ^= 1;
        assert_eq!(
            last.verify_server(&format!("v={}", B64.encode(wrong)), &keys),
            Err(ProviderError::LoginFailed)
        );
    }

    #[test]
    fn matches_the_truenas_library_without_binding() {
        check_case(&vectors(), "unbound", None);
    }

    #[test]
    fn matches_the_truenas_library_with_binding() {
        let v = vectors();
        let binding = channel_binding(&bytes(&v, "cert_der"));
        assert_eq!(binding, Some(bytes(&v, "cert_binding")));
        check_case(&v, "bound", binding);
    }

    #[test]
    fn unknown_signature_algorithm_has_no_binding() {
        assert_eq!(channel_binding(b"kein Zertifikat"), None);
        // RFC 5929 names no hash for Ed25519, so there is no binding.
        let key =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap_or_else(|e| panic!("{e}"));
        let cert = rcgen::CertificateParams::new(vec!["truenas.example".to_owned()])
            .and_then(|p| p.self_signed(&key))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(channel_binding(cert.der()), None);
    }

    #[test]
    fn signature_hash_picks_the_binding_hash() {
        for (alg, len) in [
            (&rcgen::PKCS_ECDSA_P256_SHA256, 32),
            (&rcgen::PKCS_ECDSA_P384_SHA384, 48),
        ] {
            let key = rcgen::KeyPair::generate_for(alg).unwrap_or_else(|e| panic!("{e}"));
            let cert = rcgen::CertificateParams::new(vec!["truenas.example".to_owned()])
                .and_then(|p| p.self_signed(&key))
                .unwrap_or_else(|e| panic!("{e}"));
            let binding = channel_binding(cert.der()).unwrap_or_default();
            assert_eq!(binding.len(), len, "{alg:?}");
        }
    }

    fn client() -> ClientFirst {
        ClientFirst::new("nasstats", 1, [7u8; NONCE_LEN], None)
    }

    #[test]
    fn server_first_is_checked() {
        let own = B64.encode([[7u8; NONCE_LEN], [9u8; NONCE_LEN]].concat());
        let foreign = B64.encode([[8u8; NONCE_LEN], [9u8; NONCE_LEN]].concat());
        let only_client = B64.encode([7u8; NONCE_LEN]);
        let salt = B64.encode([1u8; 16]);
        let long_salt = B64.encode([1u8; MAX_SALT + 1]);
        let ok = format!("r={own},s={salt},i=500000");
        assert!(ServerFirst::parse(&ok, &client()).is_ok());
        for raw in [
            format!("r={foreign},s={salt},i=500000"),
            format!("r={only_client},s={salt},i=500000"),
            format!("r={own},s=,i=500000"),
            format!("r={own},s={long_salt},i=500000"),
            format!("r={own},s={salt},i=49999"),
            format!("r={own},s={salt},i=5000001"),
            format!("r={own},s={salt},i=x"),
            format!("r={own},s={salt}"),
            format!("r={own},s={salt},i=500000,x=1"),
            format!("r={own},r={own},s={salt},i=500000"),
            "e=other-error".to_owned(),
        ] {
            assert!(
                matches!(
                    ServerFirst::parse(&raw, &client()),
                    Err(ProviderError::Parse(_))
                ),
                "{raw}"
            );
        }
    }

    #[test]
    fn missing_or_garbled_server_signature_fails_the_login() {
        let first = client();
        let own = B64.encode([[7u8; NONCE_LEN], [9u8; NONCE_LEN]].concat());
        let server = ServerFirst::parse(
            &format!("r={own},s={},i=50000", B64.encode([1u8; 16])),
            &first,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let keys = ScramKeys::derive(b"geheim", &server.salt, server.iterations);
        let last = ClientFinal::new(&first, &server, &keys);
        for raw in ["", "v=", "v=%%%", "e=invalid-proof", "v=AAAA,x=1"] {
            assert_eq!(
                last.verify_server(raw, &keys),
                Err(ProviderError::LoginFailed),
                "{raw}"
            );
        }
    }
}
