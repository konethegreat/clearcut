// Written by Kone Tshivhinda & Claude
//! The HTTP rules for the job endpoints, against a fake queue (no database).
//! The last group runs the real store.

use super::*;
use crate::tests::{bearer, json, send, FakeAuth, ADMIN_ONLY, READER, TENANT, WRITER};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use clearcut_core::jobs::{JobKind, JobState, Progress};
use clearcut_store::Error;
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

// ---------- test doubles ----------

#[derive(Debug, PartialEq)]
enum Call {
    List { tenant: Uuid, document: Uuid },
    Cancel { tenant: Uuid, job: Uuid, actor: Value },
}

/// A queue that records what it was asked and answers as told.
struct FakeJobs {
    calls: Mutex<Vec<Call>>,
    list: fn() -> Result<Vec<Job>, Error>,
    cancel: fn() -> Result<Job, Error>,
}

impl FakeJobs {
    fn new(list: fn() -> Result<Vec<Job>, Error>, cancel: fn() -> Result<Job, Error>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            list,
            cancel,
        })
    }

    fn calls(&self) -> std::sync::MutexGuard<'_, Vec<Call>> {
        self.calls.lock().unwrap()
    }
}

impl JobQueue for Arc<FakeJobs> {
    fn jobs_for_document<'a>(&'a self, tenant: TenantId, document: DocId) -> QueueFuture<'a, Vec<Job>> {
        Box::pin(async move {
            self.calls().push(Call::List {
                tenant: tenant.0,
                document: document.0,
            });
            (self.list)()
        })
    }

    fn cancel_job<'a>(&'a self, tenant: TenantId, job: Uuid, actor: Value) -> QueueFuture<'a, Job> {
        Box::pin(async move {
            self.calls().push(Call::Cancel {
                tenant: tenant.0,
                job,
                actor,
            });
            (self.cancel)()
        })
    }
}

fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_000_000 + seconds).unwrap()
}

const DOC: u128 = 0xD0C;
const JOB: u128 = 0x70B;

fn job(state: JobState) -> Job {
    Job {
        id: Uuid::from_u128(JOB),
        tenant: TenantId(Uuid::from_u128(TENANT)),
        document: DocId(Uuid::from_u128(DOC)),
        kind: JobKind::ProcessDocument,
        payload: serde_json::json!({ "secret_parameter": "do not show" }),
        state,
        priority: 3,
        attempts: 1,
        max_attempts: 5,
        run_after: at(10),
        deadline_at: at(172_800),
        leased_by: Some("worker-7".into()),
        lease_expires_at: Some(at(300)),
        progress: Progress::new(3, Some(12)).unwrap(),
        progress_note: Some("page 3 of 12".into()),
        checkpoint: Some(serde_json::json!({ "next_page": 4 })),
        last_error: None,
        cancel_requested_at: None,
        created_at: at(0),
        updated_at: at(20),
        started_at: Some(at(5)),
        finished_at: None,
    }
}

fn app_with(queue: &Arc<FakeJobs>) -> Router {
    let state = AppState::shared(FakeAuth::new()).with_jobs(queue.clone());
    finish(routes(), state)
}

fn list_request(token: Option<&str>, document: &str) -> Request<Body> {
    let mut builder = Request::get(format!("/v1/documents/{document}/jobs"));
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    builder.body(Body::empty()).unwrap()
}

fn cancel_request(token: Option<&str>, job: &str) -> Request<Body> {
    let mut builder = Request::post(format!("/v1/jobs/{job}/cancel"));
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, bearer(token));
    }
    builder.body(Body::empty()).unwrap()
}

fn doc_id() -> String {
    Uuid::from_u128(DOC).to_string()
}

fn job_id() -> String {
    Uuid::from_u128(JOB).to_string()
}

// ---------- who may do what ----------

