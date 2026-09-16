# threadmail

Static-site comments backed by a mailing list, not a database.

Every comment is a real RFC 5322 email, threaded via `In-Reply-To`/`References` like any mail client. No database, no JavaScript, no persistent state: the mailing list is the only system of record. A single Rust binary resolves and renders a thread on demand, and optionally relays no-JS web-form submissions into the list as email.

## Tenets

- **Privacy.** No real email address is ever rendered, in any form. The web form collects only a name and a comment body.
- **Standards-based, for posterity.** Comments live in the mailing list, not in threadmail. Nothing is persisted locally; a thread is resolved fresh from the mailbox on every request.
- **Simplicity.** One binary, no JavaScript. Embedding a thread is one `<iframe>` tag. Moderation is fully delegated to the mailing list provider.

## How it works

A bot account subscribed to the list receives a copy of every message; threadmail reads that account's own IMAP mailbox (no Google Groups API, works with any mailing list technology). A post's slug is the email `Subject`; the thread root is the earliest top-level message with that subject, and replies attach via `In-Reply-To`/`References`, falling back to "subject still carries the slug" for messages a client failed to thread correctly.

`GET /thread/<slug>` resolves and renders a thread on the spot. Nothing is cached to disk and nothing is seeded ahead of time: a thread exists exactly when a real comment exists.

Comments arrive two ways, each independently toggleable in config: a no-JS `<form>` (relayed by the bot account over SMTP) or a plain `mailto:` link for commenting directly by email. Outbound IMAP searches and SMTP submissions are capped by a configurable semaphore, so a traffic spike can't hammer the mail provider.

## Embedding on a static page

```html
<iframe src="https://comments.example.com/thread/my-post-slug"></iframe>
```

That's the entire integration surface.

## Configuration

Copy `config.example.toml`, fill in the bot account's IMAP/SMTP credentials and the mailing list's posting address, then:

```sh
threadmail --config-path ./config.toml serve
```

Config is grouped by concern: `[server]`, `[list]` (addresses and comment-behavior toggles), `[imap]`/`[smtp]` (credentials), and `[limits]` (concurrency caps, optional).

A multi-arch Docker image (`FROM scratch`, no runtime dependencies) is published to `ghcr.io/jasonrogena/threadmail` on every tagged release. Mount your config at `/etc/threadmail/config.toml` and set `bind_address` to `0.0.0.0:<port>` so the server is reachable from outside the container.

## Building and testing

```sh
make build  # static musl binary
make test   # clippy -D warnings, fmt --check, cargo test
```

TLS is rustls, not native-tls/OpenSSL, since OpenSSL is painful to statically link under musl.

## Code layout

The core (`mail`, `thread`, `render`, `compose`) is pure, I/O-free functions and data, unit-tested with literal byte fixtures. The only I/O boundaries are two small traits, `MailSource` and `MailSink`, each with one real adapter (`imap_source`, `smtp_sink`). `web/` is a thin axum layer with no business logic of its own.
