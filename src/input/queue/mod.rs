//! Queue-driven input mode.
//!
//! `QueueDriver` is the main loop for worker containers in queue mode
//! deployment: pop a `TranscribeJob`, fetch the bucket, run the pipeline,
//! record idempotency, send a `NotifyResult`, ack. Shutdown on SIGTERM /
//! ctrl_c lets the current job finish before exit.
//!
//! Layout:
//! - `job` -- wire-format types (TranscribeJob, NotifyResult).
//! - `transport` -- `Queue<T>` trait + DLQ semantics.
//! - `bucket` -- `Bucket` trait + `BucketAudioSource` adapter.
//! - `processed` -- idempotency table.
//! - `backends` -- sqlite/fs for local deployment; a remote backend may
//!   plug in later.
//! - `QueueDriver` (this file) -- main loop wiring all of the above.

pub mod backends;
pub mod bucket;
pub mod job;
pub mod processed;
pub mod transport;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::runtime::Builder;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::input::{AudioSource, InputDriver};
use crate::models::{ModelRegistry, ModelStatus};
use crate::{Error, Result};

use bucket::{is_not_found, Bucket, BucketAudioSource};
use job::{ErrorCode, JobResult, NotifyKind, NotifyResult, SourceRef, TranscribeJob, WIRE_VERSION};
use processed::{replay_notify, ProcessedRecord, ProcessedStatus, ProcessedStore};
use transport::Queue;

/// Pipeline closure (decoder -> engine -> output sink), invoked once per
/// job. Sync because the in-memory model and CPU decoding don't benefit
/// from async, and the heavy parts may be `!Send`.
///
/// Returns the `output_ref` written into the sink (e.g. the CouchDB doc id),
/// which is propagated into `NotifyResult` and `processed_jobs`.
pub trait JobProcessor {
    fn process(
        &mut self,
        pipeline: &mut crate::pipeline::Pipeline,
        source: &dyn AudioSource,
        job: &TranscribeJob,
    ) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct QueueDriverConfig {
    pub visibility_sec: u32,
    pub loaded_loop: Duration,
    pub unloaded_loop: Duration,
}

impl Default for QueueDriverConfig {
    fn default() -> Self {
        Self {
            visibility_sec: 300,
            loaded_loop: Duration::from_millis(1000),
            unloaded_loop: Duration::from_millis(60_000),
        }
    }
}

pub struct QueueDriver<TQ, NQ, B, P, JP> {
    transcribe_q: TQ,
    notify_q: NQ,
    bucket: B,
    processed: P,
    processor: JP,
    guard: crate::pipeline::ModelGuard,
    cfg: QueueDriverConfig,
    registry: ModelRegistry,
    model_name: String,
    model_load_failures: u32,
}

impl<TQ, NQ, B, P, JP> QueueDriver<TQ, NQ, B, P, JP>
where
    TQ: Queue<TranscribeJob>,
    NQ: Queue<NotifyResult>,
    B: Bucket,
    P: ProcessedStore,
    JP: JobProcessor,
{
    pub fn new(
        transcribe_q: TQ,
        notify_q: NQ,
        bucket: B,
        processed: P,
        processor: JP,
        guard: crate::pipeline::ModelGuard,
        cfg: QueueDriverConfig,
        registry: ModelRegistry,
        model_name: String,
    ) -> Self {
        Self {
            transcribe_q,
            notify_q,
            bucket,
            processed,
            processor,
            guard,
            cfg,
            registry,
            model_name,
            model_load_failures: 0,
        }
    }
}

impl<TQ, NQ, B, P, JP> InputDriver for QueueDriver<TQ, NQ, B, P, JP>
where
    TQ: Queue<TranscribeJob>,
    NQ: Queue<NotifyResult>,
    B: Bucket,
    P: ProcessedStore,
    JP: JobProcessor,
{
    fn run(&mut self) -> Result<()> {
        // Single-threaded runtime: queue/bucket ops are async, but the pipeline
        // (decoder/engine/sink) is sync and may hold !Send model state. A
        // current-thread runtime keeps the whole loop on one OS thread while
        // still allowing tokio::fs and spawn_blocking under the hood.
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Queue(format!("tokio runtime: {e}")))?;
        rt.block_on(self.run_loop())
    }
}

