//! Queue transport: backend-agnostic trait for at-least-once message queues.
//!
//! Mirrors `docs/contract.md` section 4. Implementations live under
//! `backends/`. Currently only the local SQLite backend is provided; a
//! remote backend may be added later.
//!
//! Semantics (must hold across all backends):
//! - `enqueue` is atomic: either visible to a future `pop`, or not.
//! - `pop(vis)` makes the message invisible for `vis` seconds. Without `ack`
//!   in that window the message becomes visible again.
//! - `ack` removes the message permanently.
//! - `nack` makes the message visible immediately (fast retry).
//! - After `max_receive` failed deliveries the message is moved to a DLQ
//!   (configured at backend construction time, not per call).
//!
//! Idempotency is the consumer's responsibility (see `processed` module).

use std::future::Future;

use serde::{de::DeserializeOwned, Serialize};

use crate::Result;

/// Opaque receipt identifying a popped message for `ack` / `nack`.
///
/// String shape is backend-specific (e.g. SQLite uses the decimal row id;
/// a remote backend would carry whatever opaque token the broker returns).
/// Consumers must not parse it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt(pub String);

impl Receipt {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A popped message: payload plus the receipt needed to settle it.
///
/// `receive_count` is informational (logging, metrics, debug). DLQ promotion
/// happens inside the backend before this struct is returned, so a value
/// approaching `max_receive` indicates a hot retry loop, not an imminent DLQ
/// move.
#[derive(Debug)]
pub struct Message<T> {
    pub receipt: Receipt,
    pub payload: T,
    pub receive_count: u32,
}

/// Backend-agnostic queue. Generic over payload so the same trait covers
/// `transcribe` (TranscribeJob) and `notifications` (NotifyResult) queues.
///
/// Native `async fn` in trait (Rust 1.75+); no `async_trait`. Pick static
/// dispatch (`impl Queue<T>`) at use sites -- trait objects with async fn
/// require extra ceremony and we don't need them here.
pub trait Queue<T>: Send + Sync
where
    T: Serialize + DeserializeOwned + Send,
{
    /// Append a message. Visible to consumers immediately.
    ///
    /// Payload is consumed by value: enqueueing transfers ownership and
    /// avoids the `T: Sync` bound that would be required to send a `&T`
    /// across an `await`.
    fn enqueue(&self, payload: T) -> impl Future<Output = Result<()>> + Send;

    /// Take one visible message and hide it for `visibility_sec` seconds.
    /// Returns `None` if the queue has nothing visible.
    ///
    /// Long-polling vs short-polling is a backend choice; SQLite polls,
    /// a remote backend may long-poll natively.
    fn pop(
        &self,
        visibility_sec: u32,
    ) -> impl Future<Output = Result<Option<Message<T>>>> + Send;

    /// Mark a popped message as successfully processed. Idempotent.
    fn ack(&self, receipt: &Receipt) -> impl Future<Output = Result<()>> + Send;

    /// Return a popped message to the queue immediately (fast retry).
    /// Equivalent to letting the visibility timeout expire, but without the
    /// wait. Idempotent.
    fn nack(&self, receipt: &Receipt) -> impl Future<Output = Result<()>> + Send;
}