#[tokio::test]
async fn listing_needs_a_key_with_documents_read() {
    let queue = FakeJobs::new(
        || Ok(vec![job(JobState::Running)]),
        || Ok(job(JobState::Cancelled)),
    );
    let app = app_with(&queue);

    let (status, headers, body) = send(&app, list_request(None, &doc_id())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers.get(header::WWW_AUTHENTICATE).unwrap(), "Bearer");
    assert_eq!(
        json(&body),
        serde_json::json!({ "error": "authentication failed" })
    );

    for token in [WRITER, ADMIN_ONLY] {
        let (status, _, body) = send(&app, list_request(Some(token), &doc_id())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token}");
        assert_eq!(json(&body)["required_scope"], "documents:read");
    }
    assert!(
        queue.calls().is_empty(),
        "the queue is never asked for a caller who may not see it"
    );

    let (status, ..) = send(&app, list_request(Some(READER), &doc_id())).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn cancelling_needs_a_key_with_documents_write() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);

    let (status, ..) = send(&app, cancel_request(None, &job_id())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    for token in [READER, ADMIN_ONLY] {
        let (status, _, body) = send(&app, cancel_request(Some(token), &job_id())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token}");
        assert_eq!(json(&body)["required_scope"], "documents:write");
    }
    assert!(
        queue.calls().is_empty(),
        "nothing is cancelled for a caller who may not"
    );

    let (status, ..) = send(&app, cancel_request(Some(WRITER), &job_id())).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn authentication_comes_before_the_path_is_even_read() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (status, ..) = send(&app, list_request(None, "not-a-uuid")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, ..) = send(&app, cancel_request(None, "not-a-uuid")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_id_that_is_not_a_uuid_is_a_400_for_an_authenticated_caller() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (status, _, body) = send(&app, list_request(Some(READER), "not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json(&body), serde_json::json!({ "error": "invalid id" }));
    let (status, _, body) = send(&app, cancel_request(Some(WRITER), "42")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json(&body), serde_json::json!({ "error": "invalid id" }));
    assert!(queue.calls().is_empty());
}

// ---------- listing ----------

#[tokio::test]
async fn the_queue_is_asked_for_the_callers_tenant_and_the_named_document() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    send(&app, list_request(Some(READER), &doc_id())).await;
    assert_eq!(
        *queue.calls(),
        [Call::List {
            tenant: Uuid::from_u128(TENANT),
            document: Uuid::from_u128(DOC)
        }]
    );
}

#[tokio::test]
async fn a_job_is_shown_with_its_status_and_progress_and_nothing_internal() {
    let queue = FakeJobs::new(
        || Ok(vec![job(JobState::Running)]),
        || Ok(job(JobState::Cancelled)),
    );
    let app = app_with(&queue);
    let (status, _, body) = send(&app, list_request(Some(READER), &doc_id())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json(&body),
        serde_json::json!({
            "jobs": [{
                "id": job_id(),
                "document_id": doc_id(),
                "kind": "process_document",
                "state": "running",
                "attempts": 1,
                "max_attempts": 5,
                "progress": { "done": 3, "total": 12, "percent": 25 },
                "progress_note": "page 3 of 12",
                "last_error": null,
                "cancel_requested": false,
                "created_at": "2023-11-14T22:13:20Z",
                "updated_at": "2023-11-14T22:13:40Z",
                "started_at": "2023-11-14T22:13:25Z",
                "finished_at": null,
                "run_after": "2023-11-14T22:13:30Z",
                "deadline_at": "2023-11-16T22:13:20Z",
            }]
        })
    );
    // Parameters, checkpoints, the worker's name and the lease are internal.
    for hidden in [
        "secret_parameter",
        "do not show",
        "next_page",
        "worker-7",
        "lease",
        "tenant",
        "payload",
        "checkpoint",
    ] {
        assert!(!body.contains(hidden), "{hidden} must not appear: {body}");
    }
}

#[tokio::test]
async fn the_jobs_come_back_in_the_order_the_queue_gave_them() {
    fn two() -> Result<Vec<Job>, Error> {
        let mut older = job(JobState::Succeeded);
        older.id = Uuid::from_u128(1);
        let mut newer = job(JobState::Queued);
        newer.id = Uuid::from_u128(2);
        Ok(vec![newer, older])
    }
    let queue = FakeJobs::new(two, || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (_, _, body) = send(&app, list_request(Some(READER), &doc_id())).await;
    let ids: Vec<_> = json(&body)["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids,
        [Uuid::from_u128(2).to_string(), Uuid::from_u128(1).to_string()]
    );
}

#[tokio::test]
async fn a_document_with_no_jobs_has_an_empty_list() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (status, _, body) = send(&app, list_request(Some(READER), &doc_id())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body), serde_json::json!({ "jobs": [] }));
}

#[tokio::test]
async fn a_finished_job_with_an_error_and_unknown_progress_total_is_shown_honestly() {
    fn finished() -> Result<Vec<Job>, Error> {
        let mut j = job(JobState::NeedsReview);
        j.progress = Progress::new(4, None).unwrap();
        j.progress_note = None;
        j.last_error = Some("totals never reconcile".into());
        j.cancel_requested_at = Some(at(30));
        j.finished_at = Some(at(40));
        Ok(vec![j])
    }
    let queue = FakeJobs::new(finished, || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (_, _, body) = send(&app, list_request(Some(READER), &doc_id())).await;
    let shown = &json(&body)["jobs"][0];
    assert_eq!(shown["state"], "needs_review");
    assert_eq!(
        shown["progress"],
        serde_json::json!({ "done": 4, "total": null, "percent": null })
    );
    assert_eq!(shown["progress_note"], serde_json::Value::Null);
    assert_eq!(shown["last_error"], "totals never reconcile");
    assert_eq!(shown["cancel_requested"], true);
    assert_eq!(shown["finished_at"], "2023-11-14T22:14:00Z");
}

#[tokio::test]
async fn an_unknown_document_is_a_404() {
    let queue = FakeJobs::new(|| Err(Error::DocumentNotFound), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (status, _, body) = send(&app, list_request(Some(READER), &doc_id())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body), serde_json::json!({ "error": "document not found" }));
}

// ---------- cancelling ----------

#[tokio::test]
async fn the_cancel_is_made_for_the_callers_tenant_and_audited_as_the_callers_key() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    send(&app, cancel_request(Some(WRITER), &job_id())).await;
    assert_eq!(
        *queue.calls(),
        [Call::Cancel {
            tenant: Uuid::from_u128(TENANT),
            job: Uuid::from_u128(JOB),
            // WRITER is the key with id 2 in the fake authenticator.
            actor: serde_json::json!({ "type": "api_key", "id": Uuid::from_u128(2).to_string() }),
        }]
    );
}

#[tokio::test]
async fn a_queued_job_is_cancelled_at_once_with_a_200() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let (status, _, body) = send(&app, cancel_request(Some(WRITER), &job_id())).await;
    assert_eq!(status, StatusCode::OK);
    let v = json(&body);
    assert_eq!(v["job"]["state"], "cancelled");
    assert_eq!(v["job"]["id"], job_id());
}