impl<TQ, NQ, B, P, JP> QueueDriver<TQ, NQ, B, P, JP>
where
    TQ: Queue<TranscribeJob>,
    NQ: Queue<NotifyResult>,
    B: Bucket,
    P: ProcessedStore,
    JP: JobProcessor,
{
    async fn run_loop(&mut self) -> Result<()> {
        info!(
            visibility_sec = self.cfg.visibility_sec,
            loaded_loop_ms = self.cfg.loaded_loop.as_millis() as u64,
            unloaded_loop_ms = self.cfg.unloaded_loop.as_millis() as u64,
            "queue driver started"
        );
        let mut shutdown = std::pin::pin!(wait_shutdown());
        loop {
            let had_work = tokio::select! {
                biased;
                _ = &mut shutdown => {
                    info!("shutdown signal received, exiting loop");
                    return Ok(());
                }
                result = self.step() => match result {
                    Ok(had) => had,
                    Err(e) => { error!(error = %e, "step failed; backing off"); false }
                },
            };
            if !had_work {
                self.guard.try_unload();
                let interval = if self.guard.is_loaded() {
                    self.cfg.loaded_loop
                } else {
                    self.cfg.unloaded_loop
                };
                tokio::select! {
                    biased;
                    _ = &mut shutdown => {
                        info!("shutdown signal received during idle, exiting loop");
                        return Ok(());
                    }
                    _ = sleep(interval) => {}
                }
            }
        }
    }

    /// One iteration. Returns `Ok(true)` if a message was handled (loop
    /// should immediately try again), `Ok(false)` if the queue was empty.
    async fn step(&mut self) -> Result<bool> {
        let Some(msg) = self.transcribe_q.pop(self.cfg.visibility_sec).await? else {
            return Ok(false);
        };
        let job = msg.payload;
        let receipt = msg.receipt;
        let msg_receive_count = msg.receive_count;
        debug!(
            dedup_key = %job.dedup_key,
            audio_key = %job.audio_key,
            receive_count = msg.receive_count,
            "popped job"
        );

        // Wire-format version check; mismatched payloads were already routed
        // to DLQ by the queue backend if they got that far. Defensive log only.
        if job.v != WIRE_VERSION {
            warn!(
                got = job.v,
                expected = WIRE_VERSION,
                "wire-version mismatch; acking to skip"
            );
            self.transcribe_q.ack(&receipt).await?;
            return Ok(true);
        }

        // Step 2: idempotency check.
        if let Some(prev) = self.processed.lookup(&job.dedup_key).await? {
            debug!(dedup_key = %job.dedup_key, "duplicate delivery, replaying notify");
            let notify = replay_notify(&prev, SourceRef::from_job(&job.source));
            // Best-effort enqueue: if it fails, the original notify likely
            // already landed; ack regardless to drop the duplicate.
            if let Err(e) = self.notify_q.enqueue(notify).await {
                warn!(error = %e, "replay notify enqueue failed");
            }
            self.transcribe_q.ack(&receipt).await?;
            return Ok(true);
        }

        // Step 3: fetch + run pipeline.
        let started = Instant::now();
        let outcome = self.run_pipeline(&job).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        match outcome {
            Ok(output_ref) => {
                self.finalise_ok(&job, &receipt, output_ref, duration_ms)
                    .await?;
            }
            Err(PipelineError::Deterministic(code, msg)) => {
                self.finalise_error(
                    &job,
                    &receipt,
                    code,
                    msg,
                    duration_ms,
                    true,
                    msg_receive_count,
                )
                .await?;
            }
            Err(PipelineError::Transient(code, msg)) => {
                self.finalise_error(
                    &job,
                    &receipt,
                    code,
                    msg,
                    duration_ms,
                    false,
                    msg_receive_count,
                )
                .await?;
            }
            Err(PipelineError::ModelNotReady) => {
                debug!(dedup_key = %job.dedup_key, "model not ready, nacking job");
                // Do not ack — visibility timeout will redeliver.
            }
            Err(PipelineError::ModelFatal(msg)) => {
                self.model_load_failures += 1;
                if self.model_load_failures >= 3 {
                    error!(model = %self.model_name, "model failed {n} times, shutting down", n = self.model_load_failures);
                    return Err(Error::Model(msg));
                }
                warn!(model = %self.model_name, attempt = self.model_load_failures, "model init failed, will retry");
            }
        }
        Ok(true)
    }

    async fn run_pipeline(
        &mut self,
        job: &TranscribeJob,
    ) -> std::result::Result<String, PipelineError> {
        match self.registry.get(&self.model_name) {
            Some(ModelStatus::Ready(_)) | None => {}
            Some(ModelStatus::Pulling) => return Err(PipelineError::ModelNotReady),
            Some(ModelStatus::Failed(msg)) => return Err(PipelineError::ModelFatal(msg)),
        }
        let format_hint = job
            .hints
            .as_ref()
            .and_then(|h| h.mime.as_deref())
            .and_then(|mime| {
                mime_guess::get_mime_extensions_str(mime)
                    .and_then(|exts| exts.first().copied())
                    .map(str::to_owned)
            });
        let source = match BucketAudioSource::fetch(&self.bucket, &job.audio_key, format_hint).await
        {
            Ok(s) => s,
            Err(e) if is_not_found(&e) => {
                return Err(PipelineError::Deterministic(
                    ErrorCode::AudioMissing,
                    format!("{e}"),
                ));
            }
            Err(e) => {
                // Other bucket errors (I/O, permission) are likely transient.
                return Err(PipelineError::Transient(
                    ErrorCode::Internal,
                    format!("{e}"),
                ));
            }
        };
        let pipeline = self.guard.acquire().map_err(|e| {
            PipelineError::Transient(ErrorCode::Internal, format!("model load: {e}"))
        })?;
        match self.processor.process(pipeline, &source, job) {
            Ok(out) => Ok(out),
            Err(e) => Err(classify(&e, e.to_string())),
        }
    }

    async fn finalise_ok(
        &mut self,
        job: &TranscribeJob,
        receipt: &transport::Receipt,
        output_ref: String,
        duration_ms: u64,
    ) -> Result<()> {
        let rec = ProcessedRecord {
            dedup_key: job.dedup_key.clone(),
            finished_at_ms: now_ms(),
            status: ProcessedStatus::Ok {
                output_ref: output_ref.clone(),
            },
        };
        self.processed.record(&rec).await?;
        let notify = NotifyResult {
            v: WIRE_VERSION,
            kind: NotifyKind::NotifyResult,
            dedup_key: job.dedup_key.clone(),
            source: SourceRef::from_job(&job.source),
            result: JobResult::Ok {
                output_ref,
                duration_ms,
            },
        };
        if let Err(e) = self.notify_q.enqueue(notify).await {
            warn!(error = %e, "notify enqueue failed (ok branch)");
        }
        self.transcribe_q.ack(receipt).await?;
        // Best-effort cleanup after the queue is already clear; leftover audio
        // can be collected by a future garbage-collection pass.
        if let Err(e) = self.bucket.delete(&job.audio_key).await {
            warn!(error = %e, audio_key = %job.audio_key, "bucket delete failed");
        }
        Ok(())
    }

    async fn finalise_error(
        &mut self,
        job: &TranscribeJob,
        receipt: &transport::Receipt,
        code: ErrorCode,
        msg: String,
        duration_ms: u64,
        ack: bool,
        receive_count: u32,
    ) -> Result<()> {
        if ack {
            // Persist deterministic outcome so duplicates can be absorbed.
            let rec = ProcessedRecord {
                dedup_key: job.dedup_key.clone(),
                finished_at_ms: now_ms(),
                status: ProcessedStatus::Error { error_code: code },
            };
            self.processed.record(&rec).await?;
        }
        let notify = NotifyResult {
            v: WIRE_VERSION,
            kind: NotifyKind::NotifyResult,
            dedup_key: job.dedup_key.clone(),
            source: SourceRef::from_job(&job.source),
            result: JobResult::Error {
                error_code: code,
                error_msg: msg,
                duration_ms,
            },
        };
        if let Err(e) = self.notify_q.enqueue(notify).await {
            warn!(error = %e, "notify enqueue failed (error branch)");
        }
        if ack {
            self.transcribe_q.ack(receipt).await?;
        } else {
            let delay = next_visibility_sec(receive_count);
            self.transcribe_q.nack_with_delay(receipt, delay).await?;
        }
        Ok(())
    }
}

