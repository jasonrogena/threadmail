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

A bare `<iframe>` is enough. The CSS below is optional: it shows a loading placeholder until the iframe's document starts painting, with no JS involved (an iframe has no background of its own until then, so it just shows through).

```html
<style>
  .comments-frame { display: grid; min-height: 600px;
    background: linear-gradient(90deg, #eee 25%, #ddd 37%, #eee 63%);
    background-size: 400% 100%; animation: comments-pulse 1.4s ease infinite; }
  .comments-frame > * { grid-area: 1 / 1; }
  @keyframes comments-pulse { from { background-position: 100% 50%; } to { background-position: 0 50%; } }
</style>
<div class="comments-frame">
  <span aria-hidden="true">Loading comments&hellip;</span>
  <iframe src="https://comments.example.com/thread/my-post-slug" title="Comments"
    height="600" loading="lazy" style="width: 100%; border: none;"></iframe>
</div>
```

## Configuration

Copy `config.example.toml` and fill it in, then:

```sh
threadmail --config-path ./config.toml serve
```

## Building and testing

```sh
make build  # static musl binary
make test   # clippy -D warnings, fmt --check, cargo test
```

TLS is rustls, not native-tls/OpenSSL, since OpenSSL is painful to statically link under musl.

## Code layout

The core (`mail`, `thread`, `render`, `compose`) is pure, I/O-free functions and data, unit-tested with literal byte fixtures. The only I/O boundaries are two small traits, `MailSource` and `MailSink`, each with one real adapter (`imap_source`, `smtp_sink`). `web/` is a thin axum layer with no business logic of its own.
