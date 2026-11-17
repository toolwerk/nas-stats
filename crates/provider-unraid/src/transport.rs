//! The one HTTP request the Unraid client needs, so tests can answer from
//! fixtures instead of a pinned HTTPS server.

use nas_http::NasClient;
use nas_model::{BoxFuture, ProviderError};

/// Path of the GraphQL API.
pub(crate) const GRAPHQL_PATH: &str = "/graphql";

/// Sends one GraphQL request and returns the body as text. Implementations
/// send exactly once and never retry.
pub trait Transport: Send + Sync {
    /// `POST <base>/graphql` with the JSON `body` and the header
    /// `x-api-key: <api_key>`.
    fn post_graphql<'a>(
        &'a self,
        body: &'a str,
        api_key: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>>;
}

impl Transport for NasClient {
    fn post_graphql<'a>(
        &'a self,
        body: &'a str,
        api_key: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(NasClient::post_json_text_api_key(
            self,
            GRAPHQL_PATH,
            body,
            api_key,
        ))
    }
}