#[tokio::test]
async fn a_running_job_is_only_asked_to_stop_so_the_answer_is_a_202() {
    fn asked() -> Result<Job, Error> {
        let mut j = job(JobState::Running);
        j.cancel_requested_at = Some(at(30));
        Ok(j)
    }
    let queue = FakeJobs::new(|| Ok(vec![]), asked);
    let app = app_with(&queue);
    let (status, _, body) = send(&app, cancel_request(Some(WRITER), &job_id())).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let v = json(&body);
    assert_eq!(v["job"]["state"], "running");
    assert_eq!(v["job"]["cancel_requested"], true);
}

#[tokio::test]
async fn a_job_that_already_finished_is_left_alone_with_a_200() {
    fn done() -> Result<Job, Error> {
        let mut j = job(JobState::Succeeded);
        j.finished_at = Some(at(40));
        Ok(j)
    }
    let queue = FakeJobs::new(|| Ok(vec![]), done);
    let app = app_with(&queue);
    let (status, _, body) = send(&app, cancel_request(Some(WRITER), &job_id())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["job"]["state"], "succeeded");
}

#[tokio::test]
async fn cancelling_an_unknown_job_is_a_404() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Err(Error::JobNotFound));
    let app = app_with(&queue);
    let (status, _, body) = send(&app, cancel_request(Some(WRITER), &job_id())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json(&body), serde_json::json!({ "error": "job not found" }));
}

