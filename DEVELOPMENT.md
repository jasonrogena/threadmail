# Development

```sh
make build  # plain local build; CI produces the static musl release binary
make test   # clippy -D warnings, fmt --check, cargo test
```

TLS is rustls, not native-tls/OpenSSL, since OpenSSL is painful to statically link under musl.

## Code layout

The core (`mail`, `thread`, `render`, `compose`) is pure, I/O-free functions and data, unit-tested with literal fixtures — `render` is the one exception that depends on `config` (its `Options` holds config sub-structs by reference rather than loose passthrough fields; see the convention below), the rest have no dependency on it. The I/O boundaries are four small traits: `MailSource`/`MailSink` (one real adapter each, `imap_source`/`smtp_sink`) and `IncomingCommentStore`/`OutgoingCommentStore` (both implemented by `sqlite_store`, one SQLite file). `web/` is a thin axum layer with no business logic of its own.

## Conventions

- **Traits mark real swap points, nothing else.** `MailSource`/`MailSink` (protocol could become JMAP) and `IncomingCommentStore`/`OutgoingCommentStore` (storage engine could change) are traits because a second implementation is plausible. Plain data (`Message`, `Thread`, `Node`, `Author`, `Subject`) is never behind a trait.
- **Paired names must stay literally true, not just parallel.** A pair sharing a prefix/suffix (`MailSource`/`MailSink`, `IncomingCommentStore`/`OutgoingCommentStore`) should read as a family, but check the word is still accurate before matching it for symmetry — e.g. "Inbox" was rejected for the read-side store because it implies durability the store doesn't have.
- **Methods over free functions** whenever a natural receiver or constructor exists (`Thread::resolve`, `Author::initial`). A free function is still correct with no natural owner (`render::empty`) or when it's plumbing over unrelated parameters.
- **Pass the struct, not its fields.** Once a concept has a struct (`Subject`, `Author`), functions take `&Subject`/`&Author`, not the individual strings that compose it. This extends to `Config` and its sub-structs: prefer holding/passing the relevant config struct itself over unpacking it into loose fields, even across a module boundary — e.g. `AppState` holds `Arc<Config>` rather than one field per config value, and `render::Options` holds config sub-structs by reference rather than each of their fields as separate params.
- **Comments are one line and only for a non-obvious "why."** No multi-line blocks, nothing that restates what the code already says.
- **Tests live in a sibling `tests.rs`, never inline.** Every module with tests declares `#[cfg(test)] mod tests;` in `mod.rs`; the tests themselves go in `tests.rs` next to it.
- **Errors are `thiserror`, local to each module.** Every module defines its own `pub enum Error` with `#[error("...")]` variants and `#[from]` for wrapped causes — no crate-wide error type. A trait method crossing an I/O boundary (`MailSource`, `MailSink`, `IncomingCommentStore`, `OutgoingCommentStore`) returns `BoxError` (`Box<dyn std::error::Error + Send + Sync>`) instead, since a trait can't have an object-safe associated `Error` type.
- **Keep nomenclature as tight as possible.** One concept, one name, used consistently everywhere it appears — config fields, struct fields, function names, log/error text, test names. When a concept's name changes, rename every occurrence rather than leaving old wording alongside the new one; a stale synonym for the same thing is worse than no comment at all.
