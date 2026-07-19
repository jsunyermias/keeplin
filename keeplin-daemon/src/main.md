# `main.rs` — daemon entry point

Self-contained companion for `keeplin-daemon/src/main.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be
able to understand it without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the crate doc, the module declarations, and
the imports. Marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview

mod auth;
mod config;
mod event_backend;
mod metrics;
mod proto;
mod rest;
mod search;
mod server;

use std::sync::Arc;

use clap::Parser;
use keeplin_core::collab::CollabBackend;
use keeplin_core::{
    encryption::EncryptedBackend,
    storage::{db::DbBackend, fs::FsBackend, StorageBackend},
};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing_subscriber::EnvFilter;

use crate::{
    config::{Config, Mode},
    proto::keeplin::keeplin_service_server::KeeplinServiceServer,
    server::KeeplinServer,
};
```

**What it does** — This is the binary entry point for `keeplin-daemon`, the gRPC
(+ optional REST/WebSocket) server that exposes the Keeplin note-taking API over a
network socket. The file declares all eight daemon sub-modules, parses the CLI and
configuration, selects the storage backend from the loaded `Config`, optionally
wraps it with `EncryptedBackend` for at-rest encryption, finishes the decorator
stack, attaches a constant-time Basic-Auth interceptor to every gRPC call, and
starts the tonic server (and, when `http_addr` is set, the axum REST server on a
second listener sharing the same backend `Arc`). Graceful shutdown is triggered by
Ctrl-C (SIGINT).

**Dependencies** — `clap` (CLI parsing), `tokio`/`tonic`/`axum` (async runtime and
servers), `tracing`/`tracing_subscriber` (logging), `anyhow` (error plumbing);
`keeplin_core` for the storage backends (`FsBackend`, `DbBackend`), the
`EncryptedBackend` and `CollabBackend` decorators, `migrate`, `ordering`, and
`sync::run_sync`.

**Used by** — nothing imports from `main.rs`; it is the binary root
(`[[bin]] keeplin-daemon`).

**Repeated context** — decorator stack, innermost → outermost:
`MetricsBackend(EventBackend(LinkingBackend([EncryptedBackend](Fs|Db))))`.
`LinkingBackend` sits outside encryption so it parses plaintext bodies;
`EventBackend` outside that so the live feed carries the refreshed metadata;
`MetricsBackend` outermost so it counts logical operations as clients issue them.

---

## Args

**Identification** — `struct Args`, `#[derive(Parser, Debug)]` with
`#[command(name = "keeplin-daemon", …)]`. Marker `// md:Args`.

**Code** — complete and verbatim:

```rust
// md:Args
#[derive(Parser, Debug)]
#[command(name = "keeplin-daemon", about = "Keeplin core daemon (gRPC)")]
struct Args {
    #[arg(short, long, default_value = "keeplin.toml")]
    config: std::path::PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}
```

**What it does** — The clap-parsed command line: `config: std::path::PathBuf`
(`-c`/`--config`, default `keeplin.toml`) and `command: Option<Command>` (the
optional subcommand). The config file is read **once at startup**; changes while
the daemon runs have no effect. If the file does not exist, the daemon falls back
to `Config::default` and logs a warning (see `fn load_config`).

**Dependencies** — `clap::Parser` derive; `Command` below.

**Used by** — `fn main` (`Args::parse()`); no other references (verified by grep).

**Repeated context** — CLI surface:

```
keeplin-daemon [OPTIONS]                     # run the server (default)
keeplin-daemon migrate --from <A> --to <B>   # copy a store between backends, then exit
  -c, --config <PATH>   TOML config [default: keeplin.toml]
```

---

## Command

**Identification** — `enum Command`, `#[derive(clap::Subcommand, Debug)]`. Marker
`// md:Command`.

**Code** — complete and verbatim:

```rust
// md:Command
#[derive(clap::Subcommand, Debug)]
enum Command {
    Migrate {
        #[arg(long)]
        from: std::path::PathBuf,
        #[arg(long)]
        to: std::path::PathBuf,
    },
}
```

**What it does** — The optional subcommands; currently only
`Migrate { from: PathBuf, to: PathBuf }` (both `#[arg(long)]`): copy all data from
one backend into another, then exit **without starting the server**. Each side is
described by its own config file, so any combination of filesystem/server mode and
plaintext/encrypted storage works; it is a one-shot copy of the current live state
into a fresh destination (see the "Migrating between backends" section of the
README).

**Dependencies** — `clap::Subcommand` derive.

**Used by** — `Args::command`; matched in `fn main`. No other references.

---

## fn main

**Identification** — `#[tokio::main] async fn main() -> anyhow::Result<()>`.
Marker `// md:fn main`.

**Code** — complete and verbatim:

```rust
// md:fn main
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("keeplin=info".parse()?))
        .init();

    let args = Args::parse();

    match args.command {
        Some(Command::Migrate { from, to }) => run_migrate(&from, &to).await,
        None => serve(load_config(&args.config)?).await,
    }
}
```

**What it does** — (1) Installs the `tracing_subscriber` fmt logger with
`EnvFilter::from_default_env()` plus a `keeplin=info` default directive (so
`RUST_LOG` can raise/lower it). (2) Parses `Args`. (3) Dispatches:
`Some(Command::Migrate { from, to })` → `run_migrate(&from, &to).await`; `None` →
`serve(load_config(&args.config)?).await`. Any error propagates out and the
process exits non-zero.

**Dependencies** — `tokio::main` macro, `tracing_subscriber`, `Args`,
`load_config`, `serve`, `run_migrate`.

**Used by** — process entry point; no callers.

---

## fn load_config

**Identification** — `fn load_config(path: &std::path::Path) ->
anyhow::Result<Config>`. Marker `// md:fn load_config`.

**Code** — complete and verbatim:

```rust
// md:fn load_config
fn load_config(path: &std::path::Path) -> anyhow::Result<Config> {
    let mut cfg = if path.exists() {
        Config::from_file(path)?
    } else {
        tracing::warn!(path = %path.display(), "Config file not found; using defaults");
        Config::default()
    };

    if let Ok(pw) = std::env::var("KEEPLIN_ENCRYPTION_PASSWORD") {
        cfg.encryption_password = Some(pw);
    }
    if let Ok(salt) = std::env::var("KEEPLIN_KEY_SALT") {
        cfg.key_salt = Some(salt);
    }
    if let Ok(pw) = std::env::var("KEEPLIN_AUTH_PASSWORD") {
        cfg.auth_password = Some(pw);
    }
    if let Ok(user) = std::env::var("KEEPLIN_AUTH_USERNAME") {
        cfg.auth_username = Some(user);
    }
    Ok(cfg)
}
```

