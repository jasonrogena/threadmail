use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};

use crate::config::SmtpConfig;
use crate::source::{BoxError, MailSink};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not build the SMTP transport")]
    Build(#[from] lettre::transport::smtp::Error),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
}

pub struct SmtpSink {
    transport: SmtpTransport,
}

impl SmtpSink {
    pub fn new(config: &SmtpConfig) -> Result<Self, Error> {
        let transport = SmtpTransport::starttls_relay(&config.host)?
            .port(config.port)
            .credentials(Credentials::new(config.username()?, config.password()?))
            .build();
        Ok(Self { transport })
    }
}

impl MailSink for SmtpSink {
    fn submit(&self, envelope: &lettre::address::Envelope, raw: &[u8]) -> Result<(), BoxError> {
        self.transport.send_raw(envelope, raw)?;
        Ok(())
    }
}
