// Written by Kone Tshivhinda & Claude
//! The tenant key rules against a real Postgres (as the restricted `clearcut_app` role) and a
//! fake Anthropic check. No real key is used: the sentinel below must appear nowhere it should not.

use super::*;
use clearcut_anthropic::{KeyCheckKind, Lifecycle, ModelInfo};
use sqlx::PgPool;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

const MASTER: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const SENTINEL: &str = "sk-ant-api03-SENTINEL-do-not-leak-0123456789abcdefghij";
const SERVER_KEY: &str = "sk-ant-api03-SERVERKEY-shared-0123456789abcdefghijklm";

/// An Anthropic that answers as told and counts how often it was asked.
struct Fake {
    answer: fn() -> Result<KeyCheck, KeyCheckError>,
    asked: Arc<AtomicUsize>,
}

impl KeyChecker for Fake {
    fn check<'a>(&'a self, _key: &'a ApiKey) -> CheckFuture<'a> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { (self.answer)() })
    }
}

fn good() -> Result<KeyCheck, KeyCheckError> {
    let model = |id: &str, lifecycle| ModelInfo {
        id: id.to_string(),
        display_name: id.to_string(),
        lifecycle,
        max_input_tokens: None,
        max_tokens: None,
        structured_outputs: true,
        image_input: true,
        pdf_input: true,
        batch: false,
    };
    Ok(KeyCheck {
        models: vec![
            model("claude-haiku-5-5", Lifecycle::Active),
            model("old-model", Lifecycle::Retired),
        ],
        request_id: None,
    })
}

fn rejected() -> Result<KeyCheck, KeyCheckError> {
    Err(KeyCheckError {
        kind: KeyCheckKind::Rejected,
        request_id: None,
    })
}

fn unreachable() -> Result<KeyCheck, KeyCheckError> {
    Err(KeyCheckError {
        kind: KeyCheckKind::Unreachable,
        request_id: None,
    })
}

struct World {
    keys: TenantKeys,
    store: Store,
    pool: PgPool,
    admin: Principal,
    asked: Arc<AtomicUsize>,
}

async fn world(
    pool: &PgPool,
    master: bool,
    mode: KeyMode,
    server_key: bool,
    answer: fn() -> Result<KeyCheck, KeyCheckError>,
) -> World {
    let tenant = Store::new(pool.clone()).create_tenant("Acme").await.unwrap();
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let store = Store::new(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap(),
    );
    let asked = Arc::new(AtomicUsize::new(0));
    let keys = TenantKeys::new(
        store.clone(),
        master.then(|| MasterKey::from_base64(MASTER, "k1").unwrap()),
        mode,
        server_key.then(|| ApiKey::parse(SERVER_KEY).unwrap()),
        Fake {
            answer,
            asked: asked.clone(),
        },
    );
    World {
        keys,
        store,
        pool: pool.clone(),
        admin: Principal::new(tenant.0, Uuid::from_u128(9), [Scope::KeysAdmin]),
        asked,
    }
}

