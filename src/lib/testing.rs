// In-memory MailSource/MailSink fixtures that share one backing "mailbox",
// so a submitted comment is actually findable on the next search - unlike
// two independent fakes that don't talk to each other. Used by unit tests
// directly, and by main.rs (under the `test-fixtures` feature) so the real
// binary can be driven end-to-end without a real IMAP/SMTP provider.

use std::sync::{Arc, Mutex, OnceLock};

use lettre::address::Envelope;

use crate::source::{BoxError, MailSink, MailSource, OnDelivered};

#[derive(Clone, Default)]
pub struct SharedMailbox {
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl SharedMailbox {
    // Pre-populates the mailbox, as if these messages were already there before the test started.
    pub fn seed(&self, raw_messages: &[Vec<u8>]) {
        self.messages
            .lock()
            .unwrap()
            .extend(raw_messages.iter().cloned());
    }

    pub fn source(&self) -> SharedMailboxSource {
        SharedMailboxSource {
            mailbox: self.messages.clone(),
        }
    }

    pub fn sink(&self) -> RelayingSink {
        RelayingSink {
            mailbox: self.messages.clone(),
            on_delivered: OnceLock::new(),
        }
    }
}

pub struct SharedMailboxSource {
    mailbox: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MailSource for SharedMailboxSource {
    fn search_subject(&self, _subject: &str) -> Result<Vec<Vec<u8>>, BoxError> {
        Ok(self.mailbox.lock().unwrap().clone())
    }
}

#[derive(Default)]
pub struct RelayingSink {
    mailbox: Arc<Mutex<Vec<Vec<u8>>>>,
    on_delivered: OnceLock<OnDelivered>,
}

impl MailSink for RelayingSink {
    fn boot(&self, on_delivered: OnDelivered) {
        let _ = self.on_delivered.set(on_delivered);
    }

    fn submit(&self, slug: &str, _envelope: &Envelope, raw: &[u8]) -> Result<(), BoxError> {
        self.mailbox.lock().unwrap().push(raw.to_vec());
        if let Some(on_delivered) = self.on_delivered.get() {
            on_delivered(slug);
        }
        Ok(())
    }

    fn health(&self) -> Result<(), BoxError> {
        Ok(())
    }
}