**What it does** — Loads a `Config` from `path` via `Config::from_file` when the
file exists; otherwise logs a warning and uses `Config::default()`. Then applies
the environment-variable overrides, **after** TOML parsing so operators can commit
the file to version control and inject secrets at deploy time:

| Variable | Overrides |
|----------|-----------|
| `KEEPLIN_ENCRYPTION_PASSWORD` | `cfg.encryption_password` |
| `KEEPLIN_KEY_SALT` | `cfg.key_salt` |
| `KEEPLIN_AUTH_PASSWORD` | `cfg.auth_password` |
| `KEEPLIN_AUTH_USERNAME` | `cfg.auth_username` |

**Errors** — propagates `Config::from_file` parse/IO errors.

**Dependencies** — `config::Config`, `std::env::var`.

**Used by** — `fn main` (server path) and `run_migrate` (once per side, so a
migration side is configured exactly like a running daemon). Verified by grep: no
other callers.

---

## fn acquire_store_lock

**Identification** — `fn acquire_store_lock(data_dir: &std::path::Path) ->
anyhow::Result<std::fs::File>`. Marker `// md:fn acquire_store_lock`.

**Code** — complete and verbatim:

```rust
// md:fn acquire_store_lock
fn acquire_store_lock(data_dir: &std::path::Path) -> anyhow::Result<std::fs::File> {
    let dir = data_dir.join(".keeplin");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("daemon.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => anyhow::bail!(
            "another keeplin-daemon is already running against {} (held lock: {}). \
             Run exactly one daemon per store; a second instance would corrupt \
             in-process write serialisation.",
            data_dir.display(),
            path.display()
        ),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}
```

**What it does** — Takes the per-store daemon lock: an OS-level **exclusive
advisory lock** on `{data_dir}/.keeplin/daemon.lock` via `std::fs::File::try_lock`
(no extra dependency). The decorator stack keeps in-process state (write
serialisation, the alias index, the live-change feed), so exactly **one daemon may
serve a store at a time** — a second process would silently violate the
single-writer assumptions. Creates `.keeplin/` if needed, opens the lock file with
`create(true).truncate(false).write(true)`, and matches `try_lock()`:
`Ok` → returns the open `File` (the caller must keep the handle alive for the
daemon's lifetime — the kernel releases the lock on process exit, crashes
included, so no stale lock can ever block a restart);
`Err(WouldBlock)` → `anyhow::bail!` with a message naming the store and the lock
path; `Err(Error(e))` → propagates the IO error.

**Edge cases** — the lock file lives inside `.keeplin/` (per-device, excluded from
replication) and its *contents* are irrelevant — Syncthing copying an empty file
carries no lock.

**Dependencies** — `std::fs::{OpenOptions, TryLockError}`.

**Used by** — `serve` (one lock, held for the whole serve lifetime) and
`run_migrate` (locks **both** stores for the copy); the
`store_lock_is_exclusive_per_store_and_released_on_drop` test.

---

## fn serve

**Identification** — `async fn serve(cfg: Config) -> anyhow::Result<()>`. Marker
`// md:fn serve`.

**Code** — complete and verbatim:

```rust
// md:fn serve
async fn serve(cfg: Config) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = cfg.grpc_addr.parse()?;

    if let Err(reason) = cfg.validate_auth() {
        anyhow::bail!("invalid authentication configuration: {reason}");
    }
    let auth_configured = cfg.auth_enabled();

    let _store_lock = acquire_store_lock(&cfg.data_dir)?;

    let issues = cfg.security_issues();
    if !issues.is_empty() {
        if cfg.insecure {
            for issue in &issues {
                tracing::warn!("insecure = true, starting despite: {issue}");
            }
        } else {
            let list = issues
                .iter()
                .map(|i| format!("  - {i}"))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "refusing to start — insecure configuration:\n{list}\n\
                 Fix the above, or set `insecure = true` if another layer (isolated network, \
                 mTLS, a proxy that also enforces auth) provides this protection."
            );
        }
    }

    let encrypted = cfg.encryption_password.is_some();

    if encrypted && cfg.key_salt.is_none() {
        tracing::warn!(
            path = %key_salt_path(&cfg).display(),
            "encryption is enabled but key_salt is not set: encrypted data is bound to \
             this store's persisted salt file and cannot be decrypted on other devices. \
             Back up that file (it is required, with your password, to recover the data), \
             or set the same key_salt on all devices for encrypted multi-device sync."
        );
    }

    tracing::info!(mode = ?cfg.mode, %addr, encrypted, auth = auth_configured, "Starting keeplin-daemon");

    match (cfg.mode.clone(), cfg.encryption_password.clone()) {
        (Mode::Offline, None) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            tracing::info!(data_dir = %cfg.data_dir.display(), "Offline mode");
            run_server(&cfg, addr, backend).await?;
        }
        (Mode::Offline, Some(pw)) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            let salt = resolve_key_salt(&cfg, &backend).await?;
            let enc = EncryptedBackend::new(backend, &pw, &salt).await?;
            tracing::info!(data_dir = %cfg.data_dir.display(), "Offline mode (encrypted)");
            run_server(&cfg, addr, enc).await?;
        }
        (Mode::Server, None) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            tracing::info!(db = %db_path.display(), server = %cfg.server_url, "Server mode");
            match collab_config(&cfg) {
                Some(collab_cfg) => {
                    let collab = CollabBackend::new(backend, collab_cfg)?;
                    let starter = collab_starter(&collab);
                    let handle = collab.handle();
                    run_server_with(&cfg, addr, collab, Some(starter), Some(handle)).await?;
                }
                None => run_server(&cfg, addr, backend).await?,
            }
        }
        (Mode::Server, Some(pw)) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            let salt = resolve_key_salt(&cfg, &backend).await?;
            let enc = EncryptedBackend::new(backend, &pw, &salt).await?;
            tracing::info!(db = %db_path.display(), server = %cfg.server_url, "Server mode (encrypted)");
            match collab_config(&cfg) {
                Some(collab_cfg) => {
                    let collab = CollabBackend::new(enc, collab_cfg)?;
                    let starter = collab_starter(&collab);
                    let handle = collab.handle();
                    run_server_with(&cfg, addr, collab, Some(starter), Some(handle)).await?;
                }
                None => run_server(&cfg, addr, enc).await?,
            }
        }
    }

    Ok(())
}
```

**What it does** — Builds the storage backend and runs the server until shutdown:

