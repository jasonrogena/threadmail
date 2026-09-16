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

pub fn compose(
    slug: &str,
    bot_address: &str,
    list_address: &str,
    comment: &NewComment,
) -> Result<Message, Error> {
    let from = Mailbox::new(
        Some(format!("{} (via web)", comment.name)),
        bot_address.parse()?,
    );
    let to: Mailbox = list_address.parse::<lettre::Address>()?.into();

    let subject = match comment.in_reply_to {
        Some(_) => format!("Re: {slug}"),
        None => slug.to_string(),
    };

    let mut builder = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .message_id(None);

    if let Some(parent) = comment.in_reply_to {
        builder = builder.in_reply_to(parent.to_string());
        builder = builder.references(parent.to_string());
    }

    Ok(builder.body(comment.body.to_string())?)
}
