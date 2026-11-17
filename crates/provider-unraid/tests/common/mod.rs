//! Fake transport shared by the integration tests.

// Each test file uses a different part of this module.
#![allow(dead_code)]

use std::sync::Mutex;

use nas_model::{BoxFuture, ProviderError};
use provider_unraid::Transport;

/// One request as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The GraphQL query, taken from the JSON body.
    pub query: String,
    pub api_key: String,
}

/// Answers by the root field a query asks for, records every request.
pub struct FakeTransport {
    /// Root field (e.g. `docker`) and the answer for queries that ask it.
    answers: Vec<(&'static str, Result<String, ProviderError>)>,
    pub calls: Mutex<Vec<Call>>,
}

/// Root field of each query, in the order the provider sends them.
pub const MAIN: &str = "vars";
pub const METRICS: &str = "metrics";
pub const SHARES: &str = "shares";
pub const NOTIFICATIONS: &str = "notifications";
pub const UPS: &str = "upsDevices";
pub const DOCKER: &str = "docker";
pub const PARITY_HISTORY: &str = "parityHistory";

impl FakeTransport {
    pub fn new(answers: Vec<(&'static str, Result<&str, ProviderError>)>) -> Self {
        Self {
            answers: answers
                .into_iter()
                .map(|(field, answer)| (field, answer.map(str::to_owned)))
                .collect(),
            calls: Mutex::default(),
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls").clone()
    }

    /// Root fields in the order they were asked.
    pub fn fields(&self) -> Vec<&'static str> {
        self.calls()
            .iter()
            .map(|call| field_of(&call.query))
            .collect()
    }
}

/// The root field a query asks for, by the first known field it names.
fn field_of(query: &str) -> &'static str {
    [
        MAIN,
        METRICS,
        SHARES,
        NOTIFICATIONS,
        UPS,
        DOCKER,
        PARITY_HISTORY,
    ]
    .into_iter()
    .find(|field| query.contains(&format!("{{ {field}")))
    .unwrap_or("unknown")
}

impl Transport for &FakeTransport {
    fn post_graphql<'a>(
        &'a self,
        body: &'a str,
        api_key: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let json: serde_json::Value = serde_json::from_str(body).expect("body is JSON");
        let query = json["query"].as_str().expect("query").to_owned();
        let field = field_of(&query);
        self.calls.lock().expect("calls").push(Call {
            query,
            api_key: api_key.to_owned(),
        });
        let answer = self
            .answers
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, answer)| answer.clone())
            .unwrap_or_else(|| panic!("unexpected query for {field}"));
        Box::pin(std::future::ready(answer))
    }
}
