//! Throughput per interface from the recorded `bandwidth` answer (issue #1)
//! and the fallback to `QSM40bandwidth` with a rebuilt answer.

mod common;

use common::{FakeTransport, calls, param};
use nas_model::{NetInterface, ProviderError};
use provider_qnap::QnapClient;

const LOGIN_OK: &str = include_str!("fixtures/login.xml");
const BANDWIDTH: &str = include_str!("fixtures/bandwidth.xml");
const BANDWIDTH_EMPTY: &str = include_str!("fixtures/bandwidth_qsm40.xml");
const BANDWIDTH_ITEMS: &str = include_str!("fixtures/bandwidth_items.xml");

fn chart_funcs(fake: &FakeTransport) -> Vec<String> {
    calls(fake)
        .iter()
        .filter(|c| c.path == "/cgi-bin/management/chartReq.cgi")
        .map(|c| param(c, "chart_func").unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn reads_the_recorded_bandwidth() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(BANDWIDTH)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let interfaces = client.network().await.expect("network");

    // Only Adapter 1 has a link; rx and tx are bytes per 5 s.
    assert_eq!(
        interfaces,
        vec![NetInterface {
            name: "Adapter 1".into(),
            rx_bytes_per_sec: Some(16_746.0 / 5.0),
            tx_bytes_per_sec: Some(31_895.0 / 5.0),
        }]
    );
    assert_eq!(chart_funcs(&fake), vec!["bandwidth"]);
}

#[tokio::test]
async fn falls_back_to_qsm40_when_bandwidth_is_empty() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(BANDWIDTH_EMPTY), Ok(BANDWIDTH_ITEMS)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let interfaces = client.network().await.expect("network");

    assert_eq!(
        interfaces,
        vec![
            NetInterface {
                name: "Adapter 1".into(),
                rx_bytes_per_sec: Some(1000.0),
                tx_bytes_per_sec: Some(500.0),
            },
            NetInterface {
                name: "eth1".into(),
                rx_bytes_per_sec: Some(0.0),
                tx_bytes_per_sec: Some(0.0),
            },
        ]
    );
    assert_eq!(chart_funcs(&fake), vec!["bandwidth", "QSM40bandwidth"]);
}

#[tokio::test]
async fn both_empty_gives_no_interface() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(BANDWIDTH_EMPTY), Ok(BANDWIDTH_EMPTY)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.network().await, Ok(Vec::new()));
    assert_eq!(chart_funcs(&fake), vec!["bandwidth", "QSM40bandwidth"]);
}

#[tokio::test]
async fn a_network_error_is_returned() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Err(ProviderError::Timeout)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.network().await, Err(ProviderError::Timeout));
    assert_eq!(chart_funcs(&fake), vec!["bandwidth"]);
}