1. Parses `cfg.grpc_addr` into a `SocketAddr`.
2. **Auth validation** — `cfg.validate_auth()`: refuses to start on a credential
   configuration that would silently disable auth (only one of username/password
   set, or an empty string) instead of leaving the daemon unexpectedly wide open
   (issue #73). A valid pair enables auth; both unset leaves it intentionally off.
3. **Store lock** — `acquire_store_lock(&cfg.data_dir)`, held in `_store_lock` for
   the whole lifetime of `serve` (released by the OS on exit or crash).
4. **Security gate** — `cfg.security_issues()`: refuses to start on a
   configuration that would expose data or credentials on an untrusted network
   (unauthenticated network listener, or a plaintext `ws://` sync URL that leaks
   the token), listing every issue in the error. `insecure = true` downgrades each
   to a `WARN` for deployments where another layer (isolated network, mTLS, an
   auth-enforcing proxy) provides the protection. Missing daemon-terminated TLS is
   intentionally **not** flagged — fronting TLS at a reverse proxy is supported.
5. **Salt warning** — when encryption is on without an explicit `key_salt`, warns
   (with the persisted-salt path) that encrypted data is bound to this store's
   salt file: other devices cannot decrypt it, so sync deployments must set a
   shared `key_salt` everywhere, and everyone should back the file up.
6. **Backend construction** by `(cfg.mode, cfg.encryption_password)`:

   | mode, password | backend |
   |---|---|
   | `Offline, None` | `FsBackend::new(data_dir)` |
   | `Offline, Some(pw)` | `EncryptedBackend::new(FsBackend, pw, resolve_key_salt(…))` |
   | `Server, None` | `DbBackend::new(data_dir/keeplin.db, server_url, auth_token)` |
   | `Server, Some(pw)` | `EncryptedBackend::new(DbBackend, …)` |

   In server mode, when `collab_config(&cfg)` is `Some`, the backend (or its
   encrypted wrapper — collab wraps the **decrypted** view because the line
   protocol needs plaintext bodies; the collaborative server merges by line) is
   further wrapped in `CollabBackend::new`, and `run_server_with` receives the
   `collab_starter` hook plus the `CollabHandle`. Otherwise plain `run_server`.

**Errors** — bad `grpc_addr`, invalid auth config, held store lock, insecure
config, and any backend-construction error (including the `DbBackend::new`
protocol handshake: an incompatible sync server fails startup loudly, naming
which side to upgrade; a server without `/version` warns and continues).

**Dependencies** — `Config` (`validate_auth`, `auth_enabled`, `security_issues`),
`acquire_store_lock`, `FsBackend`, `DbBackend`, `EncryptedBackend`,
`CollabBackend`, `resolve_key_salt`, `key_salt_path`, `collab_config`,
`collab_starter`, `run_server`, `run_server_with`.

**Used by** — `fn main` (the default, no-subcommand path). No other callers.

**Repeated context** — server-protocol handshake: `DbBackend::new` performs
`GET /version` against the sync server (`keeplin_core::compat`,
`PROTOCOL_VERSION = 1`, exact match required).

---

## fn resolve_key_salt

**Identification** — `async fn resolve_key_salt<B: StorageBackend>(cfg: &Config,
backend: &B) -> anyhow::Result<Vec<u8>>`. Marker `// md:fn resolve_key_salt`.

**Code** — complete and verbatim:

```rust
// md:fn resolve_key_salt
async fn resolve_key_salt<B: StorageBackend>(cfg: &Config, backend: &B) -> anyhow::Result<Vec<u8>> {
    if let Some(salt) = &cfg.key_salt {
        return Ok(salt.as_bytes().to_vec());
    }
    let path = key_salt_path(cfg);
    match tokio::fs::read_to_string(&path).await {
        Ok(persisted) => {
            let persisted = persisted.trim();
            if !persisted.is_empty() {
                return Ok(persisted.as_bytes().to_vec());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let salt = backend.get_device_id().await?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, &salt).await?;
    tracing::warn!(
        path = %path.display(),
        "encryption key salt persisted (device-id fallback); BACK UP this file — without \
         it (or its value in `key_salt`) and your password, the encrypted data cannot be \
         recovered"
    );
    Ok(salt.into_bytes())
}
```

**What it does** — Resolves the Argon2id salt used to derive the at-rest
encryption key, with a fixed precedence that is a **data-recovery contract**:

1. Configured `cfg.key_salt` → its bytes (the value that must be shared across
   devices for portable encryption). Nothing is persisted.
2. Otherwise read `{data_dir}/.keeplin/key_salt`; a non-empty trimmed value wins.
   (An empty file cannot have been the salt of any real key; fall through.)
3. Otherwise derive from this device's ID (`backend.get_device_id()`), **persist
   it** to that file (creating parents), log a prominent BACK-UP warning, and
   return it.

Persisting the fallback matters for recovery: the salt is required (with the
password) to derive the key, and before this file existed it lived only implicitly
in `.keeplin/device_id` — losing that one file made encrypted data unrecoverable
even with the correct password. Now there is a single, explicitly named,
plaintext-safe file the user can back up, and restoring it into a fresh store is
all a recovery needs. The file also takes precedence over the device ID, so a
store whose device-id file was regenerated (or adopted by another machine) still
decrypts.

**Errors** — any IO error other than `NotFound` when reading the salt file;
`get_device_id` / write errors.

**Dependencies** — `tokio::fs`, `key_salt_path`, `StorageBackend::get_device_id`.

**Used by** — `serve` (both encrypted arms), `build_storage` (both encrypted
arms), and the three `key_salt_*` tests.

---

## fn key_salt_path

**Identification** — `fn key_salt_path(cfg: &Config) -> std::path::PathBuf`.
Marker `// md:fn key_salt_path`.

**Code** — complete and verbatim:

```rust
// md:fn key_salt_path
fn key_salt_path(cfg: &Config) -> std::path::PathBuf {
    cfg.data_dir.join(".keeplin").join("key_salt")
}
```

**What it does** — Where the effective encryption salt is persisted when
`key_salt` is not configured: `{data_dir}/.keeplin/key_salt`. The salt is not
secret (see SECURITY.md), so a plain file is fine; what matters is that it is
explicit, stable, and easy to back up.

**Used by** — `resolve_key_salt`, the salt warning in `serve`, and the
`key_salt_*` tests.

---

## fn build_storage

**Identification** — `async fn build_storage(cfg: &Config) ->
anyhow::Result<Arc<dyn StorageBackend>>`. Marker `// md:fn build_storage`.

**Code** — complete and verbatim:

```rust
// md:fn build_storage
async fn build_storage(cfg: &Config) -> anyhow::Result<Arc<dyn StorageBackend>> {
    tokio::fs::create_dir_all(&cfg.data_dir).await?;
    Ok(match (cfg.mode.clone(), cfg.encryption_password.clone()) {
        (Mode::Offline, None) => Arc::new(FsBackend::new(&cfg.data_dir).await?) as _,
        (Mode::Offline, Some(pw)) => {
            let backend = FsBackend::new(&cfg.data_dir).await?;
            let salt = resolve_key_salt(cfg, &backend).await?;
            Arc::new(EncryptedBackend::new(backend, &pw, &salt).await?) as _
        }
        (Mode::Server, None) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            Arc::new(DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?) as _
        }
        (Mode::Server, Some(pw)) => {
            let db_path = cfg.data_dir.join("keeplin.db");
            let backend = DbBackend::new(&db_path, &cfg.server_url, &cfg.auth_token).await?;
            let salt = resolve_key_salt(cfg, &backend).await?;
            Arc::new(EncryptedBackend::new(backend, &pw, &salt).await?) as _
        }
    })
}
```

**What it does** — Builds the **base** storage stack described by `cfg`,
type-erased behind `Arc<dyn StorageBackend>`: `FsBackend`/`DbBackend` plus an
optional `EncryptedBackend`, **without** the `LinkingBackend`/`EventBackend`/
`MetricsBackend` decorators the server adds. First ensures `cfg.data_dir` exists:
`FsBackend::new` already does this, but `DbBackend::new` opens the `.db` file
directly and fails (SQLITE_CANTOPEN) if the parent is missing — the common case
for a fresh migration destination. Then the same four-way `(mode, password)`
match as `serve`, each arm coerced `as _` to the trait object.

**Why type-erased** — `run_migrate` must hold two heterogeneous backends at once
and needs neither link derivation nor the live feed. The server path keeps its own
generic (monomorphised) construction in `serve`, because `run_server<B>` and the
decorator wrapping need the concrete backend type.

**Dependencies** — `FsBackend`, `DbBackend`, `EncryptedBackend`,
`resolve_key_salt`, `tokio::fs::create_dir_all`.

**Used by** — `run_migrate` (both sides). No other callers (verified by grep).

---

## fn run_migrate

**Identification** — `async fn run_migrate(from: &std::path::Path, to:
&std::path::Path) -> anyhow::Result<()>`. Marker `// md:fn run_migrate`.

**Code** — complete and verbatim:

```rust
// md:fn run_migrate
async fn run_migrate(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    let from_cfg = load_config(from)?;
    let to_cfg = load_config(to)?;
    tracing::info!(
        from = %from.display(), from_mode = ?from_cfg.mode,
        to = %to.display(), to_mode = ?to_cfg.mode,
        "Starting migration",
    );

    let _from_lock = acquire_store_lock(&from_cfg.data_dir)?;
    let _to_lock = acquire_store_lock(&to_cfg.data_dir)?;

    let src = build_storage(&from_cfg).await?;
    let dst = build_storage(&to_cfg).await?;

    let report = keeplin_core::migrate::migrate(src.as_ref(), dst.as_ref()).await?;
    tracing::info!(
        notebooks = report.notebooks,
        tags = report.tags,
        notes = report.notes,
        note_tags = report.note_tags,
        resources = report.resources,
        "Migration complete",
    );
    println!(
        "Migration complete: {} notebooks, {} tags, {} notes, {} note-tags, {} resources",
        report.notebooks, report.tags, report.notes, report.note_tags, report.resources,
    );
    Ok(())
}
```

**What it does** — Implements the `migrate` subcommand: copies all data from the
backend described by `from` into the backend described by `to`, then exits.
(1) `load_config` each side independently — modes, paths, and encryption keys are
separate, so `Fs ↔ Db` and plaintext ↔ encrypted (even different keys) all work.
(2) Takes **both** store locks — migration must not race a live daemon on either
side; a running daemon on either store makes this fail fast. (3) `build_storage`
each side. (4) `keeplin_core::migrate::migrate(src, dst)`, which copies every live
entity through the typed `create_*` methods so the destination stores and
re-indexes natively; logs and `println!`s the `MigrationReport` counts (notebooks,
tags, notes, note-tags, resources).

**Dependencies** — `load_config`, `acquire_store_lock`, `build_storage`,
`keeplin_core::migrate::migrate`.

**Used by** — `fn main` (the `Migrate` subcommand arm). No other callers.

---

## StackHook

**Identification** — `type StackHook = Box<dyn
FnOnce(Arc<dyn keeplin_core::storage::StorageBackend>) + Send>`. Marker
`// md:StackHook`.

**Code** — complete and verbatim:

```rust
// md:StackHook
type StackHook = Box<dyn FnOnce(Arc<dyn keeplin_core::storage::StorageBackend>) + Send>;
```

**What it does** — Callback invoked with the fully-built decorator stack, used to
hand the collaborative client its top-of-stack handle: remote writes must flow
through linking + eventing + metrics exactly like local ones, so the collab
connection task cannot be spawned until the final `Arc` exists.

**Used by** — produced by `collab_starter`, consumed by `run_server_with`
(`stack_hook: Option<StackHook>`).

---

## fn collab_config

**Identification** — `fn collab_config(cfg: &Config) ->
Option<keeplin_core::collab::CollabConfig>`. Marker `// md:fn collab_config`.

**Code** — complete and verbatim:

```rust
// md:fn collab_config
fn collab_config(cfg: &Config) -> Option<keeplin_core::collab::CollabConfig> {
    let api_url = cfg.collab_api_url.clone()?;
    let ws_url = format!(
        "{}/api/ws",
        api_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    Some(keeplin_core::collab::CollabConfig {
        api_url,
        ws_url,
        token: cfg.auth_token.clone(),
    })
}
```

**What it does** — Derives the collaborative-channel settings from the daemon
config. Returns `None` when `cfg.collab_api_url` is unset (collab disabled).
Otherwise builds `ws_url` as `{api_url}/api/ws` with the scheme rewritten
`https:// → wss://` and `http:// → ws://` (first occurrence only), and returns
`CollabConfig { api_url, ws_url, token: cfg.auth_token.clone() }`.

**Dependencies** — `keeplin_core::collab::CollabConfig`.

**Used by** — `serve` (both server-mode arms). No other callers.

---

## fn collab_starter

**Identification** — `fn collab_starter<B: keeplin_core::storage::StorageBackend>(
collab: &CollabBackend<B>) -> StackHook`. Marker `// md:fn collab_starter`.

**Code** — complete and verbatim:

```rust
// md:fn collab_starter
fn collab_starter<B: keeplin_core::storage::StorageBackend>(
    collab: &CollabBackend<B>,
) -> StackHook {
    let collab = collab.clone();
    Box::new(move |top| {
        tokio::spawn(async move {
            if let Err(e) = collab.start(top).await {
                tracing::error!(error = %e, "collaborative channel disabled");
            }
        });
    })
}
```

**What it does** — Builds the hook that starts the collab connection task once the
stack `Arc` exists: clones the `CollabBackend` and returns a boxed `FnOnce` that
`tokio::spawn`s `collab.start(top)`. An incompatible server refuses the session
(no sync is attempted) — the relay handshake in `DbBackend::new` already failed
the daemon's startup for the same server, so here, for a collab-only drift (a
collab URL pointing at a *different* server than `server_url`), a loud
`collaborative channel disabled` error log is the surface.

