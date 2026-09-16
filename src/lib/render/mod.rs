//! Pure rendering of a resolved `Thread` into a semantic HTML5 fragment.
//!
//! No JavaScript, no client-side templating: this is the exact HTML that
//! gets served at `GET /thread/<slug>` for the page's `<iframe>` to display.
//! Every piece of message content is HTML-escaped here — this is the one
//! place that stands between a commenter's text and a rendered page, so it
//! is deliberately conservative rather than clever.

use crate::thread::{Node, Thread};

#[cfg(test)]
mod tests;

/// Rendering knobs a deployment can turn on or off independently of one
/// another (see `Config::relay_comments`/`Config::show_email_link`).
pub struct Options<'a> {
    /// URL the no-JS reply forms post to.
    pub comment_action: &'a str,
    /// The mailing list's own posting address, used for the `mailto:` link.
    pub mailto_address: &'a str,
    /// Whether to render the no-JS comment forms at all.
    pub allow_relay: bool,
    /// Whether to render the `mailto:` "comment by email" link.
    pub show_email_link: bool,
}

/// Renders an existing, resolved thread.
pub fn render(thread: &Thread, options: &Options) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "<section class=\"thread\" data-slug=\"{}\">\n",
        escape(&thread.slug)
    ));
    render_node(&thread.root, options, &mut out);
    push_email_hint(&mut out, &thread.slug, options);
    out.push_str("</section>\n");
    out
}

/// Renders the state for a slug with no comments yet. Still offers a
/// comment form (replying to nothing, i.e. a new top-level comment) when
/// relaying is enabled, since a thread's root is just whichever real
/// comment arrives first — nothing needs to exist beforehand.
pub fn empty(slug: &str, options: &Options) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "<section class=\"thread\" data-slug=\"{}\">\n",
        escape(slug)
    ));
    out.push_str("<p>No comments yet. Be the first.</p>\n");
    if options.allow_relay {
        out.push_str(&comment_form(options.comment_action, ""));
    }
    push_email_hint(&mut out, slug, options);
    out.push_str("</section>\n");
    out
}

fn push_email_hint(out: &mut String, slug: &str, options: &Options) {
    if !options.show_email_link {
        return;
    }
    out.push_str(&format!(
        "<p class=\"email-hint\"><a href=\"mailto:{addr}?subject={slug}\">Comment by email</a></p>\n",
        addr = escape(options.mailto_address),
        slug = escape(&url_encode_subject(slug))
    ));
}

fn render_node(node: &Node, options: &Options, out: &mut String) {
    out.push_str("<article class=\"comment\">\n");
    out.push_str(&format!(
        "<header>{}</header>\n",
        escape(&node.message.display_name)
    ));
    out.push_str(&format!(
        "<div class=\"body\">{}</div>\n",
        escape_paragraphs(&node.message.body)
    ));
    if options.allow_relay {
        out.push_str(&comment_form(
            options.comment_action,
            &node.message.message_id,
        ));
    }

    if !node.replies.is_empty() {
        out.push_str("<ol class=\"replies\">\n");
        for reply in &node.replies {
            out.push_str("<li>\n");
            render_node(reply, options, out);
            out.push_str("</li>\n");
        }
        out.push_str("</ol>\n");
    }

    out.push_str("</article>\n");
}

/// `in_reply_to` of `""` means "reply to nothing", i.e. a new top-level
/// comment — the handler already treats an empty submitted value as `None`.
fn comment_form(action: &str, in_reply_to: &str) -> String {
    format!(
        "<form method=\"post\" action=\"{action}\">\n\
         <input type=\"hidden\" name=\"in_reply_to\" value=\"{parent}\">\n\
         <label>Name <input type=\"text\" name=\"name\" required></label>\n\
         <label>Comment <textarea name=\"body\" required></textarea></label>\n\
         <button type=\"submit\">Reply</button>\n\
         </form>\n",
        action = escape(action),
        parent = escape(in_reply_to)
    )
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

/// Slugs are expected to already be URL-safe (kebab-case); this only guards
/// against the one character a `mailto:` query value can't carry raw.
fn url_encode_subject(slug: &str) -> String {
    slug.replace(' ', "%20")
}
