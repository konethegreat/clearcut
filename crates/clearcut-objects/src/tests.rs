// Written by Kone Tshivhinda & Claude
use super::*;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use uuid::Uuid;

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn tenant(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn limits(max_bytes: u64) -> PutLimits {
    PutLimits { max_bytes }
}

fn store() -> (TempDir, LocalDisk) {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalDisk::new(dir.path());
    (dir, store)
}

async fn read_all(store: &LocalDisk, key: &ObjectKey) -> Vec<u8> {
    let mut bytes = Vec::new();
    store
        .open(key)
        .await
        .unwrap()
        .read_to_end(&mut bytes)
        .await
        .unwrap();
    bytes
}

/// Files left in the temporary upload area (should always be none).
fn leftovers(dir: &TempDir) -> usize {
    match std::fs::read_dir(dir.path().join("tmp")) {
        Ok(entries) => entries.count(),
        Err(_) => 0,
    }
}

/// All regular files under the store root, ignoring the temporary area.
fn stored_files(dir: &TempDir) -> Vec<String> {
    fn walk(path: &std::path::Path, out: &mut Vec<String>, root: &std::path::Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out, root);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir.path(), &mut out, dir.path());
    out.retain(|p| !p.starts_with("tmp/"));
    out.sort();
    out
}

/// A reader that yields some bytes and then fails.
struct FailsMidway {
    sent: bool,
}

impl AsyncRead for FailsMidway {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.sent {
            return Poll::Ready(Err(io::Error::other("connection reset")));
        }
        self.sent = true;
        buf.put_slice(b"partial upload");
        Poll::Ready(Ok(()))
    }
}

// ---------- digests ----------

#[test]
fn a_digest_prints_and_parses_as_lowercase_hex() {
    let digest = Sha256Digest::from_hex(ABC_SHA256).unwrap();
    assert_eq!(digest.to_hex(), ABC_SHA256);
    assert_eq!(digest.as_bytes()[0], 0xba);
}

#[test]
fn malformed_hex_is_not_a_digest() {
    assert!(Sha256Digest::from_hex("").is_none());
    assert!(Sha256Digest::from_hex(&ABC_SHA256[..63]).is_none());
    assert!(Sha256Digest::from_hex(&format!("{ABC_SHA256}0")).is_none());
    assert!(Sha256Digest::from_hex(&ABC_SHA256.replace('b', "g")).is_none());
    assert!(Sha256Digest::from_hex(&ABC_SHA256.to_uppercase()).is_none());
}

// ---------- keys ----------

#[test]
fn a_key_is_built_from_the_tenant_and_the_content_hash() {
    let digest = Sha256Digest::from_hex(ABC_SHA256).unwrap();
    let key = ObjectKey::for_document(tenant(1), &digest);
    assert_eq!(
        key.as_str(),
        format!("tenants/00000000-0000-0000-0000-000000000001/documents/{ABC_SHA256}")
    );
}

#[test]
fn a_key_parses_back_to_itself() {
    let digest = Sha256Digest::from_hex(ABC_SHA256).unwrap();
    let key = ObjectKey::for_document(tenant(7), &digest);
    assert_eq!(ObjectKey::parse(key.as_str()).unwrap(), key);
}

#[test]
fn keys_that_are_not_exactly_that_shape_are_refused() {
    let good = format!("tenants/00000000-0000-0000-0000-000000000001/documents/{ABC_SHA256}");
    assert!(ObjectKey::parse(&good).is_ok());
    let bad = [
        String::new(),
        "../../etc/passwd".to_string(),
        format!("tenants/../documents/{ABC_SHA256}"),
        format!("/tenants/00000000-0000-0000-0000-000000000001/documents/{ABC_SHA256}"),
        format!("tenants/00000000-0000-0000-0000-000000000001/documents/{ABC_SHA256}/"),
        format!("tenants/00000000-0000-0000-0000-000000000001/documents/../{ABC_SHA256}"),
        format!("tenants\\00000000-0000-0000-0000-000000000001\\documents\\{ABC_SHA256}"),
        format!("tenants/00000000-0000-0000-0000-000000000001/other/{ABC_SHA256}"),
        "tenants/00000000-0000-0000-0000-000000000001/documents/invoice.pdf".to_string(),
        format!("tenants/not-a-uuid/documents/{ABC_SHA256}"),
        // Valid UUIDs, but not in the one canonical (lowercase, hyphenated) form:
        format!("tenants/0000000000000000000000000000000a/documents/{ABC_SHA256}"),
        format!("tenants/00000000-0000-0000-0000-00000000000A/documents/{ABC_SHA256}"),
        format!("tenants/{{00000000-0000-0000-0000-00000000000a}}/documents/{ABC_SHA256}"),
        format!("tenants/urn:uuid:00000000-0000-0000-0000-00000000000a/documents/{ABC_SHA256}"),
        format!(
            "tenants/00000000-0000-0000-0000-000000000001/documents/{}",
            ABC_SHA256.to_uppercase()
        ),
        format!("{good}\0"),
    ];
    for key in bad {
        assert!(ObjectKey::parse(&key).is_err(), "should refuse {key:?}");
    }
}

// ---------- put and get ----------

