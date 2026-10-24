// Written by Kone Tshivhinda & Claude
//! Page reads against a real Postgres, as the restricted `clearcut_app` role the service
//! runs as. `pool` (from `sqlx::test`) is the owner connection: tests use it to look from
//! outside and to try rows the application could not write.

use super::*;
use crate::tests::{app_store, count, db_message, new_document, new_tenant, setup};
use clearcut_core::page_read::{OcrRun, PageEvidence, PageRead, ReadRoute};
use clearcut_core::text_quality::TextVerdict;
use sqlx::PgPool;
use uuid::Uuid;

const GOOD: &str = "TAX INVOICE\nInvoice No: INV-2024-001\nDate: 2024-03-01\n\
                    Description Quantity Price Amount\nTotal R1,150.00 VAT R150.00";
const BROKEN: &str = "Invoice \u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}";

fn read(page: u32, text: &str, ocr: Option<OcrRun>) -> PageRead {
    PageRead::decide(&PageEvidence {
        page,
        text,
        fonts: &[],
        ocr,
    })
}

fn good(page: u32) -> PageRead {
    read(page, GOOD, None)
}

/// Three pages, each a different kind of answer.
fn three_pages() -> Vec<PageRead> {
    vec![
        read(1, GOOD, Some(OcrRun { agreement: Some(950) })),
        read(2, BROKEN, Some(OcrRun { agreement: None })),
        read(3, "", None),
    ]
}

async fn stored_count(pool: &PgPool, doc: DocId) -> i64 {
    count(
        pool,
        &format!("select count(*) from page_reads where document_id = '{}'", doc.0),
    )
    .await
}

async fn stored_page_count(pool: &PgPool, doc: DocId) -> Option<i32> {
    sqlx::query_scalar("select page_count from documents where id = $1")
        .bind(doc.0)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ---------- storing and loading ----------

#[sqlx::test]
async fn what_is_stored_comes_back_unchanged_and_in_page_order(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let reads = three_pages();
    store.replace_page_reads(tenant, doc, &reads).await.unwrap();
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), reads);
    // The pages may be given in any order.
    let mut shuffled = reads.clone();
    shuffled.reverse();
    store.replace_page_reads(tenant, doc, &shuffled).await.unwrap();
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), reads);
}

#[sqlx::test]
async fn the_page_count_of_the_document_is_set_with_the_rows(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // The test document starts out claiming two pages.
    assert_eq!(stored_page_count(&pool, doc).await, Some(2));
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    assert_eq!(stored_page_count(&pool, doc).await, Some(3));
}

#[sqlx::test]
async fn a_document_with_no_reads_yet_loads_as_an_empty_list(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), Vec::new());
}

#[sqlx::test]
async fn running_the_stage_again_replaces_the_rows(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    let again = vec![good(1), good(2)];
    store.replace_page_reads(tenant, doc, &again).await.unwrap();
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), again);
    assert_eq!(stored_count(&pool, doc).await, 2);
    assert_eq!(stored_page_count(&pool, doc).await, Some(2));
    // The same answer twice is the same rows.
    store.replace_page_reads(tenant, doc, &again).await.unwrap();
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), again);
    assert_eq!(stored_count(&pool, doc).await, 2);
}

// ---------- what is refused before anything is written ----------

#[sqlx::test]
async fn pages_must_be_exactly_one_to_n(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let original = three_pages();
    store.replace_page_reads(tenant, doc, &original).await.unwrap();

    for (bad, expected) in [
        (vec![], "a document has at least one page"),
        (vec![good(1), good(3)], "the pages are not exactly 1 to n"),
        (vec![good(1), good(1)], "the pages are not exactly 1 to n"),
        (vec![good(0), good(1)], "the pages are not exactly 1 to n"),
        (vec![good(2)], "the pages are not exactly 1 to n"),
    ] {
        let error = store.replace_page_reads(tenant, doc, &bad).await.unwrap_err();
        assert!(
            matches!(&error, Error::InvalidPageReads(why) if *why == expected),
            "{error:?}"
        );
    }
    // Nothing was touched.
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), original);
    assert_eq!(stored_page_count(&pool, doc).await, Some(3));
}

