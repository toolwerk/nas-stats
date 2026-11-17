//! Access to one TrueNAS: user name and API key.

use std::fmt;

use nas_model::ProviderError;

/// Advice for the access the app asks for: a user of its own with the role
/// `READONLY_ADMIN` and an API key of that user. TrueNAS needs the user name
/// for both login methods.
pub const ROLE_HINT: &str = "Lege in TrueNAS einen eigenen Benutzer mit der Rolle \
    READONLY_ADMIN an und erzeuge für ihn einen API-Key. NAS Stats liest damit nur.";

/// User name and API key of a TrueNAS user.
///
/// The key has the form TrueNAS shows when it creates one, `<id>-<secret>`
/// (digits, a dash, letters and digits). The user name may not be empty and
/// may not contain `,` or `=`, which SCRAM messages use as separators.
#[derive(Clone, PartialEq, Eq)]
pub struct TruenasCredentials {
    username: String,
    api_key: String,
    key_id: u64,
}

impl TruenasCredentials {
    /// Checks both values; a wrong form is [`ProviderError::Parse`].
    pub fn new(
        username: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let username = username.into();
        let api_key = api_key.into();
        if username.trim().is_empty() {
            return Err(ProviderError::Parse("Benutzername fehlt".into()));
        }
        if username.contains([',', '=']) || username.chars().any(char::is_control) {
            return Err(ProviderError::Parse(
                "Benutzername darf weder ',' noch '=' enthalten".into(),
            ));
        }
        let key_id = parse_key(&api_key).ok_or_else(|| {
            ProviderError::Parse("API-Key hat nicht die Form <Nummer>-<Schlüssel>".into())
        })?;
        Ok(Self {
            username,
            api_key,
            key_id,
        })
    }

    /// The user the key belongs to.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The whole key as TrueNAS shows it, for `API_KEY_PLAIN`.
    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    /// The number before the dash, which names the key for SCRAM.
    pub(crate) fn key_id(&self) -> u64 {
        self.key_id
    }

    /// The part after the first dash, the secret SCRAM derives its keys from.
    pub(crate) fn secret(&self) -> &str {
        self.api_key
            .split_once('-')
            .map_or("", |(_, secret)| secret)
    }
}

/// The id of a key `<digits>-<letters and digits>`, or `None`.
fn parse_key(key: &str) -> Option<u64> {
    let (id, secret) = key.split_once('-')?;
    let digits = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
    let alnum = !secret.is_empty() && secret.bytes().all(|b| b.is_ascii_alphanumeric());
    if digits && alnum {
        id.parse().ok()
    } else {
        None
    }
}

impl fmt::Debug for TruenasCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TruenasCredentials")
            .field("username", &self.username)
            .field("api_key", &"<verborgen>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_key_as_truenas_shows_it() {
        let c = TruenasCredentials::new("nasstats", "12-AbC123xyz").unwrap_or_else(|e| {
            panic!("{e:?}");
        });
        assert_eq!(c.username(), "nasstats");
        assert_eq!(c.key_id(), 12);
        assert_eq!(c.secret(), "AbC123xyz");
        assert_eq!(c.api_key(), "12-AbC123xyz");
    }

    #[test]
    fn rejects_keys_of_another_form() {
        for key in [
            "",
            "abc",
            "-abc",
            "1-",
            "x1-abc",
            "1-ab-c",
            "1-ab c",
            "1-äbc",
            "99999999999999999999999-abc",
        ] {
            assert!(
                matches!(
                    TruenasCredentials::new("u", key),
                    Err(ProviderError::Parse(_))
                ),
                "{key:?}"
            );
        }
    }

    #[test]
    fn rejects_empty_names_and_separators() {
        for name in ["", "  ", "a,b", "a=b", "a\nb"] {
            assert!(
                matches!(
                    TruenasCredentials::new(name, "1-abc"),
                    Err(ProviderError::Parse(_))
                ),
                "{name:?}"
            );
        }
    }

    #[test]
    fn debug_hides_the_key() {
        let c = TruenasCredentials::new("nasstats", "1-geheim").unwrap_or_else(|e| {
            panic!("{e:?}");
        });
        let text = format!("{c:?}");
        assert!(text.contains("nasstats"), "{text}");
        assert!(!text.contains("geheim"), "{text}");
    }
}
