// True end-to-end tests: these spawn the actual compiled `threadmail` binary
// as a real OS process, bound to a real TCP port, and drive it over real
// HTTP - no in-process router, no mocked handlers. The binary runs its
// unmodified production code path: the real `ImapSource`/`SmtpSink`, talking
// real IMAP and SMTP to a real GreenMail mail server
// (https://greenmail-mail-test.github.io). Nothing about GreenMail's
// lifecycle lives here: it's brought up externally, on fixed ports 3465
// (SMTPS) and 3993 (IMAPS), using the keystore in tests/fixtures (see
// .github/workflows/test.yml for CI, DEVELOPMENT.md for running this
// locally). The per-test config fixtures (tests/configs/e2e-*.toml) hardcode
// those same ports, so nothing here needs to generate or template config -
// if GreenMail isn't listening on them, these tests just fail to connect,
// with a message pointing at DEVELOPMENT.md.
//
// GreenMail doesn't implement SMTP STARTTLS (confirmed by hand: its EHLO
// response never advertises it, and the command is rejected outright), only
// plaintext or implicit TLS ("smtps", port 3465). That's why `smtp.tls_mode`
// exists in production config (src/lib/config/mod.rs) - most real providers
// want Starttls, but this is exactly the Implicit case, and it's a real
// provider shape, not a test-only knob.
//
// With `-Dgreenmail.auth.disabled`, GreenMail accepts any IMAP/SMTP
// credentials and routes a delivered message straight into the mailbox
// matching its exact SMTP envelope recipient, created on first use - there's
// no real mailing-list fan-out, and no isolation between tests sharing one
// GreenMail instance beyond using different addresses. So each test's
// "mailing list" is its own address, acting as both the SMTP recipient and
// the IMAP login, standing in for "the bot account is itself a list member."

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};

const GREENMAIL_SMTPS_PORT: u16 = 3465;
const GREENMAIL_IMAPS_PORT: u16 = 3993;
const GREENMAIL_CA_FILE: &str = "tests/fixtures/greenmail-ca.pem";

struct TestInstance {
    config_path: &'static str,
    addr: &'static str,
    mailbox_address: &'static str,
}

const TOP_LEVEL: TestInstance = TestInstance {
    config_path: "tests/configs/e2e-top-level.toml",
    addr: "127.0.0.1:18091",
    mailbox_address: "e2e-top-level@example.com",
};

const REPLY: TestInstance = TestInstance {
    config_path: "tests/configs/e2e-reply.toml",
    addr: "127.0.0.1:18092",
    mailbox_address: "e2e-reply@example.com",
};

const RETRIEVAL: TestInstance = TestInstance {
    config_path: "tests/configs/e2e-retrieval.toml",
    addr: "127.0.0.1:18093",
    mailbox_address: "e2e-retrieval@example.com",
};

struct Server {
    child: Child,
    instance: &'static TestInstance,
}