#[sqlx::test]
async fn the_most_pages_one_document_may_have_is_ten_thousand(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    assert_eq!(MAX_PAGES, 10_000);
    let too_many: Vec<PageRead> = (1..=10_001).map(good).collect();
    let error = store
        .replace_page_reads(tenant, doc, &too_many)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::InvalidPageReads("a document has too many pages")),
        "{error:?}"
    );
    assert_eq!(stored_count(&pool, doc).await, 0);
}

#[sqlx::test]
async fn a_large_document_is_stored_whole(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let pages: Vec<PageRead> = (1..=500).map(good).collect();
    store.replace_page_reads(tenant, doc, &pages).await.unwrap();
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), pages);
    assert_eq!(stored_page_count(&pool, doc).await, Some(500));
}

// ---------- tenants ----------

#[sqlx::test]
async fn another_tenants_document_looks_like_it_does_not_exist(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();

    let error = store
        .replace_page_reads(other, doc, &three_pages())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
    let error = store.load_page_reads(other, doc).await.unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
    // Nothing of the owner's changed.
    assert_eq!(store.load_page_reads(tenant, doc).await.unwrap(), three_pages());
}

#[sqlx::test]
async fn a_document_that_does_not_exist_is_not_found(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let missing = DocId(Uuid::new_v4());
    let error = store
        .replace_page_reads(tenant, missing, &[good(1)])
        .await
        .unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
    let error = store.load_page_reads(tenant, missing).await.unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
}

#[sqlx::test]
async fn row_level_security_stops_a_row_for_another_tenant(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let mut tx = store.begin(other).await.unwrap();
    // The other tenant, acting for itself, tries to write a row for the first tenant's document.
    let result = sqlx::query(
        "insert into page_reads (tenant_id, document_id, page, text_source, verdict, route, analysed_with) \
         values ($1, $2, 1, 'text_layer', 'good', 'text_and_image', 'routing-v1')",
    )
    .bind(tenant.0)
    .bind(doc.0)
    .execute(&mut *tx)
    .await;
    assert!(db_message(result).contains("row-level security"));
}

#[sqlx::test]
async fn a_tenant_sees_only_its_own_rows_even_with_a_query_that_forgets_to_ask(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    store.replace_page_reads(other, theirs, &[good(1)]).await.unwrap();

    let mut tx = store.begin(tenant).await.unwrap();
    let seen: i64 = sqlx::query_scalar("select count(*) from page_reads")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen, 3);
    let mut tx = store.begin(other).await.unwrap();
    let seen: i64 = sqlx::query_scalar("select count(*) from page_reads")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen, 1);
}

// ---------- what the database itself refuses ----------

async fn insert_raw(pool: &PgPool, doc: DocId, tenant: TenantId, set: &str, values: &str) -> String {
    let sql = format!(
        "insert into page_reads (tenant_id, document_id, page, {set}) values ('{}', '{}', 1, {values})",
        tenant.0, doc.0
    );
    db_message(sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await)
}

#[sqlx::test]
async fn the_database_refuses_a_row_that_contradicts_the_rules(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let cols = "text_source, verdict, route, analysed_with";
    // A route that does not follow from the verdict.
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'text_layer', 'good', 'image_only', 'routing-v1'",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    // A good page whose text came from OCR.
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'ocr', 'good', 'text_and_image', 'routing-v1'",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    // A rejected page that still claims its text layer.
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'text_layer', 'unusable', 'image_only', 'routing-v1'",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    // Names that are not in the vocabulary.
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'magic', 'good', 'text_and_image', 'routing-v1'",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'text_layer', 'great', 'text_and_image', 'routing-v1'",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    // An empty or over-long rules version.
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'text_layer', 'good', 'text_and_image', ''",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    let long = "v".repeat(65);
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        &format!("'text_layer', 'good', 'text_and_image', '{long}'"),
    )
    .await;
    assert!(m.contains("check"), "{m}");
    assert_eq!(stored_count(&pool, doc).await, 0);
}

