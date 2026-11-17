//! Sends GraphQL queries with the API key and reads the answer.

use std::fmt;

use nas_model::ProviderError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::transport::Transport;

/// Client for one Unraid server. It holds the API key in memory and sends
/// each query exactly once; there is no session to keep.
pub struct UnraidClient<T: Transport> {
    transport: T,
    api_key: Mutex<String>,
}

impl<T: Transport> fmt::Debug for UnraidClient<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The API key does not belong in a log.
        f.debug_struct("UnraidClient").finish_non_exhaustive()
    }
}

impl<T: Transport> UnraidClient<T> {
    /// Client with the API key `api_key`; nothing is sent yet.
    pub fn new(transport: T, api_key: impl Into<String>) -> Self {
        Self {
            transport,
            api_key: Mutex::new(api_key.into()),
        }
    }

    /// User action „Zugang ändern“: a new API key for the next query.
    /// Sends nothing.
    pub async fn set_api_key(&self, api_key: impl Into<String>) {
        *self.api_key.lock().await = api_key.into();
    }

    /// Sends `{"query": query}` and returns the field `data` of the answer.
    ///
    /// A refused key is [`ProviderError::LoginFailed`], missing rights are
    /// [`ProviderError::PermissionDenied`]. Other GraphQL errors are logged
    /// when the answer still has `data`, else they are
    /// [`ProviderError::Parse`] with the first message.
    pub async fn query(&self, query: &str) -> Result<Value, ProviderError> {
        let body = serde_json::json!({ "query": query }).to_string();
        let api_key = self.api_key.lock().await.clone();
        let answer = self.transport.post_graphql(&body, &api_key).await?;
        parse_answer(&answer)
    }
}

/// `data` of a GraphQL answer, or the error it reports.
pub(crate) fn parse_answer(body: &str) -> Result<Value, ProviderError> {
    let answer: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::Parse("GraphQL: keine JSON-Antwort".into()))?;
    let errors: Vec<(String, String)> = answer
        .get("errors")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(error_parts).collect())
        .unwrap_or_default();
    let data = answer.get("data").filter(|d| d.is_object()).cloned();
    // The code decides; the text only counts for an error without a code,
    // so e.g. an internal error that quotes "Unauthorized" does not stop
    // polling.
    let is = |wanted: &str, text: &str| {
        errors.iter().any(|(code, message)| {
            code == wanted || (code.is_empty() && message.to_ascii_lowercase().contains(text))
        })
    };
    if is("UNAUTHENTICATED", "unauthorized") {
        return Err(ProviderError::LoginFailed);
    }
    if is("FORBIDDEN", "forbidden") {
        return Err(ProviderError::PermissionDenied);
    }
    let first = errors.first().map(|(_, message)| message.as_str());
    match (data, first) {
        (Some(data), None) => Ok(data),
        (Some(data), Some(message)) => {
            log::warn!(
                "Unraid GraphQL answered {} error(s) with data, first: {message}",
                errors.len()
            );
            Ok(data)
        }
        (None, Some(message)) => Err(ProviderError::Parse(format!("GraphQL: {message}"))),
        (None, None) => Err(ProviderError::Parse("GraphQL: Antwort ohne data".into())),
    }
}

/// Code (`extensions.code`, upper case) and message of one GraphQL error.
fn error_parts(error: &Value) -> (String, String) {
    let code = error
        .pointer("/extensions/code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("ohne Meldung")
        .to_owned();
    (code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_without_errors_is_returned() {
        let data = parse_answer(r#"{"data":{"vars":{"name":"tower"}}}"#).expect("data");
        assert_eq!(data["vars"]["name"], "tower");
    }

    #[test]
    fn unauthenticated_is_login_failed() {
        let by_code =
            r#"{"errors":[{"message":"x","extensions":{"code":"UNAUTHENTICATED"}}],"data":null}"#;
        let by_text = r#"{"errors":[{"message":"Unauthorized"}]}"#;
        assert_eq!(parse_answer(by_code), Err(ProviderError::LoginFailed));
        assert_eq!(parse_answer(by_text), Err(ProviderError::LoginFailed));
    }

    #[test]
    fn forbidden_is_permission_denied() {
        let by_code =
            r#"{"errors":[{"message":"x","extensions":{"code":"FORBIDDEN"}}],"data":{"a":1}}"#;
        let by_text = r#"{"errors":[{"message":"Forbidden resource"}],"data":null}"#;
        assert_eq!(parse_answer(by_code), Err(ProviderError::PermissionDenied));
        assert_eq!(parse_answer(by_text), Err(ProviderError::PermissionDenied));
    }

    #[test]
    fn text_counts_only_for_errors_without_a_code() {
        let internal = r#"{"errors":[{"message":"Docker: Unauthorized socket","extensions":{"code":"INTERNAL_SERVER_ERROR"}}],"data":null}"#;
        assert_eq!(
            parse_answer(internal),
            Err(ProviderError::Parse(
                "GraphQL: Docker: Unauthorized socket".into()
            ))
        );
        let internal = r#"{"errors":[{"message":"forbidden path","extensions":{"code":"INTERNAL_SERVER_ERROR"}}],"data":{"a":1}}"#;
        assert_eq!(parse_answer(internal).expect("data")["a"], 1);
    }

    #[test]
    fn other_errors_keep_the_data_or_become_parse() {
        let with_data = r#"{"errors":[{"message":"one field failed"}],"data":{"a":1}}"#;
        assert_eq!(parse_answer(with_data).expect("data")["a"], 1);
        let without = r#"{"errors":[{"message":"No parity history"}],"data":null}"#;
        assert_eq!(
            parse_answer(without),
            Err(ProviderError::Parse("GraphQL: No parity history".into()))
        );
    }

    #[test]
    fn no_json_or_no_data_is_parse() {
        assert!(matches!(
            parse_answer("<html>"),
            Err(ProviderError::Parse(_))
        ));
        assert!(matches!(parse_answer("{}"), Err(ProviderError::Parse(_))));
        assert!(matches!(
            parse_answer(r#"{"data":[1]}"#),
            Err(ProviderError::Parse(_))
        ));
    }
}
