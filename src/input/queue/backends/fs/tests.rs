
use super::*;
use tokio::runtime::Runtime;

fn rt() -> Runtime {
    Runtime::new().unwrap()
}

fn tmp_root() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "notemill-worker-fsbucket-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    p
}

#[test]
fn key_validation() {
    assert!(validate_key("audio/2026/05/07/abc.oga").is_ok());
    assert!(validate_key("a").is_ok());
    assert!(validate_key("").is_err());
    assert!(validate_key("/abs").is_err());
    assert!(validate_key("a/../b").is_err());
    assert!(validate_key("a\\b").is_err());
    assert!(validate_key("./x").is_err());
}

#[test]
fn put_get_delete_head() {
    let rt = rt();
    let root = tmp_root();
    rt.block_on(async {
        let store = FsBucket::open(&root).unwrap();
        let key = "audio/2026/05/07/abc.oga";

        assert!(store.get(key).await.unwrap().is_none());
        assert!(store.head(key).await.unwrap().is_none());

        store.put(key, b"hello").await.unwrap();
        let got = store.get(key).await.unwrap().unwrap();
        assert_eq!(got, b"hello");

        let meta = store.head(key).await.unwrap().unwrap();
        assert_eq!(meta.size, 5);
        assert_eq!(meta.etag.len(), 64); // sha256 hex

        // put twice -> AlreadyExists.
        let err = store.put(key, b"again").await.unwrap_err();
        assert!(matches!(err, Error::Bucket(ref m) if m.starts_with("already_exists:")));

        store.delete(key).await.unwrap();
        assert!(store.get(key).await.unwrap().is_none());
        // delete missing -> ok.
        store.delete(key).await.unwrap();
    });
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rejects_traversal() {
    let rt = rt();
    let root = tmp_root();
    rt.block_on(async {
        let store = FsBucket::open(&root).unwrap();
        assert!(store.put("../escape", b"x").await.is_err());
        assert!(store.get("/abs").await.is_err());
    });
    let _ = std::fs::remove_dir_all(&root);
}
