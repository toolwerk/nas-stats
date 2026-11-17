//! One JSON-RPC 2.0 connection to TrueNAS over WebSocket, on top of the
//! pinned TLS stream of `nas-http`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nas_http::{Fingerprint, PinnedStream, ProviderError};
use serde_json::{Value, json};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// Path of the JSON-RPC endpoint below the address of the NAS.
const ENDPOINT: &str = "/api/current";

/// An error object of a JSON-RPC answer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    pub(crate) data: Value,
}

impl RpcError {
    /// The error as the app sees it. TrueNAS puts the details into `data`:
    /// `errname` (`EACCES` …), `error` (errno) and `reason`.
    pub(crate) fn into_provider_error(self) -> ProviderError {
        let errname = self.data.get("errname").and_then(Value::as_str);
        let errno = self.data.get("error").and_then(Value::as_i64);
        if errname == Some("EACCES") || errno == Some(13) {
            return ProviderError::PermissionDenied;
        }
        if self.code == -32601 {
            return ProviderError::Http(404);
        }
        let reason = self
            .data
            .get("reason")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
            .map_or(self.message, str::to_owned);
        ProviderError::Parse(reason)
    }
}

/// A WebSocket connection to `wss://<host>/api/current`. Calls go out one
/// after another; answers are matched by `id`, everything else (events,
/// answers to calls that ran out of time) is skipped.
///
/// A [`Session`] from [`crate::TruenasClient::session`] is logged in.
pub struct Session {
    ws: WebSocketStream<PinnedStream>,
    peer_certificate: Option<Vec<u8>>,
    next_id: u64,
    timeout: Duration,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("next_id", &self.next_id)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// `wss://<host[:port]><path without trailing />/api/current` for an
/// `https://` address. The address is read like `nas_http` reads it, so an
/// international host name arrives as Punycode and user info, query and
/// fragment are left out.
pub(crate) fn endpoint_url(base_url: &str) -> Result<String, ProviderError> {
    let base = url::Url::parse(base_url)
        .map_err(|e| ProviderError::Parse(format!("Adresse des NAS: {e}")))?;
    if base.scheme() != "https" {
        return Err(ProviderError::Parse(
            "Adresse des NAS: nur https:// ist erlaubt".into(),
        ));
    }
    let host = base
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| ProviderError::Parse("Adresse des NAS: Host fehlt".into()))?;
    let port = base.port().map(|p| format!(":{p}")).unwrap_or_default();
    Ok(format!(
        "wss://{host}{port}{}{ENDPOINT}",
        base.path().trim_end_matches('/')
    ))
}

impl Session {
    /// Opens the TLS connection with the pin and upgrades it to WebSocket.
    /// Nothing is logged in yet.
    pub(crate) async fn open(
        base_url: &str,
        pin: Fingerprint,
        timeout: Duration,
    ) -> Result<Self, ProviderError> {
        let url = endpoint_url(base_url)?;
        let stream = nas_http::connect_pinned(base_url, pin).await?;
        let peer_certificate = nas_http::peer_certificate(&stream);
        let upgrade = tokio::time::timeout(
            nas_http::TIMEOUT,
            tokio_tungstenite::client_async(url, stream),
        )
        .await
        .map_err(|_| ProviderError::Timeout)?;
        let (ws, _) = upgrade.map_err(|e| match e {
            WsError::Http(response) => match response.status().as_u16() {
                403 => ProviderError::PermissionDenied,
                status => ProviderError::Http(status),
            },
            other => ProviderError::Unreachable(format!("WebSocket: {other}")),
        })?;
        Ok(Self {
            ws,
            peer_certificate,
            next_id: 1,
            timeout,
        })
    }

    /// The certificate the NAS presented, in DER form.
    pub(crate) fn peer_certificate(&self) -> Option<&[u8]> {
        self.peer_certificate.as_deref()
    }

