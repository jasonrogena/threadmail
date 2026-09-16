//! The real `MailSink` adapter: submits a composed comment to the mailing
//! list over SMTP with STARTTLS, using the bot account's own credentials.

use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};

use crate::source::{BoxError, MailSink};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not build the SMTP transport")]
    Build(#[from] lettre::transport::smtp::Error),
}

pub struct SmtpSink {
    transport: SmtpTransport,
}

impl SmtpSink {
    pub fn new(
        host: &str,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, Error> {
        let transport = SmtpTransport::starttls_relay(host)?
            .port(port)
            .credentials(Credentials::new(username.into(), password.into()))
            .build();
        Ok(Self { transport })
    }
}

impl MailSink for SmtpSink {
    fn submit(&self, message: &lettre::Message) -> Result<(), BoxError> {
        self.transport.send(message)?;
        Ok(())
    }
}
