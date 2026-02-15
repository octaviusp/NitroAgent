# Claude Code CLI Integration Spec

> Comprehensive reference for integrating a Telegram bot with Claude Code's headless CLI.
> Version: 2.1.42 | Tested: 2026-02-15 | Model: claude-opus-4-6

---

## Table of Contents

1. [Environment Setup](#1-environment-setup)
2. [Stream-JSON Output Format](#2-stream-json-output-format)
3. [Session Management](#3-session-management)
4. [Stream-JSON Input Format](#4-stream-json-input-format)
5. [Permission Modes](#5-permission-modes)
6. [System Prompts](#6-system-prompts)
7. [Tool Configuration](#7-tool-configuration)
8. [MCP Configuration](#8-mcp-configuration)
9. [Budget Control](#9-budget-control)
10. [Model Selection](#10-model-selection)
11. [Structured Output](#11-structured-output)
12. [Bot Command → CLI Mapping](#12-bot-command--cli-mapping)
13. [Error Handling](#13-error-handling)
14. [Edge Cases & Known Issues](#14-edge-cases--known-issues)

---

## 1. Environment Setup

### Critical: Strip Nesting Variables

When running Claude CLI from inside a Claude Code session (e.g., the bot process itself runs inside Claude Code), you **must** strip environment variables to avoid nesting conflicts:

```bash
env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS \
  claude -p "<prompt>" [flags...]
```

Without this, the child Claude process may inherit parent session state and fail or behave unpredictably.

### Base Invocation

```bash
env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS \
  claude -p "<prompt>" \
  --output-format stream-json \
  --verbose \
  --dangerously-skip-permissions
```

**Required flags for programmatic use:**
- `-p` / `--print` — Non-interactive mode, exit after response
- `--output-format stream-json` — NDJSON output for parsing
- `--verbose` — Required when using `stream-json` output (CLI enforces this)

**Recommended flags:**
- `--dangerously-skip-permissions` — Skip permission prompts (required for autonomous bot)
- `--no-session-persistence` — Don't save session to disk (for one-shot queries)
- `--include-partial-messages` — Get token-by-token streaming events

---

## 2. Stream-JSON Output Format

Output is **newline-delimited JSON** (NDJSON). Each line is a self-contained JSON object with a `type` field.

### 2.1 Message Type: `system` (init)

**Always the first message.** Contains session metadata.

```json
{
  "type": "system",
  "subtype": "init",
  "cwd": "/path/to/working/directory",
  "session_id": "8b5de1fc-6b32-4a54-bacc-e9328bec7d25",
  "tools": ["Bash", "Read", "Edit", "Write", "Grep", "Glob", "WebFetch", "WebSearch", "..."],
  "mcp_servers": [{"name": "chromedevtools", "status": "connected"}],
  "model": "claude-opus-4-6",
  "permissionMode": "default",
  "claude_code_version": "2.1.42",
  "slash_commands": ["compact", "context", "cost", "..."],
  "agents": ["Bash", "Explore", "Plan", "..."],
  "skills": ["commit", "debug", "..."],
  "plugins": [{"name": "plugin-name", "path": "/path/to/plugin"}],
  "uuid": "e875b84a-2db8-4a8e-a0bd-780062fba5a1",
  "fast_mode_state": "off"
}
```

**Key fields for the bot:**
- `session_id` — Save this for session resume
- `model` — Confirms which model is running
- `tools` — List of available tools (verify expected tools are present)
- `permissionMode` — Should be `"bypassPermissions"` when using `--dangerously-skip-permissions`

### 2.2 Message Type: `stream_event` (partial streaming)

**Only emitted with `--include-partial-messages`.** Wraps Anthropic API streaming events.

#### `message_start`
```json
{
  "type": "stream_event",
  "event": {
    "type": "message_start",
    "message": {
      "model": "claude-opus-4-6",
      "id": "msg_01Hby5rP3NxMeHJsm2h3XP4U",
      "type": "message",
      "role": "assistant",
      "content": [],
      "stop_reason": null,
      "usage": {"input_tokens": 3, "output_tokens": 8}
    }
  },
  "session_id": "...",
  "parent_tool_use_id": null,
  "uuid": "..."
}
```

#### `content_block_start`
```json
{
  "type": "stream_event",
  "event": {
    "type": "content_block_start",
    "index": 0,
    "content_block": {"type": "text", "text": ""}
  },
  "session_id": "...",
  "parent_tool_use_id": null,
  "uuid": "..."
}
```

#### `content_block_delta` (THE KEY EVENT FOR STREAMING TEXT)
```json
{
  "type": "stream_event",
  "event": {
    "type": "content_block_delta",
    "index": 0,
    "delta": {"type": "text_delta", "text": "HELLO_WORLD"}
  },
  "session_id": "...",
  "parent_tool_use_id": null,
  "uuid": "..."
}
```

**To stream text to the user:** filter for `type == "stream_event"` where `event.delta.type == "text_delta"` and extract `event.delta.text`.

#### `content_block_stop`
```json
{
  "type": "stream_event",
  "event": {"type": "content_block_stop", "index": 0},
  "session_id": "...",
  "parent_tool_use_id": null,
  "uuid": "..."
}
```

#### `message_delta`
```json
{
  "type": "stream_event",
  "event": {
    "type": "message_delta",
    "delta": {"stop_reason": "end_turn", "stop_sequence": null},
    "usage": {"input_tokens": 3, "output_tokens": 8},
    "context_management": {"applied_edits": []}
  },
  "session_id": "...",
  "parent_tool_use_id": null,
  "uuid": "..."
}
```

#### `message_stop`
```json
{
  "type": "stream_event",
  "event": {"type": "message_stop"},
  "session_id": "...",
  "parent_tool_use_id": null,
  "uuid": "..."
}
```

### 2.3 Message Type: `assistant`

Complete assistant message. Emitted after all stream events (or directly without `--include-partial-messages`).

**Text response:**
```json
{
  "type": "assistant",
  "message": {
    "model": "claude-opus-4-6",
    "id": "msg_01VnJvzAJYCdTGwEXFdD3hTM",
    "type": "message",
    "role": "assistant",
    "content": [{"type": "text", "text": "The actual response text"}],
    "stop_reason": null,
    "usage": {
      "input_tokens": 3,
      "cache_creation_input_tokens": 30015,
      "cache_read_input_tokens": 0,
      "output_tokens": 8
    },
    "context_management": null
  },
  "parent_tool_use_id": null,
  "session_id": "...",
  "uuid": "..."
}
```

**Tool use response:**
```json
{
  "type": "assistant",
  "message": {
    "model": "claude-opus-4-6",
    "id": "msg_...",
    "type": "message",
    "role": "assistant",
    "content": [
      {
        "type": "tool_use",
        "id": "toolu_01MMwN4wA95KDMeos6wKD26Z",
        "name": "Read",
        "input": {"file_path": "/path/to/file.rs"},
        "caller": {"type": "direct"}
      }
    ],
    "stop_reason": null,
    "usage": {"input_tokens": 3, "output_tokens": 19}
  },
  "parent_tool_use_id": null,
  "session_id": "...",
  "uuid": "..."
}
```

### 2.4 Message Type: `user` (tool results)

Emitted when the CLI executes a tool and feeds the result back.

```json
{
  "type": "user",
  "message": {
    "role": "user",
    "content": [
      {
        "tool_use_id": "toolu_01MMwN4wA95KDMeos6wKD26Z",
        "type": "tool_result",
        "content": "     1>[package]\n     2>name = \"my-project\"..."
      }
    ]
  },
  "parent_tool_use_id": null,
  "session_id": "...",
  "uuid": "...",
  "tool_use_result": {
    "type": "text",
    "file": {
      "filePath": "/path/to/Cargo.toml",
      "content": "[package]\nname = \"my-project\"...",
      "numLines": 17,
      "startLine": 1,
      "totalLines": 17
    }
  }
}
```

### 2.5 Message Type: `result` (FINAL)

**Always the last message.** Contains the final result and metadata.

```json
{
  "type": "result",
  "subtype": "success",
  "is_error": false,
  "duration_ms": 2267,
  "duration_api_ms": 1944,
  "num_turns": 1,
  "result": "The final text response",
  "stop_reason": null,
  "session_id": "8b5de1fc-6b32-4a54-bacc-e9328bec7d25",
  "total_cost_usd": 0.18780875,
  "usage": {
    "input_tokens": 3,
    "cache_creation_input_tokens": 30015,
    "cache_read_input_tokens": 0,
    "output_tokens": 8,
    "server_tool_use": {
      "web_search_requests": 0,
      "web_fetch_requests": 0
    },
    "service_tier": "standard",
    "iterations": [],
    "speed": "standard"
  },
  "modelUsage": {
    "claude-opus-4-6": {
      "inputTokens": 3,
      "outputTokens": 8,
      "cacheReadInputTokens": 0,
      "cacheCreationInputTokens": 30015,
      "webSearchRequests": 0,
      "costUSD": 0.18780875,
      "contextWindow": 200000,
      "maxOutputTokens": 32000
    }
  },
  "permission_denials": [],
  "uuid": "..."
}
```

**Result subtypes:**
| Subtype | Meaning |
|---------|---------|
| `"success"` | Normal completion |
| `"error_max_budget_usd"` | Budget limit exceeded |

**When `--json-schema` is used**, the result includes:
```json
{
  "type": "result",
  "subtype": "success",
  "result": "",
  "structured_output": {"greeting": "Hello!", "language": "en"},
  "..."
}
```

### 2.6 Sequence Diagram

```
Without --include-partial-messages:
  system(init) → assistant → [user → assistant]* → result

With --include-partial-messages:
  system(init) → stream_event(message_start) → stream_event(content_block_start) →
  stream_event(content_block_delta)* → stream_event(content_block_stop) →
  assistant → stream_event(message_delta) → stream_event(message_stop) →
  [user → stream_events* → assistant]* → result
```

### 2.7 Extracting Text with jq

```bash
# Final result only
claude -p "..." --output-format stream-json --verbose 2>/dev/null | \
  jq -r 'select(.type == "result") | .result'

# Streaming text deltas
claude -p "..." --output-format stream-json --verbose --include-partial-messages 2>/dev/null | \
  jq -rj 'select(.type == "stream_event" and .event.delta.type? == "text_delta") | .event.delta.text'
```

---

## 3. Session Management

### 3.1 New Session (default)

Every `-p` invocation creates a new session with a random UUID.

```bash
claude -p "hello" --output-format json
# Returns session_id in result
```

### 3.2 Explicit Session ID

```bash
claude -p "hello" --session-id "550e8400-e29b-41d4-a716-446655440000"
# Must be a valid UUID format
```

### 3.3 Resume Session (`--resume`)

```bash
# Extract session_id from first invocation
session_id=$(claude -p "Start a review" --output-format json 2>/dev/null | jq -r '.session_id')

# Resume with new prompt
claude -p "Continue the review" --resume "$session_id" --output-format json
```

**Behavior:**
- Loads full conversation history from the original session
- Claude has full context of previous turns
- Same session_id is reused
- Session must have been persisted (don't use `--no-session-persistence` on the original)

**Invalid session ID:**
```
Exit code 1
Error: No conversation found with session ID: 00000000-0000-0000-0000-000000000000
```

### 3.4 Continue Most Recent (`--continue`)

```bash
claude -p "Continue" --continue
```

Continues the most recent session in the current working directory. Useful for sequential commands but not reliable for multi-user bot scenarios (use `--resume` with explicit IDs instead).

### 3.5 Fork Session (`--fork-session`)

```bash
claude -p "Try a different approach" --resume "$session_id" --fork-session
```

**Behavior:**
- Creates a **new session ID** while preserving the conversation context from the original
- The original session remains unchanged
- Useful for branching conversations (e.g., "try approach A" vs "try approach B")
- The new session_id is returned in the result

### 3.6 No Persistence (`--no-session-persistence`)

```bash
claude -p "one-shot question" --no-session-persistence
```

Session is not saved to disk. Cannot be resumed later. Use for ephemeral queries.

### 3.7 Session Storage

Sessions are stored locally by Claude Code. There is no programmatic API to list sessions. The `--resume` flag without a value opens an interactive picker (not usable in headless mode).

---

## 4. Stream-JSON Input Format

For multi-turn conversations within a single process, use `--input-format stream-json`. The process stays alive and accepts NDJSON on stdin.

### 4.1 User Message Format

```json
{"type":"user","message":{"role":"user","content":"Your prompt here"}}
```

### 4.2 Usage

```bash
echo '{"type":"user","message":{"role":"user","content":"What is 2+2?"}}' | \
  claude -p --input-format stream-json --output-format stream-json --verbose
```

### 4.3 Multi-turn (Pipe or Named Pipe)

```bash
# Using a named pipe for interactive multi-turn
mkfifo /tmp/claude-input
claude -p --input-format stream-json --output-format stream-json --verbose < /tmp/claude-input &
echo '{"type":"user","message":{"role":"user","content":"Hello"}}' > /tmp/claude-input
# ... read response ...
echo '{"type":"user","message":{"role":"user","content":"Follow up"}}' > /tmp/claude-input
```

### 4.4 Replay User Messages

Use `--replay-user-messages` to have user messages echoed back on stdout for acknowledgment:

```bash
claude -p --input-format stream-json --output-format stream-json --verbose --replay-user-messages
```

### 4.5 Known Issues

- Multi-turn stream-json input can hang after the first response (GitHub issue #3187)
- Process may not exit cleanly after final result (GitHub issue #25629)
- **Recommendation:** For the bot, prefer spawning separate `claude -p` processes per message and use `--resume` for multi-turn, rather than keeping a single long-lived process with stream-json input

---

## 5. Permission Modes

### 5.1 Available Modes

| Mode | Description | Bot Suitability |
|------|-------------|-----------------|
| `default` | Prompts for permission on each tool use | Not suitable for bot (blocks on stdin) |
| `acceptEdits` | Auto-accepts file edits, prompts for others | Partial (still prompts for Bash) |
| `plan` | Read-only, no modifications or execution | Good for safe analysis-only mode |
| `dontAsk` | Auto-denies unless pre-approved via rules | Usable with explicit allow rules |
| `bypassPermissions` | Skips all permission prompts | Recommended for bot (with safeguards) |
| `delegate` | Coordination-only for agent team leads | Not applicable |

### 5.2 Recommended for Bot

**Primary:** `--dangerously-skip-permissions` (sets `bypassPermissions` mode)

```bash
claude -p "..." --dangerously-skip-permissions
```

**Safer alternative:** `--permission-mode dontAsk` with explicit `--allowedTools`:

```bash
claude -p "..." \
  --permission-mode dontAsk \
  --allowedTools "Bash,Read,Edit,Write,Grep,Glob"
```

### 5.3 Tool Allow/Deny Patterns

```bash
# Allow specific bash commands
--allowedTools 'Bash(git *) Bash(cargo *) Read Edit'

# Deny dangerous tools
--disallowedTools 'Bash(rm *) Bash(sudo *)'
```

Pattern syntax uses glob matching. A trailing ` *` (with space) enforces word boundary:
- `Bash(git *)` matches `git commit -m "..."` but NOT `gitk`
- `Bash(git*)` matches both

---

## 6. System Prompts

### 6.1 Replace Default Prompt

```bash
claude -p "..." --system-prompt "You are a Rust expert. Only write safe, idiomatic Rust code."
```

This **removes all default Claude Code instructions** including tool usage guidance.

### 6.2 Append to Default Prompt (Recommended)

```bash
claude -p "..." --append-system-prompt "Additional context: you are helping via a Telegram bot. Keep responses concise."
```

Preserves Claude Code's default behavior while adding custom instructions.

### 6.3 Load from File

```bash
claude -p "..." --system-prompt-file ./prompts/bot-system.txt
claude -p "..." --append-system-prompt-file ./prompts/extra-rules.txt
```

### 6.4 Precedence

- `--system-prompt` and `--system-prompt-file` are mutually exclusive
- Append flags can be combined with either replacement flag
- For the bot: use `--append-system-prompt` to inject compact session summaries while keeping default Claude Code capabilities

---

## 7. Tool Configuration

### 7.1 Restrict Available Tools (`--tools`)

```bash
# Only allow specific tools
claude -p "..." --tools "Bash,Read,Edit,Write,Grep,Glob"

# Disable all tools (text-only response)
claude -p "..." --tools ""

# Use all default tools
claude -p "..." --tools "default"
```

### 7.2 Auto-Approve Tools (`--allowedTools`)

```bash
claude -p "..." --allowedTools "Bash Read Edit Write Grep Glob"
```

These tools execute without prompting. Does NOT restrict which tools are available (use `--tools` for that).

### 7.3 Deny Tools (`--disallowedTools`)

```bash
claude -p "..." --disallowedTools "WebFetch WebSearch"
```

Removes tools from the model's context entirely.

### 7.4 Custom Subagents (`--agents`)

```bash
claude -p "..." --agents '{
  "code-reviewer": {
    "description": "Expert code reviewer",
    "prompt": "You are a senior code reviewer.",
    "tools": ["Read", "Grep", "Glob"],
    "model": "sonnet"
  }
}'
```

Agent fields:
| Field | Required | Description |
|-------|----------|-------------|
| `description` | Yes | When to invoke |
| `prompt` | Yes | System prompt for agent |
| `tools` | No | Restrict tools (inherits all if omitted) |
| `disallowedTools` | No | Deny specific tools |
| `model` | No | `sonnet`, `opus`, `haiku`, `inherit` |
| `maxTurns` | No | Limit agentic turns |

---

## 8. MCP Configuration

### 8.1 List MCP Servers

```bash
claude mcp list
# Output:
# chromedevtools: npx chrome-devtools-mcp@latest ... - Connected
```

### 8.2 Get MCP Server Details

```bash
claude mcp get <name>
# Shows: scope, status, type, command, args, environment
```

### 8.3 Add MCP Server

```bash
# stdio server
claude mcp add my-server -- npx my-mcp-server

# HTTP server
claude mcp add --transport http sentry https://mcp.sentry.dev/mcp

# HTTP with auth headers
claude mcp add --transport http my-api https://api.example.com/mcp --header "Authorization: Bearer ..."

# With environment variables
claude mcp add -e API_KEY=xxx my-server -- npx my-mcp-server

# From JSON
claude mcp add-json my-server '{"command":"npx","args":["my-server"]}'
```

### 8.4 Remove MCP Server

```bash
claude mcp remove <name>
# With scope: claude mcp remove "name" -s user
```

### 8.5 Load MCP Config at Runtime

```bash
claude -p "..." --mcp-config ./mcp.json

# Only use servers from this config (ignore all other MCP configs)
claude -p "..." --mcp-config ./mcp.json --strict-mcp-config
```

**MCP config file format** (same as Claude Desktop):
```json
{
  "mcpServers": {
    "server-name": {
      "command": "npx",
      "args": ["my-mcp-server"],
      "env": {
        "API_KEY": "xxx"
      }
    }
  }
}
```

### 8.6 Restart MCP (for bot /restart command)

There is no direct CLI command to restart MCP servers. Options:
1. Kill and respawn the claude process (recommended for bot)
2. Remove and re-add: `claude mcp remove name && claude mcp add name ...`

---

## 9. Budget Control

### 9.1 Max Budget

```bash
claude -p "..." --max-budget-usd 5.00
```

**Behavior:**
- Stops execution when cumulative cost exceeds the limit
- Returns a result with `subtype: "error_max_budget_usd"`
- Cost includes all API calls (input tokens, output tokens, cache creation)
- Print mode only

**Budget exceeded result:**
```json
{
  "type": "result",
  "subtype": "error_max_budget_usd",
  "is_error": false,
  "duration_ms": 2050,
  "total_cost_usd": 0.023939,
  "session_id": "...",
  "errors": []
}
```

### 9.2 Max Turns

```bash
claude -p "..." --max-turns 3
```

Limits the number of agentic turns (tool use cycles). Exits with an error when reached.

---

## 10. Model Selection

### 10.1 Model Aliases

```bash
claude -p "..." --model sonnet    # claude-sonnet-4-5-20250929
claude -p "..." --model opus      # claude-opus-4-6
claude -p "..." --model haiku     # claude-haiku-4-5-20251001
```

### 10.2 Full Model Name

```bash
claude -p "..." --model claude-sonnet-4-5-20250929
```

### 10.3 Fallback Model

```bash
claude -p "..." --model opus --fallback-model sonnet
```

Automatically falls back to the specified model when the default is overloaded. Print mode only.

### 10.4 Effort Level

```bash
claude -p "..." --effort low      # Faster, less thorough
claude -p "..." --effort medium   # Balanced
claude -p "..." --effort high     # Most thorough (default)
```

---

## 11. Structured Output

### 11.1 JSON Schema Validation

```bash
claude -p "Extract function names from main.rs" \
  --output-format json \
  --json-schema '{"type":"object","properties":{"functions":{"type":"array","items":{"type":"string"}}},"required":["functions"]}'
```

**Response:**
```json
{
  "type": "result",
  "subtype": "success",
  "result": "",
  "structured_output": {
    "functions": ["main", "handle_message", "process_command"]
  },
  "session_id": "..."
}
```

**How it works internally:**
- Claude Code injects a `StructuredOutput` tool into the available tools
- The model calls this tool with the structured data as its input
- The `structured_output` field in the result contains the validated output
- The `result` field is empty when structured output is used

---

## 12. Bot Command → CLI Mapping

### Recommended CLI invocations for each Telegram bot command:

| Bot Command | CLI Invocation |
|-------------|---------------|
| `/ask <prompt>` | `claude -p "<prompt>" --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions --no-session-persistence` |
| `/ask <prompt>` (with session) | `claude -p "<prompt>" --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions --resume "<session_id>"` |
| `/new` | Start next `/ask` without `--resume` flag |
| `/resume <id>` | `claude -p "<prompt>" --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions --resume "<session_id>"` |
| `/fork` | Add `--fork-session` to the resume command |
| `/compact` | Not directly available via CLI. Use `--append-system-prompt` to inject a compacted summary when resuming |
| `/status` | Parse the `system(init)` message for session metadata |
| `/cost` | Parse `total_cost_usd` and `usage` from the `result` message |
| `/model <name>` | Add `--model <name>` to the next invocation |
| `/budget <usd>` | Add `--max-budget-usd <amount>` to the next invocation |
| `/tools` | Parse `tools` array from `system(init)` message |
| `/cancel` | Kill the `claude` child process (SIGTERM then SIGKILL) |
| `/clear` | Start next `/ask` without `--resume` and with `--no-session-persistence` |

### Working Directory

```bash
# Set working directory for the bot
claude -p "..." --add-dir /path/to/user/project

# The cwd is wherever claude is launched from
cd /path/to/project && claude -p "..."
```

---

## 13. Error Handling

### 13.1 Exit Codes

| Exit Code | Meaning |
|-----------|---------|
| 0 | Success |
| 1 | Error (invalid args, session not found, etc.) |

### 13.2 Error Messages (stderr)

```
Error: When using --print, --output-format=stream-json requires --verbose
Error: No conversation found with session ID: <uuid>
Error: Expected message type 'user' or 'control', got '<wrong_type>'
Error parsing streaming input line: <json>: TypeError: ...
```

### 13.3 Result Error Types

```json
{"type": "result", "subtype": "error_max_budget_usd", "is_error": false, ...}
```

### 13.4 Permission Denials

When a tool is denied (in non-bypass mode), the result includes:
```json
{"permission_denials": ["Tool X was denied"]}
```

---

## 14. Edge Cases & Known Issues

### 14.1 Stream-JSON Input Hanging

Multi-turn conversations via `--input-format stream-json` can hang after the first response. **Workaround:** Use separate `claude -p` processes with `--resume` for each turn.

### 14.2 Process Exit

The claude process may not exit cleanly after emitting the final `result` event (GitHub issue #25629). **Workaround:** Set a timeout after receiving the `result` message and kill the process if it hasn't exited.

### 14.3 Large Outputs

Very large tool results (e.g., reading a large file) are included in the `user` message type. No truncation is applied by the CLI. The model's context window (200k tokens for Opus) is the effective limit.

### 14.4 Concurrent Sessions

Multiple `claude -p` processes can run simultaneously. Each gets its own session. No locking or coordination is needed.

### 14.5 Cache Behavior

The Claude API uses prompt caching. The `usage` fields show:
- `cache_creation_input_tokens` — tokens cached for future use
- `cache_read_input_tokens` — tokens read from cache (cheaper)

First invocation is expensive (cache creation). Subsequent invocations with similar system prompts are cheaper (cache hits). This favors consistent system prompts across invocations.

### 14.6 Cost Tracking

The `total_cost_usd` in the result is the cost for that single invocation. For multi-turn via `--resume`, each invocation reports only its own cost. The bot should accumulate costs across invocations for accurate per-user tracking.

### 14.7 Version

```bash
claude --version
# 2.1.42
```

---

## Appendix A: Quick Reference — Rust Struct Mapping

Suggested Rust enum for parsing stream-json messages:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClaudeMessage {
    #[serde(rename = "system")]
    System {
        subtype: String,
        session_id: String,
        cwd: String,
        tools: Vec<String>,
        model: String,
        #[serde(rename = "permissionMode")]
        permission_mode: String,
        claude_code_version: String,
        #[serde(flatten)]
        extra: Value,
    },
    #[serde(rename = "stream_event")]
    StreamEvent {
        event: StreamEventData,
        session_id: String,
        parent_tool_use_id: Option<String>,
        uuid: String,
    },
    #[serde(rename = "assistant")]
    Assistant {
        message: AssistantMessage,
        session_id: String,
        parent_tool_use_id: Option<String>,
        uuid: String,
    },
    #[serde(rename = "user")]
    User {
        message: Value,
        session_id: String,
        uuid: String,
        #[serde(default)]
        tool_use_result: Option<Value>,
    },
    #[serde(rename = "result")]
    Result {
        subtype: String,
        is_error: bool,
        duration_ms: u64,
        num_turns: u32,
        result: String,
        session_id: String,
        total_cost_usd: f64,
        usage: Value,
        #[serde(default)]
        structured_output: Option<Value>,
        #[serde(default)]
        permission_denials: Vec<String>,
        uuid: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct StreamEventData {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub delta: Option<Delta>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Deserialize)]
pub struct Delta {
    #[serde(rename = "type")]
    pub delta_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    pub model: String,
    pub id: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}
```

---

## Appendix B: Minimal Streaming Example (Bash)

```bash
#!/bin/bash
# Stream Claude Code response to stdout in real-time

env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT -u CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS \
  claude -p "$1" \
  --output-format stream-json \
  --verbose \
  --include-partial-messages \
  --dangerously-skip-permissions \
  --no-session-persistence \
  2>/dev/null | while IFS= read -r line; do
    type=$(echo "$line" | jq -r '.type // empty')
    case "$type" in
      system)
        session_id=$(echo "$line" | jq -r '.session_id')
        echo "Session: $session_id" >&2
        ;;
      stream_event)
        text=$(echo "$line" | jq -r '.event.delta.text // empty')
        if [ -n "$text" ]; then
          printf '%s' "$text"
        fi
        ;;
      result)
        echo "" # newline after streaming
        cost=$(echo "$line" | jq -r '.total_cost_usd')
        echo "Cost: \$$cost" >&2
        ;;
    esac
  done
```