**Dependencies** — `CollabBackend::{clone, start}`, `tokio::spawn`, `StackHook`.

**Used by** — `serve` (both server-mode collab arms). No other callers.

---

## fn run_server

**Identification** — `#[allow(clippy::result_large_err)] async fn
run_server<B: keeplin_core::storage::StorageBackend>(cfg: &Config, addr:
SocketAddr, backend: B) -> anyhow::Result<()>`. Marker `// md:fn run_server`.

**Code** — complete and verbatim:

```rust
// md:fn run_server
#[allow(clippy::result_large_err)]
async fn run_server<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
) -> anyhow::Result<()> {
    run_server_with(cfg, addr, backend, None, None).await
}
```

**What it does** — Convenience wrapper: `run_server_with(cfg, addr, backend,
None, None)` — the non-collab path. Generic over `B` so the compiler produces one
monomorphised copy per backend combination (no dynamic dispatch on the way in).
The clippy allow exists because `tonic::Status`/tonic's `tls_config` error exceeds
clippy's default `Err`-variant size threshold; the error is returned once at
startup, so boxing would be pointless.

**Used by** — `serve` (the three non-collab arms). No other callers.

---

## fn run_server_with

**Identification** — `#[allow(clippy::result_large_err)] async fn
run_server_with<B: keeplin_core::storage::StorageBackend>(cfg: &Config, addr:
SocketAddr, backend: B, stack_hook: Option<StackHook>, collab_handle:
Option<keeplin_core::collab::CollabHandle>) -> anyhow::Result<()>`. Marker
`// md:fn run_server_with`.

