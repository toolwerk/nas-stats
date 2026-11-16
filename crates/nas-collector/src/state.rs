//! What the collector keeps across restarts besides the archive: per NAS
//! the start of the last query, the block after a rejected login and the
//! access (hash of user, password, address and fingerprint) it was blocked
//! with. A block stays until the access changes, so a restart of the
//! container never repeats a rejected login.
//!
//! The file `poll-state.json` is written after every query, to a temporary
//! file first and then renamed, so a crash leaves the old or the new state.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use stats_core::PollState;

/// Name of the file in the data directory.
pub const FILE: &str = "poll-state.json";

/// What is kept of one NAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub last_attempt_at: Option<i64>,
    pub blocked: bool,
    /// [`crate::config::NasConfig::access`] at the time of the query.
    pub access: String,
}

/// Start values for the planner: the time of the last query, and the block
/// only when the access did not change since.
pub fn start_values(saved: Option<&Saved>, access: &str) -> (Option<i64>, bool) {
    match saved {
        Some(saved) => (
            saved.last_attempt_at,
            saved.blocked && saved.access == access,
        ),
        None => (None, false),
    }
}

/// The state file of a data directory.
pub struct StateFile {
    path: PathBuf,
    /// Access of every configured NAS, by id.
    access: BTreeMap<String, String>,
    saved: Mutex<BTreeMap<String, Saved>>,
}

impl StateFile {
    /// Reads the file in `dir`. A missing file is an empty state; an
    /// unreadable one too, with the reason as second value for the log.
    pub fn open(dir: &Path, access: BTreeMap<String, String>) -> (Self, Option<String>) {
        let path = dir.join(FILE);
        let (saved, warning) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<BTreeMap<String, Saved>>(&bytes) {
                Ok(saved) => (saved, None),
                Err(e) => (
                    BTreeMap::new(),
                    Some(format!(
                        "{} nicht lesbar, starte ohne Stand: {e}",
                        path.display()
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(e) => (
                BTreeMap::new(),
                Some(format!(
                    "{} nicht lesbar, starte ohne Stand: {e}",
                    path.display()
                )),
            ),
        };
        // NAS no longer in the configuration are dropped with the next write.
        let saved = saved
            .into_iter()
            .filter(|(id, _)| access.contains_key(id))
            .collect();
        (
            Self {
                path,
                access,
                saved: Mutex::new(saved),
            },
            warning,
        )
    }

    /// What was kept of a NAS.
    pub fn saved(&self, nas_id: &str) -> Option<Saved> {
        self.lock().get(nas_id).cloned()
    }

    /// Records a finished query and writes the file.
    pub fn record(&self, state: &PollState) -> std::io::Result<()> {
        let Some(access) = self.access.get(&state.nas_id) else {
            return Ok(());
        };
        let mut saved = self.lock();
        saved.insert(
            state.nas_id.clone(),
            Saved {
                last_attempt_at: state.last_attempt_at,
                blocked: state.blocked,
                access: access.clone(),
            },
        );
        let json = serde_json::to_vec_pretty(&*saved).map_err(std::io::Error::other)?;
        write_atomic(&self.path, &json)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Saved>> {
        self.saved.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Writes `bytes` to a temporary file beside `path` and renames it.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nas_model::ProviderError;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nas-collector-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn polled(nas_id: &str, at: i64, blocked: bool) -> PollState {
        PollState {
            nas_id: nas_id.into(),
            snapshot: None,
            error: blocked.then_some(ProviderError::LoginFailed),
            error_at: blocked.then_some(at),
            last_attempt_at: Some(at),
            blocked,
            running: false,
        }
    }

    fn access(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(id, access)| ((*id).to_owned(), (*access).to_owned()))
            .collect()
    }

    #[test]
    fn block_survives_a_restart_until_the_access_changes() {
        let dir = temp_dir("block");
        let (file, warning) = StateFile::open(&dir, access(&[("keller", "a1")]));
        assert_eq!(warning, None);
        assert_eq!(file.saved("keller"), None);
        file.record(&polled("keller", 1_000, true))
            .expect("written");

        let (file, warning) = StateFile::open(&dir, access(&[("keller", "a1")]));
        assert_eq!(warning, None);
        let saved = file.saved("keller");
        assert_eq!(start_values(saved.as_ref(), "a1"), (Some(1_000), true));
        // New password: the time stays, the block is lifted.
        assert_eq!(start_values(saved.as_ref(), "a2"), (Some(1_000), false));
        assert_eq!(start_values(None, "a1"), (None, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nas_left_out_of_the_configuration_is_dropped() {
        let dir = temp_dir("dropped");
        let (file, _) = StateFile::open(&dir, access(&[("a", "x"), ("b", "y")]));
        file.record(&polled("a", 1, false)).expect("written");
        file.record(&polled("b", 2, false)).expect("written");
        let (file, _) = StateFile::open(&dir, access(&[("b", "y")]));
        assert_eq!(file.saved("a"), None);
        file.record(&polled("b", 3, false)).expect("written");
        let text = std::fs::read_to_string(dir.join(FILE)).expect("file");
        assert!(!text.contains("\"a\""), "{text}");
        assert!(!dir.join("poll-state.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_file_starts_empty_with_a_warning() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join(FILE), b"{kaputt").expect("written");
        let (file, warning) = StateFile::open(&dir, access(&[("keller", "a1")]));
        assert!(warning.expect("warning").contains("nicht lesbar"));
        assert_eq!(file.saved("keller"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
