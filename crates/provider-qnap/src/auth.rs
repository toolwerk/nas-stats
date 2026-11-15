//! The answer of `authLogin.cgi`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use nas_model::ProviderError;

use crate::xml::{parse_qdoc, root_text};

/// Hint for the app when QTS asks for 2-step verification: the login API
/// cannot complete it, so the app needs an account without it.
pub const SECOND_FACTOR_HINT: &str = "Für dieses Konto ist die 2-Stufen-Verifizierung aktiv, die NAS Stats nicht abschließen kann. Lege am QNAP ein eigenes Konto ohne 2-Stufen-Verifizierung mit der Rolle „System Monitoring“ an und trage es hier ein.";

/// Password as `authLogin.cgi` expects it: Base64 of the UTF-8 text (what
/// QTS calls `ezEncode`).
pub(crate) fn encode_password(password: &str) -> String {
    STANDARD.encode(password.as_bytes())
}

/// What the NAS said to a login.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LoginAnswer {
    /// Logged in, with the session id.
    Session(String),
    /// The account has 2-step verification on.
    SecondFactor,
    /// Wrong user name or password, or the account is not allowed.
    Rejected,
}

/// Reads a login answer; `need_2sv` wins over everything else.
pub(crate) fn parse_login(body: &str) -> Result<LoginAnswer, ProviderError> {
    let doc = parse_qdoc(body)?;
    if root_text(&doc, "need_2sv").as_deref() == Some("1") {
        return Ok(LoginAnswer::SecondFactor);
    }
    match (
        root_text(&doc, "authPassed").as_deref(),
        root_text(&doc, "authSid"),
    ) {
        (Some("1"), Some(sid)) => Ok(LoginAnswer::Session(sid)),
        _ => Ok(LoginAnswer::Rejected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_is_base64_of_utf8() {
        assert_eq!(encode_password("pässwort"), "cMOkc3N3b3J0");
        assert_eq!(encode_password(""), "");
    }

    #[test]
    fn login_answers() {
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login.xml")),
            Ok(LoginAnswer::Session("REDACTED".into()))
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_wrong_password.xml")),
            Ok(LoginAnswer::Rejected)
        );
        assert_eq!(
            parse_login(include_str!("../tests/fixtures/login_2sv.xml")),
            Ok(LoginAnswer::SecondFactor)
        );
    }

    #[test]
    fn passed_without_session_is_rejected() {
        assert_eq!(
            parse_login("<QDocRoot><authPassed><![CDATA[1]]></authPassed></QDocRoot>"),
            Ok(LoginAnswer::Rejected)
        );
    }
}