fn tenant_of(w: &World) -> TenantId {
    TenantId(w.admin.tenant())
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_key_is_checked_stored_sealed_audited_and_used_for_that_tenant(pool: PgPool) {
    let w = world(&pool, true, KeyMode::TenantOnly, false, good).await;
    let set = w.keys.set(&w.admin, SENTINEL).await.unwrap();
    assert_eq!(set.last4, "ghij");
    assert_eq!(set.models, ["claude-haiku-5-5"], "only models it can use");

    let view = w.keys.status(tenant_of(&w)).await.unwrap();
    assert_eq!(
        (view.configured, view.last4.as_deref(), view.source),
        (true, Some("ghij"), KeySource::Tenant)
    );

    match w.keys.resolve(tenant_of(&w)).await.unwrap() {
        Resolved::Key(key, source) => {
            assert_eq!(source, KeySource::Tenant);
            assert_eq!(key, ApiKey::parse(SENTINEL).unwrap());
        }
        _ => panic!("the tenant's key should be used"),
    }

    // The sentinel is nowhere in the database: not in the table, not in the audit log.
    let everywhere: String = sqlx::query_scalar(
        "select coalesce(string_agg(encode(ciphertext, 'escape') || last4, ' '), '') from tenant_anthropic_keys",
    )
    .fetch_one(&w.pool)
    .await
    .unwrap();
    let audit: String = sqlx::query_scalar(
        "select coalesce(string_agg(detail::text || actor::text, ' '), '') from audit_log",
    )
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert!(!everywhere.contains("SENTINEL") && !audit.contains("SENTINEL"));
    assert!(audit.contains("anthropic_key.set") || audit.contains("ghij"));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_key_that_is_not_allowed_or_not_good_is_refused_and_nothing_is_stored(pool: PgPool) {
    // Without keys:admin: refused before anything else.
    let w = world(&pool, true, KeyMode::TenantOnly, false, good).await;
    let reader = Principal::new(w.admin.tenant(), Uuid::from_u128(3), [Scope::DocumentsWrite]);
    assert!(matches!(
        w.keys.set(&reader, SENTINEL).await,
        Err(KeysError::Forbidden)
    ));
    assert!(matches!(w.keys.remove(&reader).await, Err(KeysError::Forbidden)));

    // An admin key and nonsense are refused without asking Anthropic.
    for text in ["sk-ant-admin01-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "hello"] {
        assert!(matches!(
            w.keys.set(&w.admin, text).await,
            Err(KeysError::BadKey(_))
        ));
    }
    assert_eq!(w.asked.load(Ordering::SeqCst), 0);

    // Anthropic says no: refused, with a fixed sentence that does not repeat the key.
    let w2 = world(&pool, true, KeyMode::TenantOnly, false, rejected).await;
    let refused = w2.keys.set(&w2.admin, SENTINEL).await.unwrap_err();
    assert!(matches!(refused, KeysError::Refused(_)));
    assert!(!refused.to_string().contains("SENTINEL"));
    // Anthropic cannot be reached: not stored either, but not blamed on the key.
    let w3 = world(&pool, true, KeyMode::TenantOnly, false, unreachable).await;
    assert!(matches!(
        w3.keys.set(&w3.admin, SENTINEL).await,
        Err(KeysError::CheckUnavailable(_))
    ));
    for world in [&w2, &w3] {
        assert!(!world.keys.status(tenant_of(world)).await.unwrap().configured);
    }
    // No master key configured: tenant keys are off.
    let w4 = world(&pool, false, KeyMode::Shared, false, good).await;
    assert!(matches!(
        w4.keys.set(&w4.admin, SENTINEL).await,
        Err(KeysError::NotConfigured)
    ));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_servers_own_key_is_used_only_in_shared_mode_and_a_tenants_key_always_wins(pool: PgPool) {
    // tenant-only: the server key is never used, even though one is configured.
    let only = world(&pool, true, KeyMode::TenantOnly, true, good).await;
    assert!(matches!(
        only.keys.resolve(tenant_of(&only)).await.unwrap(),
        Resolved::NoKey
    ));
    assert_eq!(
        only.keys.status(tenant_of(&only)).await.unwrap().source,
        KeySource::None
    );

    // shared: the server key fills in for a tenant with none, and a tenant's own key wins.
    let shared = world(&pool, true, KeyMode::Shared, true, good).await;
    match shared.keys.resolve(tenant_of(&shared)).await.unwrap() {
        Resolved::Key(_, source) => assert_eq!(source, KeySource::Server),
        _ => panic!("shared mode should fall back to the server key"),
    }
    shared.keys.set(&shared.admin, SENTINEL).await.unwrap();
    match shared.keys.resolve(tenant_of(&shared)).await.unwrap() {
        Resolved::Key(key, source) => {
            assert_eq!(source, KeySource::Tenant);
            assert_eq!(key, ApiKey::parse(SENTINEL).unwrap());
        }
        _ => panic!("the tenant's key should win"),
    }
    // Removed: back to the server key in shared mode.
    assert!(shared.keys.remove(&shared.admin).await.unwrap());
    assert!(!shared.keys.remove(&shared.admin).await.unwrap());
    assert!(matches!(
        shared.keys.resolve(tenant_of(&shared)).await.unwrap(),
        Resolved::Key(_, KeySource::Server)
    ));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_key_anthropic_rejects_is_not_used_and_not_quietly_replaced_by_the_servers(pool: PgPool) {
    let w = world(&pool, true, KeyMode::Shared, true, good).await;
    w.keys.set(&w.admin, SENTINEL).await.unwrap();
    w.keys.mark_rejected(tenant_of(&w)).await.unwrap();

    // The tenant must set it again: its own money is not spent on the server's key behind its back.
    assert!(matches!(
        w.keys.resolve(tenant_of(&w)).await.unwrap(),
        Resolved::Rejected
    ));
    let view = w.keys.status(tenant_of(&w)).await.unwrap();
    assert_eq!((view.configured, view.check_ok), (true, Some(false)));

    // Setting a new one makes it usable again.
    w.keys.set(&w.admin, SENTINEL).await.unwrap();
    assert!(matches!(
        w.keys.resolve(tenant_of(&w)).await.unwrap(),
        Resolved::Key(_, KeySource::Tenant)
    ));
    let _ = &w.store;
}

#[test]
fn the_key_mode_reads_only_the_two_known_words() {
    assert_eq!(KeyMode::parse(None), Ok(KeyMode::Shared));
    assert_eq!(KeyMode::parse(Some(" ")), Ok(KeyMode::Shared));
    assert_eq!(KeyMode::parse(Some("shared")), Ok(KeyMode::Shared));
    assert_eq!(KeyMode::parse(Some("tenant-only")), Ok(KeyMode::TenantOnly));
    assert!(KeyMode::parse(Some("tenant_only")).is_err());
}