#[tokio::test]
async fn stores_bytes_and_reports_their_hash_and_size() {
    let (dir, store) = store();
    let stored = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    assert_eq!(stored.sha256.to_hex(), ABC_SHA256);
    assert_eq!(stored.byte_size, 3);
    assert!(stored.created);
    assert_eq!(stored.key, ObjectKey::for_document(tenant(1), &stored.sha256));
    assert_eq!(read_all(&store, &stored.key).await, b"abc");
    assert_eq!(stored_files(&dir), [stored.key.as_str().to_string()]);
    assert_eq!(leftovers(&dir), 0);
}

#[tokio::test]
async fn the_same_content_twice_is_stored_once() {
    let (dir, store) = store();
    let first = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    let second = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    assert_eq!(first.key, second.key);
    assert!(first.created);
    assert!(!second.created);
    assert_eq!(stored_files(&dir).len(), 1);
    assert_eq!(leftovers(&dir), 0);
}

#[tokio::test]
async fn two_tenants_uploading_the_same_content_get_separate_objects() {
    let (dir, store) = store();
    let a = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    let b = store.put(tenant(2), &b"abc"[..], limits(1024)).await.unwrap();
    assert_ne!(a.key, b.key);
    assert!(a.created && b.created);
    assert_eq!(stored_files(&dir).len(), 2);
}

#[tokio::test]
async fn content_larger_than_the_limit_is_refused_and_leaves_nothing_behind() {
    let (dir, store) = store();
    let result = store.put(tenant(1), &[7u8; 11][..], limits(10)).await;
    assert!(matches!(result, Err(Error::TooLarge { limit: 10 })));
    assert!(stored_files(&dir).is_empty());
    assert_eq!(leftovers(&dir), 0);
}

#[tokio::test]
async fn content_exactly_at_the_limit_is_accepted() {
    let (_dir, store) = store();
    let stored = store.put(tenant(1), &[7u8; 10][..], limits(10)).await.unwrap();
    assert_eq!(stored.byte_size, 10);
}

#[tokio::test]
async fn the_limit_applies_to_what_is_read_not_to_what_the_sender_claims() {
    // A stream longer than any buffer, so the limit must trip mid-stream.
    let (dir, store) = store();
    let big = vec![1u8; 300_000];
    let result = store.put(tenant(1), &big[..], limits(100_000)).await;
    assert!(matches!(result, Err(Error::TooLarge { limit: 100_000 })));
    assert_eq!(leftovers(&dir), 0);
}

#[tokio::test]
async fn an_empty_upload_is_refused() {
    let (dir, store) = store();
    let result = store.put(tenant(1), &b""[..], limits(1024)).await;
    assert!(matches!(result, Err(Error::Empty)));
    assert!(stored_files(&dir).is_empty());
    assert_eq!(leftovers(&dir), 0);
}

#[tokio::test]
async fn a_failed_stream_stores_nothing() {
    let (dir, store) = store();
    let result = store
        .put(tenant(1), FailsMidway { sent: false }, limits(1024))
        .await;
    assert!(matches!(result, Err(Error::Io(_))));
    assert!(stored_files(&dir).is_empty());
    assert_eq!(leftovers(&dir), 0);
}

#[tokio::test]
async fn large_content_round_trips_intact() {
    let (_dir, store) = store();
    let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    let stored = store.put(tenant(1), &data[..], limits(2_000_000)).await.unwrap();
    assert_eq!(stored.byte_size, 1_000_000);
    assert_eq!(read_all(&store, &stored.key).await, data);
}

#[tokio::test]
async fn simultaneous_identical_uploads_all_succeed_and_leave_one_intact_object() {
    let (dir, store) = store();
    let store = Arc::new(store);
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 199) as u8).collect();
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let data = data.clone();
        tasks.push(tokio::spawn(async move {
            store.put(tenant(1), &data[..], limits(1_000_000)).await
        }));
    }
    let mut keys = Vec::new();
    for task in tasks {
        keys.push(task.await.unwrap().unwrap().key);
    }
    assert!(keys.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(read_all(&store, &keys[0]).await, data);
    assert_eq!(stored_files(&dir).len(), 1);
    assert_eq!(leftovers(&dir), 0);
}

// ---------- open, exists, delete ----------

#[tokio::test]
async fn opening_a_missing_object_says_not_found() {
    let (_dir, store) = store();
    let digest = Sha256Digest::from_hex(ABC_SHA256).unwrap();
    let key = ObjectKey::for_document(tenant(1), &digest);
    assert!(matches!(store.open(&key).await, Err(Error::NotFound)));
    assert!(!store.exists(&key).await.unwrap());
}

#[tokio::test]
async fn another_tenants_object_cannot_be_opened_with_your_tenant() {
    let (_dir, store) = store();
    let stored = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    // Same content hash, other tenant: a different key, so nothing there.
    let theirs = ObjectKey::for_document(tenant(2), &stored.sha256);
    assert!(matches!(store.open(&theirs).await, Err(Error::NotFound)));
}

#[tokio::test]
async fn delete_removes_the_object_and_is_safe_to_repeat() {
    let (_dir, store) = store();
    let stored = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    assert!(store.exists(&stored.key).await.unwrap());
    store.delete(&stored.key).await.unwrap();
    assert!(!store.exists(&stored.key).await.unwrap());
    store.delete(&stored.key).await.unwrap(); // already gone: not an error
}

#[tokio::test]
async fn after_a_delete_the_same_content_can_be_stored_again() {
    let (_dir, store) = store();
    let first = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    store.delete(&first.key).await.unwrap();
    let again = store.put(tenant(1), &b"abc"[..], limits(1024)).await.unwrap();
    assert!(again.created);
}