#[sqlx::test]
async fn the_database_refuses_an_agreement_out_of_range_and_unknown_reasons(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let ok = "'text_layer', 'good', 'text_and_image', 'routing-v1'";
    let cols = "text_source, verdict, route, analysed_with";
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        &format!("{cols}, ocr_agreement"),
        &format!("{ok}, 1001"),
    )
    .await;
    assert!(m.contains("check") || m.contains("out of range"), "{m}");
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        &format!("{cols}, ocr_agreement"),
        &format!("{ok}, -1"),
    )
    .await;
    assert!(m.contains("check"), "{m}");
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        &format!("{cols}, reasons"),
        &format!("{ok}, array['because']"),
    )
    .await;
    assert!(m.contains("check"), "{m}");
    let nine =
        "array['no_text','no_text','no_text','no_text','no_text','no_text','no_text','no_text','no_text']";
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        &format!("{cols}, reasons"),
        &format!("{ok}, {nine}"),
    )
    .await;
    assert!(m.contains("check"), "{m}");
    // The edges are fine.
    for agreement in [0, 1000] {
        let sql = format!(
            "insert into page_reads (tenant_id, document_id, page, {cols}, ocr_agreement) \
             values ('{}', '{}', {}, {ok}, {agreement})",
            tenant.0,
            doc.0,
            agreement + 1
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[sqlx::test]
async fn the_database_refuses_page_zero_and_a_second_row_for_a_page(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let ok = "text_layer', 'good', 'text_and_image', 'routing-v1";
    let sql = |page: i32| {
        format!(
            "insert into page_reads (tenant_id, document_id, page, text_source, verdict, route, analysed_with) \
             values ('{}', '{}', {page}, '{ok}')",
            tenant.0, doc.0
        )
    };
    assert!(db_message(sqlx::query(sqlx::AssertSqlSafe(sql(0))).execute(&pool).await).contains("check"));
    sqlx::query(sqlx::AssertSqlSafe(sql(1)))
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        db_message(sqlx::query(sqlx::AssertSqlSafe(sql(1))).execute(&pool).await).contains("duplicate key")
    );
}

#[sqlx::test]
async fn a_row_for_a_document_of_another_tenant_is_refused_by_the_foreign_key(pool: PgPool) {
    let (_, _, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let message = insert_raw(
        &pool,
        doc,
        other,
        "text_source, verdict, route, analysed_with",
        "'text_layer', 'good', 'text_and_image', 'routing-v1'",
    )
    .await;
    assert!(message.contains("foreign key"), "{message}");
}

// ---------- privileges ----------

#[sqlx::test]
async fn the_application_role_cannot_change_a_row_it_has_stored(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    let mut tx = store.begin(tenant).await.unwrap();
    let result = sqlx::query("update page_reads set route = 'image_only', verdict = 'unusable'")
        .execute(&mut *tx)
        .await;
    assert!(db_message(result).contains("permission denied"));
}

#[sqlx::test]
async fn the_application_role_cannot_empty_the_table(pool: PgPool) {
    let (store, _, _) = setup(&pool).await;
    let result = sqlx::query("truncate page_reads").execute(store.pool()).await;
    assert!(db_message(result).contains("permission denied"));
}

// ---------- reading back what is stored ----------

#[sqlx::test]
async fn a_stored_row_that_breaks_the_rules_is_refused_when_it_is_read(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // The same reason twice passes the database's checks but not the core's.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into page_reads (tenant_id, document_id, page, text_source, verdict, route, analysed_with, reasons) \
         values ('{}', '{}', 1, 'text_layer', 'good', 'text_and_image', 'routing-v1', array['ocr_agrees','ocr_agrees'])",
        tenant.0, doc.0
    )))
    .execute(&pool)
    .await
    .unwrap();
    let error = store.load_page_reads(tenant, doc).await.unwrap_err();
    assert!(matches!(error, Error::InvalidStoredPageRead(_)), "{error:?}");
}

