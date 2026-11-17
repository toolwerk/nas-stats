//! The answer of `SYNO.API.Auth` `login` and its error codes.

use nas_model::ProviderError;
use serde_json::Value;

use crate::api::parse_envelope;

/// Name the NAS remembers this device under once a second factor was
/// entered with `enable_device_token`.
pub const DEVICE_NAME: &str = "NAS Stats";

/// Hint for the app: DSM lets only administrators read system values.
pub const ADMIN_HINT: &str = "Synology DSM gibt Systemwerte nur an Konten der Gruppe „administrators“ heraus. Tragen Sie ein Administrator-Konto ein; NAS Stats liest nur und ändert nichts.";

/// Hint for the app when DSM asks for the second factor: the code is entered
/// once, after that the NAS remembers this device.
pub const SECOND_FACTOR_HINT: &str = "Für dieses Konto ist die 2-Faktor-Authentifizierung aktiv. Geben Sie einmal den aktuellen Code aus der Authenticator-App ein; danach merkt sich das NAS dieses Gerät als „NAS Stats“ und fragt nicht mehr danach.";

/// Hint for the app after DSM error 407.
pub const IP_BLOCKED_HINT: &str = "Das NAS hat die Adresse dieses Geräts nach zu vielen fehlgeschlagenen Anmeldungen gesperrt. Entsperren Sie sie in DSM unter Systemsteuerung › Sicherheit › Schutz oder warten Sie die Sperrzeit ab, dann melden Sie sich erneut an.";

/// What the NAS said to a login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoginAnswer {
    /// Logged in, with the session id and, after a login with code and
    /// `enable_device_token`, the device token.
    Session { sid: String, did: Option<String> },
    /// The NAS refused the login.
    Refused(ProviderError),
}

/// Error of a refused login by its DSM code (DSM Login Web API Guide):
/// 403, 404 and 406 want a (new) second factor, 407 is a blocked IP, 400,
/// 401, 402 and every other code count as a failed login.
pub(crate) fn login_error(code: Option<u32>) -> ProviderError {
    match code {
        Some(403 | 404 | 406) => ProviderError::SecondFactorRequired,
        Some(407) => ProviderError::IpBlocked,
        _ => ProviderError::LoginFailed,
    }
}

/// Reads a login answer. A success without session id is a format error.
pub(crate) fn parse_login(body: &str) -> Result<LoginAnswer, ProviderError> {
    let envelope = parse_envelope(body)?;
    if !envelope.success {
        return Ok(LoginAnswer::Refused(login_error(envelope.code)));
    }
    let data = envelope.data.unwrap_or(Value::Null);
    let text = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let sid = text("sid").ok_or_else(|| ProviderError::Parse("Anmeldung ohne Sitzung".into()))?;
    // DSM 7 names the token `did`; some versions send `device_id` as well.
    let did = text("did").or_else(|| text("device_id"));
    Ok(LoginAnswer::Session { sid, did })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_answers() {
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login.json")),
            Ok(LoginAnswer::Session {
                sid: "REDACTED".into(),
                did: None
            })
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_device_token.json")),
            Ok(LoginAnswer::Session {
                sid: "REDACTED".into(),
                did: Some("REDACTED-DID".into())
            })
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_2fa.json")),
            Ok(LoginAnswer::Refused(ProviderError::SecondFactorRequired))
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_code_rejected.json")),
            Ok(LoginAnswer::Refused(ProviderError::SecondFactorRequired))
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_wrong_password.json")),
            Ok(LoginAnswer::Refused(ProviderError::LoginFailed))
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_ip_blocked.json")),
            Ok(LoginAnswer::Refused(ProviderError::IpBlocked))
        );
    }

    #[test]
    fn error_codes() {
        for code in [400, 401, 402, 408, 409, 410, 999] {
            assert_eq!(
                login_error(Some(code)),
                ProviderError::LoginFailed,
                "{code}"
            );
        }
        for code in [403, 404, 406] {
            assert_eq!(
                login_error(Some(code)),
                ProviderError::SecondFactorRequired,
                "{code}"
            );
        }
        assert_eq!(login_error(Some(407)), ProviderError::IpBlocked);
        assert_eq!(login_error(None), ProviderError::LoginFailed);
    }

    #[test]
    fn empty_device_token_is_none_and_device_id_counts() {
        assert_eq!(
            parse_login(r#"{"success":true,"data":{"sid":"s","did":""}}"#),
            Ok(LoginAnswer::Session {
                sid: "s".into(),
                did: None
            })
        );
        assert_eq!(
            parse_login(r#"{"success":true,"data":{"sid":"s","device_id":"d"}}"#),
            Ok(LoginAnswer::Session {
                sid: "s".into(),
                did: Some("d".into())
            })
        );
    }

    #[test]
    fn success_without_session_is_a_parse_error() {
        assert!(matches!(
            parse_login(r#"{"success":true,"data":{"did":"d"}}"#),
            Err(ProviderError::Parse(_))
        ));
    }
}
