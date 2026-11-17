//! The answer envelope of the DSM Web API and the API catalog of
//! `SYNO.API.Info`.

use std::collections::HashMap;

use nas_model::ProviderError;
use serde_json::Value;

/// APIs the client asks `SYNO.API.Info` about. The data requests (#32) add
/// theirs here.
pub(crate) const APIS: &[&str] = &["SYNO.API.Auth"];

/// API of the login.
pub(crate) const AUTH_API: &str = "SYNO.API.Auth";

/// Path of `SYNO.API.Info`, the same on every DSM.
pub(crate) const INFO_PATH: &str = "/webapi/entry.cgi";

/// Where an API lives and the highest version the NAS speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiInfo {
    /// Path below `/webapi/`, e.g. `entry.cgi`.
    pub path: String,
    pub max_version: u32,
}

/// API name → path and version.
pub(crate) type Catalog = HashMap<String, ApiInfo>;

/// Every DSM answer: `{"success": true, "data": …}` or
/// `{"success": false, "error": {"code": …}}`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Envelope {
    pub success: bool,
    pub data: Option<Value>,
    /// `error.code`, when the NAS sent one.
    pub code: Option<u32>,
}

/// Reads the envelope; anything that is not a JSON object with `success`
/// is a format error.
pub(crate) fn parse_envelope(body: &str) -> Result<Envelope, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::Parse("Antwort ist kein JSON von DSM".into()))?;
    let success = value
        .get("success")
        .and_then(Value::as_bool)
        .ok_or_else(|| ProviderError::Parse("Antwort ohne „success“".into()))?;
    let code = value
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(Value::as_u64)
        .and_then(|c| u32::try_from(c).ok());
    let data = value.get("data").cloned();
    Ok(Envelope {
        success,
        data,
        code,
    })
}

/// Query of `SYNO.API.Info` for [`APIS`].
pub(crate) fn catalog_query() -> String {
    APIS.join(",")
}

/// Reads the answer of `SYNO.API.Info`. An entry without path or version is
/// left out. A refused query or a catalog without `SYNO.API.Auth` means the
/// address is no DSM: a format error.
pub(crate) fn parse_catalog(body: &str) -> Result<Catalog, ProviderError> {
    let envelope = parse_envelope(body)?;
    if !envelope.success {
        return Err(ProviderError::Parse(format!(
            "SYNO.API.Info abgelehnt (DSM-Fehler {}), ist das ein Synology-NAS?",
            code_text(envelope.code)
        )));
    }
    let mut catalog = Catalog::new();
    if let Some(Value::Object(entries)) = envelope.data {
        for (name, entry) in entries {
            let path = entry.get("path").and_then(Value::as_str);
            let max_version = entry
                .get("maxVersion")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            if let (Some(path), Some(max_version)) = (path, max_version) {
                catalog.insert(
                    name,
                    ApiInfo {
                        path: path.to_owned(),
                        max_version,
                    },
                );
            }
        }
    }
    if !catalog.contains_key(AUTH_API) {
        return Err(ProviderError::Parse(
            "SYNO.API.Auth fehlt, ist das ein Synology-NAS?".into(),
        ));
    }
    Ok(catalog)
}

/// `error.code` for a message, `?` without one.
pub(crate) fn code_text(code: Option<u32>) -> String {
    code.map_or_else(|| "?".to_owned(), |c| c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_from_fixture() {
        let catalog =
            parse_catalog(include_str!("../tests/fixtures/api_info.json")).expect("catalog");
        assert_eq!(
            catalog.get("SYNO.API.Auth"),
            Some(&ApiInfo {
                path: "entry.cgi".into(),
                max_version: 7,
            })
        );
        assert_eq!(
            catalog.get("SYNO.Core.System").map(|i| i.path.as_str()),
            Some("entry.cgi")
        );
    }

    #[test]
    fn catalog_query_names_every_api() {
        assert_eq!(catalog_query(), "SYNO.API.Auth");
    }

    #[test]
    fn refused_or_foreign_catalog_is_a_parse_error() {
        assert!(matches!(
            parse_catalog(r#"{"success":false,"error":{"code":102}}"#),
            Err(ProviderError::Parse(_))
        ));
        assert!(matches!(
            parse_catalog(
                r#"{"success":true,"data":{"SYNO.Other":{"path":"x.cgi","maxVersion":1}}}"#
            ),
            Err(ProviderError::Parse(_))
        ));
        assert!(matches!(
            parse_catalog("<html>Not Found</html>"),
            Err(ProviderError::Parse(_))
        ));
    }

    #[test]
    fn entries_without_path_or_version_are_left_out() {
        let catalog = parse_catalog(
            r#"{"success":true,"data":{
                "SYNO.API.Auth":{"path":"entry.cgi","maxVersion":7},
                "SYNO.A":{"maxVersion":1},
                "SYNO.B":{"path":"b.cgi"}}}"#,
        )
        .expect("catalog");
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn envelope_reads_code_and_data() {
        let failed = parse_envelope(include_str!("../tests/fixtures/session_expired.json"))
            .expect("envelope");
        assert!(!failed.success);
        assert_eq!(failed.code, Some(119));
        let ok = parse_envelope(r#"{"success":true}"#).expect("envelope");
        assert_eq!((ok.success, ok.data, ok.code), (true, None, None));
        assert!(matches!(
            parse_envelope(r#"{"data":{}}"#),
            Err(ProviderError::Parse(_))
        ));
    }
}