#[sqlx::test]
async fn the_rows_use_the_agreed_names(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    type Row = (
        i32,
        String,
        String,
        String,
        Vec<String>,
        Option<i16>,
        String,
        String,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "select page, text_source, verdict, route, reasons, ocr_agreement, analysed_with, tenant_id::text \
         from page_reads order by page",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows[0].1, "text_layer");
    assert_eq!(rows[0].2, TextVerdict::Good.as_str());
    assert_eq!(rows[0].3, ReadRoute::TextAndImage.as_str());
    assert_eq!(rows[0].4, ["ocr_agrees"]);
    assert_eq!(rows[0].5, Some(950));
    assert_eq!(rows[0].6, "routing-v1");
    assert_eq!(rows[0].7, tenant.0.to_string());
    assert_eq!(
        (rows[1].1.as_str(), rows[1].2.as_str(), rows[1].3.as_str()),
        ("ocr", "unusable", "image_only")
    );
    assert_eq!(rows[1].4, ["replacement_characters"]);
    assert_eq!(rows[1].5, None);
    assert_eq!(
        (rows[2].1.as_str(), rows[2].2.as_str(), rows[2].3.as_str()),
        ("none", "unusable", "image_only")
    );
    assert_eq!(rows[2].4, ["no_text"]);
}

// ---------- erasure ----------

#[sqlx::test]
async fn erasing_a_document_takes_its_page_reads_with_it_and_leaves_the_others(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    store
        .replace_page_reads(other, theirs, &[good(1), good(2)])
        .await
        .unwrap();

    store.erase_document(tenant, doc).await.unwrap();

    assert_eq!(stored_count(&pool, doc).await, 0);
    assert_eq!(stored_count(&pool, theirs).await, 2);
    assert_eq!(
        count(
            &pool,
            &format!("select count(*) from documents where id = '{}'", doc.0)
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn the_eraser_can_read_and_delete_page_reads_but_not_add_or_change_them(pool: PgPool) {
    let _ = app_store(&pool).await; // runs the migrations
    let privileges: Vec<(String,)> = sqlx::query_as(
        "select privilege_type::text from information_schema.role_table_grants \
         where table_name = 'page_reads' and grantee = 'clearcut_eraser' order by 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let names: Vec<&str> = privileges.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(names, ["DELETE", "SELECT"]);
    let app: Vec<(String,)> = sqlx::query_as(
        "select privilege_type::text from information_schema.role_table_grants \
         where table_name = 'page_reads' and grantee = 'clearcut_app' order by 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let names: Vec<&str> = app.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(names, ["DELETE", "INSERT", "SELECT"]);
}

#[test]
fn exactly_ten_thousand_pages_pass_the_check_and_one_more_does_not() {
    let pages = |n: u32| -> Vec<PageRead> { (1..=n).map(good).collect() };
    assert!(check_pages(&pages(10_000)).is_ok());
    assert!(check_pages(&pages(10_001)).is_err());
    assert!(check_pages(&pages(1)).is_ok());
}

#[sqlx::test]
async fn a_page_has_one_row_however_the_second_one_differs(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let cols = "text_source, verdict, route, analysed_with";
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into page_reads (tenant_id, document_id, page, {cols})          values ('{}', '{}', 1, 'text_layer', 'good', 'text_and_image', 'routing-v1')",
        tenant.0, doc.0
    )))
    .execute(&pool)
    .await
    .unwrap();
    // Same page, different but consistent values.
    let message = insert_raw(
        &pool,
        doc,
        tenant,
        cols,
        "'ocr', 'unusable', 'image_only', 'routing-v2'",
    )
    .await;
    assert!(message.contains("duplicate key"), "{message}");
}

#[sqlx::test]
async fn a_doubtful_page_keeps_its_text_layer_as_the_hint(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let cols = "text_source, verdict, route, analysed_with";
    for source in ["ocr", "none"] {
        let values = format!("'{source}', 'needs_check', 'image_first', 'routing-v1'");
        let message = insert_raw(&pool, doc, tenant, cols, &values).await;
        assert!(message.contains("check"), "{source}: {message}");
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into page_reads (tenant_id, document_id, page, {cols})          values ('{}', '{}', 1, 'text_layer', 'needs_check', 'image_first', 'routing-v1')",
        tenant.0, doc.0
    )))
    .execute(&pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn a_tenant_cannot_delete_another_tenants_rows(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;
    store
        .replace_page_reads(tenant, doc, &three_pages())
        .await
        .unwrap();
    store.replace_page_reads(other, theirs, &[good(1)]).await.unwrap();

    // A delete with no where clause, acting for the first tenant.
    let mut tx = store.begin(tenant).await.unwrap();
    let removed = sqlx::query("delete from page_reads")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(removed.rows_affected(), 3);
    assert_eq!(stored_count(&pool, theirs).await, 1);
}
