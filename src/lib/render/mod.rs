use askama::Template;

use crate::thread::{Node, Thread};

#[cfg(test)]
mod tests;

pub struct Options<'a> {
    pub comment_action: &'a str,
    pub mailto_address: &'a str,
    pub allow_relay: bool,
    pub show_email_link: bool,
}

#[derive(Template)]
#[template(path = "thread.html")]
struct ThreadTemplate<'a> {
    slug: &'a str,
    root_html: String,
    mailto_address: &'a str,
    show_email_link: bool,
}

#[derive(Template)]
#[template(path = "empty.html")]
struct EmptyTemplate<'a> {
    slug: &'a str,
    form_html: String,
    mailto_address: &'a str,
    show_email_link: bool,
}

#[derive(Template)]
#[template(path = "comment.html")]
struct CommentTemplate<'a> {
    id: &'a str,
    initial: String,
    display_name: &'a str,
    body_html: String,
    form_html: String,
    replies_html: Vec<String>,
}

#[derive(Template)]
#[template(path = "comment_form.html")]
struct CommentFormTemplate<'a> {
    action: &'a str,
    in_reply_to: &'a str,
    label: &'a str,
}

pub fn render(thread: &Thread, options: &Options) -> String {
    let root_html = render_node(&thread.root, options);
    ThreadTemplate {
        slug: &thread.slug,
        root_html,
        mailto_address: options.mailto_address,
        show_email_link: options.show_email_link,
    }
    .render()
    .expect("thread template is valid")
}

pub fn empty(slug: &str, options: &Options) -> String {
    let form_html = if options.allow_relay {
        comment_form(options.comment_action, "", "Leave a comment")
    } else {
        String::new()
    };
    EmptyTemplate {
        slug,
        form_html,
        mailto_address: options.mailto_address,
        show_email_link: options.show_email_link,
    }
    .render()
    .expect("empty template is valid")
}

fn render_node(node: &Node, options: &Options) -> String {
    let form_html = if options.allow_relay {
        comment_form(options.comment_action, &node.message.message_id, "Reply")
    } else {
        String::new()
    };
    let replies_html = node
        .replies
        .iter()
        .map(|r| render_node(r, options))
        .collect();

    CommentTemplate {
        id: &node.message.message_id,
        initial: initial(&node.message.display_name),
        display_name: &node.message.display_name,
        body_html: escape_paragraphs(&node.message.body),
        form_html,
        replies_html,
    }
    .render()
    .expect("comment template is valid")
}

fn comment_form(action: &str, in_reply_to: &str, label: &str) -> String {
    CommentFormTemplate {
        action,
        in_reply_to,
        label,
    }
    .render()
    .expect("comment form template is valid")
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_paragraphs(input: &str) -> String {
    input
        .split("\n\n")
        .map(|p| format!("<p>{}</p>", escape(p).replace('\n', "<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}