**Code** — complete and verbatim:

```rust
// md:fn run_server_with
#[allow(clippy::result_large_err)]
async fn run_server_with<B: keeplin_core::storage::StorageBackend>(
    cfg: &Config,
    addr: std::net::SocketAddr,
    backend: B,
    stack_hook: Option<StackHook>,
    collab_handle: Option<keeplin_core::collab::CollabHandle>,
) -> anyhow::Result<()> {
    let backend = keeplin_core::linking::LinkingBackend::new(backend);
    let (events, _rx) = tokio::sync::broadcast::channel::<keeplin_core::models::Change>(1024);
    let backend = event_backend::EventBackend::new(backend, events.clone());
    let metrics = Arc::new(metrics::Metrics::new());
    let backend = Arc::new(metrics::MetricsBackend::new(backend, metrics.clone()));

    keeplin_core::ordering::ensure_inbox(backend.as_ref()).await?;

    if let Some(hook) = stack_hook {
        hook(backend.clone());
    }

    if cfg.sync_interval_secs > 0 {
        let backend = backend.clone();
        let period = std::time::Duration::from_secs(cfg.sync_interval_secs);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(period);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = keeplin_core::sync::run_sync(backend.as_ref(), |_, _| {}).await {
                    tracing::warn!(error = %e, "background sync cycle failed");
                }
            }
        });
    }

    let (auth_user, auth_pass) = (cfg.auth_username.clone(), cfg.auth_password.clone());

    let svc_inner = KeeplinServiceServer::new(KeeplinServer::from_shared(
        backend.clone(),
        cfg.journal_retention_days,
        cfg.resource_purge_days,
        cfg.max_upload_bytes,
    ))
    .max_decoding_message_size(cfg.max_message_size)
    .max_encoding_message_size(cfg.max_message_size);

    let svc = InterceptedService::new(svc_inner, move |req: tonic::Request<()>| {
        validate_basic_auth(req, auth_user.as_deref(), auth_pass.as_deref())
    });

    let mut builder = Server::builder();
    if let (Some(cert_path), Some(key_path)) = (&cfg.tls_cert_path, &cfg.tls_key_path) {
        let cert = tokio::fs::read(cert_path).await?;
        let key = tokio::fs::read(key_path).await?;
        let identity = Identity::from_pem(cert, key);
        builder = builder.tls_config(ServerTlsConfig::new().identity(identity))?;
        tracing::info!("TLS enabled (gRPC)");
    }

    tracing::info!(%addr, "gRPC server listening");
    let grpc = builder
        .add_service(svc)
        .serve_with_shutdown(addr, shutdown_signal());

    if let Some(http_addr) = &cfg.http_addr {
        let http_addr: std::net::SocketAddr = http_addr.parse()?;
        let search = search::start(backend.clone(), events.clone()).await;
        let state = Arc::new(rest::AppState {
            backend: backend.clone(),
            collab: collab_handle.clone(),
            search,
            events: events.clone(),
            metrics: metrics.clone(),
            max_body_bytes: cfg.max_message_size,
            max_upload_bytes: cfg.max_upload_bytes,
            journal_retention_days: cfg.journal_retention_days,
            resource_purge_days: cfg.resource_purge_days,
            auth_username: cfg.auth_username.clone(),
            auth_password: cfg.auth_password.clone(),
        });
        let app = rest::router(state);
        let listener = tokio::net::TcpListener::bind(http_addr).await?;
        tracing::info!(%http_addr, "HTTP (REST) server listening");
        let http = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

        tokio::try_join!(
            async move { grpc.await.map_err(anyhow::Error::from) },
            async move { http.await.map_err(anyhow::Error::from) },
        )?;
    } else {
        grpc.await?;
    }

    Ok(())
}
```

**What it does** — Finishes the decorator stack and runs the server(s) until
shutdown:

