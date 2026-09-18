// rustls, not native-tls, so the release binary can statically link musl.

use std::net::TcpStream;
use std::sync::Arc;

use imap::Session;
use rustls::pki_types::ServerName;
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
}

pub struct ImapSource {
    host: String,
    port: u16,
    username: String,
    password: String,
}

impl ImapSource {
    pub fn new(config: &ImapConfig) -> Result<Self, Error> {
        Ok(Self {
            host: config.host.clone(),
            port: config.port,
            username: config.username()?,
            password: config.password()?,
        })
    }

    fn connect(&self) -> Result<Session<TlsStream>, Error> {
        // Ignoring the error: it just means a provider is already installed.
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

        // HEADER search is a substring match, so this also catches "Re: <slug>".
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
