use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Convert markdown text to Telegram-compatible HTML.
///
/// Supports: bold, italic, code, code blocks, links, blockquotes, headers, lists.
/// Falls back to `<pre>` wrapping if the input has no recognizable markdown.
pub fn md_to_telegram_html(md: &str) -> String {
    let trimmed = md.trim();
    if trimmed.is_empty() {
        return "(no output)".to_string();
    }

    let opts = Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(trimmed, opts);

    let mut html = String::with_capacity(trimmed.len());
    let mut in_code_block = false;
    let mut list_depth: usize = 0;
    let mut had_markdown = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { .. } => {
                    html.push_str("<b>");
                    had_markdown = true;
                }
                Tag::Strong => {
                    html.push_str("<b>");
                    had_markdown = true;
                }
                Tag::Emphasis => {
                    html.push_str("<i>");
                    had_markdown = true;
                }
                Tag::Strikethrough => {
                    html.push_str("<s>");
                    had_markdown = true;
                }
                Tag::CodeBlock(_) => {
                    html.push_str("<pre>");
                    in_code_block = true;
                    had_markdown = true;
                }
                Tag::Link { dest_url, .. } => {
                    html.push_str(&format!("<a href=\"{}\">", escape_attr(&dest_url)));
                    had_markdown = true;
                }
                Tag::BlockQuote(_) => {
                    html.push_str("<blockquote>");
                    had_markdown = true;
                }
                Tag::List(_) => {
                    list_depth += 1;
                }
                Tag::Item => {
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    html.push_str(&indent);
                    html.push_str("• ");
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    html.push_str("\n\n");
                }
                TagEnd::Heading(_) => {
                    html.push_str("</b>\n");
                }
                TagEnd::Strong => {
                    html.push_str("</b>");
                }
                TagEnd::Emphasis => {
                    html.push_str("</i>");
                }
                TagEnd::Strikethrough => {
                    html.push_str("</s>");
                }
                TagEnd::CodeBlock => {
                    html.push_str("</pre>");
                    in_code_block = false;
                }
                TagEnd::Link => {
                    html.push_str("</a>");
                }
                TagEnd::BlockQuote(_) => {
                    html.push_str("</blockquote>");
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                }
                TagEnd::Item => {
                    html.push('\n');
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    html.push_str(&html_escape(&text));
                } else {
                    html.push_str(&html_escape(&text));
                }
            }
            Event::Code(code) => {
                html.push_str("<code>");
                html.push_str(&html_escape(&code));
                html.push_str("</code>");
                had_markdown = true;
            }
            Event::SoftBreak => {
                html.push('\n');
            }
            Event::HardBreak => {
                html.push('\n');
            }
            Event::Rule => {
                html.push_str("\n———\n");
            }
            // Raw HTML in markdown — escape it for safe display
            Event::Html(raw) | Event::InlineHtml(raw) => {
                html.push_str(&html_escape(&raw));
            }
            _ => {}
        }
    }

    // If no markdown was detected, wrap in <pre> for clean display
    let result = html.trim().to_string();
    if !had_markdown && !result.is_empty() {
        return format!("<pre>{}</pre>", html_escape(trimmed));
    }

    result
}

/// Split text for Telegram's 4096-char limit.
///
/// Splits at `\n\n`, then `\n`, then force-splits. Avoids splitting inside
/// `<pre>` or `<code>` tags when possible.
pub fn split_for_telegram(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find a split point within max_len
        let search_area = &remaining[..max_len];

        // Try to split at \n\n first
        let split_at = search_area
            .rfind("\n\n")
            // Then try \n
            .or_else(|| search_area.rfind('\n'))
            // Force-split at char boundary
            .unwrap_or_else(|| {
                remaining
                    .char_indices()
                    .take_while(|(i, _)| *i < max_len)
                    .last()
                    .map(|(i, c)| i + c.len_utf8())
                    .unwrap_or(max_len)
            });

        // Don't split in the middle of a <pre> block if we can avoid it
        let chunk = &remaining[..split_at];
        let open_pre = chunk.matches("<pre>").count();
        let close_pre = chunk.matches("</pre>").count();
        if open_pre > close_pre {
            // We're inside a <pre> block — try to find </pre> after split point
            if let Some(pre_end) = remaining[split_at..].find("</pre>") {
                let extended = split_at + pre_end + 6; // 6 = "</pre>".len()
                if extended <= remaining.len() {
                    chunks.push(remaining[..extended].to_string());
                    remaining = remaining[extended..].trim_start();
                    continue;
                }
            }
        }

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }

    if chunks.is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_wraps_in_pre() {
        let result = md_to_telegram_html("hello world");
        assert!(result.contains("<pre>"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn bold_converts() {
        let result = md_to_telegram_html("**bold text**");
        assert!(result.contains("<b>bold text</b>"));
    }

    #[test]
    fn code_block_converts() {
        let result = md_to_telegram_html("```\nsome code\n```");
        assert!(result.contains("<pre>"));
        assert!(result.contains("some code"));
    }

    #[test]
    fn inline_code_converts() {
        let result = md_to_telegram_html("use `foo()` here");
        assert!(result.contains("<code>foo()</code>"));
    }

    #[test]
    fn split_short_text() {
        let chunks = split_for_telegram("short", 4000);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn split_long_text() {
        let text = "a\n\n".repeat(2000);
        let chunks = split_for_telegram(&text, 100);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 110); // Some tolerance for trim
        }
    }

    #[test]
    fn empty_input() {
        assert_eq!(md_to_telegram_html(""), "(no output)");
    }

    #[test]
    fn escapes_html_in_plain() {
        let result = md_to_telegram_html("<script>alert(1)</script>");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
    }
}
