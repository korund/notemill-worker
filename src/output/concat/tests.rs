use super::*;

#[test]
fn none_separator_never_emits_prefix() {
    let mut s = SeparatorState::new(None, false);
    assert_eq!(s.next_prefix(), None);
    assert_eq!(s.next_prefix(), None);
    assert_eq!(s.next_prefix(), None);
}

#[test]
fn none_separator_even_when_primed() {
    // No separator configured => no prefix, regardless of priming.
    let mut s = SeparatorState::new(None, true);
    assert_eq!(s.next_prefix(), None);
    assert_eq!(s.next_prefix(), None);
}

#[test]
fn unprimed_skips_first_then_emits() {
    let mut s = SeparatorState::new(Some("---".to_string()), false);
    assert_eq!(s.next_prefix(), None);
    assert_eq!(s.next_prefix(), Some("\n---\n".to_string()));
    assert_eq!(s.next_prefix(), Some("\n---\n".to_string()));
}

#[test]
fn primed_emits_from_first_call() {
    // Pre-existing content in destination: separate immediately.
    let mut s = SeparatorState::new(Some("---".to_string()), true);
    assert_eq!(s.next_prefix(), Some("\n---\n".to_string()));
    assert_eq!(s.next_prefix(), Some("\n---\n".to_string()));
}

#[test]
fn custom_separator_is_wrapped_in_newlines() {
    let mut s = SeparatorState::new(Some("=====".to_string()), false);
    assert_eq!(s.next_prefix(), None);
    assert_eq!(s.next_prefix(), Some("\n=====\n".to_string()));
}

#[test]
fn empty_string_separator_still_wrapped() {
    // Edge case: explicit empty string separator yields "\n\n" (a blank line).
    let mut s = SeparatorState::new(Some(String::new()), false);
    assert_eq!(s.next_prefix(), None);
    assert_eq!(s.next_prefix(), Some("\n\n".to_string()));
}
