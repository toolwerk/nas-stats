//! SHA-256 fingerprint of a server certificate, shown as `AB:CD:…` like
//! browsers and the NAS web interfaces do.

use std::fmt;
use std::str::FromStr;

use nas_model::NasError;
use sha2::{Digest, Sha256};

/// SHA-256 over the DER bytes of the end-entity certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    /// Fingerprint of a certificate in DER form.
    pub fn of_der(der: &[u8]) -> Self {
        Self(Sha256::digest(der).into())
    }

    /// The raw 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            write!(f, "{byte:02X}")?;
        }
        Ok(())
    }
}

impl FromStr for Fingerprint {
    type Err = NasError;

    /// Accepts upper or lower case, with or without colons.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: Vec<u8> = s.trim().bytes().filter(|b| *b != b':').collect();
        if hex.len() != 64 {
            return Err(NasError::Parse(format!(
                "Fingerabdruck braucht 64 Hex-Zeichen, hat {}",
                hex.len()
            )));
        }
        let mut bytes = [0u8; 32];
        let (pairs, _) = hex.as_chunks::<2>();
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            match (nibble(*high), nibble(*low)) {
                (Some(h), Some(l)) => *byte = h << 4 | l,
                _ => {
                    return Err(NasError::Parse(
                        "Fingerabdruck enthält Zeichen außer 0-9, A-F und ':'".into(),
                    ));
                }
            }
        }
        Ok(Self(bytes))
    }
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SHA-256 of the empty input.
    const EMPTY: &str = "E3:B0:C4:42:98:FC:1C:14:9A:FB:F4:C8:99:6F:B9:24:\
                         27:AE:41:E4:64:9B:93:4C:A4:95:99:1B:78:52:B8:55";

    #[test]
    fn display_is_upper_case_with_colons() {
        assert_eq!(Fingerprint::of_der(b"").to_string(), EMPTY);
    }

    #[test]
    fn parses_its_own_display() {
        let fp = Fingerprint::of_der(b"cert");
        assert_eq!(fp.to_string().parse::<Fingerprint>(), Ok(fp));
    }

    #[test]
    fn parses_lower_case_without_colons() {
        let plain = EMPTY.replace(':', "").to_lowercase();
        assert_eq!(plain.parse::<Fingerprint>(), Ok(Fingerprint::of_der(b"")));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(matches!(
            "AB:CD".parse::<Fingerprint>(),
            Err(NasError::Parse(_))
        ));
    }

    #[test]
    fn rejects_non_hex() {
        let bad = EMPTY.replacen("E3", "G3", 1);
        assert!(matches!(
            bad.parse::<Fingerprint>(),
            Err(NasError::Parse(_))
        ));
    }
}
