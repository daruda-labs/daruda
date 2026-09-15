//! Preserve literal administrative text; translate only agent-authored Markdown.

use super::Message;
use crate::remote_channel::bridge::MessageTail;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub fn slack(message: &Message) -> (String, bool) {
    match &message.tail {
        MessageTail::Plain(text) => (text.clone(), false),
        MessageTail::Markdown(text) => (slack_markdown(text), true),
    }
}

pub fn discord(message: &Message) -> String {
    let tail = match &message.tail {
        MessageTail::Plain(text) => discord_literal(text),
        MessageTail::Markdown(text) => text.clone(),
    };
    if message.header.is_empty() {
        tail
    } else {
        format!("{}\n{tail}", discord_literal(&message.header))
    }
}

fn discord_literal(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '#' | '[' | ']'
        ) {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}

fn slack_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn slack_markdown(text: &str) -> String {
    let mut result = String::new();
    let mut links = Vec::new();
    for event in Parser::new_ext(
        text,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
    ) {
        match event {
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineHtml(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text) => result.push_str(&slack_escape(&text)),
            Event::Code(text) => result.push_str(&format!("`{}`", slack_escape(&text))),
            Event::Start(Tag::Strong | Tag::Heading { .. }) | Event::End(TagEnd::Strong) => {
                result.push('*')
            }
            Event::End(TagEnd::Heading(_)) => result.push_str("*\n"),
            Event::Start(Tag::Emphasis) | Event::End(TagEnd::Emphasis) => result.push('_'),
            Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough) => {
                result.push('~')
            }
            Event::Start(Tag::CodeBlock(_)) => result.push_str("\n```\n"),
            Event::End(TagEnd::CodeBlock) => result.push_str("\n```\n"),
            Event::Start(Tag::Item) => result.push_str("- "),
            Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::TableRow)
            | Event::SoftBreak
            | Event::HardBreak => result.push('\n'),
            Event::End(TagEnd::TableCell) => result.push_str(" | "),
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                links.push(dest_url)
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some(url) = links.pop() {
                    result.push_str(&format!(" ({})", slack_escape(&url)));
                }
            }
            Event::TaskListMarker(checked) => {
                result.push_str(if checked { "[x] " } else { "[ ] " })
            }
            Event::Rule => result.push_str("\n---\n"),
            _ => {}
        }
    }
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_are_not_markdown_parsed() {
        let message = Message::plain("file_name.txt * <@U1>".into(), None);
        assert_eq!(slack(&message), ("file_name.txt * <@U1>".into(), false));
        assert_eq!(discord(&message), "file\\_name.txt \\* <@U1\\>");
    }

    #[test]
    fn agent_markdown_uses_platform_syntax_without_slack_mentions() {
        let message = Message {
            header: "my_project".into(),
            tail: MessageTail::Markdown("**Done** and `file_name` <@U1>".into()),
            keyboard: None,
        };
        let (text, markdown) = slack(&message);
        assert!(markdown);
        assert!(text.contains("*Done*"));
        assert!(text.contains("&lt;@U1&gt;"));
        assert!(discord(&message).starts_with("my\\_project\n**Done**"));
    }
}
