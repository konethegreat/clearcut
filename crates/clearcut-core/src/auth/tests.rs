// Written by Kone Tshivhinda & Claude
use super::*;
use crate::Error;
use uuid::Uuid;

fn tenant() -> Uuid {
    Uuid::from_u128(1)
}

fn key() -> Uuid {
    Uuid::from_u128(2)
}

fn principal(scopes: &[Scope]) -> Principal {
    Principal::new(tenant(), key(), scopes.iter().copied())
}

// ---------- the scope list ----------

#[test]
fn the_scopes_and_their_names_are_exactly_the_agreed_list() {
    // ADR-0007. These strings are stored in the database and sent in API
    // calls, so changing one is a breaking change.
    let expected = [
        (Scope::DocumentsRead, "documents:read"),
        (Scope::DocumentsWrite, "documents:write"),
        (Scope::EventsRead, "events:read"),
        (Scope::ReviewWrite, "review:write"),
        (Scope::KeysAdmin, "keys:admin"),
    ];
    for (scope, name) in expected {
        assert_eq!(scope.as_str(), name);
    }
    assert_eq!(Scope::ALL.len(), expected.len());
    for scope in Scope::ALL {
        assert!(expected.iter().any(|(s, _)| *s == scope));
    }
}

#[test]
fn a_scope_name_parses_back_to_the_scope() {
    for scope in Scope::ALL {
        assert_eq!(Scope::parse(scope.as_str()), Ok(scope));
    }
}

#[test]
fn unknown_or_loosely_written_scope_names_are_refused() {
    for text in [
        "",
        "documents",
        "documents:delete",
        "Documents:Read",
        " documents:read",
        "documents:read ",
        "documents:read,events:read",
        "*",
        "admin",
    ] {
        assert_eq!(
            Scope::parse(text),
            Err(Error::UnknownScope(text.to_string())),
            "{text:?}"
        );
    }
}

#[test]
fn scopes_are_written_as_their_names_in_json() {
    assert_eq!(
        serde_json::to_string(&Scope::ReviewWrite).unwrap(),
        "\"review:write\""
    );
    let parsed: Scope = serde_json::from_str("\"keys:admin\"").unwrap();
    assert_eq!(parsed, Scope::KeysAdmin);
    assert!(serde_json::from_str::<Scope>("\"keys:everything\"").is_err());
}

#[test]
fn a_list_of_names_parses_all_or_fails_on_the_first_unknown() {
    assert_eq!(
        Scope::parse_all(["events:read", "documents:read"]),
        Ok(vec![Scope::EventsRead, Scope::DocumentsRead])
    );
    // A stored list with an unknown scope is an error, never silently trimmed.
    assert_eq!(
        Scope::parse_all(["events:read", "bogus", "alsobogus"]),
        Err(Error::UnknownScope("bogus".to_string()))
    );
    assert_eq!(Scope::parse_all(Vec::<String>::new()), Ok(Vec::new()));
}

// ---------- the principal ----------

#[test]
fn a_principal_knows_its_tenant_and_key() {
    let p = principal(&[Scope::DocumentsRead]);
    assert_eq!(p.tenant(), tenant());
    assert_eq!(p.key_id(), key());
}

#[test]
fn a_principal_has_exactly_the_scopes_it_was_given() {
    let p = principal(&[Scope::DocumentsRead, Scope::EventsRead]);
    assert!(p.has(Scope::DocumentsRead));
    assert!(p.has(Scope::EventsRead));
    assert!(!p.has(Scope::DocumentsWrite));
    assert!(!p.has(Scope::ReviewWrite));
    assert!(!p.has(Scope::KeysAdmin));
}

#[test]
fn require_passes_for_a_held_scope_and_names_the_missing_one_otherwise() {
    let p = principal(&[Scope::DocumentsRead]);
    assert_eq!(p.require(Scope::DocumentsRead), Ok(()));
    assert_eq!(
        p.require(Scope::DocumentsWrite),
        Err(Error::MissingScope(Scope::DocumentsWrite))
    );
}

#[test]
fn no_scope_implies_another_not_even_key_administration() {
    let admin = principal(&[Scope::KeysAdmin]);
    for scope in Scope::ALL {
        if scope != Scope::KeysAdmin {
            assert_eq!(admin.require(scope), Err(Error::MissingScope(scope)), "{scope:?}");
        }
    }
}

#[test]
fn a_principal_with_no_scopes_can_do_nothing() {
    let nobody = principal(&[]);
    for scope in Scope::ALL {
        assert!(!nobody.has(scope));
        assert!(nobody.require(scope).is_err());
    }
}

#[test]
fn duplicate_scopes_are_collapsed_and_listed_in_a_fixed_order() {
    let p = principal(&[
        Scope::KeysAdmin,
        Scope::DocumentsRead,
        Scope::KeysAdmin,
        Scope::DocumentsRead,
    ]);
    assert_eq!(p.scopes(), vec![Scope::DocumentsRead, Scope::KeysAdmin]);
}

#[test]
fn the_error_message_says_which_scope_is_needed() {
    let error = principal(&[]).require(Scope::ReviewWrite).unwrap_err();
    assert_eq!(error.to_string(), "missing required scope review:write");
    let unknown = Scope::parse("nope").unwrap_err();
    assert_eq!(unknown.to_string(), "unknown scope \"nope\"");
}