impl Server {
    fn start(instance: &'static TestInstance) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_threadmail"))
            .args(["--config-path", instance.config_path, "serve"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start the threadmail binary");

        let server = Self { child, instance };
        server.wait_until_healthy();
        server
    }

    fn wait_until_healthy(&self) {
        for _ in 0..100 {
            if let Some((200, _)) = try_request(self.instance.addr, "GET", "/healthz", "") {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "server never became healthy - is GreenMail listening on 127.0.0.1:{GREENMAIL_IMAPS_PORT}? see DEVELOPMENT.md"
        );
    }

    fn get(&self, path: &str) -> (u16, String) {
        request(self.instance.addr, "GET", path, "")
    }

    fn post_form(&self, path: &str, body: &str) -> (u16, String) {
        request(self.instance.addr, "POST", path, body)
    }

    // Fetches straight from GreenMail over a fresh IMAP connection of the
    // test's own, independent of the app's own IMAP search/cache: this is
    // what actually arrived over SMTP, not what the app later reports.
    fn fetch_mailbox(&self) -> Vec<String> {
        let mut session = self.imap_connect();
        // No SEARCH: the production app's own IMAP connection can be
        // fetching concurrently, and GreenMail may then send this
        // connection an unsolicited FETCH notification that the `imap`
        // crate's SEARCH response parser doesn't expect. A plain FETCH
        // tolerates unsolicited responses interleaved with its own.
        let mailbox = session.select("INBOX").expect("SELECT INBOX failed");
        let raw = if mailbox.exists == 0 {
            Vec::new()
        } else {
            session
                .fetch("1:*", "RFC822")
                .expect("FETCH RFC822 failed")
                .iter()
                .filter_map(|f| f.body().map(|b| String::from_utf8_lossy(b).into_owned()))
                .collect()
        };
        let _ = session.logout();
        raw
    }

    fn imap_connect(
        &self,
    ) -> imap::Session<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
        let config = trusting_tls_config();
        let server_name = ServerName::try_from("127.0.0.1").unwrap();
        let conn = rustls::ClientConnection::new(config, server_name).unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", GREENMAIL_IMAPS_PORT)).unwrap_or_else(|err| {
            panic!(
                "could not reach GreenMail on 127.0.0.1:{GREENMAIL_IMAPS_PORT} ({err}) - see DEVELOPMENT.md"
            )
        });
        let stream = rustls::StreamOwned::new(conn, tcp);
        imap::Client::new(stream)
            .login(self.instance.mailbox_address, "unused")
            .map_err(|(err, _)| err)
            .expect("IMAP login against GreenMail failed")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn trusting_tls_config() -> Arc<rustls::ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut store = rustls::RootCertStore::empty();
    let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(GREENMAIL_CA_FILE)
        .expect("ca file is readable")
        .collect::<Result<_, _>>()
        .expect("ca file is valid PEM");
    store.add_parsable_certificates(certs);
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(store)
            .with_no_client_auth(),
    )
}

// Delivers a message straight to GreenMail over SMTP, standing in for a
// real email client replying directly (the mailto: path) - never touching
// the app's own web form or SmtpSink at all.
fn send_direct_email_reply(to: &str, subject: &str, body: &str) {
    use lettre::transport::smtp::client::{
        Certificate, CertificateStore, Tls, TlsParametersBuilder,
    };
    use lettre::{Message, SmtpTransport, Transport};

    let pem = std::fs::read(GREENMAIL_CA_FILE).expect("ca file is readable");
    let tls_parameters = TlsParametersBuilder::new("127.0.0.1".to_string())
        .certificate_store(CertificateStore::None)
        .add_root_certificate(Certificate::from_pem(&pem).expect("ca file is valid PEM"))
        .build()
        .expect("tls parameters are valid");
    let transport = SmtpTransport::builder_dangerous("127.0.0.1")
        .port(GREENMAIL_SMTPS_PORT)
        .tls(Tls::Wrapper(tls_parameters))
        .build();

    let email = Message::builder()
        .from("A Real Person <someone@example.com>".parse().unwrap())
        .to(to.parse().unwrap())
        .subject(subject)
        .message_id(None)
        .body(body.to_string())
        .expect("message builds");

    transport
        .send(&email)
        .expect("direct email reply failed to reach GreenMail");
}

// ---- Raw HTTP/1.1 client, driving threadmail's own HTTP surface ----

fn request(addr: &str, method: &str, path: &str, body: &str) -> (u16, String) {
    try_request(addr, method, path, body).expect("request failed")
}

fn try_request(addr: &str, method: &str, path: &str, body: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(addr).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;

    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if !body.is_empty() {
        request.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);

    stream.write_all(request.as_bytes()).ok()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let raw = String::from_utf8_lossy(&raw);

    let (head, rest) = raw.split_once("\r\n\r\n")?;
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((status, rest.to_string()))
}

