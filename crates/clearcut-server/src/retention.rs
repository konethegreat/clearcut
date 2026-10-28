// Written by Kone Tshivhinda & Claude
//! The retention sweep: documents older than the configured period are erased, across every
//! account, through the same erasure a person's deletion uses.
//!
//! `CLEARCUT_RETENTION_DAYS` (default 365) sets the period; `0` turns the sweep off, and
//! documents are then kept until their account deletes them. Age is counted from when a document
//! was uploaded, so reading it again does not extend its life. What stays: the audit entry naming
//! the document and the call log's costs (neither holds content).

use clearcut_intake::{Intake, IntakeError};
use clearcut_objects::ObjectStore;
use clearcut_store::Store;
use std::time::Duration;
use tokio::sync::watch::Receiver;
use tokio::task::JoinHandle;

/// The most documents one query returns; a pass asks again while it is getting full batches.
const BATCH: u16 = 100;
/// The first pass waits a moment, so a restart does not compete with serving requests.
const FIRST_PASS: Duration = Duration::from_secs(60);
/// How often a pass runs after that. The period is counted in days, so an hour is close enough.
const BETWEEN_PASSES: Duration = Duration::from_secs(60 * 60);
/// The default period: one year.
const DEFAULT_DAYS: u16 = 365;
/// The longest period accepted: a century.
const MAX_DAYS: u16 = 36_500;

/// Reads `CLEARCUT_RETENTION_DAYS`: `None` when unset or empty (one year), `Some(None)` when it
/// is 0 (off), otherwise the period. `Err` is a message for the person starting the server.
pub(crate) fn retention_days(value: Option<&str>) -> Result<Option<u16>, String> {
    let Some(value) = value else {
        return Ok(Some(DEFAULT_DAYS));
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(Some(DEFAULT_DAYS));
    }
    match value.parse::<u16>() {
        Ok(0) => Ok(None),
        Ok(days) if days <= MAX_DAYS => Ok(Some(days)),
        _ => Err(format!(
            "CLEARCUT_RETENTION_DAYS must be a whole number of days from 0 to {MAX_DAYS} (0 turns it off)"
        )),
    }
}

/// Starts the sweep, which runs until the server stops.
pub(crate) fn start<O: ObjectStore + 'static>(
    intake: Intake<O>,
    store: Store,
    days: u16,
    mut stopped: Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if wait(FIRST_PASS, &mut stopped).await {
            return;
        }
        loop {
            sweep(&intake, &store, days).await;
            if wait(BETWEEN_PASSES, &mut stopped).await {
                return;
            }
        }
    })
}

/// True when the server is stopping while waiting.
async fn wait(duration: Duration, stopped: &mut Receiver<bool>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = stopped.wait_for(|stopped| *stopped) => true,
    }
}

/// One pass over every account. A document being read right now, or one already gone, is left
/// for the next pass; so is one whose erasure failed (the reason is logged, not the content).
async fn sweep<O: ObjectStore>(intake: &Intake<O>, store: &Store, days: u16) {
    let mut erased = 0u64;
    let mut left = 0u64;
    loop {
        let candidates = match store.retention_candidates(days, BATCH).await {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::error!(%error, "the retention sweep could not list aged documents");
                break;
            }
        };
        let full_batch = candidates.len() == usize::from(BATCH);
        let mut erased_now = 0u64;
        for (tenant, document) in candidates {
            match intake.erase_expired(tenant, document, days).await {
                Ok(()) => {
                    erased += 1;
                    erased_now += 1;
                }
                Err(IntakeError::Busy)
                | Err(IntakeError::Database(clearcut_store::Error::DocumentNotFound)) => left += 1,
                Err(error) => {
                    left += 1;
                    tracing::warn!(%error, document = %document.0, "the retention sweep could not erase a document");
                }
            }
        }
        // Nothing erased means asking again now changes nothing: whatever is left is busy or
        // failing, and the next pass will look again.
        if !full_batch || erased_now == 0 {
            break;
        }
    }
    if erased > 0 || left > 0 {
        tracing::info!(erased, left, days, "retention sweep finished");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clearcut_core::auth::{Principal, Scope};
    use clearcut_intake::{Limits, Upload};
    use clearcut_objects::LocalDisk;
    use sqlx::PgPool;
    use uuid::Uuid;

    const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF";
    const OTHER_PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>><</Type/Page>>\nendobj\n%%EOF";

    async fn app_store(pool: &PgPool) -> Store {
        let options = pool.connect_options().as_ref().clone().username("clearcut_app");
        let app_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap();
        Store::new(app_pool)
    }

    fn upload<'a>(bytes: &'a [u8], name: &str) -> Upload<&'a [u8]> {
        Upload {
            filename: Some(name.to_string()),
            content_type: "application/pdf".into(),
            declared_length: Some(bytes.len() as u64),
            body: bytes,
        }
    }

    #[test]
    fn the_period_is_365_days_unless_set_and_zero_turns_it_off() {
        assert_eq!(retention_days(None), Ok(Some(365)));
        assert_eq!(retention_days(Some(" ")), Ok(Some(365)));
        assert_eq!(retention_days(Some("30")), Ok(Some(30)));
        assert_eq!(retention_days(Some("0")), Ok(None));
        for bad in ["-1", "many", "1.5", "36501", "70000"] {
            assert!(retention_days(Some(bad)).is_err(), "{bad}");
        }
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_sweep_erases_only_documents_past_the_period(pool: PgPool) {
        let tenant = Store::new(pool.clone()).create_tenant("Acme").await.unwrap();
        let store = app_store(&pool).await;
        let dir = tempfile::tempdir().unwrap();
        let intake = Intake::new(
            store.clone(),
            LocalDisk::new(dir.path()),
            Limits {
                max_upload_bytes: 1_000_000,
                tenant_quota_bytes: u64::MAX,
            },
        );
        let by = Principal::new(tenant.0, Uuid::from_u128(1), [Scope::DocumentsWrite]);
        let old = intake.upload(&by, upload(PDF, "old.pdf")).await.unwrap();
        let fresh = intake.upload(&by, upload(OTHER_PDF, "fresh.pdf")).await.unwrap();
        sqlx::query("update documents set created_at = now() - interval '400 days' where id = $1")
            .bind(old.document.0)
            .execute(&pool)
            .await
            .unwrap();

        // Both documents still have their queued job; the sweep must cancel the aged one's job
        // itself, as a server with no worker would leave it queued forever.
        sweep(&intake, &store, 365).await;

        let left: Vec<Uuid> = sqlx::query_scalar("select id from documents")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, vec![fresh.document.0]);
        let jobs: Vec<Uuid> = sqlx::query_scalar("select id from jobs")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(jobs, vec![fresh.job.unwrap()]);
    }
}
