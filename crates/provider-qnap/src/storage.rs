//! Volumes and disks.
//!
//! Sources, read-only:
//!
//! - `management/chartReq.cgi?chart_func=disk_usage&disk_select=all&include=all`
//!   for the volumes: `volumeList` has name, RAID type and a status code,
//!   `volumeUseList` total and free bytes, joined by `volumeValue`.
//! - `disk/qsmart.cgi?func=all_hd_data` for the disks: slot, model, serial,
//!   temperature and the SMART summary. It has no power-on hours.
//!
//! Storage pools are not read: `disk_manage.cgi?store=poolList` and its
//! siblings answer a GET with `unknown cmd format.` on QTS 5.2.11 (the web
//! interface POSTs them), so the pool of a volume stays `None`.

use nas_model::{Disk, Health, ProviderError, Volume, VolumeKind, hash_serial};
use roxmltree::{Document, Node};

use crate::client::QnapClient;
use crate::system::optional;
use crate::transport::Transport;
use crate::xml::{child, child_text, node_text, number, parse_qdoc};

const USAGE_PATH: &str = "management/chartReq.cgi";
const USAGE_QUERY: [(&str, &str); 3] = [
    ("chart_func", "disk_usage"),
    ("disk_select", "all"),
    ("include", "all"),
];
const SMART_PATH: &str = "disk/qsmart.cgi";
const SMART_QUERY: [(&str, &str); 1] = [("func", "all_hd_data")];

/// Upper bound for the entries of one answer, so a broken answer cannot
/// make the parser build millions of them.
const MAX_ENTRIES: usize = 256;

/// `volumeStatus` codes of `disk_usage` and what they mean. QTS sends only
/// the number; `0` is Ready, `44` the Warning QTS shows for a volume that is
/// more than 94 % full. An unknown code gives no health and is logged.
const VOLUME_STATUS: [(&str, Health); 2] = [("0", Health::Ok), ("44", Health::Warning)];

/// Volumes and disks of one QNAP.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QnapStorage {
    pub volumes: Vec<Volume>,
    pub disks: Vec<Disk>,
}

impl<T: Transport> QnapClient<T> {
    /// Reads the volumes and the disks.
    ///
    /// `nas_id` salts the hash of the disk serials. `PermissionDenied` or
    /// `Parse` of a request give an empty list for that part; any other
    /// error is returned.
    pub async fn storage(&self, nas_id: &str) -> Result<QnapStorage, ProviderError> {
        let volumes = match optional(self.get_xml(USAGE_PATH, &USAGE_QUERY).await)? {
            Some(body) => parse_volumes(&body),
            None => Vec::new(),
        };
        let disks = match optional(self.get_xml(SMART_PATH, &SMART_QUERY).await)? {
            Some(body) => parse_disks(&body, nas_id),
            None => Vec::new(),
        };
        Ok(QnapStorage { volumes, disks })
    }
}

fn elements<'a, 'input>(
    doc: &'a Document<'input>,
    path: &[&str],
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    let mut node = Some(doc.root_element());
    for step in path {
        node = node.and_then(|n| child(n, step));
    }
    node.into_iter()
        .flat_map(|n| n.children())
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
        .take(MAX_ENTRIES)
}

fn bytes(node: Node<'_, '_>, name: &str) -> Option<u64> {
    child_text(node, name)?.parse().ok()
}

/// Parses a `disk_usage` answer into volumes. Entries without
/// `volumeValue` are skipped; a value that is missing stays `None`.
pub(crate) fn parse_volumes(body: &str) -> Vec<Volume> {
    let Ok(doc) = parse_qdoc(body) else {
        return Vec::new();
    };
    let usage: Vec<(String, Option<u64>, Option<u64>)> =
        elements(&doc, &["volumeUseList"], "volumeUse")
            .filter_map(|n| {
                Some((
                    child_text(n, "volumeValue")?,
                    bytes(n, "total_size"),
                    bytes(n, "free_size"),
                ))
            })
            .collect();

    elements(&doc, &["volumeList"], "volume")
        .filter_map(|node| {
            let id = child_text(node, "volumeValue")?;
            let (total, free) = usage
                .iter()
                .find(|(value, _, _)| *value == id)
                .map_or((None, None), |(_, total, free)| (*total, *free));
            let total_bytes = total.filter(|t| *t > 0);
            let used_bytes = match (total_bytes, free) {
                (Some(total), Some(free)) if free <= total => Some(total - free),
                _ => None,
            };
            let status_text = child_text(node, "volumeStatus");
            let health = status_text.as_deref().and_then(|code| {
                let health = VOLUME_STATUS
                    .iter()
                    .find(|(known, _)| *known == code)
                    .map(|(_, health)| *health);
                if health.is_none() {
                    log::warn!("QNAP volume {id}: unknown status code {code}");
                }
                health
            });
            Some(Volume {
                name: child_text(node, "volumeLabel").unwrap_or_else(|| format!("Volume {id}")),
                id,
                kind: VolumeKind::Volume,
                pool_id: None,
                used_bytes,
                total_bytes,
                health,
                status_text,
                raid: child_text(node, "volumeStat"),
                scrub: None,
            })
        })
        .collect()
}

/// SMART summary text to health, compared without case. Unknown text is
/// `None` and logged.
fn smart_health(disk: &str, text: &str) -> Option<Health> {
    match text.to_ascii_lowercase().as_str() {
        "ok" | "good" | "normal" => Some(Health::Ok),
        "warning" => Some(Health::Warning),
        "abnormal" | "error" | "bad" => Some(Health::Critical),
        _ => {
            log::warn!("QNAP disk {disk}: unknown SMART summary {text}");
            None
        }
    }
}

/// Parses a `qsmart` answer into disks. Empty slots (no model) and entries
/// without `HDNo` are left out.
pub(crate) fn parse_disks(body: &str, nas_id: &str) -> Vec<Disk> {
    let Ok(doc) = parse_qdoc(body) else {
        return Vec::new();
    };
    elements(&doc, &["Disk_Info"], "entry")
        .filter_map(|node| {
            let id = child_text(node, "HDNo")?;
            let model = child_text(node, "Model")?;
            // `0:3` is slot 3 of enclosure 0; other enclosures keep the full id.
            let name = match id.split_once(':') {
                Some(("0", slot)) => format!("Platte {slot}"),
                _ => format!("Platte {id}"),
            };
            let smart_text = child_text(node, "Health");
            let health = smart_text.as_deref().and_then(|t| smart_health(&id, t));
            Some(Disk {
                serial_hash: child_text(node, "Serial").map(|s| hash_serial(nas_id, &s)),
                temperature_c: node_text(node, &["Temperature", "oC"])
                    .as_deref()
                    .and_then(number)
                    .filter(|t| *t >= 0.0),
                model: Some(model),
                health,
                smart_text,
                power_on_hours: None,
                spinning: None,
                id,
                name,
            })
        })
        .collect()
}
