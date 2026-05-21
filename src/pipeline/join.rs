//! Text-level transcript join with overlap deduplication.
//!
//! After the chunker splits long audio into overlapping PCM windows,
//! each window is transcribed independently. This module stitches the
//! resulting strings back into one continuous transcript. When the
//! producing chunk had `has_overlap_with_next == true`, the tail of the
//! previous text and the head of the next likely repeat the same words;
//! `join_texts` removes that duplicate prefix before appending.

/// Join transcript fragments produced from chunks. When the producing
/// chunk had `has_overlap_with_next == true`, the tail of `prev` and the
/// head of `next` likely repeat the same words. We dedup by finding the
/// longest suffix of `prev` (up to ~10 words) that is a prefix of `next`
/// and dropping it from `next`.
pub fn join_texts(parts: &[(String, bool)]) -> String {
    let mut result = String::new();
    for (i, (text, _has_overlap)) in parts.iter().enumerate() {
        if i == 0 {
            result.push_str(text.trim());
            continue;
        }
        let prev_has_overlap = parts[i - 1].1;
        if prev_has_overlap {
            let trimmed = dedup_overlap(&result, text);
            if !trimmed.is_empty() {
                if !result.ends_with(' ') {
                    result.push(' ');
                }
                result.push_str(&trimmed);
            }
        } else {
            if !text.trim().is_empty() {
                if !result.ends_with('\n') && !result.is_empty() {
                    result.push(' ');
                }
                result.push_str(text.trim());
            }
        }
    }
    result
}

/// Drop from `next` the longest leading sequence of words (up to
/// `MAX_WORDS`) that also appears at the very end of `prev`.
fn dedup_overlap(prev: &str, next: &str) -> String {
    const MAX_WORDS: usize = 10;
    let prev_words: Vec<&str> = prev.split_whitespace().collect();
    let next_words: Vec<&str> = next.split_whitespace().collect();
    if prev_words.is_empty() || next_words.is_empty() {
        return next.trim().to_string();
    }
    let max_k = MAX_WORDS.min(prev_words.len()).min(next_words.len());
    let mut best = 0;
    for k in (1..=max_k).rev() {
        let prev_tail = &prev_words[prev_words.len() - k..];
        let next_head = &next_words[..k];
        if eq_ci(prev_tail, next_head) {
            best = k;
            break;
        }
    }
    next_words[best..].join(" ")
}

fn eq_ci(a: &[&str], b: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.to_lowercase() == y.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_texts_concatenates_with_space_when_no_overlap() {
        let parts = vec![
            ("hello world".to_string(), false),
            ("foo bar".to_string(), false),
        ];
        assert_eq!(join_texts(&parts), "hello world foo bar");
    }

    #[test]
    fn join_texts_dedups_overlap_at_boundary() {
        // First chunk ends "the quick brown fox"; second starts "brown fox
        // jumps over"; with overlap flag we expect dedup of two words.
        let parts = vec![
            ("the quick brown fox".to_string(), true),
            ("brown fox jumps over".to_string(), false),
        ];
        assert_eq!(join_texts(&parts), "the quick brown fox jumps over");
    }

    #[test]
    fn join_texts_case_insensitive_dedup() {
        let parts = vec![
            ("Hello World".to_string(), true),
            ("world is round".to_string(), false),
        ];
        assert_eq!(join_texts(&parts), "Hello World is round");
    }

    #[test]
    fn join_texts_no_overlap_match_keeps_full_next() {
        let parts = vec![
            ("alpha beta gamma".to_string(), true),
            ("xyz qux".to_string(), false),
        ];
        assert_eq!(join_texts(&parts), "alpha beta gamma xyz qux");
    }

    #[test]
    fn join_texts_empty_inputs_safe() {
        assert_eq!(join_texts(&[]), "");
        let parts = vec![("only".to_string(), false)];
        assert_eq!(join_texts(&parts), "only");
    }
}