// ---------- failures and routing ----------

#[tokio::test]
async fn a_database_failure_is_a_500_that_says_nothing() {
    fn broken_list() -> Result<Vec<Job>, Error> {
        Err(Error::InvalidStoredJob("secret detail from the database".into()))
    }
    fn broken_cancel() -> Result<Job, Error> {
        Err(Error::InvalidStoredJob("secret detail from the database".into()))
    }
    let queue = FakeJobs::new(broken_list, broken_cancel);
    let app = app_with(&queue);
    for request in [
        list_request(Some(READER), &doc_id()),
        cancel_request(Some(WRITER), &job_id()),
    ] {
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json(&body), serde_json::json!({ "error": "internal error" }));
        assert!(!body.contains("secret detail"));
    }
}

#[tokio::test]
async fn with_no_queue_configured_both_routes_are_a_503() {
    let app = finish(routes(), AppState::shared(FakeAuth::new()));
    for request in [
        list_request(Some(READER), &doc_id()),
        cancel_request(Some(WRITER), &job_id()),
    ] {
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json(&body),
            serde_json::json!({ "error": "jobs are not available" })
        );
    }
}

#[tokio::test]
async fn each_route_answers_only_its_own_method() {
    let queue = FakeJobs::new(|| Ok(vec![]), || Ok(job(JobState::Cancelled)));
    let app = app_with(&queue);
    let wrong_list = Request::post(format!("/v1/documents/{}/jobs", doc_id()))
        .header(header::AUTHORIZATION, bearer(READER))
        .body(Body::empty())
        .unwrap();
    let wrong_cancel = Request::get(format!("/v1/jobs/{}/cancel", job_id()))
        .header(header::AUTHORIZATION, bearer(WRITER))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, wrong_list).await.0, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(send(&app, wrong_cancel).await.0, StatusCode::METHOD_NOT_ALLOWED);
    assert!(queue.calls().is_empty());
}

// ---------- the real store ----------

mod with_postgres {
    use super::*;
    use crate::tests::with_postgres::{first_key, service_store};
    use clearcut_core::auth::Scope;
    use clearcut_store::{NewDocument, NewJob};
    use sqlx::PgPool;

    fn document(seed: u8) -> NewDocument {
        NewDocument {
            original_filename: format!("invoice-{seed}.pdf"),
            content_type: "application/pdf".into(),
            byte_size: 10,
            sha256: [seed; 32],
            storage_key: format!("tenants/t/docs/{seed}"),
            page_count: None,
        }
    }

    /// A tenant with a read key and a write key, a document and a queued job.
    struct World {
        app: Router,
        store: clearcut_store::Store,
        tenant: TenantId,
        reader: String,
        writer: String,
        doc: DocId,
        job: Uuid,
    }