1. **Stack assembly** (innermost → outermost): the caller's backend (already
   `EncryptedBackend`-wrapped if a password is set) → `LinkingBackend` (derives
   bookmarks/links from each plaintext note body, resolves references) →
   `EventBackend` (publishes every mutation to a `tokio::sync::broadcast` channel
   of `keeplin_core::models::Change`, capacity 1024) → `MetricsBackend`
   (records each operation for `/api/metrics`), the whole thing in one `Arc`
   shared by every surface. `LinkingBackend` sits outside encryption so it parses
   plaintext; `EventBackend` outside that so the feed carries refreshed metadata;
   `MetricsBackend` outermost so it counts logical client operations.
2. **Inbox bootstrap** — `keeplin_core::ordering::ensure_inbox(backend)`: the
   Inbox system notebook (nil UUID) must exist before any request, because new
   notes without a notebook land in it. Idempotent on every startup.
3. **Collab hand-off** — if `stack_hook` is `Some`, invokes it with
   `backend.clone()` so remote writes flow through the full stack.
4. **Background sync loop** (issue #111) — with `cfg.sync_interval_secs > 0`,
   spawns a `tokio::time::interval` task (first immediate tick consumed) calling
   `keeplin_core::sync::run_sync(backend, |_, _| {})` each period; with `0`,
   syncing stays frontend-driven. A cycle that errors is logged (`WARN`) and
   retried next tick — `run_sync` leaves the watermark untouched on failure.
5. **gRPC service** — `KeeplinServiceServer::new(KeeplinServer::from_shared(
   backend, journal_retention_days, resource_purge_days, max_upload_bytes))` with
   `max_decoding/encoding_message_size = cfg.max_message_size`, wrapped in
   `InterceptedService` running `validate_basic_auth` on **every** RPC before the
   handler (uniform across storage modes; transparent no-op without credentials).
6. **TLS** — when both `tls_cert_path` and `tls_key_path` are set, loads the PEM
   pair into an `Identity` and enables `ServerTlsConfig` on the builder.
7. **Serve** — gRPC via `serve_with_shutdown(addr, shutdown_signal())`. When
   `cfg.http_addr` is set, additionally: starts the full-text search index
   (`search::start(backend, events)` — rebuilt from the store, kept live off the
   change stream, exposed at `GET /api/search`), builds `rest::AppState` (same
   backend `Arc`, the `CollabHandle`, the search handle, a clone of the broadcast
   `Sender`, the `Metrics` registry, size limits, retention settings, and the
   Basic-Auth credentials), binds `rest::router(state)` on that port with axum,
   and runs **both** servers under `tokio::try_join!` — Ctrl-C drains both; if
   either exits with an error, the join aborts.

**Errors** — bad `http_addr`, `ensure_inbox` failure, TLS file IO, tonic/axum
serve errors.

**Dependencies** — `linking::LinkingBackend`, `event_backend::EventBackend`,
`metrics::{Metrics, MetricsBackend}`, `ordering::ensure_inbox`, `sync::run_sync`,
`search::start`, `rest::{AppState, router}`, `KeeplinServer`,
`validate_basic_auth`, `shutdown_signal`, tonic + axum.

**Used by** — `run_server` and `serve` (the two collab arms). No other callers.

**Repeated context** — one shared backend instance behind every surface: gRPC and
REST both hold a clone of the same `Arc`, so an operation from either surface is
linked, published, and counted exactly once.

---

## fn shutdown_signal

**Identification** — `async fn shutdown_signal()`. Marker
`// md:fn shutdown_signal`.

**Code** — complete and verbatim:

```rust
// md:fn shutdown_signal
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received, draining connections");
}
```

**What it does** — Resolves when the process receives Ctrl-C (SIGINT), then logs
"Shutdown signal received, draining connections". Each server awaits its own copy;
on Unix every `tokio::signal::ctrl_c()` future fires on the same signal, so both
listeners drain together. Errors from `ctrl_c()` are swallowed (`let _`).

**Used by** — `run_server_with` (once for gRPC, once for HTTP). No other callers.

---

## fn validate_basic_auth

**Identification** — `#[allow(clippy::result_large_err)] fn validate_basic_auth(
req: tonic::Request<()>, expected_user: Option<&str>, expected_pass:
Option<&str>) -> Result<tonic::Request<()>, tonic::Status>`. Marker
`// md:fn validate_basic_auth`.

**Code** — complete and verbatim:

```rust
// md:fn validate_basic_auth
#[allow(clippy::result_large_err)]
fn validate_basic_auth(
    req: tonic::Request<()>,
    expected_user: Option<&str>,
    expected_pass: Option<&str>,
) -> Result<tonic::Request<()>, tonic::Status> {
    let (Some(expected_user), Some(expected_pass)) = (expected_user, expected_pass) else {
        return Ok(req);
    };

    let header = req
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    if auth::verify_basic(header, expected_user, expected_pass) {
        Ok(req)
    } else {
        Err(tonic::Status::unauthenticated("invalid credentials"))
    }
}
```

**What it does** — Validates the `Authorization: Basic <base64(user ":" pass)>`
header on an incoming gRPC request. When `expected_user`/`expected_pass` are not
**both** `Some` (auth not configured), returns `Ok(req)` immediately without
inspecting any header. Otherwise extracts the `authorization` metadata entry
(`to_str().ok()` → `None` on non-ASCII) and hands it to `auth::verify_basic`,
which: (1) rejects empty expected credentials outright (an empty pair would accept
`Basic Og==`); (2) parses the scheme per RFC 7617/7235 — case-insensitive, any
separating whitespace — then Base64-decodes the token; (3) splits the decoded
value on the **first** colon (passwords may contain colons); (4) compares both
parts with `subtle::ConstantTimeEq` to prevent timing side-channels. On failure
returns `tonic::Status::unauthenticated("invalid credentials")` — intentionally
terse to avoid leaking information to an unauthenticated caller.

**Edge cases** — a half-configured or empty credential pair never reaches this
point: the daemon refuses to start on it (`Config::validate_auth`). The clippy
allow exists because `tonic::Status` exceeds clippy's `Err`-size threshold and
boxing would add a heap allocation to every RPC.

**Dependencies** — `auth::verify_basic`, `tonic::{Request, Status}`.

**Used by** — the `InterceptedService` closure in `run_server_with`, and the nine
`auth_*` tests.

---

## mod tests

**Identification** — `#[cfg(test)] mod tests`. Marker `// md:mod tests`. Imports
`super::*`, base64's `STANDARD` engine, and `keeplin_core::storage::SyncBackend as _`
(for `get_device_id` on `FsBackend`).

**Code** — container: members documented as sub-blocks below: fn store_lock_is_exclusive_per_store_and_released_on_drop, fn cfg_at, fn key_salt_config_value_wins_and_persists_nothing, fn key_salt_fallback_is_persisted_and_stable, fn key_salt_file_survives_a_regenerated_device_id, fn make_req, fn basic, fn auth_not_configured_allows_all, fn auth_valid_credentials_pass, fn auth_wrong_password_rejected, fn auth_wrong_user_rejected, fn auth_missing_header_rejected, fn auth_bearer_scheme_rejected, fn auth_malformed_base64_rejected, fn auth_no_colon_in_credentials_rejected, fn auth_password_containing_colon_works.

### fn store_lock_is_exclusive_per_store_and_released_on_drop

**Identification** — `#[test]`; marker
`// md:mod tests > fn store_lock_is_exclusive_per_store_and_released_on_drop`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn store_lock_is_exclusive_per_store_and_released_on_drop
    #[test]
    fn store_lock_is_exclusive_per_store_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_store_lock(dir.path()).unwrap();

        let err = acquire_store_lock(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("already running"),
            "second daemon on the same store must be refused: {err}"
        );

        let other = tempfile::tempdir().unwrap();
        let _other_lock = acquire_store_lock(other.path()).unwrap();

        drop(first);
        let _second = acquire_store_lock(dir.path()).unwrap();
    }
