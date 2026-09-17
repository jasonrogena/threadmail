use askama::Template;

use crate::thread::{Node, Thread};

#[cfg(test)]
mod tests;

pub struct Options<'a> {
    pub comment_action: &'a str,
    pub mailto_address: &'a str,
    pub allow_relay: bool,
    pub show_email_link: bool,
    pub theme: &'a str,
    pub just_posted: bool,
    pub refresh_interval_secs: u64,
}

#[derive(Template)]
#[template(path = "style.html")]
struct StyleTemplate<'a> {
    theme: &'a str,
}

#[derive(Template)]
#[template(path = "thread.html")]
struct ThreadTemplate<'a> {
    slug: &'a str,
    style_html: String,
    root_html: String,
    mailto_address: &'a str,
    show_email_link: bool,
    just_posted: bool,
    refresh_interval_secs: u64,
}

#[derive(Template)]
#[template(path = "empty.html")]
struct EmptyTemplate<'a> {
    slug: &'a str,
    style_html: String,
    form_html: String,
    mailto_address: &'a str,
    show_email_link: bool,
    just_posted: bool,
    refresh_interval_secs: u64,
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

fn style(theme: &str) -> String {
    StyleTemplate { theme }
        .render()
        .expect("style template is valid")
}

impl Thread {
    pub fn render(&self, options: &Options) -> String {
        let root_html = self.root.render(options);
        ThreadTemplate {
            slug: &self.slug,
            style_html: style(options.theme),
            root_html,
            mailto_address: options.mailto_address,
            show_email_link: options.show_email_link,
            just_posted: options.just_posted,
            refresh_interval_secs: options.refresh_interval_secs,
        }
        .render()
        .expect("thread template is valid")
    }
}

pub fn empty(slug: &str, options: &Options) -> String {
    let form_html = if options.allow_relay {
        comment_form(options.comment_action, "", "Leave a comment")
    } else {
        String::new()
    };
    EmptyTemplate {
        slug,
        style_html: style(options.theme),
        form_html,
        mailto_address: options.mailto_address,
        show_email_link: options.show_email_link,
        just_posted: options.just_posted,
        refresh_interval_secs: options.refresh_interval_secs,
    }
    .render()
    .expect("empty template is valid")
}

impl Node {
    fn render(&self, options: &Options) -> String {
        let form_html = if options.allow_relay {
            comment_form(options.comment_action, &self.message.message_id, "Reply")
        } else {
            String::new()
        };
        let replies_html = self.replies.iter().map(|r| r.render(options)).collect();

        CommentTemplate {
            id: &self.message.message_id,
            initial: initial(&self.message.display_name),
            display_name: &self.message.display_name,
            body_html: escape_paragraphs(&self.message.body),
            form_html,
            replies_html,
        }
        .render()
        .expect("comment template is valid")
    }
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
