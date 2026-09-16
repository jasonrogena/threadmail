//! The real `MailSource` adapter: an IMAP client talking to the bot
//! account's own mailbox (it is a normal subscribed member of the mailing
//! list, so every list message lands here as a copy). No background
//! poller, no IDLE loop, no persisted state — a connection is opened fresh
//! for each search, which is what keeps this adapter this small.
//!
//! TLS is rustls (with `webpki-roots` for the certificate store), not
//! native-tls/OpenSSL — this binary is built statically against musl, and
//! rustls's pure-Rust crypto (via `ring`) avoids the usual pain of
//! statically linking OpenSSL under musl.

use std::net::TcpStream;
use std::sync::Arc;

use imap::Session;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::source::{BoxError, MailSource};

#[cfg(test)]
mod tests;

type TlsStream = StreamOwned<ClientConnection, TcpStream>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the configured IMAP host is not a valid TLS server name")]
    InvalidServerName,
    #[error("could not establish TLS to the IMAP server")]
    Tls(#[from] rustls::Error),
    #[error("could not open a TCP connection to the IMAP server")]
    Io(#[from] std::io::Error),
    #[error("could not authenticate to the IMAP server")]
    Connect(#[source] imap::Error),
}

pub struct ImapSource {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl ImapSource {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            password: password.into(),
        }
    }

    fn connect(&self) -> Result<Session<TlsStream>, Error> {
        // Idempotent: a second install (e.g. one already made by lettre's
        // own rustls transport) just returns the existing provider, which
        // is fine to ignore here.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let root_store = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );

        let server_name =
            ServerName::try_from(self.host.clone()).map_err(|_| Error::InvalidServerName)?;
        let connection = ClientConnection::new(config, server_name)?;
        let tcp = TcpStream::connect((self.host.as_str(), self.port))?;
        let tls = StreamOwned::new(connection, tcp);

        let client = imap::Client::new(tls);
        client
            .login(&self.username, &self.password)
            .map_err(|(e, _)| Error::Connect(e))
    }
}

impl MailSource for ImapSource {
    fn search_subject(&self, subject: &str) -> Result<Vec<Vec<u8>>, BoxError> {
        let mut session = self.connect()?;
        session.select("INBOX")?;

        // IMAP's HEADER search is a case-insensitive substring match, which
        // is exactly what we want: it matches both the exact slug on a
        // thread's root and the "Re: <slug>" subject a reply's mail client
        // produces, with no extra logic needed on our side.
        let query = format!("HEADER Subject \"{}\"", escape_search_term(subject));
        let ids = session.search(&query)?;

        if ids.is_empty() {
            let _ = session.logout();
            return Ok(Vec::new());
        }

        let id_list = ids
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let fetches = session.fetch(&id_list, "RFC822")?;

        let messages = fetches
            .iter()
            .filter_map(|fetch| fetch.body().map(|b| b.to_vec()))
            .collect();

        let _ = session.logout();
        Ok(messages)
    }
}

fn escape_search_term(term: &str) -> String {
    term.replace('\\', "\\\\").replace('"', "\\\"")
}