```

**What it does** — First lock succeeds; a second `acquire_store_lock` on the same
dir fails with a message containing "already running"; a *different* store is
unaffected; dropping the first lock lets the next daemon in.

### fn cfg_at

**Identification** — helper; marker `// md:mod tests > fn cfg_at`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn cfg_at
    fn cfg_at(dir: &std::path::Path) -> Config {
        Config {
            data_dir: dir.to_path_buf(),
            ..Config::default()
        }
    }
```

**What it does** — A default (offline, unencrypted) `Config` rooted at `dir`.

### fn key_salt_config_value_wins_and_persists_nothing

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn key_salt_config_value_wins_and_persists_nothing`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn key_salt_config_value_wins_and_persists_nothing
    #[tokio::test]
    async fn key_salt_config_value_wins_and_persists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let mut cfg = cfg_at(dir.path());
        cfg.key_salt = Some("shared-salt".into());

        let salt = resolve_key_salt(&cfg, &backend).await.unwrap();
        assert_eq!(salt, b"shared-salt");
        assert!(
            !key_salt_path(&cfg).exists(),
            "a configured salt must not be shadowed by a persisted file"
        );
    }
```

**What it does** — With `cfg.key_salt = Some("shared-salt")`, `resolve_key_salt`
returns those bytes and the persisted-salt file is **not** created (a configured
salt must not be shadowed by a persisted file).

### fn key_salt_fallback_is_persisted_and_stable

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn key_salt_fallback_is_persisted_and_stable`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn key_salt_fallback_is_persisted_and_stable
    #[tokio::test]
    async fn key_salt_fallback_is_persisted_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let cfg = cfg_at(dir.path());

        let first = resolve_key_salt(&cfg, &backend).await.unwrap();
        let device_id = backend.get_device_id().await.unwrap();
        assert_eq!(first, device_id.as_bytes());
        let on_disk = std::fs::read_to_string(key_salt_path(&cfg)).unwrap();
        assert_eq!(on_disk.trim().as_bytes(), first.as_slice());

        let second = resolve_key_salt(&cfg, &backend).await.unwrap();
        assert_eq!(second, first);
    }
```

**What it does** — First resolution falls back to the device id and writes the
salt file (on-disk trimmed contents equal the returned salt); a second resolution
reads the file and returns the same salt, so the key stays derivable.

### fn key_salt_file_survives_a_regenerated_device_id

**Identification** — `#[tokio::test]`; marker
`// md:mod tests > fn key_salt_file_survives_a_regenerated_device_id`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn key_salt_file_survives_a_regenerated_device_id
    #[tokio::test]
    async fn key_salt_file_survives_a_regenerated_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FsBackend::new(dir.path()).await.unwrap();
        let cfg = cfg_at(dir.path());
        let original = resolve_key_salt(&cfg, &backend).await.unwrap();

        std::fs::remove_file(dir.path().join(".keeplin").join("device_id")).unwrap();
        let reopened = FsBackend::new(dir.path()).await.unwrap();
        assert_ne!(
            reopened.get_device_id().await.unwrap().as_bytes(),
            original.as_slice(),
            "precondition: the regenerated device id differs"
        );
        let resolved = resolve_key_salt(&cfg, &reopened).await.unwrap();
        assert_eq!(resolved, original, "salt must come from the persisted file");
    }
```

**What it does** — The recovery scenario: after deleting
`.keeplin/device_id` and reopening the store (precondition: the regenerated id
differs), `resolve_key_salt` still returns the original salt — the persisted file
wins, keeping data decryptable.

### fn make_req

**Identification** — helper; marker `// md:mod tests > fn make_req`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn make_req
    fn make_req(auth_header: Option<&str>) -> tonic::Request<()> {
        let mut req = tonic::Request::new(());
        if let Some(v) = auth_header {
            req.metadata_mut()
                .insert("authorization", v.parse().unwrap());
        }
        req
    }
```

**What it does** — Builds a bare `tonic::Request<()>` and optionally attaches an
`authorization` metadata entry; the value must already be wire-format
(e.g. `"Basic <base64>"`).

### fn basic

**Identification** — helper; marker `// md:mod tests > fn basic`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn basic
    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
    }
```

**What it does** — Formats a well-formed `Authorization: Basic` value for a
user/password pair — colon-joined then Base64-encoded, matching RFC 7617.

### fn auth_not_configured_allows_all

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_not_configured_allows_all`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_not_configured_allows_all
    #[test]
    fn auth_not_configured_allows_all() {
        let req = make_req(None);
        assert!(validate_basic_auth(req, None, None).is_ok());
    }
```

**What it does** — `(None, None)` expected credentials accept a request with no
header.

### fn auth_valid_credentials_pass

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_valid_credentials_pass`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_valid_credentials_pass
    #[test]
    fn auth_valid_credentials_pass() {
        let req = make_req(Some(&basic("alice", "s3cr3t")));
        assert!(validate_basic_auth(req, Some("alice"), Some("s3cr3t")).is_ok());
    }
```

**What it does** — Correct user + password → `Ok`.

### fn auth_wrong_password_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_wrong_password_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_wrong_password_rejected
    #[test]
    fn auth_wrong_password_rejected() {
        let req = make_req(Some(&basic("alice", "wrong")));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
```

**What it does** — Wrong password → `Unauthenticated`.

