use crate::thread::{Node, Thread};

#[cfg(test)]
mod tests;

pub struct Options<'a> {
    pub comment_action: &'a str,
    pub mailto_address: &'a str,
    pub allow_relay: bool,
    pub show_email_link: bool,
}

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

// Empty in_reply_to means a new top-level comment.
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

fn url_encode_subject(slug: &str) -> String {
    slug.replace(' ', "%20")
}