    /// Calls `method` with `params` (a JSON array, or an object for named
    /// parameters) and returns `result`.
    ///
    /// Errors: no answer within the time limit is [`ProviderError::Timeout`],
    /// a closed connection [`ProviderError::Unreachable`], an unreadable
    /// answer [`ProviderError::Parse`]. An error object becomes
    /// [`ProviderError::PermissionDenied`] (`EACCES`),
    /// [`ProviderError::Http`] 404 (method unknown) or
    /// [`ProviderError::Parse`] with the reason TrueNAS gives.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, ProviderError> {
        self.call_rpc(method, params)
            .await?
            .map_err(RpcError::into_provider_error)
    }

    /// Like [`Session::call`], with the error object unmapped. The outer
    /// error is the transport.
    pub(crate) async fn call_rpc(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Result<Value, RpcError>, ProviderError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let timeout = self.timeout;
        tokio::time::timeout(timeout, self.exchange(id, request.to_string()))
            .await
            .map_err(|_| ProviderError::Timeout)?
    }

    async fn exchange(
        &mut self,
        id: u64,
        request: String,
    ) -> Result<Result<Value, RpcError>, ProviderError> {
        self.ws
            .send(Message::text(request))
            .await
            .map_err(transport)?;
        loop {
            let message = match self.ws.next().await {
                Some(message) => message.map_err(transport)?,
                None => return Err(closed()),
            };
            let text = match &message {
                Message::Text(text) => text.as_bytes(),
                Message::Binary(bytes) => bytes.as_ref(),
                Message::Close(_) => return Err(closed()),
                // Pings are answered by tungstenite itself.
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            };
            let answer: Value = serde_json::from_slice(text)
                .map_err(|e| ProviderError::Parse(format!("JSON-RPC: {e}")))?;
            if answer.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = answer.get("error").filter(|e| !e.is_null()) {
                return Ok(Err(RpcError {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Fehler")
                        .to_owned(),
                    data: error.get("data").cloned().unwrap_or(Value::Null),
                }));
            }
            return Ok(Ok(answer.get("result").cloned().unwrap_or(Value::Null)));
        }
    }

    /// Closes the connection; errors on the way are of no interest.
    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.ws.close(None)).await;
    }
}

fn closed() -> ProviderError {
    ProviderError::Unreachable("Verbindung vom NAS geschlossen".into())
}

fn transport(e: WsError) -> ProviderError {
    match e {
        WsError::ConnectionClosed | WsError::AlreadyClosed => closed(),
        WsError::Utf8(_) => ProviderError::Parse(format!("WebSocket: {e}")),
        other => ProviderError::Unreachable(format!("WebSocket: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_keeps_host_port_and_path() {
        for (base, url) in [
            ("https://nas.local", "wss://nas.local/api/current"),
            ("https://nas.local/", "wss://nas.local/api/current"),
            (
                "https://nas.local:8443/",
                "wss://nas.local:8443/api/current",
            ),
            (
                "https://[2001:db8::1]:8443",
                "wss://[2001:db8::1]:8443/api/current",
            ),
            ("https://nas.local:443/", "wss://nas.local/api/current"),
            (
                "https://nas-büro.local/",
                "wss://xn--nas-bro-r2a.local/api/current",
            ),
            ("https://leser@nas.local/", "wss://nas.local/api/current"),
            (
                "https://proxy.lan/truenas//",
                "wss://proxy.lan/truenas/api/current",
            ),
            ("https://nas.local/ui?x=1", "wss://nas.local/ui/api/current"),
            ("HTTPS://nas.local", "wss://nas.local/api/current"),
        ] {
            assert_eq!(endpoint_url(base), Ok(url.to_owned()), "{base}");
        }
    }

    #[test]
    fn endpoint_needs_https_and_a_host() {
        for base in [
            "http://nas.local",
            "ws://nas.local",
            "wss://nas.local",
            "https://",
        ] {
            assert!(
                matches!(endpoint_url(base), Err(ProviderError::Parse(_))),
                "{base}"
            );
        }
    }

    fn error(code: i64, data: Value) -> RpcError {
        RpcError {
            code,
            message: "Fehlermeldung".into(),
            data,
        }
    }

    #[test]
    fn error_objects_map_to_provider_errors() {
        assert_eq!(
            error(-32001, json!({"errname": "EACCES", "reason": "x"})).into_provider_error(),
            ProviderError::PermissionDenied
        );
        assert_eq!(
            error(-32001, json!({"error": 13})).into_provider_error(),
            ProviderError::PermissionDenied
        );
        assert_eq!(
            error(-32601, Value::Null).into_provider_error(),
            ProviderError::Http(404)
        );
        assert_eq!(
            error(-32001, json!({"reason": "Pool fehlt"})).into_provider_error(),
            ProviderError::Parse("Pool fehlt".into())
        );
        assert_eq!(
            error(-32001, json!({"reason": ""})).into_provider_error(),
            ProviderError::Parse("Fehlermeldung".into())
        );
    }
}
