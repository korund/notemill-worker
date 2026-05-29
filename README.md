# Notemill

Dictating a voice note is faster than typing one, but a voice note is useless until it is searchable text. Notemill turns voice messages sent to a bot in a messenger, transcribes them, and delivers Markdown to the note-taking system.

Voice notes are my last-mile solution for catching what I think on the go - notes for future-me, the raw material I come back to later. There are more thoughts than memory can hold, and things slip away if I do not catch them. For this to be useful, the transcription has to be good.

Phone dictation is not good. The transcription that comes built into some chat apps is workable but locked to a weak model. I use [Handy](https://handy.computer/), and the quality there is excellent - but Handy only runs on a desktop (Windows, Linux, Mac). I did not want to write my own mobile app, and that's where a chat bot turned out to be the right choice: I already write into chats all day - so a bot is just another chat window. It already feels natural.

Notemill leans on the same crate Handy is built on ([transcribe-rs](https://github.com/cjpais/transcribe-rs)), which means the same models and the same transcription quality - now available wherever I am. The bot piece - today [notemill-telegram](https://github.com/korund/notemill-telegram), a sibling project - puts jobs into the queue; this repo is the worker that picks them up.

The worker pulls a transcription job off the queue, decodes the audio from a bucket, runs voice-activity detection, transcribes it, and writes the resulting text to a configured sink. Queue, bucket, and sink are interfaces - the binary ships with one implementation of each today, and the same pipeline is extendable for new ones without touching the rest of the code.

The setup the worker is validated against:

- Producer: [notemill-telegram](https://github.com/korund/notemill-telegram) - a Telegram bot that drops voice messages into the queue.
- Queue: SQLite WAL file on a shared volume.
- Bucket: local filesystem under the same volume (claim-check pattern: queue carries the job metadata, bucket holds the audio bytes).
- Sink: CouchDB instance fronting an Obsidian vault via Self-hosted LiveSync.
- Host: a small Synology NAS (Intel Celeron J4125, no AVX).

For "why exactly this way" see [ARCHITECTURE.md](ARCHITECTURE.md).

## What it does

- A loop: `receive -> decode -> VAD -> chunk -> transcribe -> join -> write -> ack`.
- A model lifecycle manager: lazy-loaded on the first task, unloaded after an idle timeout, missing model files pulled in the background on startup.
- A small admin CLI: inspect the dead-letter queue, probe a configured CouchDB target, list / add / pull models from the catalog, transcribe a single file off-queue for debugging.

## What it does not do

- It does not own the audio source. Producers (Telegram, web, file-drop, whatever) put jobs into the queue; the worker reads them. The worker has no idea what platform the audio came from.
- It does not deliver any user-facing notification. After a job is done the worker writes a `NotifyResult` back into a result queue; the producer that originated the job picks it up and decides what to tell the user.
- It does not pick a transcription model for you. The worker wires a curated set of `transcribe-rs` engines (see Models); you choose one in the config based on the language you actually speak.
- It does not manage Obsidian. CouchDB + LiveSync sit between the worker and the vault; the worker only writes Markdown documents to CouchDB.

## Why this shape

The worker is one of several services in the Notemill project. The shape of the abstractions is shared across the project, not invented per-component:

- **Queue** is an interface, not a database. The SQLite implementation is convenient for a single-host deployment and the only backend shipped today; the trait is shaped to accept a cloud queue (SQS, Kafka, ...).
- **Bucket** is an interface, not a path. Jobs reference audio by a logical key, never an absolute path; the only backend shipped today is the local filesystem, and the trait keeps the door open for an object store (S3, GCS).
- **Sink** is an interface, not a store. Whether the result goes to `stdout`, a file, or a CouchDB document is a configuration choice.
- **Producer** is a role, not a project. Anything that feeds the worker through one of its input modes counts as a producer; the worker stays agnostic about the source.

The structure exists so that swapping any one slot for a cloud equivalent does not require a rewrite. The cloud-targeted backends are not built yet; see Status.

## Quick start

The validated path is Docker Compose on a Linux host. There is no host-level Rust toolchain requirement - the Dockerfile downloads and pins everything it needs (ONNX Runtime, Silero VAD) at build time.

```yaml
# docker-compose.yml (excerpt)
services:
  worker:
    image: ghcr.io/korund/notemill-worker:latest
    command: ["run", "queue", "--config", "/etc/notemill/worker.yaml"]
    volumes:
      - queue-data:/data                                  # shared with the producer
      - ./worker.yaml:/etc/notemill/worker.yaml:ro
      - ./models:/app/models
    secrets:
      - couchdb_password
    environment:
      RUST_LOG: info
    restart: unless-stopped

volumes:
  queue-data:

secrets:
  couchdb_password:
    file: ./secrets/couchdb_password
```

A minimal `worker.yaml` based on `config/config.example.yaml`:

```yaml
model:
  name: whisper-medium       # or any catalog entry; pick one for your language
  family: whisper
  dir: /app/models

input:
  driver: queue
  queue:
    backend: sqlite
    sqlite:
      path: /data/queue.db
    visibility_timeout_sec: 300
    max_receive: 5
    bucket:
      backend: fs
      fs:
        root: /data/buckets

output:
  sink: couchdb
  name:
    type: message_id         # or { type: datetime, format: "%Y-%m-%dT%H-%M-%SZ" }
  couchdb:
    url: http://couchdb:5984
    database: obsidian
    username: notemill
    password_file: /run/secrets/couchdb_password
    target: Inbox/Voice      # vault-relative folder for new notes
```

The producer (e.g. `notemill-telegram`) shares the same `queue-data` volume and writes jobs into `queue.db` and audio blobs into `buckets/`. On the first start the worker downloads any missing model from the catalog (progress is logged) and then begins polling the queue.

For local development without Docker, build the worker against the `full` feature set under WSL or Linux (the engine and decoder pull `ffmpeg-next` and `ort`, which need a real Linux build environment with `pkg-config`, `libavdevice-dev`, `cmake`, and `libsqlite3-dev`):

```
cargo build --release --features full
```

The default feature set is `["download"]` only - enough for `admin models pull` but not enough to run `run queue` against real audio. Building without `full` and then running the worker on real audio fails fast with `NotImplemented("engine: enable feature 'engine-transcribe' ...")`.

## Configuration

Config is YAML. By default, the worker looks for `config/config.yaml` in the working directory. Override with `--config <path>`. Individual values can be overridden from the CLI with `--set key.path=value` (repeatable, dotted keys, YAML scalars).

Top-level sections:

| Section | What it configures |
|---|---|
| `model` | Which model to use (`name`, `family`, on-disk `dir`). For the families the worker recognises, see Models below. |
| `input` | The active input mode plus its mode-specific settings.<br/>Each input mode that needs YAML config has its own sub-block (today: `queue`, with backend + bucket); the `file` mode is selected by the `run file <path>` subcommand and does not need a block here. |
| `audio` | Audio decoding and any preprocessing applied before the audio reaches the engine. |
| `output` | Which sink is active (`sink`), the note-naming strategy (`name`), and per-sink blocks (`stdout`, `file`, `couchdb`). |

The shipped `config/config.example.yaml` is the canonical, fully commented reference - start there. Two examples of `--set` overrides:

```
notemill-worker run queue --set input.queue.sqlite.path=/tmp/dev/queue.db
notemill-worker run queue --set output.couchdb.target=Inbox/Drafts
```

Environment variables read directly by the worker:

- `RUST_LOG` - tracing filter, e.g. `info`, `debug`, `notemill_worker=debug,info`.
- Whatever `password_env` points at, when a sink resolves a secret from the environment (default for the CouchDB sink is `COUCHDB_PASSWORD`). The worker does not consume any other implicit env vars.

The CouchDB sink resolves its password in this order: `password_file` (recommended for Docker secrets) takes precedence over `password_env`.

## Commands

| Command | What it does |
|---|---|
| `run queue` | Long-running queue worker. Polls the configured queue, processes one job at a time, writes results to the configured sink. |
| `run file <path>` | One-shot transcription of a local audio file. Useful for debugging models, sinks, or audio shape outside the queue flow. |
| `admin models list` | Shows the model catalog (`config/models.toml`) and which entries are already on disk. |
| `admin models pull <name>` | Downloads a catalog entry. Idempotent: skips files already present. |
| `admin models add` | Registers a new model in the catalog by URL. |
| `admin couchdb` | Probes the configured CouchDB sink: connectivity, database existence, LiveSync schema detection. |
| `admin queue dlq` | Inspects the dead-letter queue and re-queues entries by id. |

Global flags: `--config <path>`, `--set key.path=value` (repeatable).

Most sink and model fields also have dedicated CLI flags (`--output`, `--target`, `--overwrite`, `--model-name`, `--model-family`, `--model-dir`) that override the corresponding YAML values for a single invocation.

## Models

The worker supports a curated subset of the engines [transcribe-rs](https://github.com/cjpais/transcribe-rs) provides. Support is not automatic: each family has to be added to the worker in code, so the set grows with worker releases, not with upstream.

Supported today:

- **whisper.cpp:** Whisper
- **ONNX (int8):** Parakeet, GigaAM, SenseVoice, Canary, Cohere

Not supported yet: Moonshine, Whisperfile, the remote (OpenAI API) engine.

The worker ships a catalog file (`config/models.toml`) of known entries with their download URLs. To add another model of an already-supported family, run `admin models add <url> --family <family>`; it appends an entry to the catalog at runtime, with no rebuild.

The family list is fixed in code, but within a family the model is not. The bundled catalog entries are a convenience, not a lock-in: you can point at your own copy instead - a different ONNX int8 export of a supported family, for example - as long as its on-disk layout matches what that family's loader expects (the same file names the bundled model uses). The catalog entry only says where to fetch the model and which family loads it.

Three behaviors worth knowing about:

- **Lazy load.** A model is loaded into memory only on the first job that needs it, not on startup. After an idle window (default 5 minutes, configured as `input.queue.model.unload_after_ms`) it is unloaded. This matches the actual load profile: short bursts of work separated by long idle periods.
- **Background pull on the first start.** If a configured model is missing on disk, the worker spawns a background download instead of failing. The queue loop starts immediately. Jobs that arrive while a model is still pulling get NACKed back to the queue and naturally retry once the download finishes. If a pull fails three times, the worker shuts down with a loud error - this is a case for admin investigation.
- **Silero VAD is a model too.** It is part of the catalog under `[[model.vad]]` (default name `silero-vad-v6`). In a host build the worker pulls it like any other model; the Docker image bakes the file in at build time (SHA-pinned), so a Dockerized run finds it already on disk.

Picking a model is a config-time decision. The model `family` plus `name` is enough; the worker resolves the on-disk directory and loads the right engine.

## Outputs (sinks)

Three sinks ship in the binary:

- `stdout` - writes the transcribed text to standard output with an optional separator between records. Mostly a development sink.
- `file` - appends to a single text file with an optional separator. Used in `run file` and for low-friction queue runs without CouchDB. `--overwrite` truncates on open.
- `couchdb` - writes a Markdown document per job to a CouchDB database that an Obsidian vault syncs through [Self-hosted LiveSync](https://github.com/vrtmrz/obsidian-livesync). This is the sink the worker is validated against.

Note naming under per-document sinks (today: CouchDB) is configurable through the top-level `output.name` block:

- `message_id` (default) - the filename is derived deterministically from the job's `dedup_key`. Naming is idempotent: the same source always lands at the same path.
- `datetime` - a strftime-formatted timestamp via a `format` field. Collision-safe across distinct jobs (different times); on the rare collision the sink appends `-1`, `-2`, ... before the extension.

The CouchDB sink keeps a small cache (`config/.cache/livesync.yaml`) of the vault's LiveSync schema fingerprint so it does not re-probe on every start. Delete it if you change vaults, or re-run `admin couchdb` to refresh it.

## Reliability

The queue contract is at-least-once with a visibility timeout. The worker treats this strictly:

- Idempotency is the worker's job, not the queue's. Every job carries a `dedup_key`; a `ProcessedStore` records the outcome by key so a redelivered job is recognized and not re-transcribed.
- On a transient failure (decode error, engine load error, etc.), the worker emits a delayed NACK. The queue re-delivers after the visibility timeout (default 300s). Retries are exponential.
- After `max_receive` deliveries (default 5) the entry is moved to the dead-letter queue. `admin queue dlq` lists and re-queues entries; nothing is silently dropped.
- A job whose audio contains no speech is not a failure. The pipeline produces an explicit `NoSpeech` result, the worker writes a `NotifyResult` back to the result queue with that status, and the producer can show a useful message to the user instead of silence.

## Repository layout

```
notemill-worker/
|-- README.md, ARCHITECTURE.md, CHANGELOG.md
|-- Cargo.toml, Cargo.lock, Dockerfile
|-- .github/workflows/        # docker build/push, release-please, gitleaks
|-- config/
|   |-- config.example.yaml   # canonical, commented example
|   |-- models.toml           # model catalog (built-in entries)
|-- models/.gitkeep           # mount point for downloaded models
|-- src/
|   |-- main.rs, lib.rs, cli.rs
|   |-- commands/             # one module per subcommand (run/, admin/, decode)
|   |-- config.rs             # YAML schema + load + --set overrides
|   |-- pipeline.rs           # decode -> VAD -> chunk -> transcribe -> join -> sink
|   |-- input/                # AudioSource trait; queue + file impls
|   |-- input/queue/backends/ # sqlite + fs queue transports
|   |-- decode/, preprocess/  # ffmpeg decode; Silero VAD; chunker; deafness check
|   |-- engine/               # transcribe-rs adapter
|   |-- models/               # catalog, registry, manager (lazy load / idle unload)
|   |-- output/               # OutputSink trait; stdout/file/couchdb; frontmatter
|   |-- state.rs              # LiveSync schema cache
|   |-- error.rs              # error taxonomy
|-- tests/
|   |-- decode_test.rs        # integration test (the per-module ones live in src/.../tests.rs)
```

Real configs (`config/config.yaml`, `config/config.local.yaml`, ...), downloaded models, secret files, and the local LiveSync cache are gitignored.

## Status

The intent: producer, queue, bucket, and sink are independent contracts. The reality today: one local implementation per slot (SQLite queue, fs bucket, CouchDB sink, Telegram producer in a sibling repo), running on a Synology NAS with an Intel Celeron J4125. Cloud-targeted implementations (SQS, S3) are valid future additions, not a current promise.

Known caveats:

- The CouchDB sink only targets Obsidian Self-hosted LiveSync; using CouchDB for anything else would mean writing a different sink.
- In local development with the producer on Windows and the worker in WSL, both sides must share the same filesystem. SQLite WAL coordination through DrvFs / 9P does not work; the producer's writes are invisible to the worker. Docker Compose sidesteps this - the shared volume puts both sides on a single filesystem.
- Tests cover the configuration loader, CLI resolvers, the separator state machine, the file sink, the SQLite + fs queue backends, the bucket abstraction, the processed store, VAD, chunker, deafness check, pipeline join, and decoding. The end-to-end CouchDB-into-Obsidian path is validated by manual smoke tests on the canonical setup, not by CI.

## Build and runtime notes

- Validated CPU baseline: Intel Celeron J4125 (Goldmont Plus, no AVX). The Dockerfile downloads MS-official ONNX Runtime (SHA-pinned) at build time and links it dynamically at runtime, so the binary runs on commodity hardware without AVX. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full story.
- Build features: `default = ["download"]`. `engine-transcribe` and `decode-ffmpeg` are opt-in; combine all three via `--features full` for an end-to-end build. The Dockerfile builds with `full`.
- Repositories: this is `notemill-worker`. The canonical producer is `notemill-telegram`. Queue and bucket backends currently ship inside the worker crate; if a cloud-targeted implementation is added later, it may live in a separate crate.

## License

MIT.
