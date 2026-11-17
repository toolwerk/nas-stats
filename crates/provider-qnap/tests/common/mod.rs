//! Fake transport shared by the integration tests.

// Each test file uses a different part of this module.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Mutex;

use nas_model::{BoxFuture, ProviderError};
use provider_qnap::Transport;

/// One request as the fake saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: &'static str,
    pub path: String,
    pub params: Vec<(String, String)>,
}

/// Answers from a queue, records every request.
#[derive(Default)]
pub struct FakeTransport {
    pub answers: Mutex<VecDeque<Result<String, ProviderError>>>,
    pub calls: Mutex<Vec<Call>>,
    /// Yield once before answering, so concurrent requests overlap.
    pub slow: bool,
}

impl FakeTransport {
    pub fn new(answers: Vec<Result<&str, ProviderError>>) -> Self {
        Self {
            answers: Mutex::new(answers.into_iter().map(|a| a.map(str::to_owned)).collect()),
            calls: Mutex::default(),
            slow: false,
        }
    }

    pub fn slow(answers: Vec<Result<&str, ProviderError>>) -> Self {
        Self {
            slow: true,
            ..Self::new(answers)
        }
    }

    fn answer(
        &self,
        method: &'static str,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<String, ProviderError> {
        self.calls.lock().expect("calls").push(Call {
            method,
            path: path.to_owned(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        });
        self.answers
            .lock()
            .expect("answers")
            .pop_front()
            .expect("unexpected request")
    }
}

impl Transport for &FakeTransport {
    fn get_text<'a>(
        &'a self,
        path: &'a str,
        query: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let result = self.answer("GET", path, query);
        let slow = self.slow;
        Box::pin(async move {
            if slow {
                tokio::task::yield_now().await;
            }
            result
        })
    }

    /// Recorded as `GET` with each header as param `header:<name>` after
    /// the query.
    fn get_text_with_headers<'a>(
        &'a self,
        path: &'a str,
        query: &'a [(&'a str, &'a str)],
        headers: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let names: Vec<String> = headers.iter().map(|(k, _)| format!("header:{k}")).collect();
        let mut params: Vec<(&str, &str)> = query.to_vec();
        params.extend(
            names
                .iter()
                .map(String::as_str)
                .zip(headers.iter().map(|(_, v)| *v)),
        );
        let result = self.answer("GET", path, &params);
        let slow = self.slow;
        Box::pin(async move {
            if slow {
                tokio::task::yield_now().await;
            }
            result
        })
    }

    fn post_form_text<'a>(
        &'a self,
        path: &'a str,
        form: &'a [(&'a str, &'a str)],
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let result = self.answer("POST", path, form);
        let slow = self.slow;
        Box::pin(async move {
            if slow {
                tokio::task::yield_now().await;
            }
            result
        })
    }

    /// Recorded as `POST` with the body as param `json`.
    fn post_json_text<'a>(
        &'a self,
        path: &'a str,
        body: &'a str,
    ) -> BoxFuture<'a, Result<String, ProviderError>> {
        let result = self.answer("POST", path, &[("json", body)]);
        let slow = self.slow;
        Box::pin(async move {
            if slow {
                tokio::task::yield_now().await;
            }
            result
        })
    }
}

pub fn calls(fake: &FakeTransport) -> Vec<Call> {
    fake.calls.lock().expect("calls").clone()
}

pub fn logins(fake: &FakeTransport) -> usize {
    calls(fake)
        .iter()
        .filter(|c| c.path == "/cgi-bin/authLogin.cgi")
        .count()
}

pub fn param<'a>(call: &'a Call, key: &str) -> Option<&'a str> {
    call.params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}
