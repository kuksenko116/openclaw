# openclaw-cli

AI agent CLI that talks to LLMs and executes tools. Supports Anthropic, OpenAI, Ollama, and OpenAI-compatible providers (OpenRouter, Together, Google/Gemini).

**Features:** streaming responses, tool execution (bash, file ops, web fetch), session persistence, context auto-compaction, extended thinking (Anthropic), image input, prompt caching.

## Build

Requires Rust 1.75+.

```bash
cargo build --release
# Binary: target/release/openclaw-cli (3.6 MB)
```

## Quick Start

```bash
# Set your API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Single prompt
openclaw-cli chat "Explain the difference between TCP and UDP"

# Interactive REPL
openclaw-cli chat -i

# Resume a named session
openclaw-cli chat -i -s my-project

# Use a different model
openclaw-cli chat -m opus "Review this code for security issues"

# Pipe input
echo "Summarize this" | openclaw-cli chat
```

## Commands

### `chat` — Send a prompt or start interactive mode

```
openclaw-cli chat [OPTIONS] [PROMPT]
```

| Flag | Description |
|------|-------------|
| `-i, --interactive` | Start interactive REPL with line editing and history |
| `-s, --session <NAME>` | Resume or create a named session |
| `-m, --model <MODEL>` | Override model (supports aliases: `sonnet`, `opus`, `haiku`, `gpt4o`, `gpt4o-mini`) |
| `-p, --provider <NAME>` | Override provider (`anthropic`, `openai`, `ollama`, `openrouter`, `together`, `google`) |
| `--api-key <KEY>` | Override API key |
| `--base-url <URL>` | Override provider base URL |
| `--system-prompt <TEXT>` | Override system prompt |
| `--no-tools` | Disable tool execution |
| `--max-tokens <N>` | Override max output tokens |

**Examples:**

```bash
# One-shot with a specific model
openclaw-cli chat -m haiku "What is 2+2?"

# Interactive with tools disabled
openclaw-cli chat -i --no-tools

# Use Ollama locally
openclaw-cli chat -p ollama -m llama3 "Hello"

# Custom API endpoint
openclaw-cli chat --base-url https://my-proxy.example.com -m gpt-4o "Hi"
```

### `sessions` — Manage saved sessions

```bash
# List all sessions (newest first)
openclaw-cli sessions list
```

Sessions are stored as JSON in `~/.openclaw-cli/sessions/`.

### `providers` — Show provider info

```bash
# Show configured provider, model, base URL, API key status
openclaw-cli providers list
```

## Interactive REPL

Start with `openclaw-cli chat -i`. Supports line editing (arrow keys, Ctrl+A/E/U/K), persistent history, and slash commands.

### Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show all commands and model aliases |
| `/new [model]` | Reset session, optionally switch model |
| `/reset` | Clear all messages |
| `/status` | Show session info: model, messages, token estimate, context usage % |
| `/compact [instructions]` | Summarize old messages to reduce context size |
| `/model [name]` | Show or switch model (supports aliases) |
| `/usage` | Show cumulative tokens and estimated cost |
| `/think [level]` | Set thinking mode: `off`, `low`, `medium`, `high` |
| `/verbose [on\|off]` | Toggle verbose output |
| `/info` | Show provider, model, base URL, max tokens |

Exit with `exit`, `quit`, or Ctrl+D.

### Image Input

Image paths in prompts are automatically detected and attached:

```
> Describe what you see in /home/alex/screenshot.png
> Compare /tmp/before.jpg and /tmp/after.jpg
```

Supported formats: JPG, PNG, WebP, GIF.

## Configuration

Config file: `~/.openclaw-cli/config.yaml` (override with `$OPENCLAW_CLI_CONFIG`).

```yaml
# Provider & model
provider: "anthropic"                    # anthropic, openai, ollama, openrouter, together, google
model: "claude-sonnet-4-20250514"        # or use aliases: sonnet, opus, haiku, gpt4o
api_key: "${ANTHROPIC_API_KEY}"          # supports ${ENV_VAR} substitution
base_url: "https://api.anthropic.com"    # optional custom endpoint

# Generation
max_tokens: 4096             # default varies by model
temperature: 0.7             # 0.0-2.0, disabled when thinking is active
system_prompt: "You are a helpful assistant"

# Extended thinking (Anthropic only)
thinking_budget: 4096        # off, or token count (low=1024, medium=4096, high=16384)

# Sessions
sessions_dir: "~/.openclaw-cli/sessions"

# Tool access control
tools:
  profile: "full"            # full, coding, minimal, none
  exec:
    security: "full"         # full, deny, allowlist
    allowlist:               # when security=allowlist
      - "git "               # trailing space = prefix match
      - "cargo "
      - "ls"                 # no space = exact binary match
```

