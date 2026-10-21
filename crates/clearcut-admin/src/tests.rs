// Written by Kone Tshivhinda & Claude
//! The commands run against a real Postgres (`DATABASE_URL`, one throwaway
//! database per test). The `pool` that `sqlx::test` provides is a superuser
//! connection, which stands in for the operator's owner connection.

use super::*;
use clap::Parser;
use clearcut_core::auth::Scope;
use clearcut_store::{keys, Store, TenantId};
use sqlx::PgPool;
use uuid::Uuid;

const TENANT: &str = "11111111-1111-4111-8111-111111111111";

fn parse(args: &[&str]) -> Result<Command, clap::Error> {
    let mut all = vec!["clearcut-admin"];
    all.extend_from_slice(args);
    Cli::try_parse_from(all).map(|cli| cli.command)
}

fn parse_error(args: &[&str]) -> String {
    parse(args).expect_err("should not parse").to_string()
}

// ---------- reading the command line ----------

#[test]
fn the_four_commands_parse() {
    assert!(matches!(parse(&["migrate"]), Ok(Command::Migrate)));
    assert!(matches!(
        parse(&["create-tenant", "Acme Advisory"]),
        Ok(Command::CreateTenant { name }) if name == "Acme Advisory"
    ));
    assert!(matches!(
        parse(&["revoke-key", "--tenant", TENANT, "--key", TENANT]),
        Ok(Command::RevokeKey { .. })
    ));
    match parse(&[
        "create-key",
        "--tenant",
        TENANT,
        "--name",
        "ci uploader",
        "--scopes",
        "documents:write,events:read",
        "--days",
        "30",
    ]) {
        Ok(Command::CreateKey {
            tenant,
            name,
            scopes,
            days,
        }) => {
            assert_eq!(tenant, Uuid::parse_str(TENANT).unwrap());
            assert_eq!(name, "ci uploader");
            assert_eq!(scopes, [Scope::DocumentsWrite, Scope::EventsRead]);
            assert_eq!(days, Some(30));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn the_life_of_a_key_defaults_when_days_is_left_out() {
    match parse(&[
        "create-key",
        "--tenant",
        TENANT,
        "--name",
        "k",
        "--scopes",
        "keys:admin",
    ]) {
        Ok(Command::CreateKey { days, .. }) => assert_eq!(days, None),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn an_unknown_scope_is_refused_and_the_valid_ones_are_listed() {
    let message = parse_error(&[
        "create-key",
        "--tenant",
        TENANT,
        "--name",
        "k",
        "--scopes",
        "documents:delete",
    ]);
    assert!(message.contains("unknown scope"), "{message}");
    for scope in Scope::ALL {
        assert!(message.contains(scope.as_str()), "{message}");
    }
}

#[test]
fn loosely_written_scope_lists_are_refused() {
    for scopes in [
        "",
        "documents:read,",
        ",documents:read",
        "documents:read, events:read",
        "Documents:Read",
        "*",
    ] {
        assert!(
            parse(&[
                "create-key",
                "--tenant",
                TENANT,
                "--name",
                "k",
                "--scopes",
                scopes
            ])
            .is_err(),
            "{scopes:?}"
        );
    }
}

#[test]
fn a_key_cannot_be_asked_to_live_for_zero_or_more_than_365_days() {
    for days in ["0", "366", "-1", "abc", "1000000"] {
        assert!(
            parse(&[
                "create-key",
                "--tenant",
                TENANT,
                "--name",
                "k",
                "--scopes",
                "keys:admin",
                "--days",
                days
            ])
            .is_err(),
            "{days}"
        );
    }
    for days in ["1", "365"] {
        assert!(
            parse(&[
                "create-key",
                "--tenant",
                TENANT,
                "--name",
                "k",
                "--scopes",
                "keys:admin",
                "--days",
                days
            ])
            .is_ok(),
            "{days}"
        );
    }
}

#[test]
fn required_arguments_and_well_formed_ids_are_enforced() {
    assert!(parse(&[]).is_err());
    assert!(parse(&["create-tenant"]).is_err());
    assert!(parse(&["create-key", "--name", "k", "--scopes", "keys:admin"]).is_err());
    assert!(parse(&["create-key", "--tenant", TENANT, "--scopes", "keys:admin"]).is_err());
    assert!(parse(&["create-key", "--tenant", TENANT, "--name", "k"]).is_err());
    assert!(parse(&["revoke-key", "--tenant", TENANT]).is_err());
    assert!(parse(&["revoke-key", "--key", TENANT]).is_err());
    assert!(parse(&[
        "create-key",
        "--tenant",
        "nope",
        "--name",
        "k",
        "--scopes",
        "keys:admin"
    ])
    .is_err());
    assert!(parse(&["delete-everything"]).is_err());
}

#[test]
fn help_names_every_command() {
    let help = parse_error(&["--help"]);
    for command in ["migrate", "create-tenant", "create-key", "revoke-key"] {
        assert!(help.contains(command), "{help}");
    }
}

// ---------- running the commands ----------

struct Run {
    out: String,
    err: String,
}

async fn run_command(store: &Store, command: Command) -> (Result<(), AdminError>, Run) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = run(command, store, &mut out, &mut err).await;
    (
        result,
        Run {
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        },
    )
}

fn create_key(tenant: Uuid, name: &str, scopes: &[Scope], days: Option<u32>) -> Command {
    Command::CreateKey {
        tenant,
        name: name.into(),
        scopes: scopes.to_vec(),
        days,
    }
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// The restricted service role, which must not be able to run these commands.
async fn service_store(pool: &PgPool) -> Store {
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let service_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    Store::new(service_pool)
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn creating_a_tenant_prints_only_its_id_and_audits_it(pool: PgPool) {
    let store = Store::new(pool.clone());
    let (result, run) = run_command(
        &store,
        Command::CreateTenant {
            name: "  Acme Advisory  ".into(),
        },
    )
    .await;
    result.unwrap();

    let id = run
        .out
        .trim_end()
        .parse::<Uuid>()
        .expect("stdout is just the tenant id");
    assert_eq!(run.out, format!("{id}\n"));
    assert!(run.err.contains("Acme Advisory") && run.err.contains(&id.to_string()));

    let name: String = sqlx::query_scalar("select name from tenants where id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "Acme Advisory");
    let (actor, action): (serde_json::Value, String) =
        sqlx::query_as("select actor, action from audit_log where tenant_id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(actor, serde_json::json!({ "type": "operator" }));
    assert_eq!(action, "tenant.created");
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_blank_tenant_name_is_refused(pool: PgPool) {
    let store = Store::new(pool.clone());
    for name in ["", "   "] {
        let (result, run) = run_command(&store, Command::CreateTenant { name: name.into() }).await;
        assert!(matches!(result, Err(AdminError::BlankName)));
        assert!(run.out.is_empty());
    }
    assert_eq!(count(&pool, "select count(*) from tenants").await, 0);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_new_key_is_printed_once_on_its_own_and_works(pool: PgPool) {
    let store = Store::new(pool.clone());
    let (_, tenant_run) = run_command(&store, Command::CreateTenant { name: "Acme".into() }).await;
    let tenant: Uuid = tenant_run.out.trim().parse().unwrap();

    let (result, run) = run_command(
        &store,
        create_key(
            tenant,
            "first admin",
            &[Scope::KeysAdmin, Scope::DocumentsRead],
            None,
        ),
    )
    .await;
    result.unwrap();

    // stdout is exactly the token, so it can be redirected into a secret store.
    let token = run.out.strip_suffix('\n').expect("one line");
    assert!(!token.contains('\n') && token.starts_with("ccut_"));
    let principal = store.authenticate(token).await.unwrap();
    assert_eq!(principal.tenant(), tenant);
    assert_eq!(principal.scopes(), vec![Scope::DocumentsRead, Scope::KeysAdmin]);

    // The human notes say what was made, and never contain the secret.
    let secret = &token["ccut_".len() + 32 + 1..];
    assert!(!run.err.contains(secret) && !run.err.contains(token));
    assert!(run.err.contains(&principal.key_id().to_string()));
    assert!(run.err.contains(&tenant.to_string()));
    assert!(run.err.contains("documents:read") && run.err.contains("keys:admin"));
    assert!(run.err.to_lowercase().contains("only time"));
    assert!(run.err.contains("expires"));
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_shorter_life_can_be_asked_for_and_the_default_is_365_days(pool: PgPool) {
    let store = Store::new(pool.clone());
    let tenant = store.create_tenant("Acme").await.unwrap();
    let (a, b) = (
        run_command(&store, create_key(tenant.0, "default", &[Scope::KeysAdmin], None)).await,
        run_command(
            &store,
            create_key(tenant.0, "short", &[Scope::KeysAdmin], Some(30)),
        )
        .await,
    );
    a.0.unwrap();
    b.0.unwrap();
    for (name, days) in [("default", 365), ("short", 30)] {
        let exact: bool = sqlx::query_scalar(
            "select expires_at - created_at = make_interval(days => $2) from api_keys where name = $1",
        )
        .bind(name)
        .bind(days)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exact, "{name}");
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn a_key_for_a_tenant_that_does_not_exist_is_refused_cleanly(pool: PgPool) {
    let store = Store::new(pool.clone());
    let (result, run) = run_command(
        &store,
        create_key(Uuid::from_u128(77), "orphan", &[Scope::KeysAdmin], None),
    )
    .await;
    assert!(matches!(result, Err(AdminError::TenantNotFound)));
    assert!(
        run.out.is_empty(),
        "no key may be printed when nothing was created"
    );
    assert_eq!(count(&pool, "select count(*) from api_keys").await, 0);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn revoking_a_key_stops_it_working_and_prints_nothing_to_stdout(pool: PgPool) {
    let store = Store::new(pool.clone());
    let tenant = store.create_tenant("Acme").await.unwrap();
    let (_, made) = run_command(
        &store,
        create_key(tenant.0, "leaked", &[Scope::DocumentsRead], None),
    )
    .await;
    let token = made.out.trim().to_string();
    let key_id = store.authenticate(&token).await.unwrap().key_id();

    let (result, run) = run_command(
        &store,
        Command::RevokeKey {
            tenant: tenant.0,
            key: key_id,
        },
    )
    .await;
    result.unwrap();
    assert!(run.out.is_empty());
    assert!(run.err.contains(&key_id.to_string()));
    assert!(store.authenticate(&token).await.is_err());
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn revoking_an_unknown_key_or_one_of_another_tenant_is_refused(pool: PgPool) {
    let store = Store::new(pool.clone());
    let acme = store.create_tenant("Acme").await.unwrap();
    let other = store.create_tenant("Other").await.unwrap();
    let (_, made) = run_command(&store, create_key(acme.0, "k", &[Scope::DocumentsRead], None)).await;
    let token = made.out.trim().to_string();
    let key_id = store.authenticate(&token).await.unwrap().key_id();

    let (unknown, _) = run_command(
        &store,
        Command::RevokeKey {
            tenant: acme.0,
            key: Uuid::from_u128(5),
        },
    )
    .await;
    assert!(matches!(unknown, Err(AdminError::KeyNotFound)));
    let (wrong_tenant, _) = run_command(
        &store,
        Command::RevokeKey {
            tenant: other.0,
            key: key_id,
        },
    )
    .await;
    assert!(matches!(wrong_tenant, Err(AdminError::KeyNotFound)));
    assert!(
        store.authenticate(&token).await.is_ok(),
        "the key must be untouched"
    );
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn migrating_again_is_harmless(pool: PgPool) {
    let store = Store::new(pool.clone());
    for _ in 0..2 {
        let (result, run) = run_command(&store, Command::Migrate).await;
        result.unwrap();
        assert!(run.out.is_empty());
        assert!(run.err.to_lowercase().contains("migrat"));
    }
    assert!(count(&pool, "select count(*) from _sqlx_migrations").await >= 4);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_service_role_cannot_run_the_operator_commands(pool: PgPool) {
    // These commands need the owner connection. Running them as the restricted
    // role must fail with a message that says so, and change nothing.
    let service = service_store(&pool).await;
    let owner = Store::new(pool.clone());
    let tenant = owner.create_tenant("Acme").await.unwrap();

    let (result, run) = run_command(
        &service,
        Command::CreateTenant {
            name: "Sneaky".into(),
        },
    )
    .await;
    assert!(
        matches!(result, Err(AdminError::NeedsOwnerConnection)),
        "{result:?}"
    );
    assert!(run.out.is_empty());
    assert_eq!(count(&pool, "select count(*) from tenants").await, 1);

    let (result, _) = run_command(&service, Command::Migrate).await;
    assert!(result.is_err());
    let _ = tenant;
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
async fn the_whole_bootstrap_works_end_to_end(pool: PgPool) {
    // Operator: tenant, then a first admin key. Admin: then manages keys through
    // the normal scoped API without the operator.
    let operator = Store::new(pool.clone());
    let (_, t) = run_command(
        &operator,
        Command::CreateTenant {
            name: "Acme Advisory".into(),
        },
    )
    .await;
    let tenant: Uuid = t.out.trim().parse().unwrap();
    let (_, k) = run_command(
        &operator,
        create_key(
            tenant,
            "first admin",
            &[Scope::KeysAdmin, Scope::DocumentsWrite],
            None,
        ),
    )
    .await;

    let service = service_store(&pool).await;
    let admin = service.authenticate(k.out.trim()).await.unwrap();
    let uploader = service
        .create_api_key(
            &admin,
            &clearcut_store::NewApiKey {
                name: "uploader".into(),
                scopes: vec![Scope::DocumentsWrite],
                lifetime_days: None,
            },
        )
        .await
        .unwrap();
    let principal = service.authenticate(uploader.token.expose()).await.unwrap();
    assert_eq!(principal.tenant(), tenant);
    assert_eq!(TenantId(principal.tenant()).0, tenant);
    let _ = keys::KEY_PREFIX;
}