fn url_encode(input: &str) -> String {
    input
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn form_body(name: &str, body: &str, in_reply_to: &str) -> String {
    format!(
        "name={}&body={}&in_reply_to={}",
        url_encode(name),
        url_encode(body),
        url_encode(in_reply_to)
    )
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    for _ in 0..100 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("condition never became true");
}

fn message_id_of_first_comment(html: &str) -> String {
    let after = html.split_once("id=\"c-").expect("no rendered comment").1;
    after
        .split_once('"')
        .expect("unterminated id attribute")
        .0
        .to_string()
}

#[test]
fn submitting_a_top_level_comment_makes_it_visible_in_the_rendered_thread() {
    let server = Server::start(&TOP_LEVEL);

    wait_for(|| {
        server
            .get("/thread/e2e-top-level")
            .1
            .contains("No comments yet")
    });

    let (status, _) = server.post_form(
        "/thread/e2e-top-level",
        &form_body("E2E Bob", "Hello from e2e", ""),
    );
    assert_eq!(status, 303);

    // GreenMail's own mailbox first: this is what actually arrived over
    // SMTP, independent of the app's later IMAP search/cache/rendering.
    wait_for(|| server.fetch_mailbox().len() == 1);
    let delivered = server.fetch_mailbox().remove(0);
    assert!(delivered.contains("Subject: e2e-top-level"));
    assert!(delivered.contains("Hello from e2e"));

    wait_for(|| {
        server
            .get("/thread/e2e-top-level")
            .1
            .contains("Hello from e2e")
    });

    let (status, html) = server.get("/thread/e2e-top-level");
    assert_eq!(status, 200);
    assert!(html.contains("E2E Bob (via web)"));
}

#[test]
fn submitting_a_reply_nests_it_under_its_parent_in_the_rendered_thread() {
    let server = Server::start(&REPLY);

    wait_for(|| {
        server
            .get("/thread/e2e-reply")
            .1
            .contains("No comments yet")
    });

    let (status, _) = server.post_form(
        "/thread/e2e-reply",
        &form_body("E2E Bob", "Hello from e2e", ""),
    );
    assert_eq!(status, 303);
    wait_for(|| server.fetch_mailbox().len() == 1);
    wait_for(|| server.get("/thread/e2e-reply").1.contains("Hello from e2e"));

    let root_message_id = message_id_of_first_comment(&server.get("/thread/e2e-reply").1);

    let (status, _) = server.post_form(
        "/thread/e2e-reply",
        &form_body("E2E Carol", "Totally agree", &root_message_id),
    );
    assert_eq!(status, 303);

    // GreenMail must have received a reply that actually references the
    // root message, not just a second, unrelated post.
    wait_for(|| server.fetch_mailbox().len() == 2);
    let reply = server
        .fetch_mailbox()
        .into_iter()
        .find(|m| m.contains("Totally agree"))
        .expect("reply never arrived at GreenMail");
    assert!(reply.contains("Subject: Re: e2e-reply"));
    assert!(reply.contains(&format!("In-Reply-To: {root_message_id}")));

    wait_for(|| server.get("/thread/e2e-reply").1.contains("Totally agree"));

    let (_, html) = server.get("/thread/e2e-reply");

    // The reply must render inside the root's own `<ol class="replies">`,
    // not merely appear somewhere later on the page.
    let root_idx = html.find("Hello from e2e").expect("root comment missing");
    let replies_idx = html
        .find("class=\"replies\"")
        .expect("no nested replies list");
    let reply_idx = html.find("Totally agree").expect("reply comment missing");

    assert!(root_idx < replies_idx);
    assert!(replies_idx < reply_idx);
    assert!(html.contains("E2E Carol (via web)"));
}

#[test]
fn a_comment_that_arrived_by_direct_email_reply_is_retrievable_without_ever_posting_to_the_web_form()
 {
    // Delivered straight to GreenMail, as if a real email client replied to
    // the list - the app's own web form and SmtpSink are never involved.
    send_direct_email_reply(
        RETRIEVAL.mailbox_address,
        "e2e-retrieval",
        "Reply sent directly by email",
    );

    let server = Server::start(&RETRIEVAL);

    wait_for(|| {
        server
            .get("/thread/e2e-retrieval")
            .1
            .contains("Reply sent directly by email")
    });

    let (status, html) = server.get("/thread/e2e-retrieval");
    assert_eq!(status, 200);
    assert!(html.contains("A Real Person"));
    assert!(html.contains("Reply sent directly by email"));
}
