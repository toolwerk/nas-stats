//! The HTTP requests the Synology client needs, so tests can answer from
//! fixtures instead of a pinned HTTPS server.

use nas_http::NasClient;
use nas_model::{BoxFuture, ProviderError};

/// Sends one request and returns the body as text. Implementations send
/// exactly once and never retry.
pub trait Transport: Send + Sync {
    /// `GET <base><path>?<query>`.
    fn get_text<'a>(
        &'a self,
        path: &'a str,
        query: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>>;

    /// `POST <base><path>` with a URL-encoded form.
    fn post_form_text<'a>(
        &'a self,
        path: &'a str,
        form: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>>;
}

impl Transport for NasClient {
    fn get_text<'a>(
        &'a self,
        path: &'a str,
        query: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(NasClient::get_text(self, path, query))
    }

    fn post_form_text<'a>(
        &'a self,
        path: &'a str,
        form: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        Box::pin(NasClient::post_form_text(self, path, form))
    }
}
