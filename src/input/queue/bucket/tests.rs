
use super::*;

#[test]
fn extension_extraction() {
    assert_eq!(extension_of("audio/2026/05/07/abc.oga"), Some("oga".into()));
    assert_eq!(extension_of("audio/2026/05/07/abc"), None);
    assert_eq!(extension_of("audio/2026/05/07/abc."), None);
}

#[test]
fn from_bytes_reads_back() {
    let s = BucketAudioSource::from_bytes("test", vec![1, 2, 3], Some("oga".into()));
    assert_eq!(s.name(), "test");
    let raw = s.read().unwrap();
    assert_eq!(raw.bytes, vec![1, 2, 3]);
    assert_eq!(raw.format_hint.as_deref(), Some("oga"));
}

#[test]
fn error_classification() {
    assert!(is_not_found(&Error::Bucket("not_found: x".into())));
    assert!(!is_not_found(&Error::Bucket("other".into())));
    assert!(is_already_exists(&Error::Bucket(
        "already_exists: x".into()
    )));
    assert!(!is_already_exists(&Error::Bucket("not_found: x".into())));
}
