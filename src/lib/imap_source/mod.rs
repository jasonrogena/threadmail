// rustls, not native-tls, so the release binary can statically link musl.

use std::collections::HashSet;
use std::net::TcpStream;
use std::sync::Arc;

use imap::Session;
use imap::error::ParseError;
use imap::types::Seq;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::config::ImapConfig;
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
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    #[error("could not read the configured CA certificate file")]
    CaFile(#[from] rustls::pki_types::pem::Error),
}

pub struct ImapSource {
    host: String,
    port: u16,
    username: String,
    password: String,
    mailbox: String,
    ca_file: Option<String>,
}

impl ImapSource {
    pub fn new(config: &ImapConfig) -> Result<Self, Error> {
        Ok(Self {
            host: config.host.clone(),
            port: config.port,
            username: config.username()?,
            password: config.password()?,
            mailbox: config.mailbox.clone(),
            ca_file: config.ca_file.clone(),
        })
    }

    fn root_store(&self) -> Result<RootCertStore, Error> {
        match &self.ca_file {
            Some(path) => {
                let mut store = RootCertStore::empty();
                let certs: Vec<CertificateDer> =
                    CertificateDer::pem_file_iter(path)?.collect::<Result<_, _>>()?;
                store.add_parsable_certificates(certs);
                Ok(store)
            }
            None => Ok(RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
            )),
        }
    }

    fn connect(&self) -> Result<Session<TlsStream>, Error> {
        // Ignoring the error: it just means a provider is already installed.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let root_store = self.root_store()?;
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
        session.select(&self.mailbox)?;

        // HEADER search is a substring match, so this also catches "Re: <slug>".
        let query = format!("HEADER Subject \"{}\"", escape_search_term(subject));
        let ids = search_with_retry(&mut session, &query)?;

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

// A concurrent IMAP client's own activity (e.g. marking a message \Seen)
// can make the server push an unsolicited FETCH into our SEARCH response;
// the imap crate treats that as a fatal parse error rather than skipping it
// as the unrelated unilateral response it is, so retry once. The full
// response was already drained off the wire before parsing failed, so the
// connection itself is left in a consistent state for the retry.
fn search_with_retry(session: &mut Session<TlsStream>, query: &str) -> imap::Result<HashSet<Seq>> {
    match session.search(query) {
        Err(imap::Error::Parse(ParseError::Unexpected(_))) => session.search(query),
        result => result,
    }
}

fn escape_search_term(term: &str) -> String {
    term.replace('\\', "\\\\").replace('"', "\\\"")
}
