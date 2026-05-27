# Architectural decisions

This document explains why `notemill-worker` is built the way it is. The README covers what it does and how to run it; this one is the back of the diagram - what was true when each decision got made, what was on the table, and what won.

The system has one non-negotiable and several pressures pulling against it.

The non-negotiable is transcription quality. Voice notes are only useful if the resulting text is good enough to read months later. The models that produce text at that level exist. [Handy](https://handy.computer) uses them on the desktop for live dictation, building on the Rust crate [`transcribe-rs`](https://github.com/cjpais/transcribe-rs); this project starts from the same place. That decision - a real model rather than what the phone or messenger ships with - is what brings ONNX Runtime and a transcription crate into the picture. Most of what follows is built around them.

The pressures are mundane. The target hardware is small (a 2 GB Synology NAS today, a free-tier cloud instance later); the worker is in Rust to keep the resource envelope realistic on that hardware. The interface is a chatbot, because messengers are already the daily place to send text, audio, images, and links - the input UI is polished, the habit is there, and a bot reuses both. That puts the audio source on a phone, asynchronous and remote from the engine, so a queue between producer and worker is not an option but a requirement.

Many of the decisions below sit at the intersection of those two forces. A few are historical accidents - they landed early, they work, and there has been no reason to revisit them. Where that is the case, the section says so.

The document is by topic, not chronology. Each numbered section collects the decisions for one slice of the system. Section 8 lists what is still open. Section 9 is honest about what the test suite does and does not cover.

---

## 1. Contracts and pluggability

### 1.1. Contract is a spec, not a Rust type

**Decision.** The producer / worker boundary is a JSON envelope with a versioned discriminator. The Rust types in `src/input/queue/job.rs` are what the worker reads. Producers write the same fields into the queue; there is no separate published spec.

**Why.** Producers are not constrained to Rust. The Telegram producer is in Node, a future web producer might be in any other language, and we want each one to talk to the queue without reaching into the worker's source tree to discover the wire structure. A contract written as a spec can be implemented multiple times; a contract written as a Rust struct cannot.

**Consequence.** The wire format is not advertised as a stable, public API. "Talk to the queue" means matching the Rust types as they exist in this repository.

### 1.2. Three independent contracts: Queue, Bucket, Sink

**Decision.** The worker depends on three separate abstractions:

- **Queue**: carries jobs in (with at-least-once semantics) and results out (best-effort notify).
- **Bucket**: holds the audio bytes referenced by a job, keyed by a logical name.
- **Sink**: accepts the transcribed text and decides where it lands.

Each is independently swappable. The worker depends on a trait per contract and is configured with one implementation per slot.

**Why.** Coupling them ("queue includes the audio inline", "sink is implicit in the queue payload") feels simpler in a single-host setup, but blocks a cloud migration: in a cloud topology, jobs flow through SQS-like queues, audio lives in object storage, and notes still land in CouchDB. Three contracts mean the migration is three independent swaps; one combined contract is a rewrite.

**Alternative.** Inline audio bytes in the job payload. Rejected: messes with cloud queue size limits, blows up the queue database, and locks the wire format to a particular host topology.

### 1.3. Wire-format v1 is deliberately narrow

**Decision.** The v1 contract hardcodes one payload type ("transcribe") and one source kind ("telegram"). Every envelope carries an explicit `v: 1` field and a `type` discriminator, both checked on the worker side.

**Why.** Pretending the contract is more general than it actually is would mean shipping fields nobody validates and asymmetric behavior nobody tests. v1 is the smallest contract that lets the target setup work; if more producers or payload types arrive, v2 is the next negotiation, not a gradual drift in v1.

**Consequence.** The worker rejects unknown `v` and unknown `type` loudly.

### 1.4. Asymmetric wire compatibility (producer-ahead detection)

**Decision.** When the worker receives a job whose `v` is higher than what it understands, it does not silently NACK or crash. It logs a clear "producer is ahead of worker" message, parks the entry, and routes it to the dead-letter queue. The reverse case (worker is ahead of producer) is handled by accepting older envelopes as long as `v` is in the supported range.

**Why.** In a deployment where producer and worker are independent processes (different containers, different release cadences), one side will always be ahead at upgrade time. The default reaction of either crashing or silently dropping is dangerous; the asymmetric explicit-failure path gives the operator a chance to act before queue depth grows.

---

## 2. Queue semantics

### 2.1. At-least-once with visibility timeout

**Decision.** The queue contract mirrors the SQS contract: a successful `receive` makes the entry invisible for a visibility window (default 300s); the consumer must either delete the entry (ack) or wait for the window to elapse so the entry reappears (implicit nack). The local SQLite implementation honors exactly the same semantics.

**Why.** Modeling the local queue on the eventual cloud queue means the worker has only one mental model to deal with. Exactly-once delivery is not a contract that any sane queue offers; pretending otherwise just hides the place where duplicates would appear.

**Consequence.** Idempotency is the worker's job, not the queue's - see section 2.2.

### 2.2. Idempotency via `dedup_key` + `ProcessedStore`

**Decision.** Each job carries a `dedup_key` chosen by the producer (the Telegram producer uses `telegram_update_id`, falling back to `chat_id:message_id`). The worker keeps a `ProcessedStore` (`src/input/queue/processed.rs`) mapping `dedup_key -> { source_ref, status }`. Before doing work, the worker consults the store; a redelivered job with a recorded outcome is acknowledged without re-transcription.

**Why.** At-least-once delivery plus expensive work (decoding, model inference) means duplicates are not optional to handle. A duplicate transcription is also not idempotent from the user's perspective - it would produce a second note. The store removes both costs.

**Alternative.** Idempotency through the sink (e.g. an upsert keyed on a deterministic doc id). Rejected: forces every sink implementation to carry the same key format and the same conflict semantics. Cheaper to centralize it at the worker boundary.

Two refinements follow: section 2.6 explains why the key comes from the producer rather than the audio content, and section 2.7 explains why the store records the outcome but not the error text.

### 2.3. Delayed NACK + bounded retries, then DLQ

**Decision.** Pipeline failures fall into two buckets. **Deterministic** failures (decode error, engine error, ...) ack the job immediately and emit a `NotifyResult::Failure` with an error code - retrying would only repeat the same failure. **Transient** failures (sink write, I/O, queue error) trigger a delayed NACK with exponential backoff (10s base, doubles each attempt, capped at 6h). The retry count is tracked per entry; after `max_receive` deliveries (default 23, roughly three days) the entry is moved to the dead-letter queue and is no longer redelivered.

**Why.** Plain "NACK and retry instantly" turns a transient downstream outage into a tight loop that wastes CPU and saturates logs. A delayed NACK with exponential backoff gives the downstream room to recover. Capping the total at about three days draws the line where "the outage will be over soon" becomes "something that needs human attention". It should be long enough that a weekend-long incident gets a chance to resolve before the job is given up on.

**Consequence.** The producer learns about deterministic failures immediately, through the notify queue. It learns nothing about transient failures while they are still being retried, and nothing about jobs that finally land in the DLQ - the queue simply stops delivering them, and the worker does not emit a notification for that case. From the producer's side, an in-flight job, a slow retry, and a DLQ'd job are all indistinguishable. `admin queue dlq` exists so the operator can inspect and re-queue such entries.

### 2.4. Results go through a separate notify queue

**Decision.** When a job completes (success, no-speech, or final failure), the worker writes a `NotifyResult` envelope to a second queue. It does not call back to the producer, and it does not write to the job queue.

**Why.** A direct callback would couple the worker to the producer's transport (HTTP endpoint, websocket, whatever) and would not survive the producer being down at completion time. A second queue keeps both sides asynchronous and gives the result the same at-least-once guarantees the job side has.

**Consequence.** The producer is responsible for consuming the notify queue and reacting to results (showing the user a transcription, an error, or "no speech detected"). The worker is silent toward the user; that is the producer's job.

### 2.5. Finalization order: record, notify, ack, delete

**Decision.** A finished job is committed to terminal state in a fixed sequence: write to `ProcessedStore` first, enqueue the `NotifyResult` second, ack the job on the transcribe queue third, delete the audio from the bucket last (see section 3.2 for why the worker, not the producer, owns the delete).

**Why.** Each step makes the next one safe to lose. If the worker crashes after `record` but before `notify`, the next delivery sees the dedup row and replays the notify (see section 2.2). If it crashes after `notify` but before `ack`, the next delivery still finds the dedup row and acks idempotently. If it crashes after `ack` but before `delete`, the audio is orphaned but no work is lost. Reversing any of these (e.g. acking before recording) opens a window where a duplicate would re-run the pipeline or where the user would get no notification.

**Consequence.** The two queue writes (`notify`, `ack`) and the bucket `delete` are best-effort: a failure is logged and the next step still runs. The only step that must be durable before continuing is the `ProcessedStore` write; everything after it is recoverable on redelivery.

### 2.6. dedup_key is producer-derived, not a content hash

**Decision.** The dedup key is constructed by the producer from message metadata available before the audio is fetched (the Telegram producer uses `tg:{chat_id}:{message_id}`). It is not a hash of the audio bytes.

**Why.** Three properties matter: the producer must know the key before it has finished downloading from Telegram (a content hash forces a full read first); a deliberate resend of the same recording must remain a distinct event (a content hash would collapse it); and the key has to be readable in logs when something goes wrong (`tg:123:456` beats `sha256:...`). All three favor metadata over content.

**Consequence.** Two genuinely identical uploads through two different messages produce two notes, not one. That is the intended semantics for a voice-note tool; a deduplicating-by-content tool would have made the opposite call.

### 2.7. ProcessedStore stores outcome, not error message

**Decision.** The idempotency table records `status` (ok / no_speech / error), `output_ref` for the ok case, and `error_code` / `no_speech_reason` for the others. The free-text `error_msg` carried by a `NotifyResult` is not persisted.

**Why.** The producer's UX is driven by `status` + `error_code`: which reaction emoji to show, which canned message to send. The exact wording of an internal error is one-shot diagnostic data attached to the first notify; replaying it on a duplicate delivery weeks later adds no value and risks leaking stale context.

**Consequence.** A `NotifyResult` replayed from `ProcessedStore` carries an empty `error_msg`. The first notify is the one that has the full text; everything after it is the abbreviated form.

---

## 3. Bucket invariants

### 3.1. Audio is referenced by logical key, never absolute path

**Decision.** Job envelopes carry an `audio_key` - a relative, logical identifier. They never carry a host-absolute path, a URL, or a file handle. The worker resolves the key through the configured bucket implementation.

**Why.** The day the bucket implementation becomes object storage, no producer needs to know - the same key resolves to an object instead of a file. The day the worker runs on a different host than the producer, the same invariant means nothing breaks.

**Consequence.** Producers must put audio into the bucket before enqueuing the job. The local filesystem bucket uses a shared volume; an object-storage bucket would use a presigned PUT. The mechanism is the bucket's concern, not the wire format's.

### 3.2. The worker deletes audio after finalization, not the producer

**Decision.** When a job reaches a terminal state (success, no-speech, or wire-incompat ack), the worker calls `bucket.delete(audio_key)` as the last step of the finalization sequence (record -> notify -> ack -> delete; see section 2.5). The producer never deletes the artifact; the bucket has no separate retention mechanism.

**Why.** Only the worker knows when an `audio_key` has been fully processed. Producer-side deletion (e.g. on a TTL) would race against in-flight jobs; a separate GC sweeper would duplicate the same knowledge the worker already has at finalization time. Folding the delete into the finalization path means the bucket converges on "only in-flight audio is resident" without an external operator step.

**Consequence.** The delete is best-effort: a failed `delete` logs a warning and the job still acks, so an `audio_key` orphaned by transient I/O failure stays until manually cleared. The broader retention question (cap on total bucket size, sweep of orphans from past failures) is not addressed and would need a separate sweeper if it ever bites.

---

## 4. Pipeline stages

### 4.1. Decode -> VAD -> chunk -> transcribe -> join -> sink

**Decision.** The pipeline is six explicit stages, each a module:

1. **Decode** (`src/decode/`): turn whatever container the producer uploaded into 16 kHz mono PCM via ffmpeg.
2. **VAD** (`src/preprocess/vad.rs`): Silero VAD over the PCM, producing a speech verdict and a list of speech segments.
3. **Chunk** (`src/preprocess/chunker.rs`): subdivide long segments into inference-friendly windows with a small overlap.
4. **Transcribe** (`src/engine/`): run each chunk through `transcribe-rs`, producing text per chunk.
5. **Join** (`src/pipeline/join.rs`): stitch chunk texts together, smoothing the overlap.
6. **Sink** (`src/output/`): write the joined text where the config says.

**Why.** Each stage has a single failure mode and a single observable output. A regression in any of them - a decoder that produces clipped audio, a VAD that loses faint speech, a chunker that splits mid-word - can be localized and reproduced in isolation. A single monolithic "transcribe an audio file" function would not give that.

**Consequence.** The seams are explicit traits (`AudioDecoder`, `Transcriber`, `OutputSink`). Swapping the implementation behind any one of them - a different decoder, a different transcription engine, a different sink - leaves the rest of the pipeline untouched.

### 4.2. VAD and chunker exist because the encoder runs out of memory on long audio

**Context.** On the production Synology (2 GB RAM, debian-slim container, other services co-resident), feeding the engine a voice message longer than roughly 100 seconds fails inside ONNX Runtime with `std::bad_alloc` in a softmax node. The self-attention block scales quadratically with sequence length; on a small machine, long clips simply do not fit.

**Decision.** Insert two preprocessing stages between decode and transcribe: Silero VAD trims away non-speech regions, and the chunker caps the audio fed to the engine in any one pass. Together they bound the worst-case sequence length the engine sees, regardless of how long the user talked.

**Why.** Without them, the target hardware cannot transcribe a 5-minute voice note. Buying more RAM is not the worker's job; making the load fit the available memory is. Both stages are configurable - on a beefier host one can raise `chunking.max_seconds` or disable chunking entirely.

### 4.3. NoSpeech is an explicit result, not a silent drop

**Decision.** When VAD classifies the audio as silent (verdict `None`), the pipeline does not write anything to the sink. It returns a `RunOutcome::NoSpeech`, which the queue path turns into a `NotifyResult` with status `NoSpeech` and acks the job.

**Why.** A silent voice message is a real user scenario: pocket dial, microphone closed, user changed their mind. The naive options are bad: writing an empty note pollutes the vault; failing the job retries forever; silently acking gives the user no feedback. Surfacing it as a first-class result lets the producer say "we heard nothing" without inventing a fake error.

### 4.4. Chunker is separate from VAD

**Decision.** Voice-activity detection produces speech segments; the chunker takes those segments and subdivides them into inference-friendly windows. They are sibling stages, not the same stage.

**Why.** The two have different reasons to change. VAD parameters are about audio physics (threshold, model). Chunker parameters are about engine throughput (max chunk seconds, overlap). Conflating them once meant tweaking chunk size could quietly change VAD sensitivity; splitting them removed that. The refactor that did the split is in the git history under `refactor(preprocess): group VAD+chunker into Preprocess; move ffmpeg under audio`.

### 4.5. `join` lives outside the chunker

**Decision.** The "stitch chunk outputs back together" function is its own module (`src/pipeline/join.rs`), not a method on the chunker.

**Why.** The chunker knows how it cut. The joiner knows how to merge. Putting both on the same type made the chunker stateful across the pipeline, which is wrong: by the time we join, the chunker has done its work and is gone.

### 4.6. ffmpeg in the decoder, despite the weight it adds

**Context.** Telegram delivers voice messages as OGG/Opus, and forwarded files arrive in whatever container the original sender produced (mp3, m4a, wav, ...). The decoder has to handle all of them.

**Decision.** Decode through `ffmpeg-next` against the system libav libraries, even though this pulls a large native dependency that has to be present at build time and at runtime.

**Why.** Three alternatives were considered before settling on ffmpeg:

- **Pure-Rust alternative (`symphonia`).** `symphonia` was the obvious candidate and was tested: its Opus support is incomplete, and Opus is the format the dominant input source actually uses. That breaks the simplest path.

- **Narrow per-format Rust crates.** A separate crate per format (one for Opus, one for AAC, one for WAV, ...) would likely have lower aggregate binary weight than ffmpeg. The trade-off shows up elsewhere: each crate has its own conventions for framing, resampling, channel layout, and quirks the worker would have to absorb behind a dispatch layer. The cost is also not static - every new producer type can introduce a container or codec the worker has not seen, and each addition is another integration to write and maintain. ffmpeg encapsulates that work behind one API; the price for the encapsulation is the size.

- **Pre-conversion at the producer.** Forcing the producer to upload a canonical format (PCM or a single codec) would push the same decoder requirement onto the producer side. The producer is intentionally thin; giving it a decoder relocates the problem and multiplies it for new producer types rather than solving it. The worker is also the right side of the boundary to own format handling - it can be updated independently of the bot's release cycle.

**Consequence.** The Docker image carries `libavdevice` and its transitive dependencies at runtime, and the build image carries the `-dev` packages on top of that. The decoder module is also gated behind the `decode-ffmpeg` feature so a docs-only or test-only build does not have to drag in libav (see section 7.3).

### 4.7. Silero VAD, not amplitude-based

**Decision.** The VAD stage is a neural model (Silero v6) running through ONNX Runtime, not a classical energy/zero-crossing detector.

**Why.** Two real failure modes of voice-note input shape the choice:

- whispered dictation in a quiet private environment. Low amplitude, real speech that an energy detector would drop;
- recording in a noisy urban environment. High amplitude, no speech that an energy detector would happily forward to the engine.

Both have to be handled, and a single threshold cannot tell them apart. A semantic model trained on speech vs. non-speech does, at the cost of pulling ONNX Runtime into the dependency tree.

**Consequence.** Silero is the reason `ort` is a direct dependency of the worker; the transcription engine pulls it too, but even a deployment that swapped engines would still need ORT for VAD. The Dockerfile pin and CPU-dispatch workaround described in section 7.1 are downstream of this choice.

### 4.8. Audio is held in RAM, not streamed

**Context.** A voice note from a phone is typically short, tens of seconds to a few minutes. When the worker picks a job off the queue, it processes the audio promptly; the asynchrony in the user's workflow lives downstream of that. The user comes back to read the transcribed text hours or days later, when convenient. The worker is not under first-byte latency pressure either way.

**Decision.** The bucket fetch reads the entire audio object into memory before the decoder touches it. The decode, VAD, chunk, and transcribe stages all operate on materialized buffers, not on streams.

**Why.** Streaming buys something when memory is tight relative to input size, or when a consumer cannot wait for the whole input to be read. Neither applies. Typical clips fit comfortably in memory even on the 2 GB Synology, and the bot reply happens when the worker is done regardless of how the bytes flow inside. Without either pressure, streaming is the added complexity of three coordinated sub-pipelines (libav decoder, ONNX VAD with its windowed inference, chunker that needs to know segment lengths up front), each with its own back-pressure semantics, for no observable payoff.

**Consequence.** A pathologically large upload could exhaust worker memory; today the producer does not enforce a size limit, so this is an implicit trust assumption on the bot side. A workload shift to long-form audio (lectures, podcasts, multi-hour recordings) would make the limit visible and would need actual streaming work, not a config flag.

### 4.9. VAD produces three verdicts, not two

**Decision.** The segmenter classifies its input as one of `Detected` (one or more speech segments above the threshold), `Faint` (no segments above the threshold but the model heard something), or `None` (silent). `Detected` runs the normal pipeline. `Faint` falls back to transcribing the full PCM rather than declaring silence. `None` short-circuits with `NoSpeech`.

**Why.** A two-verdict split (speech / no speech) loses near-threshold audio: a quiet recording where Silero sees signal but not enough to draw segment boundaries would either be silently dropped or transcribed with all the silence padding still in place. The `Faint` middle ground says "the model is not confident enough to segment, but it is confident enough that something is there" and falls back to the safest behavior, which is to let the engine see the original audio. That preserves transcripts that would otherwise be reported as silent.

**Consequence.** The cutoff between `Faint` and `None` is currently a fixed constant, not a config value. Its exact value is open for revision as more real-world inputs are observed.

---

## 5. Model lifecycle

### 5.1. Lazy load + idle TTL via `ModelGuard`

**Context.** A transcription model is several hundred megabytes resident. The target hardware (a 2 GB Synology, or an AWS EC2 free-tier instance) cannot afford to dedicate that much to a service that mostly sits idle. The container, when no work is running, must be efficient to host.

**Decision.** `ModelGuard` (`src/models/`) wraps the engine handle. The model is not loaded at startup; the first job that needs it triggers the load. After an idle window (default 5 minutes, configured as `input.queue.model.unload_after_ms`) the model is unloaded and the memory is freed. Together they keep the container's idle footprint at single-digit megabytes..

**Consequence on the queue.** Because the model is not always in memory, the queue loop has to coordinate with the loading state: a job that arrives while the engine is cold triggers a load before transcription, and a job that arrives while a missing model is still being pulled from upstream gets NACKed and naturally retries (see section 5.2).

**Future room.** The same lifecycle is the seed of a hot-swap path. If different jobs need different models, the worker can drain jobs for one model, let it idle out, and then load the model needed for the next batch. This pays off in any setting where the local language coexists with English as a working lingua franca. Not implemented today; nothing in the architecture stands in the way. Touches section 5.3.

### 5.2. Background pull on the first start, NACK during pull

**Decision.** If a configured model is missing on disk at startup, the worker spawns a background download instead of failing. The queue loop starts immediately. Jobs that arrive while a download is in flight get NACKed (with the visibility timeout, so they reappear later); jobs that arrive after the download completes get processed normally.

**Why.** Failing at startup turns a first-time deployment into a manual two-step ("install, then pull, then run"). Background pull turns it into one step. NACK is the right reaction during the pull because the queue's visibility window is exactly the right mechanism for "try this again in a minute".

**Consequence.** If a pull fails three times, the worker shuts down loudly. This is a case for a human to look at (wrong URL, network outage, disk full); silently looping on a broken pull would hide the problem.

### 5.3. Model catalog is config-driven; the runnable set is owned by `transcribe-rs`

**Decision.** The worker ships a catalog (`config/models.toml`) listing a few known entries with their download URLs and family identifiers, and `admin models add` lets the operator register more without editing the binary. The set of *runnable* models is whatever `transcribe-rs` accepts.

**Why.** Maintaining a model registry is not the worker's job. `transcribe-rs` already decides which model families it supports (Whisper, Parakeet, GigaAM via ONNX). Reproducing that list inside the worker would only create a second source of truth that goes stale.

**Consequence.** The operator chooses a model based on the language they actually speak. There is no automatic language detection; the model is a config value.

### 5.4. Silero VAD is a catalog entry; the Docker image bakes it in

**Decision.** Silero VAD is treated as just another model in the catalog (under `[[model.vad]]`, default `silero-vad-v6`). It has the same lifecycle as a transcription model. The Docker image additionally downloads `silero_vad.onnx` at build time (SHA-pinned) and bakes it into `/app/models/`, so a Dockerized first start finds the file already on disk.

**Why.** Treating VAD as a special case meant a second download path, a second on-disk layout, and a second lifecycle. Folding it into the catalog meant the existing `admin models` commands work for it for free, and the lifecycle code has one path instead of two. Pre-baking the file in the image means the first transcription does not wait on a network round-trip for a tiny, infrequently changing model - it ships as a build dependency.

**Consequence.** Two code paths coexist (catalog pull on host builds, baked file in Docker), but they converge at "the file is at the expected path"; the rest of the worker is identical. If the VAD model needs to change, both paths update (catalog entry plus Dockerfile pin).

**Future room.** A `SpeechSegmenter` trait sits above the Silero implementation, so swapping in a different VAD (a newer Silero with a changed signature, or a non-Silero algorithm) is a matter of writing a sibling impl. The catalog and the lifecycle stay the same; the Silero-specific code in `src/preprocess/vad.rs` is what gets replaced. Not on the roadmap, but the door is open.

### 5.5. Queue polling cadence follows model residency

**Decision.** The queue loop polls every 1 second while the model is loaded and every 60 seconds while it is not. Both values are configurable; the asymmetry is the point.

**Why.** A hot model is the expensive resource the worker exists to amortize. While it is resident, the next job should be picked up immediately, so the idle TTL has a chance to reset before the model unloads itself out from under the next arrival. With the model unloaded, the cost balance flips: a tight poll loop wakes the worker continuously to find nothing, burns CPU on a low-resource host, and would pay the model-load latency anyway when something finally arrives. A long poll is the right behavior for that state.

**Consequence.** Maximum cold-start latency is roughly one minute (the unloaded poll interval) plus the model load itself. That is acceptable for the usage pattern in section 4.8: nobody is watching the queue empty in real time.

---

## 6. Sinks

### 6.1. `OutputSink` as the boundary

**Decision.** Every sink implements one trait (`src/output/mod.rs`). The trait is small: write a record, flush, close. The pipeline does not know which sink it is talking to.

**Why.** The "where the note ends up" decision is the most likely to change per-deployment (local debug = stdout, single-host = file, production = CouchDB, future = something else). Hiding it behind a trait keeps the pipeline stable across those changes.

### 6.2. Note naming strategies

**Decision.** Two strategies live in `output.name.type`:

- `message_id` (default): the filename is derived deterministically from the job's `dedup_key`. Reprocessing the same source produces the same document path; the sink upserts.
- `datetime`: the filename is a strftime-formatted timestamp (via `output.name.format`). Collision-safe across distinct jobs (different times). On a rare collision the sink appends `-1`, `-2`, ... before the extension.

**Why.** Determinism vs. human-readability is a real trade-off, and which one matters depends on the user. The dedup-keyed name is the right default because it preserves the idempotency guarantee end-to-end. The datetime variant exists because a human looking at the vault file list cares about "when" more than about "which message".

A third option - deriving the name from the transcript text itself - is tracked under section 8 as future work.

### 6.3. Output is plain text in a Markdown-friendly container

**Decision.** The text the pipeline writes is plain - paragraphs of prose with no headings, list markers, or any other Markdown structure added by the worker. The file sink writes that text to a `.md` extension; the CouchDB sink stores it inside a LiveSync document whose type field marks it as Markdown; stdout prints it as-is.

**Why.** The end user opens the result in Obsidian. A `.md` file (or a `type: plain` LiveSync document) drops straight into a vault and renders without an import step; the same byte stream under `.txt` would also be readable, but adds friction for the user. The Markdown envelope is a fit-with-the-consumer convention, not a content format. The actual content inside is whatever text the transcription engine produced.

**Consequence.** A consumer that wanted real Markdown structure (headings, lists, links generated from the transcript) would need a post-processing step the worker does not provide. A consumer with no Markdown affinity at all would be served just as well by the same bytes under a different extension; the envelope is a convention, not a constraint on the content.

### 6.4. CouchDB through Self-hosted LiveSync

**Decision.** The production sink writes a Markdown document into a CouchDB database. An Obsidian vault syncs that database through Self-hosted LiveSync; the document then propagates to every device the vault is open on.

**Why through LiveSync.** Writing files directly into a vault folder works on a single host. It does not survive how Obsidian is actually used: vaults live on phones, laptops, and desktops, and the supported way to keep them in sync is LiveSync. Going through LiveSync gives the worker one stable target - the LiveSync backend - regardless of which devices happen to have the vault open right now.

**Why CouchDB specifically.** Honestly, preference. LiveSync supports several backends (a folder on disk, S3-compatible object storage); any of them would let the same worker reach the same vaults. I had a CouchDB instance handy and liked the idea of "the worker is stateless, the state lives in one boring database it does not own", so that is what got wired up. A folder backend is equally workable for someone who would rather not run a database.

**Consequence.** The worker writes documents directly to CouchDB using LiveSync's document schema - it does not talk to LiveSync over any kind of API. Switching LiveSync to a different backend (folder, S3) therefore means writing a different `OutputSink` impl in the worker, not just flipping a config. One concrete inheritance from the schema: the main document `_id` is the vault path lowercased - the worker mirrors LiveSync's own convention rather than choosing its own identifier shape.

**Future room.** In a cloud deployment the trade-off could tilt: an S3 LiveSync backend would reuse the same object-storage infrastructure as the audio bucket - different buckets per concern (audio is transient input, notes are durable output), one stack to operate. No S3 sink exists in the worker today; this is a future option, not a current call.

### 6.5. v1 targets the simplest LiveSync configuration

**Decision.** The CouchDB sink supports plaintext, modern-schema (children + eden), non-obfuscated LiveSync databases. Other LiveSync configurations are out of scope for v1; the schema probe rejects them with an explicit error rather than guessing.

**Why.** The simpler configuration is what the user of this worker actually runs, and supporting the alternatives means writing and maintaining additional read/write paths that nobody is currently exercising. A loud refusal is more honest than a sink that silently writes documents the vault then fails to read back.

**Consequence.** Adding support for a different LiveSync variant later is an additive change: a new branch in the probe, a new schema marker in the cache, a new write path. Nothing in the existing sink needs to be reshaped to accommodate it.

### 6.6. LiveSync schema cache in `state.rs`

**Decision.** On first contact with a CouchDB database, the sink detects the LiveSync schema (algorithm version, hash function, document layout) and caches the fingerprint in `config/.cache/livesync.yaml`. On subsequent runs, it reads the cache instead of re-probing.

**Why.** Schema detection is several round-trips; on every startup it would add seconds of latency before the first job. Caching makes the second start instant. The cache is keyed on the database fingerprint, so switching vaults invalidates it.

**Consequence.** If LiveSync upgrades its schema, the cache needs to be deleted. The probe under `admin couchdb` re-runs detection and overwrites the cache.

**Required permissions.** The schema probe needs the configured CouchDB user to be an admin of the database the vault syncs into (not a server-wide admin / root). Without those rights, the probe fails and the sink cannot start.

---

## 7. Deployment realities

### 7.1. The validated hardware is an Intel Celeron J4125 (a small Synology NAS)

**Context.** The deployment target is a Synology NAS built around an Intel Celeron J4125 at 2.0 GHz (Goldmont Plus microarchitecture). SSE through SSE4.2 are present; AVX, AVX2, FMA, BMI1, BMI2 are not. The `ort` 2.0.0-rc.12 crate, by default, downloads a prebuilt static `libonnxruntime.a` from pyke.io. That prebuilt unconditionally emits AVX instructions in some kernels (the binary contains `VBROADCASTSS` inside an onnxruntime symbol), so on Goldmont Plus it crashes with SIGILL on startup.

**Decision.** Bypass the pyke prebuilt. At build time, the Dockerfile downloads the MS-official `libonnxruntime.so` from the upstream GitHub release (SHA-pinned via `ADD --checksum=`), sets `ORT_LIB_PATH` + `ORT_PREFER_DYNAMIC_LINK=1` so the `ort` crate links against it, and ships the same `.so` into `/usr/local/lib/` in the runtime image (with `ldconfig`). The binary loads it at runtime from there.

**Why this works.** The MS-official build does runtime CPU dispatch: it ships both vectorized and non-vectorized kernels and picks at startup based on CPUID. On a J4125 the non-vectorized path is selected and the binary runs cleanly; on a modern CPU the vectorized path is used and there is no penalty for caring about old hardware. Pyke's prebuilt does not do this - it commits to AVX at compile time and cannot fall back.

**Alternative.** Build `libonnxruntime.so` from source with vector kernels disabled. Prebuilts commonly tagged "novec" still ship vectorized code and rely on the same runtime dispatch as the MS-official build, so they are not a real fallback. A from-source build is what would save the deployment if every upstream prebuilt one day dropped its non-vector kernels altogether. Not currently shipped.

**Consequence.** The binary is portable across x86_64 down to Goldmont Plus, which covers modest NAS and SBC hardware. The downside is that the image build pulls a ~25 MB tarball over the network; the upside is that the same image is also fast on a machine that does have AVX.

### 7.2. ort version is pinned to whatever `transcribe-rs` resolves

**Decision.** The worker's direct `ort` dependency (used by the Silero session in `src/preprocess/vad.rs`) is held to the exact version that `transcribe-rs` itself pulls in. Cargo.toml comments call this out so the next bump is not accidental.

**Why.** Two `ort` versions in one binary do not coexist - they each bring their own static `OrtEnv`, and the link step produces something that runs but breaks ABI inside the second session that initializes. The worker has two `ort` consumers in process (the engine, via `transcribe-rs`; the VAD, directly) and they must share one runtime. `transcribe-rs` owns the choice because it ships the heavier of the two integrations; the worker follows.

**Consequence.** When `transcribe-rs` upgrades `ort`, the worker upgrades in lockstep. A worker upgrade that picks a newer `ort` than what `transcribe-rs` resolves is a silent way to break the binary at runtime, with no compile-time warning.

### 7.3. Default feature set is minimal

**State.** `Cargo.toml` defaults to `["download"]`. The heavy native modules - `engine-transcribe` (pulls `transcribe-rs` and `ort`) and `decode-ffmpeg` (pulls `ffmpeg-next`, requires libav* at build time) - are opt-in via the `full` feature. This was not a deliberate decision: the split landed early in development and has had no reason to change since.

**Effect.** A build without `full` is small and quick. It produces a binary that can still pull a model, list the DLQ, or build a docs-only check in CI without dragging in ffmpeg and ONNX Runtime. The same minimal scope would also be enough for integration tests that exercise the queue, bucket, and config layers - the parts that do not need a model or a codec to run. None of these uses are common in practice today; the Dockerfile and any end-to-end work always pass `--features full`. But the structure is there if any of them ever becomes important.

**Caveat.** Building without features and then running `run queue` against real audio fails fast with a clear error message, not a mystery crash.

### 7.4. SQLite WAL does not coordinate across DrvFs / 9P

**Context.** In local development, putting the queue database on a Windows-mounted path (`/mnt/c/...` from inside WSL) and writing from both sides - producer on Windows, worker in WSL - silently fails. The worker never picks up the producer's rows, even though `wal_checkpoint(TRUNCATE)` confirms the data is on disk.

**Decision.** Documented as a non-issue in production (Docker shared volume = single filesystem = no problem) and as a development-time constraint (both sides on WSL-native FS, e.g. `/tmp/notes-capture-dev/`).

**Why.** SQLite in WAL mode keeps the active-frames index in `*-shm`, which is mmap'ed shared memory. DrvFs (Windows -> WSL) and 9P (WSL -> Windows) do not propagate mmap updates across the FS boundary. Each side ends up with its own view of the WAL index, and the worker has nothing to pop.

**Consequence.** Local dev convention: both producer and worker share a path under a single FS. The README's Status section calls this out.

### 7.5. Stateless container philosophy

**Decision.** The worker container holds no state of its own. The state lives in:

- Queue (jobs in, results out).
- Bucket (audio bytes).
- CouchDB (the produced documents).
- The models' volume (downloaded models, the LiveSync schema cache).

The container is restart-safe at any point.

**Why.** Stateless containers compose with the broader operational story (graceful SIGTERM, restart policy, blue/green-friendly deploys). On a cloud platform this is no longer optional - tasks come and go. Building the local NAS deployment on the same lines means a future cloud move is not also a state migration.

### 7.6. The worker process is single-tracked

**Decision.** A single worker process handles one job at a time end to end. There is no in-process job pool, no parallel decode/transcribe pipeline, no worker thread fanout. The runtime that hosts the queue loop is single-threaded; the pipeline is free to hold a non-thread-safe state without coordination.

**Why.** A loaded transcription model already occupies most of the memory budget on the target hardware (see section 5.1). Running two jobs in parallel inside one process would either double the resident memory (two model instances) or serialize on a shared one. No real parallelism gained on the constrained CPU either way. The real unit of in-process work is one job at a time, and everything inside the process is shaped accordingly: a current-thread async runtime, a `!Send` pipeline, no reference-counted model handle.

**Consequence.** Memory peak is bounded by one job's resident footprint - model plus a single audio buffer plus its intermediate PCM and segments - which is what makes the lazy-load + idle-TTL budget described in section 5.1 fit inside the 2 GB envelope. The pipeline keeps its state without `Mutex`/`Arc` discipline: `!Send` types from `ffmpeg-next` and the transcription engines flow through the stages directly, no wrapping needed. Locks only appear at the async/sync boundary (SQLite connection, model registry) where they serve a different purpose. A panic inside the pipeline takes out at most the one in-flight job, and at-least-once + dedup recover it on the next delivery.

**Scale path.** If throughput ever needs to grow, the unit to multiply is the worker container, not the work inside it. Replicas consume from the same queue without coordinating: at-least-once delivery (section 2.1) plus the `dedup_key` store (section 2.2) make a job briefly seen by two replicas idempotent, and the stateless container philosophy (section 7.5) makes replicas interchangeable. Nothing inside the worker has to change for this to work. A single replica handles the current load; this is the direction additional parallelism is expected to come from when it is necessary.

---

## 8. Open items

The decisions above are the ones that are settled. These are the threads that are not.

### `received_at` is the wrong timestamp

The Telegram producer currently sets `received_at` to the time the webhook fired, not the time the user actually recorded the message (`message.date`). The two diverge if the bot was down.

**State.** Agreed to switch to `message.date`. Deferred to avoid scope creep in unrelated work.

### Note name from transcription text

Both naming strategies (`message_id`, `datetime`) produce non-human-readable filenames. A name derived from the first sentence or two of the transcript would be much more useful.

**State.** Blocked on two things: there is no LLM agent available to generate a name, and the pipeline order would need to be inverted (transcribe -> name -> write instead of name -> write). Deferred until a real use case.

### Cloud queue / bucket implementations

The Queue and Bucket traits exist, the SQS+S3 contract is what was modeled, and no cloud-targeted implementation has been written.

**State.** Not blocked on any design decision. Both are straightforward trait impls, just not done yet. Lives in the "until a real reason to deploy to a cloud arrives" bucket.

---

## 9. Test coverage

Tests cover the parts where regressions historically bit; the rest leans on manual smoke tests.

Unit tests exist for:

- the configuration loader;
- the CLI resolvers;
- the separator state machine;
- the file sink;
- the SQLite and fs queue backends;
- the bucket abstraction;
- the processed store;
- the VAD wrapper;
- the chunker;
- the deafness check;
- pipeline join;
- decoding;
- the job wire-format types.

The engine and the CouchDB sink against a real Obsidian vault are validated by manual smoke tests on the target setup, not by CI. Raising the CI bar requires either fixtures (audio + expected text) or a CouchDB harness, neither of which is cheap.
