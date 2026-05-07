//! Shared helper for sinks that concatenate multiple writes into one stream.
//!
//! Owns the "should I prepend a separator before the next chunk?" state machine.
//! `FileSink` and (future) the queue-mode `StdoutSink` both delegate to this so
//! the rule lives in one place.

/// Tracks whether the next written chunk should be preceded by a separator.
///
/// `primed = true` means: the next call to `next_prefix` returns the wrapped
/// separator (and the state stays primed for all further calls).
/// `primed = false` means: the next call returns `None` and primes the state.
pub struct SeparatorState {
    sep: Option<String>,
    primed: bool,
}

impl SeparatorState {
    /// `initially_primed` should be `true` when the destination already has
    /// content before the sink starts writing (e.g. an existing non-empty file
    /// in append mode), so the first write is also separated from prior data.
    pub fn new(sep: Option<String>, initially_primed: bool) -> Self {
        Self { sep, primed: initially_primed }
    }

    /// Returns the prefix to write before the next chunk, advancing state.
    /// `None` when no prefix is needed (first chunk into empty destination,
    /// or no separator configured).
    pub fn next_prefix(&mut self) -> Option<String> {
        let out = if self.primed {
            self.sep.as_ref().map(|s| format!("\n{s}\n"))
        } else {
            None
        };
        self.primed = true;
        out
    }
}