### Tool Profiles

| Profile | Available Tools |
|---------|----------------|
| `full` | bash, read, write, edit, glob, grep, web_fetch |
| `coding` | bash, read, write, edit, glob, grep, web_fetch |
| `minimal` | read, glob, grep |
| `none` | (none) |

### Bash Security Modes

- **`full`** — Unrestricted command execution
- **`deny`** — All bash commands blocked
- **`allowlist`** — Only whitelisted commands allowed; shell metacharacters (`;`, `&&`, `|`, `` ` ``, `$(...)`, heredocs) are rejected

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `ANTHROPIC_API_KEY` | API key for Anthropic |
| `OPENAI_API_KEY` | API key for OpenAI / OpenRouter / Together |
| `GOOGLE_API_KEY` | API key for Google / Gemini |
| `OPENCLAW_CLI_CONFIG` | Override config file path |
| `SHELL` | Shell for bash tool (default: `/bin/bash`) |
| `RUST_LOG` | Tracing verbosity (`RUST_LOG=openclaw_cli=debug`) |

## Tools

The agent has access to these tools during conversation:

### `bash` — Execute shell commands

Runs commands via `$SHELL -c`. Timeout default: 2 minutes (max: 10 minutes). Output truncated at 30,000 characters.

### `read` — Read a file

Returns file contents with line numbers. Default limit: 2,000 lines. Blocks access to sensitive paths (`.ssh/`, `.aws/credentials`, etc.).

### `write` — Write a file

Creates or overwrites a file. Creates parent directories as needed. Atomic write via temp file + rename.

### `edit` — Replace text in a file

Exact string match and replace. Fails if the search string isn't found or matches multiple times (unless `replace_all=true`).

### `glob` — Find files by pattern

Glob pattern matching (`**/*.rs`, `src/*/test.ts`). Returns up to 10,000 results.

### `grep` — Search file contents

Regex search across files. Uses `rg` (ripgrep) if available, falls back to `grep -rn`.

### `web_fetch` — Fetch a URL

HTTP GET with 30s timeout. Strips HTML tags, decodes entities, pretty-prints JSON. Output truncated at 50,000 characters.

## Model Aliases

| Alias | Resolves To |
|-------|-------------|
| `sonnet` | claude-sonnet-4-20250514 |
| `opus` | claude-opus-4-20250514 |
| `haiku` | claude-haiku-3-20250307 |
| `gpt4o` | gpt-4o |
| `gpt4o-mini` | gpt-4o-mini |
| `gpt4-turbo` | gpt-4-turbo |

Unknown model names are passed through as-is.

## Providers

### Anthropic (default)

Default base URL: `https://api.anthropic.com`. Supports streaming, extended thinking, prompt caching, and images.

### OpenAI

Default base URL: `https://api.openai.com/v1`. Supports streaming and images.

### Ollama

Default base URL: `http://127.0.0.1:11434`. No API key required. Streams via NDJSON. Extended timeout (600s).

### OpenAI-Compatible

OpenRouter, Together, Google/Gemini use the OpenAI protocol internally. Set `base_url` in config to point to the provider's endpoint.

## Agent Behavior

- **Max turns per request:** 20
- **Auto-compaction:** When context exceeds 75% of the model's window, old messages are summarized via the LLM to free space
- **Retry logic:** Retries up to 2 times on rate limit (429), overload (529), or server error (503)
- **Token usage:** Printed to stderr after each response with estimated cost

## Directory Structure

```
~/.openclaw-cli/
├── config.yaml      # Configuration
├── history          # REPL command history
├── sessions/        # Saved sessions (.json)
└── memory/          # Agent memory files
```

## Running Tests

```bash
cargo test                              # 151 tests
cargo clippy -- -D warnings             # Lint check
cargo fmt -- --check                    # Format check
```

## License

See the repository root for license information.
