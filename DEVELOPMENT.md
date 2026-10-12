# Development

```sh
make build  # plain local build; CI produces the static musl release binary
make test   # clippy -D warnings, fmt --check, cargo test
```

TLS is rustls, not native-tls/OpenSSL, since OpenSSL is painful to statically link under musl.

## Running the e2e tests locally

`make test`/`cargo test` run `tests/e2e.rs` unconditionally, so a real [GreenMail](https://greenmail-mail-test.github.io) mail server must already be listening on 127.0.0.1:3465 (SMTPS) and :3993 (IMAPS) before you run them — these aren't skipped if it's missing, they just fail to connect. CI starts one automatically (see `.github/workflows/test.yml`); locally, start the same thing yourself with Docker or Podman:

```sh
docker run -d --name greenmail \
  -p 3465:3465 -p 3993:3993 \
  -v "$(pwd)/tests/fixtures/greenmail.p12:/keystore.p12" \
  -e GREENMAIL_OPTS='-Dgreenmail.setup.test.smtps -Dgreenmail.setup.test.imaps -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled -Dgreenmail.tls.keystore.file=/keystore.p12 -Dgreenmail.tls.keystore.password=threadmail-e2e' \
  docker.io/greenmail/standalone:2.1.8
```

The keystore (`tests/fixtures/greenmail.p12` + its CA, `greenmail-ca.pem`) is a committed, throwaway test-only cert: GreenMail's own default cert has no Subject Alternative Name, which rustls rejects outright, so this stands in for it. It's not a secret and isn't used for anything but letting GreenMail serve TLS here.

The container keeps whatever mail it received across test runs, so a second run against the same container sees leftover messages from the first (each e2e test asserts an exact count). Restart it (`docker rm -f greenmail` and rerun the command above) between local runs of `cargo test --test e2e` for a clean slate — a fresh GitHub Actions job never has this problem, since its service container is always new.

## Dependency updates

[Dependabot](.github/dependabot.yml) opens a PR weekly for outdated Cargo and GitHub Actions dependencies. [`.github/workflows/dependabot-auto-merge.yml`](.github/workflows/dependabot-auto-merge.yml) approves and auto-merges every one of those PRs, including major-version bumps, once the `Test` workflow passes — nothing is held back for manual review. If a bump breaks something the test suite doesn't catch, that's a gap in the tests to fix, not a reason to gate the merge by hand.

This needs two repository settings that can't be set from a workflow file, both under **Settings → General → Pull Requests** (and **Settings → Branches** for the second):

- **Allow auto-merge** must be enabled, or `gh pr merge --auto` has nothing to queue onto.
- **main**'s branch protection must **require the `Test` status check to pass before merging**. Without this, `--auto` merges immediately instead of waiting for CI, since there's nothing configured to wait for.

## Code layout

The core (`mail`, `thread`, `render`, `compose`) is pure, I/O-free functions and data, unit-tested with literal fixtures — `render` is the one exception that depends on `config` (its `Options` holds config sub-structs by reference rather than loose passthrough fields; see the convention below), the rest have no dependency on it. The I/O boundaries are four small traits: `MailSource`/`MailSink` (one real adapter each, `imap_source`/`smtp_sink`) and `IncomingCommentStore`/`OutgoingCommentStore` (both implemented by `sqlite_store`, one SQLite file). `MailSink::submit` means "accepted for delivery": `smtp_sink` queues to `OutgoingCommentStore` and delivers from the outgoing comment worker, which runs only while something is queued (`boot` does the first sweep after a restart), reporting each delivery back through the `on_delivered` hook. The crate root (`src/lib/mod.rs`) owns `AppState`, the wiring of those adapters plus the background IMAP refresh, and `boot()`s the sink at startup.

## Conventions

- **Traits mark real swap points, nothing else.** `MailSource`/`MailSink` (protocol could become JMAP) and `IncomingCommentStore`/`OutgoingCommentStore` (storage engine could change) are traits because a second implementation is plausible. Plain data (`Message`, `Thread`, `Node`, `Author`, `Subject`) is never behind a trait.
- **Paired names must stay literally true, not just parallel.** A pair sharing a prefix/suffix (`MailSource`/`MailSink`, `IncomingCommentStore`/`OutgoingCommentStore`) should read as a family, but check the word is still accurate before matching it for symmetry — e.g. "Inbox" was rejected for the read-side store because it implies durability the store doesn't have.
- **Route handlers are named `handle_<verb>_<route>`** (`handle_get_thread`, `handle_post_thread`), so a handler is recognisable at a glance and maps directly to its line in `WebState::router`.
- **Methods over free functions** whenever a natural receiver or constructor exists (`Thread::resolve`, `Author::initial`). A free function is still correct with no natural owner (`render::empty`) or when it's plumbing over unrelated parameters.
- **Pass the struct, not its fields.** Once a concept has a struct (`Subject`, `Author`), functions take `&Subject`/`&Author`, not the individual strings that compose it. This extends to `Config` and its sub-structs: prefer holding/passing the relevant config struct itself over unpacking it into loose fields, even across a module boundary — e.g. `AppState` holds the whole `Config` rather than one field per config value, and `render::Options` holds config sub-structs by reference rather than each of their fields as separate params.
- **Comments are one line and only for a non-obvious "why."** No multi-line blocks, nothing that restates what the code already says.
- **Tests live in a sibling `tests.rs`, never inline.** Every module with tests declares `#[cfg(test)] mod tests;` in `mod.rs`; the tests themselves go in `tests.rs` next to it.
- **Errors are `thiserror`, local to each module.** Every module that can fail defines its own `pub enum Error` with `#[error("...")]` variants and `#[from]` for wrapped causes — no crate-wide error type. Four have no `Error` because nothing in them returns one: `render` (askama checks templates at compile time, so rendering `.expect`s), `source` (only the I/O traits, whose methods return `BoxError`), `web` (every failure becomes an HTTP status), and the crate root's `AppState` (a failed IMAP refresh is logged and recorded with `mark_refresh_attempted`, never returned). A trait method crossing an I/O boundary (`MailSource`, `MailSink`, `IncomingCommentStore`, `OutgoingCommentStore`) returns `BoxError` (`Box<dyn std::error::Error + Send + Sync>`) instead, since a trait can't have an object-safe associated `Error` type.
- **Keep nomenclature as tight as possible.** One concept, one name, used consistently everywhere it appears — config fields, struct fields, function names, log/error text, test names. When a concept's name changes, rename every occurrence rather than leaving old wording alongside the new one; a stale synonym for the same thing is worse than no comment at all.