/// Exponential backoff for transient retries: 10s -> 6h cap.
/// `receive_count` is 1 on the first attempt.
fn next_visibility_sec(receive_count: u32) -> u32 {
    const BASE: u32 = 10;
    const CAP: u32 = 6 * 60 * 60;
    let shift = receive_count.saturating_sub(1).min(31);
    BASE.saturating_mul(1u32 << shift).min(CAP)
}

enum PipelineError {
    /// The job will fail every retry (e.g. malformed audio). Ack and record.
    Deterministic(ErrorCode, String),
    /// Likely succeeds on retry (e.g. transient I/O). Do not ack.
    Transient(ErrorCode, String),
    /// Model not yet available (still pulling). Silently nack; no notify.
    ModelNotReady,
    /// Model failed to initialize after retries. Fatal.
    ModelFatal(String),
}

/// Classify a pipeline `Error` into a deterministic vs transient bucket.
/// Tunable: today we treat decode/engine as deterministic and I/O / output
/// as transient. Output errors against CouchDB are very likely network
/// hiccups, and replaying them is cheaper than going through DLQ.
fn classify(err: &Error, msg: String) -> PipelineError {
    match err {
        Error::Decode(_) => PipelineError::Deterministic(ErrorCode::DecodeFailed, msg),
        Error::Engine(_) => PipelineError::Deterministic(ErrorCode::EngineFailed, msg),
        Error::Output(_) => PipelineError::Transient(ErrorCode::OutputFailed, msg),
        Error::Io(_) => PipelineError::Transient(ErrorCode::Internal, msg),
        Error::Bucket(_) => PipelineError::Deterministic(ErrorCode::Internal, msg),
        Error::Queue(_) => PipelineError::Transient(ErrorCode::Internal, msg),
        // Config / Model / NotImplemented / Other / Input -- treat as deterministic
        // to avoid hot-looping on a misconfiguration.
        _ => PipelineError::Deterministic(ErrorCode::Internal, msg),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Wait until a shutdown signal arrives. Returns on ctrl_c (all platforms)
/// or SIGTERM (unix). On Windows only ctrl_c is wired.
async fn wait_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                // Fall back to ctrl_c only.
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests;
