// Written by Kone Tshivhinda & Claude
//! A tenant's sealed Anthropic key against a real Postgres, as the restricted `clearcut_app` role.
//! The store never sees a key in the clear: these tests use opaque bytes.

use super::*;
use crate::tests::{app_store, new_document, new_tenant, setup};
use crate::NewJob;
use clearcut_core::jobs::JobKind;
use sqlx::PgPool;

fn sealed(tail: &str) -> SealedKey {
    SealedKey {
        nonce: vec![7; 12],
        ciphertext: vec![9; 60],
        master_key_id: "k1".to_string(),
        last4: tail.to_string(),
    }
}

#[sqlx::test]
async fn a_key_is_stored_read_back_replaced_rejected_and_removed(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    assert_eq!(store.load_sealed_key(tenant).await.unwrap(), None);
    assert_eq!(store.sealed_key_status(tenant).await.unwrap(), None);

    let by = Uuid::from_u128(5);
    store
        .set_sealed_key(tenant, &sealed("abcd"), by, true)
        .await
        .unwrap();
    assert_eq!(store.load_sealed_key(tenant).await.unwrap(), Some(sealed("abcd")));
    let status = store.sealed_key_status(tenant).await.unwrap().unwrap();
    assert_eq!(
        (status.last4.as_str(), status.set_by, status.check_ok),
        ("abcd", Some(by), Some(true))
    );

    // Replacing keeps one row.
    store
        .set_sealed_key(tenant, &sealed("wxyz"), by, true)
        .await
        .unwrap();
    assert_eq!(
        store.load_sealed_key(tenant).await.unwrap().unwrap().last4,
        "wxyz"
    );

    // Once Anthropic has rejected it, it is no longer handed to the runner, but the screen can say so.
    store.mark_sealed_key_rejected(tenant).await.unwrap();
    assert_eq!(store.load_sealed_key(tenant).await.unwrap(), None);
    assert_eq!(
        store.sealed_key_status(tenant).await.unwrap().unwrap().check_ok,
        Some(false)
    );

    assert!(store.delete_sealed_key(tenant).await.unwrap());
    assert!(!store.delete_sealed_key(tenant).await.unwrap());
    assert_eq!(store.sealed_key_status(tenant).await.unwrap(), None);
}

#[sqlx::test]
async fn a_badly_shaped_key_is_refused_before_it_reaches_the_database(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    for bad in [
        SealedKey {
            nonce: vec![0; 11],
            ..sealed("abcd")
        },
        SealedKey {
            ciphertext: vec![0; 5],
            ..sealed("abcd")
        },
        SealedKey {
            ciphertext: vec![0; 513],
            ..sealed("abcd")
        },
        sealed("abc"),
        SealedKey {
            master_key_id: String::new(),
            ..sealed("abcd")
        },
    ] {
        let refused = store.set_sealed_key(tenant, &bad, Uuid::nil(), true).await;
        assert!(matches!(refused, Err(Error::InvalidTenantKey(_))), "{bad:?}");
    }
}

#[sqlx::test]
async fn the_application_role_cannot_read_the_table_and_cannot_reach_another_tenants_key(pool: PgPool) {
    let (store, a, _) = setup(&pool).await;
    let b = new_tenant(&pool, "Other").await.unwrap();
    store
        .set_sealed_key(a, &sealed("abcd"), Uuid::nil(), true)
        .await
        .unwrap();
    store
        .set_sealed_key(b, &sealed("wxyz"), Uuid::nil(), true)
        .await
        .unwrap();

    // No privilege on the table itself, for the application role.
    let direct = sqlx::query("select ciphertext from tenant_anthropic_keys")
        .execute(&store.pool)
        .await;
    assert!(direct.unwrap_err().to_string().contains("permission denied"));

    // Tenant A's session asking the functions about tenant B is refused.
    let mut tx = store.begin(a).await.unwrap();
    let asked = sqlx::query("select * from load_tenant_anthropic_key($1)")
        .bind(b.0)
        .execute(&mut *tx)
        .await;
    assert!(asked
        .unwrap_err()
        .to_string()
        .contains("does not match the current tenant"));
    drop(tx);
    // And each tenant sees only its own.
    assert_eq!(store.load_sealed_key(a).await.unwrap().unwrap().last4, "abcd");
    assert_eq!(store.load_sealed_key(b).await.unwrap().unwrap().last4, "wxyz");
}

#[sqlx::test]
async fn a_document_with_a_live_job_cannot_be_erased_and_nothing_is_lost(pool: PgPool) {
    let tenant = new_tenant(&pool, "Acme").await.unwrap();
    let store = app_store(&pool).await;
    let doc = store.add_document(tenant, &new_document(1)).await.unwrap().id;
    let job = store
        .enqueue_job(
            tenant,
            &NewJob::new(doc, JobKind::ProcessDocument),
            &serde_json::json!({}),
        )
        .await
        .unwrap();

    // Queued: refused, and the whole erasure is rolled back.
    assert!(matches!(
        store.erase_document(tenant, doc).await,
        Err(Error::DocumentBusy)
    ));
    assert!(store.document_info(tenant, doc).await.is_ok());
    assert_eq!(crate::tests::count(&pool, "select count(*) from jobs").await, 1);

    // Ended: it can be erased, and its job goes with it.
    sqlx::query("update jobs set state = 'succeeded', finished_at = now() where id = $1")
        .bind(job.id)
        .execute(&pool)
        .await
        .unwrap();
    store.erase_document(tenant, doc).await.unwrap();
    assert_eq!(crate::tests::count(&pool, "select count(*) from jobs").await, 0);
    assert_eq!(
        crate::tests::count(&pool, "select count(*) from documents").await,
        0
    );
}