    async fn world(pool: &PgPool) -> World {
        let store = service_store(pool).await;
        let (tenant, reader, _) = first_key(pool, &[Scope::DocumentsRead]).await;
        // A second key for the same tenant, with the write scope.
        let owner = clearcut_store::Store::new(pool.clone());
        let writer = owner
            .create_api_key_as_operator(
                TenantId(tenant),
                &clearcut_store::NewApiKey {
                    name: "writer".into(),
                    scopes: vec![Scope::DocumentsWrite],
                    lifetime_days: None,
                },
            )
            .await
            .unwrap()
            .token
            .expose()
            .to_string();
        let tenant = TenantId(tenant);
        let doc = store.add_document(tenant, &document(1)).await.unwrap().id;
        let job = store
            .enqueue_job(
                tenant,
                &NewJob::new(doc, JobKind::ProcessDocument),
                &serde_json::json!({ "type": "system" }),
            )
            .await
            .unwrap()
            .id;
        let app = finish(routes(), AppState::new(store.clone()).with_jobs(store.clone()));
        World {
            app,
            store,
            tenant,
            reader,
            writer,
            doc,
            job,
        }
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_real_job_is_listed_cancelled_and_listed_again(pool: PgPool) {
        let w = world(&pool).await;
        let (status, _, body) = send(&w.app, list_request(Some(&w.reader), &w.doc.0.to_string())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let jobs = json(&body)["jobs"].as_array().unwrap().clone();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["id"], w.job.to_string());
        assert_eq!(jobs[0]["state"], "queued");

        let (status, _, body) = send(&w.app, cancel_request(Some(&w.writer), &w.job.to_string())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(json(&body)["job"]["state"], "cancelled");

        let (_, _, body) = send(&w.app, list_request(Some(&w.reader), &w.doc.0.to_string())).await;
        assert_eq!(json(&body)["jobs"][0]["state"], "cancelled");
        assert!(json(&body)["jobs"][0]["finished_at"].is_string());

        // The cancel was audited as the key that asked.
        let who: Value = sqlx::query_scalar(
            "select actor from audit_log where subject_id = $1 and action = 'job.cancelled'",
        )
        .bind(w.job.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(who["type"], "api_key");
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_running_real_job_gets_a_202_and_a_cancel_request(pool: PgPool) {
        let w = world(&pool).await;
        let claimed = w
            .store
            .claim_job(
                "w",
                clearcut_core::jobs::LeasePolicy::default(),
                &JobKind::ALL,
                None,
            )
            .await
            .unwrap()
            .expect("the job is due");
        assert_eq!(claimed.job.id, w.job);
        let (status, _, body) = send(&w.app, cancel_request(Some(&w.writer), &w.job.to_string())).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let v = json(&body);
        assert_eq!(v["job"]["state"], "running");
        assert_eq!(v["job"]["cancel_requested"], true);
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn another_tenants_key_sees_and_cancels_nothing(pool: PgPool) {
        let w = world(&pool).await;
        // A second tenant with its own keys.
        let owner = clearcut_store::Store::new(pool.clone());
        let other = owner.create_tenant("Other").await.unwrap();
        let mint = |scope: Scope| {
            let owner = owner.clone();
            async move {
                owner
                    .create_api_key_as_operator(
                        other,
                        &clearcut_store::NewApiKey {
                            name: "k".into(),
                            scopes: vec![scope],
                            lifetime_days: None,
                        },
                    )
                    .await
                    .unwrap()
                    .token
                    .expose()
                    .to_string()
            }
        };
        let other_reader = mint(Scope::DocumentsRead).await;
        let other_writer = mint(Scope::DocumentsWrite).await;

        let (status, _, body) = send(&w.app, list_request(Some(&other_reader), &w.doc.0.to_string())).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        let (status, _, body) = send(&w.app, cancel_request(Some(&other_writer), &w.job.to_string())).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        let still = w.store.get_job(w.tenant, w.job).await.unwrap();
        assert_eq!(still.state, JobState::Queued, "untouched");
    }

    #[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
    async fn a_document_that_does_not_exist_is_a_404_not_an_empty_list(pool: PgPool) {
        let w = world(&pool).await;
        let missing = Uuid::from_u128(0xDEAD).to_string();
        let (status, ..) = send(&w.app, list_request(Some(&w.reader), &missing)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, ..) = send(&w.app, cancel_request(Some(&w.writer), &missing)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
