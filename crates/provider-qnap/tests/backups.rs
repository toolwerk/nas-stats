//! Backup jobs of Hybrid Backup Sync 3 (issue #41) from reconstructed
//! answers, and the cases where the list stays empty without an error.

mod common;

use common::{FakeTransport, calls, logins, param};
use nas_model::{BackupKind, BackupResult, BackupTask, ProviderError};
use provider_qnap::QnapClient;

const LOGIN_OK: &str = include_str!("fixtures/login.xml");
const HBS_JOBS: &str = include_str!("fixtures/hbs_jobs.json");
const HBS_JOBS_MIXED: &str = include_str!("fixtures/hbs_jobs_mixed.json");

const JOBS_PATH: &str = "/cc3/v1/users/system/jobs";

#[tokio::test]
async fn reads_the_finished_backup_job() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(HBS_JOBS)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let backups = client.backups().await.expect("backups");

    assert_eq!(
        backups,
        vec![BackupTask {
            id: "job-0001".into(),
            name: "Backup 1".into(),
            kind: BackupKind::Hbs3,
            // 2026-11-17T01:00:03+01:00
            last_run: Some(1_794_873_603),
            last_result: Some(BackupResult::Ok),
            next_run: None,
        }]
    );

    let calls = calls(&fake);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].method, "GET");
    assert_eq!(calls[1].path, JOBS_PATH);
    assert_eq!(param(&calls[1], "header:X-QNAP-SID"), Some("REDACTED"));
    assert_eq!(
        param(&calls[1], "header:Content-Type"),
        Some("application/json")
    );
    // The session goes in the header only, never in the query.
    assert_eq!(param(&calls[1], "sid"), None);
}

#[tokio::test]
async fn keeps_only_enabled_backup_jobs_and_leaves_unknown_status_open() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(HBS_JOBS_MIXED)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    let backups = client.backups().await.expect("backups");

    assert_eq!(
        backups,
        vec![
            // Status `Running` is not known from a real NAS: unknown.
            BackupTask {
                id: "job-0002".into(),
                name: "Backup 2".into(),
                kind: BackupKind::Hbs3,
                // 2026-11-16T23:30:00Z
                last_run: Some(1_794_871_800),
                last_result: None,
                next_run: None,
            },
            // Without `enable` and without `statistics`: kept, all open.
            BackupTask {
                id: "job-0005".into(),
                name: "Backup 5".into(),
                kind: BackupKind::Hbs3,
                last_run: None,
                last_result: None,
                next_run: None,
            },
        ]
    );
    // `has_more` does not lead to a second page.
    assert_eq!(calls(&fake).len(), 2);
}

#[tokio::test]
async fn the_session_is_reused() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Ok(HBS_JOBS), Ok(HBS_JOBS)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    client.backups().await.expect("first");
    client.backups().await.expect("second");
    assert_eq!(logins(&fake), 1);
}

async fn empty_without_second_login(answer: Result<&str, ProviderError>) {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), answer]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.backups().await, Ok(Vec::new()));
    assert_eq!(logins(&fake), 1);
    assert_eq!(calls(&fake).len(), 2);
}

#[tokio::test]
async fn http_401_gives_no_backups_and_no_new_login() {
    empty_without_second_login(Err(ProviderError::LoginFailed)).await;
}

#[tokio::test]
async fn http_403_gives_no_backups() {
    empty_without_second_login(Err(ProviderError::PermissionDenied)).await;
}

#[tokio::test]
async fn http_404_gives_no_backups() {
    empty_without_second_login(Err(ProviderError::Http(404))).await;
}

#[tokio::test]
async fn http_502_gives_no_backups() {
    empty_without_second_login(Err(ProviderError::Http(502))).await;
}

#[tokio::test]
async fn http_503_gives_no_backups() {
    empty_without_second_login(Err(ProviderError::Http(503))).await;
}

#[tokio::test]
async fn an_answer_that_is_no_json_gives_no_backups() {
    empty_without_second_login(Ok("<html><body>Bad Gateway</body></html>")).await;
}

#[tokio::test]
async fn json_without_jobs_gives_no_backups() {
    empty_without_second_login(Ok(r#"{"error":"not found"}"#)).await;
}

#[tokio::test]
async fn a_timeout_is_returned() {
    let fake = FakeTransport::new(vec![Ok(LOGIN_OK), Err(ProviderError::Timeout)]);
    let client = QnapClient::new(&fake, "admin", "secret");
    assert_eq!(client.backups().await, Err(ProviderError::Timeout));
}

#[tokio::test]
async fn after_logout_nothing_is_sent() {
    let fake = FakeTransport::new(vec![]);
    let client = QnapClient::new(&fake, "admin", "secret");
    client.logout().await.expect("logout");
    assert_eq!(client.backups().await, Err(ProviderError::LoginFailed));
    assert!(calls(&fake).is_empty());
}
