use lettre::Message;
use lettre::message::Mailbox;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the bot or list address is not a valid email address")]
    InvalidAddress(#[from] lettre::address::AddressError),
    #[error("the message could not be assembled")]
    Build(#[from] lettre::error::Error),
}

pub struct NewComment<'a> {
    pub name: &'a str,
    pub body: &'a str,
    pub in_reply_to: Option<&'a str>,
}

impl NewComment<'_> {
    pub fn compose(
        &self,
        slug: &str,
        bot_address: &str,
        list_address: &str,
    ) -> Result<Message, Error> {
        let from = Mailbox::new(
            Some(format!("{} (via web)", self.name)),
            bot_address.parse()?,
        );
        let to: Mailbox = list_address.parse::<lettre::Address>()?.into();

        let subject = match self.in_reply_to {
            Some(_) => format!("Re: {slug}"),
            None => slug.to_string(),
        };

        let mut builder = Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .message_id(None);

        if let Some(parent) = self.in_reply_to {
            builder = builder.in_reply_to(parent.to_string());
            builder = builder.references(parent.to_string());
        }

        Ok(builder.body(self.body.to_string())?)
    }
}
