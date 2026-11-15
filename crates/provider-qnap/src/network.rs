//! Throughput per network interface.
//!
//! Source, read-only: `management/chartReq.cgi?chart_func=bandwidth`, the
//! request the QTS Resource Monitor uses. `chart_func=QSM40bandwidth` is
//! only the fallback when `bandwidth` has no values: on QTS 5.2.11 it
//! answers `authPassed` 1 without any (issue #1).
//!
//! `rx` and `tx` are bytes per measuring interval (`bandwidth_interval`,
//! 5 s on QTS 5.2), not per second. Interfaces without a link
//! (`link_running` 0) are left out.

use nas_model::{NetInterface, ProviderError};
use roxmltree::Node;

use crate::client::QnapClient;
use crate::transport::Transport;
use crate::xml::{child, child_text, number, parse_qdoc};

const CHART_PATH: &str = "management/chartReq.cgi";
const CHART_FUNCS: [&str; 2] = ["bandwidth", "QSM40bandwidth"];

/// Measuring interval in seconds when the answer names none (the value
/// QTS 5.2 uses and qnapstats assumes).
const DEFAULT_INTERVAL: f64 = 5.0;

/// Upper bound for the interfaces of one answer, so a broken answer cannot
/// make the parser build millions of them.
const MAX_INTERFACES: usize = 64;

impl<T: Transport> QnapClient<T> {
    /// Reads the throughput of every interface with a link, in bytes per
    /// second.
    ///
    /// Asks `bandwidth` first and `QSM40bandwidth` only when `bandwidth`
    /// gives no interface. `PermissionDenied` or `Parse` of a request count
    /// as no interface; any other error is returned.
    pub async fn network(&self) -> Result<Vec<NetInterface>, ProviderError> {
        for func in CHART_FUNCS {
            let interfaces = match self.get_xml(CHART_PATH, &[("chart_func", func)]).await {
                Ok(body) => parse_bandwidth(&body),
                Err(ProviderError::PermissionDenied | ProviderError::Parse(_)) => Vec::new(),
                Err(e) => return Err(e),
            };
            if !interfaces.is_empty() {
                return Ok(interfaces);
            }
        }
        Ok(Vec::new())
    }
}

/// Parses a `bandwidth` or `QSM40bandwidth` answer. The values sit in
/// `bandwidth_info`, either as `item` children or as `eth<n>`/`wlan<n>`
/// children listed in `eth_index_list` and `wlan_index_list`. Anything
/// unreadable gives no interface; a rate that cannot be read stays `None`.
pub(crate) fn parse_bandwidth(body: &str) -> Vec<NetInterface> {
    let Ok(doc) = parse_qdoc(body) else {
        return Vec::new();
    };
    let root = doc.root_element();
    let Some(info) = child(root, "bandwidth_info") else {
        return Vec::new();
    };
    let interval = [root, info]
        .into_iter()
        .find_map(|node| {
            child_text(node, "bandwidth_interval")
                .as_deref()
                .and_then(number)
        })
        .filter(|i| *i > 0.0)
        .unwrap_or(DEFAULT_INTERVAL);

    let items: Vec<Node<'_, '_>> = info
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "item")
        .collect();
    let named: Vec<(Node<'_, '_>, Option<String>)> = if items.is_empty() {
        ["eth", "wlan"]
            .into_iter()
            .flat_map(|kind| {
                child_text(info, &format!("{kind}_index_list"))
                    .unwrap_or_default()
                    .split(',')
                    .map(str::trim)
                    .filter(|i| !i.is_empty())
                    .map(|i| format!("{kind}{i}"))
                    .collect::<Vec<_>>()
            })
            // A broken list may repeat an index or be huge: each key once,
            // and no more keys than interfaces are kept.
            .scan(Vec::new(), |seen: &mut Vec<String>, key| {
                if seen.contains(&key) {
                    return Some(None);
                }
                seen.push(key.clone());
                Some(Some(key))
            })
            .flatten()
            .take(MAX_INTERFACES)
            .filter_map(|key| child(info, &key).map(|node| (node, Some(key))))
            .collect()
    } else {
        items
            .into_iter()
            .map(|item| {
                let fallback = child_text(item, "name").or_else(|| child_text(item, "id"));
                (item, fallback)
            })
            .collect()
    };

    named
        .into_iter()
        .filter(|(node, _)| child_text(*node, "link_running").as_deref() != Some("0"))
        .filter_map(|(node, fallback)| {
            let name = child_text(node, "dname").or(fallback)?;
            let rate = |key: &str| {
                child_text(node, key)
                    .as_deref()
                    .and_then(number)
                    .filter(|v| *v >= 0.0)
                    .map(|v| v / interval)
            };
            Some(NetInterface {
                name,
                rx_bytes_per_sec: rate("rx"),
                tx_bytes_per_sec: rate("tx"),
            })
        })
        .take(MAX_INTERFACES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(inner: &str) -> String {
        format!("<QDocRoot><bandwidth_info>{inner}</bandwidth_info></QDocRoot>")
    }

    #[test]
    fn interval_in_bandwidth_info_and_missing_interval() {
        let body = info(
            "<bandwidth_interval>10</bandwidth_interval><eth_index_list>0</eth_index_list><eth0><rx>100</rx><tx>n/a</tx></eth0>",
        );
        assert_eq!(
            parse_bandwidth(&body),
            vec![NetInterface {
                name: "eth0".into(),
                rx_bytes_per_sec: Some(10.0),
                tx_bytes_per_sec: None,
            }]
        );
        let body = info("<eth_index_list>0</eth_index_list><eth0><rx>100</rx></eth0>");
        assert_eq!(parse_bandwidth(&body)[0].rx_bytes_per_sec, Some(20.0));
    }

    #[test]
    fn wlan_and_listed_interfaces_without_node() {
        let body = info(
            "<eth_index_list>0,7</eth_index_list><wlan_index_list>0</wlan_index_list><eth0><dname>LAN</dname></eth0><wlan0><dname>WLAN</dname></wlan0>",
        );
        let names: Vec<_> = parse_bandwidth(&body).into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["LAN", "WLAN"]);
    }

    #[test]
    fn an_item_without_any_name_is_left_out() {
        let body = info("<item><rx>1</rx></item>");
        assert!(parse_bandwidth(&body).is_empty());
    }

    #[test]
    fn a_repeated_index_gives_one_interface() {
        let body = info("<eth_index_list>0,0,0</eth_index_list><eth0><rx>5</rx></eth0>");
        assert_eq!(parse_bandwidth(&body).len(), 1);
    }

    #[test]
    fn many_items_are_cut() {
        let body = info(&"<item><id>x</id></item>".repeat(100));
        assert_eq!(parse_bandwidth(&body).len(), MAX_INTERFACES);
    }

    #[test]
    fn no_bandwidth_info_or_no_xml_is_empty() {
        assert!(parse_bandwidth("<QDocRoot><authPassed>1</authPassed></QDocRoot>").is_empty());
        assert!(parse_bandwidth("<html/>").is_empty());
    }
}
