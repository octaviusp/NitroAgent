use serde_json::Value;

/// Result of parsing a single stream-json line from Claude CLI.
#[derive(Debug, Default)]
pub struct StreamEvent {
    /// Text content extracted from this event (empty if none).
    pub text: String,
    /// Session ID if discovered in this event.
    pub session_id: Option<String>,
}

/// Session ID key names to scan for recursively.
const SESSION_KEYS: &[&str] = &[
    "session_id",
    "sessionId",
    "conversation_id",
    "conversationId",
    "thread_id",
    "threadId",
];

/// Parse a single line of stream-json output from Claude CLI.
///
/// Returns extracted text content and optional session ID.
/// Mirrors the Python `parse_engine_stream_line` logic.
pub fn parse_stream_line(line: &str) -> StreamEvent {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return StreamEvent::default();
    }

    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            // Non-JSON line, return as raw text
            return StreamEvent {
                text: line.to_string(),
                session_id: None,
            };
        }
    };

    let obj = match parsed.as_object() {
        Some(o) => o,
        None => return StreamEvent::default(),
    };

    let session_id = extract_session_id(&parsed);
    let text = extract_event_text(obj);

    StreamEvent { text, session_id }
}

/// Extract text content from a stream-json event object.
///
/// Event types from Claude Code CLI:
/// - `stream_event` -> content_block_delta text
/// - `user` -> tool_result content
/// - `error` / `warning` -> message text
/// - `assistant`, `result`, `system` -> skip (avoid duplication)
fn extract_event_text(event: &serde_json::Map<String, Value>) -> String {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "stream_event" => {
            let inner = match event.get("event").and_then(|v| v.as_object()) {
                Some(e) => e,
                None => return String::new(),
            };
            let inner_type = inner.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if inner_type == "content_block_delta" {
                if let Some(delta) = inner.get("delta").and_then(|v| v.as_object()) {
                    let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if delta_type == "text_delta" {
                        return delta
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                }
            }
            String::new()
        }
        "user" => {
            let content_list = event
                .get("message")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_array());

            match content_list {
                Some(items) => {
                    let parts: Vec<&str> = items
                        .iter()
                        .filter_map(|item| {
                            let obj = item.as_object()?;
                            if obj.get("type")?.as_str()? == "tool_result" {
                                obj.get("content")?.as_str()
                            } else {
                                None
                            }
                        })
                        .collect();
                    parts.join("\n")
                }
                None => String::new(),
            }
        }
        "error" => {
            let msg = event.get("message").and_then(|v| v.as_str()).unwrap_or("");
            format!("[error] {msg}")
        }
        "warning" => {
            let msg = event.get("message").and_then(|v| v.as_str()).unwrap_or("");
            format!("[warning] {msg}")
        }
        // assistant, result, system -> skip
        _ => String::new(),
    }
}

/// Recursively scan a JSON value for session ID keys.
fn extract_session_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if SESSION_KEYS.contains(&key.as_str()) {
                    match val {
                        Value::String(s) => return Some(s.clone()),
                        Value::Number(n) => return Some(n.to_string()),
                        _ => {}
                    }
                }
                if let Some(found) = extract_session_id(val) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(found) = extract_session_id(item) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// Rolling buffer that keeps the last N characters of appended text.
pub struct RollingBuffer {
    buf: String,
    max_chars: usize,
}

impl RollingBuffer {
    pub fn new(max_chars: usize) -> Self {
        Self {
            buf: String::with_capacity(max_chars),
            max_chars,
        }
    }

    pub fn append(&mut self, text: &str) {
        self.buf.push_str(text);
        if self.buf.len() > self.max_chars {
            let excess = self.buf.len() - self.max_chars;
            // Find a char boundary to drain from
            let drain_to = self.buf.ceil_char_boundary(excess);
            self.buf.drain(..drain_to);
        }
    }

    pub fn value(&self) -> &str {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_line() {
        let result = parse_stream_line("");
        assert!(result.text.is_empty());
        assert!(result.session_id.is_none());
    }

    #[test]
    fn parse_text_delta() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}}"#;
        let result = parse_stream_line(line);
        assert_eq!(result.text, "Hello");
    }

    #[test]
    fn parse_session_id() {
        let line = r#"{"type":"system","session_id":"abc-123"}"#;
        let result = parse_stream_line(line);
        assert_eq!(result.session_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn parse_error_event() {
        let line = r#"{"type":"error","message":"something went wrong"}"#;
        let result = parse_stream_line(line);
        assert_eq!(result.text, "[error] something went wrong");
    }

    #[test]
    fn rolling_buffer_truncates() {
        let mut buf = RollingBuffer::new(10);
        buf.append("hello world!!");
        assert!(buf.value().len() <= 10);
    }

    #[test]
    fn parse_non_json_line() {
        let result = parse_stream_line("just plain text");
        assert_eq!(result.text, "just plain text");
        assert!(result.session_id.is_none());
    }
}