### fn auth_wrong_user_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_wrong_user_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_wrong_user_rejected
    #[test]
    fn auth_wrong_user_rejected() {
        let req = make_req(Some(&basic("mallory", "s3cr3t")));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
```

**What it does** — Wrong username → `Unauthenticated`.

### fn auth_missing_header_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_missing_header_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_missing_header_rejected
    #[test]
    fn auth_missing_header_rejected() {
        let req = make_req(None);
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
```

**What it does** — Auth configured but no header → `Unauthenticated`.

### fn auth_bearer_scheme_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_bearer_scheme_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_bearer_scheme_rejected
    #[test]
    fn auth_bearer_scheme_rejected() {
        let req = make_req(Some("Bearer some-opaque-token"));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
```

**What it does** — `Bearer <token>` scheme → `Unauthenticated`.

### fn auth_malformed_base64_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_malformed_base64_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_malformed_base64_rejected
    #[test]
    fn auth_malformed_base64_rejected() {
        let req = make_req(Some("Basic !!!notbase64!!!"));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
```

**What it does** — `Basic !!!notbase64!!!` → `Unauthenticated`.

### fn auth_no_colon_in_credentials_rejected

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_no_colon_in_credentials_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_no_colon_in_credentials_rejected
    #[test]
    fn auth_no_colon_in_credentials_rejected() {
        let req = make_req(Some(&format!("Basic {}", STANDARD.encode("nocolon"))));
        let err = validate_basic_auth(req, Some("alice"), Some("s3cr3t")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }
```

**What it does** — Decoded credentials without a colon → `Unauthenticated`.

### fn auth_password_containing_colon_works

**Identification** — `#[test]`; marker
`// md:mod tests > fn auth_password_containing_colon_works`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn auth_password_containing_colon_works
    #[test]
    fn auth_password_containing_colon_works() {
        let pass = "p:a:s:s:word";
        let req = make_req(Some(&basic("alice", pass)));
        assert!(validate_basic_auth(req, Some("alice"), Some(pass)).is_ok());
    }
```

**What it does** — RFC 7617 requires splitting on the **first** colon only, so a
password like `p:a:s:s:word` must be accepted.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `build_storage()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `collab_config()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `run_server_with()` — defined here (EXTRACTED; 2 cross-file edge(s))
- `load_config()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `serve()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `resolve_key_salt()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `key_salt_path()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `collab_starter()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `run_server()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `cfg_at()` — defined here (EXTRACTED; 1 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `keeplin-core/src/collab/mod.rs` — client of the keeplin-srv collaborative channel (EXTRACTED: imports_from×1, references×3; e.g. `CollabConfig`, `CollabHandle`, `CollabBackend`)
- `keeplin-core/src/storage/backend.rs` — the `StorageBackend` supertrait (EXTRACTED: references×1; e.g. `StorageBackend`)
- `keeplin-daemon/src/config.rs` — daemon configuration (EXTRACTED: references×9; e.g. `Config`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Decorator order is fixed: `MetricsBackend(EventBackend(LinkingBackend([EncryptedBackend](Fs|Db))))` — linking needs plaintext, eventing needs final metadata, metrics counts logical operations.
- One daemon per store: the exclusive `daemon.lock` is taken before any I/O; a second daemon must fail fast.
- Startup must fail loudly on an incompatible sync server (the `DbBackend::new` handshake) and on insecure config without the explicit override.
- The encryption salt resolution order (config `key_salt` > persisted `.keeplin/key_salt` > derived-from-device-id, then persisted) must not change — it is a data-recovery contract.

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `Args` | `// md:Args` |
| 3 | `Command` | `// md:Command` |
| 4 | `fn main` | `// md:fn main` |
| 5 | `fn load_config` | `// md:fn load_config` |
| 6 | `fn acquire_store_lock` | `// md:fn acquire_store_lock` |
| 7 | `fn serve` | `// md:fn serve` |
| 8 | `fn resolve_key_salt` | `// md:fn resolve_key_salt` |
| 9 | `fn key_salt_path` | `// md:fn key_salt_path` |
| 10 | `fn build_storage` | `// md:fn build_storage` |
| 11 | `fn run_migrate` | `// md:fn run_migrate` |
| 12 | `StackHook` | `// md:StackHook` |
| 13 | `fn collab_config` | `// md:fn collab_config` |
| 14 | `fn collab_starter` | `// md:fn collab_starter` |
| 15 | `fn run_server` | `// md:fn run_server` |
| 16 | `fn run_server_with` | `// md:fn run_server_with` |
| 17 | `fn shutdown_signal` | `// md:fn shutdown_signal` |
| 18 | `fn validate_basic_auth` | `// md:fn validate_basic_auth` |
| 19 | `mod tests` (container) | `// md:mod tests` |
| 20 | `fn store_lock_is_exclusive_per_store_and_released_on_drop` | `// md:mod tests > fn store_lock_is_exclusive_per_store_and_released_on_drop` |
| 21 | `fn cfg_at` | `// md:mod tests > fn cfg_at` |
| 22 | `fn key_salt_config_value_wins_and_persists_nothing` | `// md:mod tests > fn key_salt_config_value_wins_and_persists_nothing` |
| 23 | `fn key_salt_fallback_is_persisted_and_stable` | `// md:mod tests > fn key_salt_fallback_is_persisted_and_stable` |
| 24 | `fn key_salt_file_survives_a_regenerated_device_id` | `// md:mod tests > fn key_salt_file_survives_a_regenerated_device_id` |
| 25 | `fn make_req` | `// md:mod tests > fn make_req` |
| 26 | `fn basic` | `// md:mod tests > fn basic` |
| 27 | `fn auth_not_configured_allows_all` | `// md:mod tests > fn auth_not_configured_allows_all` |
| 28 | `fn auth_valid_credentials_pass` | `// md:mod tests > fn auth_valid_credentials_pass` |
| 29 | `fn auth_wrong_password_rejected` | `// md:mod tests > fn auth_wrong_password_rejected` |
| 30 | `fn auth_wrong_user_rejected` | `// md:mod tests > fn auth_wrong_user_rejected` |
| 31 | `fn auth_missing_header_rejected` | `// md:mod tests > fn auth_missing_header_rejected` |
| 32 | `fn auth_bearer_scheme_rejected` | `// md:mod tests > fn auth_bearer_scheme_rejected` |
| 33 | `fn auth_malformed_base64_rejected` | `// md:mod tests > fn auth_malformed_base64_rejected` |
| 34 | `fn auth_no_colon_in_credentials_rejected` | `// md:mod tests > fn auth_no_colon_in_credentials_rejected` |
| 35 | `fn auth_password_containing_colon_works` | `// md:mod tests > fn auth_password_containing_colon_works` |