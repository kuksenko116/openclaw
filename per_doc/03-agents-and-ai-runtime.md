# Agents and AI Runtime Architecture

This document covers the complete AI agent runtime: from an incoming user message through model resolution, tool creation, streaming execution, and response delivery. All file paths are relative to `src/agents/` unless stated otherwise.

---

## Table of Contents

1. [Agent Execution Entry Point](#agent-execution-entry-point)
2. [Streaming and Subscription](#streaming-and-subscription)
3. [Tool Management](#tool-management)
4. [Tool Policy](#tool-policy)
5. [Tool Schema Normalization](#tool-schema-normalization)
6. [Tool Result Guard](#tool-result-guard)
7. [Model Providers](#model-providers)
8. [Model Selection and Compatibility](#model-selection-and-compatibility)
9. [Auth Profiles](#auth-profiles)
10. [Bash Tool System](#bash-tool-system)
11. [Session Management](#session-management)
12. [Sandbox](#sandbox)
13. [System Prompt Building](#system-prompt-building)
14. [Usage and Metrics](#usage-and-metrics)
15. [Complete Request-to-Response Flow](#complete-request-to-response-flow)

---

## Agent Execution Entry Point

**File:** `pi-embedded-runner/run.ts`

### Function Signature

```typescript
export async function runEmbeddedPiAgent(
  params: RunEmbeddedPiAgentParams,
): Promise<EmbeddedPiRunResult>
```

### RunEmbeddedPiAgentParams (pi-embedded-runner/run/params.ts)

The top-level params type drives every aspect of a run:

```typescript
type RunEmbeddedPiAgentParams = {
  sessionId: string;
  sessionKey?: string;
  agentId?: string;
  messageChannel?: string;          // e.g. "telegram", "slack", "discord"
  messageProvider?: string;
  agentAccountId?: string;
  messageTo?: string;                // delivery target for topic/thread routing
  messageThreadId?: string | number;
  groupId?: string | null;           // channel-level tool policy resolution
  groupChannel?: string | null;
  groupSpace?: string | null;
  spawnedBy?: string | null;         // parent session key for subagent inheritance
  senderId?: string | null;
  senderName?: string | null;
  senderUsername?: string | null;
  senderE164?: string | null;
  senderIsOwner?: boolean;
  currentChannelId?: string;         // Slack auto-threading
  currentThreadTs?: string;
  replyToMode?: "off" | "first" | "all";
  hasRepliedRef?: { value: boolean };
  requireExplicitMessageTarget?: boolean;
  disableMessageTool?: boolean;
  sessionFile: string;
  workspaceDir: string;
  agentDir?: string;
  config?: OpenClawConfig;
  skillsSnapshot?: SkillSnapshot;
  prompt: string;
  images?: ImageContent[];
  clientTools?: ClientToolDefinition[];  // OpenResponses hosted tools
  disableTools?: boolean;
  provider?: string;
  model?: string;
  authProfileId?: string;
  authProfileIdSource?: "auto" | "user";
  thinkLevel?: ThinkLevel;
  verboseLevel?: VerboseLevel;
  reasoningLevel?: ReasoningLevel;
  toolResultFormat?: ToolResultFormat;
  execOverrides?: Pick<ExecToolDefaults, "host" | "security" | "ask" | "node">;
  bashElevated?: ExecElevatedDefaults;
  timeoutMs: number;
  runId: string;
  abortSignal?: AbortSignal;
  // Callback hooks
  shouldEmitToolResult?: () => boolean;
  shouldEmitToolOutput?: () => boolean;
  onPartialReply?: (payload) => void | Promise<void>;
  onAssistantMessageStart?: () => void | Promise<void>;
  onBlockReply?: (payload) => void | Promise<void>;
  onBlockReplyFlush?: () => void | Promise<void>;
  blockReplyBreak?: "text_end" | "message_end";
  blockReplyChunking?: BlockReplyChunking;
  onReasoningStream?: (payload) => void | Promise<void>;
  onToolResult?: (payload) => void | Promise<void>;
  onAgentEvent?: (evt) => void;
  lane?: string;
  enqueue?: typeof enqueueCommand;
  extraSystemPrompt?: string;
  inputProvenance?: InputProvenance;
  streamParams?: AgentStreamParams;
  ownerNumbers?: string[];
  enforceFinalTag?: boolean;
};
```

### Setup Phase

1. **Lane Queueing:** The run is enqueued on both a session lane (keyed by `sessionKey` or `sessionId`) and a global lane (keyed by `lane`). This serializes concurrent requests to the same session while also throttling global concurrency.

2. **Workspace Resolution:** `resolveRunWorkspaceDir()` determines the effective workspace directory, applying fallback logic when the configured workspace is unavailable. A fallback warning is logged when this occurs.

3. **Model Resolution:** `resolveModel()` (from `pi-embedded-runner/model.ts`) performs:
   - Auth storage discovery from `agentDir`
   - Model registry lookup from the `models.json` config file
   - Inline provider fallback for unrecognized model references
   - Forward-compatibility synthesis for newer model IDs (e.g., claude-opus-4-6 from a claude-opus-4-5 template)

4. **Context Window Validation:** `resolveContextWindowInfo()` and `evaluateContextWindowGuard()` verify the model's context window is sufficient. Constants:
   - `CONTEXT_WINDOW_HARD_MIN_TOKENS`: below this, the run is blocked with a `FailoverError`
   - `CONTEXT_WINDOW_WARN_BELOW_TOKENS`: below this, a warning is logged but the run proceeds

5. **Auth Profile Initialization:** `ensureAuthProfileStore()` loads the credential store, `resolveAuthProfileOrder()` determines the ordered list of profile candidates, and the first viable candidate is selected (skipping those in cooldown).

### Retry/Failover Loop

The core of `runEmbeddedPiAgent` is a `while (true)` loop that handles:

- **Auth Profile Rotation:** On auth/billing/rate-limit/failover errors, `advanceAuthProfile()` tries the next profile candidate. If a user-locked profile (`authProfileIdSource: "user"`) is set, rotation is disabled.
- **Thinking Level Fallback:** `pickFallbackThinkingLevel()` downgrades the thinking level (e.g., `high` to `minimal`) when the provider rejects the requested thinking level. Tracked via `attemptedThinking: Set<ThinkLevel>` to avoid infinite loops.
- **Context Overflow Recovery:** Up to `MAX_OVERFLOW_COMPACTION_ATTEMPTS` (3) auto-compaction attempts via `compactEmbeddedPiSessionDirect()`. If compaction fails, falls back to `truncateOversizedToolResultsInSession()` for sessions with single oversized tool results.
- **FailoverError Propagation:** When `fallbackConfigured` is true (model fallback chain configured in config), errors are wrapped in `FailoverError` to trigger the model-fallback system upstream.

### Usage Accumulation

```typescript
type UsageAccumulator = {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  total: number;
  lastCacheRead: number;   // from most recent API call only
  lastCacheWrite: number;  // from most recent API call only
  lastInput: number;       // from most recent API call only
};
```

The accumulator sums token usage across all API calls in a run (including tool-use round-trips). However, for context-window utilization display, only `lastCacheRead`/`lastCacheWrite`/`lastInput` from the most recent call are used, because accumulated `cacheRead` inflates to `N * context_size` across `N` calls.

### Anti-Refusal Scrubbing

Anthropic's magic refusal-test string (`ANTHROPIC_MAGIC_STRING_TRIGGER_REFUSAL`) is stripped from prompts before submission to prevent it from poisoning session transcripts.

### Return Type

```typescript
type EmbeddedPiRunResult = {
  payloads?: Array<{
    text?: string;
    mediaUrl?: string;
    mediaUrls?: string[];
    replyToId?: string;
    isError?: boolean;
  }>;
  meta: EmbeddedPiRunMeta;
  didSendViaMessagingTool?: boolean;
  messagingToolSentTexts?: string[];
  messagingToolSentTargets?: MessagingToolSend[];
};

type EmbeddedPiRunMeta = {
  durationMs: number;
  agentMeta?: EmbeddedPiAgentMeta;
  aborted?: boolean;
  systemPromptReport?: SessionSystemPromptReport;
  error?: {
    kind: "context_overflow" | "compaction_failure" | "role_ordering" | "image_size";
    message: string;
  };
  stopReason?: string;
  pendingToolCalls?: Array<{ id: string; name: string; arguments: string }>;
};

type EmbeddedPiAgentMeta = {
  sessionId: string;
  provider: string;
  model: string;
  compactionCount?: number;
  promptTokens?: number;
  usage?: { input?; output?; cacheRead?; cacheWrite?; total? };
  lastCallUsage?: { input?; output?; cacheRead?; cacheWrite?; total? };
};
```

---

## Streaming and Subscription

**File:** `pi-embedded-subscribe.ts`

### Function Signature

```typescript
export function subscribeEmbeddedPiSession(
  params: SubscribeEmbeddedPiSessionParams
)
```

Returns an object with:
- `assistantTexts: string[]` -- accumulated assistant text blocks
- `toolMetas: Array<{ toolName?; meta? }>` -- tool execution metadata
- `unsubscribe()` -- teardown function
- `isCompacting()` -- whether compaction is in-flight or pending retry
- `waitForCompactionRetry()` -- promise that resolves when all pending compactions complete
- `didSendViaMessagingTool()` -- whether a messaging tool successfully sent a message
- `getMessagingToolSentTexts()` -- texts sent via messaging tools
- `getMessagingToolSentTargets()` -- targets that received messages
- `getLastToolError()` -- last tool error summary
- `getUsageTotals()` -- accumulated token usage
- `getCompactionCount()` -- number of auto-compactions during this run

### Subscription State

```typescript
type EmbeddedPiSubscribeState = {
  assistantTexts: string[];
  toolMetas: Array<{ toolName?; meta? }>;
  toolMetaById: Map<string, string | undefined>;
  toolSummaryById: Set<string>;
  lastToolError?: ToolErrorSummary;

  blockReplyBreak: "text_end" | "message_end";
  reasoningMode: ReasoningLevel;    // "off" | "on" | "stream"
  includeReasoning: boolean;
  shouldEmitPartialReplies: boolean;
  streamReasoning: boolean;

  deltaBuffer: string;
  blockBuffer: string;
  blockState: { thinking: boolean; final: boolean; inlineCode: InlineCodeState };
  partialBlockState: { thinking: boolean; final: boolean; inlineCode: InlineCodeState };

  lastStreamedAssistant?: string;
  lastStreamedAssistantCleaned?: string;
  emittedAssistantUpdate: boolean;
  lastStreamedReasoning?: string;
  lastBlockReplyText?: string;
  assistantMessageIndex: number;
  lastAssistantTextMessageIndex: number;
  lastAssistantTextNormalized?: string;
  lastAssistantTextTrimmed?: string;
  assistantTextBaseline: number;
  suppressBlockChunks: boolean;
  lastReasoningSent?: string;

  compactionInFlight: boolean;
  pendingCompactionRetry: number;
  compactionRetryResolve?: () => void;
  compactionRetryReject?: (reason?) => void;
  compactionRetryPromise: Promise<void> | null;
  unsubscribed: boolean;

  messagingToolSentTexts: string[];
  messagingToolSentTextsNormalized: string[];
  messagingToolSentTargets: MessagingToolSend[];
  pendingMessagingTexts: Map<string, string>;
  pendingMessagingTargets: Map<string, MessagingToolSend>;
  lastAssistant?: AgentMessage;
};
```

### Event Handler Dispatch

`createEmbeddedPiSessionEventHandler()` (in `pi-embedded-subscribe.handlers.ts`) routes events to specialized handlers:

| Event Type                | Handler File                                     | Function                     |
|---------------------------|--------------------------------------------------|------------------------------|
| `message_start`           | `pi-embedded-subscribe.handlers.messages.ts`     | `handleMessageStart`         |
| `message_update`          | `pi-embedded-subscribe.handlers.messages.ts`     | `handleMessageUpdate`        |
| `message_end`             | `pi-embedded-subscribe.handlers.messages.ts`     | `handleMessageEnd`           |
| `tool_execution_start`    | `pi-embedded-subscribe.handlers.tools.ts`        | `handleToolExecutionStart`   |
| `tool_execution_update`   | `pi-embedded-subscribe.handlers.tools.ts`        | `handleToolExecutionUpdate`  |
| `tool_execution_end`      | `pi-embedded-subscribe.handlers.tools.ts`        | `handleToolExecutionEnd`     |
| `agent_start`             | `pi-embedded-subscribe.handlers.lifecycle.ts`    | `handleAgentStart`           |
| `agent_end`               | `pi-embedded-subscribe.handlers.lifecycle.ts`    | `handleAgentEnd`             |
| `auto_compaction_start`   | `pi-embedded-subscribe.handlers.compaction.ts`   | `handleAutoCompactionStart`  |
| `auto_compaction_end`     | `pi-embedded-subscribe.handlers.compaction.ts`   | `handleAutoCompactionEnd`    |

### Block Reply Chunking

The `EmbeddedBlockChunker` (from `pi-embedded-block-chunker.ts`) buffers streamed text and emits it in chunks according to `BlockReplyChunking` settings. Chunks are emitted via `emitBlockChunk()`, which:

1. Strips `<think>...</think>` and `<final>...</final>` tags across chunk boundaries using stateful regex scanning
2. Strips downgraded tool call text patterns (`[Tool Call: ...]`, `[Historical context: ...]`)
3. Deduplicates against previously sent block replies
4. Checks against committed messaging tool texts to suppress duplicates
5. Invokes the `onBlockReply` callback with parsed reply directives (media URLs, audio-as-voice, reply-to)

### Reasoning Mode

Three modes control how model reasoning/thinking content is handled:

- **`"off"`** (default): Strips `<think>...</think>` tags and their content. Uses stateful regex `THINKING_TAG_SCAN_RE` across chunk boundaries. Also supports `<thinking>`, `<thought>`, and `<antthinking>` variants.
- **`"on"`**: Includes thinking content in the output. When `onBlockReply` is not set, reasoning text is accumulated and merged into `assistantTexts` at `message_end`.
- **`"stream"`**: Streams thinking content in real-time via `emitReasoningStream()`, which broadcasts to WebSocket clients via `emitAgentEvent()` and invokes the `onReasoningStream` callback. Delta computation ensures only new text is sent.

### Final Tag Processing

When `enforceFinalTag` is enabled, only text inside `<final>...</final>` blocks is emitted. Text outside `<final>` blocks (including model "thinking out loud" without `<think>` tags) is silently suppressed. This is a strict mode that prevents leakage of reasoning that some models emit without proper `<think>` wrapping.

### Messaging Tool Deduplication

The subscription tracks texts sent via messaging tools (Telegram, WhatsApp, Discord, Slack, `sessions_send`) through two parallel tracking systems:

- **Pending texts** (`pendingMessagingTexts: Map<string, string>`): Tracked during tool execution. Not used for suppression to avoid lost messages if the tool fails.
- **Committed texts** (`messagingToolSentTexts: string[]`, `messagingToolSentTextsNormalized: string[]`): Committed on successful tool completion. Used for duplicate suppression.

The `isMessagingToolDuplicateNormalized()` function performs fuzzy matching after `normalizeTextForComparison()` to handle whitespace/formatting differences.

### Compaction Coordination

The subscription manages compaction state with a promise-based coordination mechanism:

- `noteCompactionRetry()`: Increments `pendingCompactionRetry`, creates the compaction promise
- `resolveCompactionRetry()`: Decrements counter, resolves promise when counter reaches zero and no compaction is in-flight
- `waitForCompactionRetry()`: Returns the promise; rejects with `AbortError` if unsubscribed
- On unsubscribe: rejects pending compaction promise to unblock awaiting code; aborts in-flight compaction via `session.abortCompaction()`

---

## Tool Management

**File:** `pi-tools.ts`

### Function Signature

```typescript
export function createOpenClawCodingTools(options?: {
  exec?: ExecToolDefaults & ProcessToolDefaults;
  messageProvider?: string;
  agentAccountId?: string;
  messageTo?: string;
  messageThreadId?: string | number;
  sandbox?: SandboxContext | null;
  sessionKey?: string;
  agentDir?: string;
  workspaceDir?: string;
  config?: OpenClawConfig;
  abortSignal?: AbortSignal;
  modelProvider?: string;
  modelId?: string;
  modelAuthMode?: ModelAuthMode;
  currentChannelId?: string;
  currentThreadTs?: string;
  groupId?: string | null;
  groupChannel?: string | null;
  groupSpace?: string | null;
  spawnedBy?: string | null;
  senderIsOwner?: boolean;
  // ... more options
}): AnyAgentTool[]
```

### Tool Assembly Pipeline

1. **Base Coding Tools:** Imported from `@mariozechner/pi-coding-agent` (`codingTools`). The `read`, `write`, `edit` tools are replaced with OpenClaw-specific versions that handle sandbox filesystem bridging and parameter normalization.

2. **Exec Tool:** `createExecTool()` from `bash-tools.ts` -- command execution with sandbox support, background process management, per-agent scope isolation.

3. **Process Tool:** `createProcessTool()` -- manages background processes (list, kill, send-keys).

4. **Apply Patch Tool:** `createApplyPatchTool()` -- enabled only for OpenAI providers and gated by `applyPatch.enabled` config and `applyPatch.allowModels` list.

5. **Channel Agent Tools:** `listChannelAgentTools()` -- channel-specific tools like `whatsapp_login`.

6. **OpenClaw Native Tools** (`openclaw-tools.ts` via `createOpenClawTools()`):
   - `browser` -- browser control via CDP
   - `web_search` -- web search
   - `web_fetch` -- URL content fetching
   - `image` -- image generation/manipulation
   - `message` -- cross-channel messaging
   - `sessions_list`, `sessions_history`, `sessions_send`, `sessions_spawn` -- session management
   - `session_status` -- session status
   - `canvas` -- canvas/drawing tool
   - `cron` -- scheduled tasks
   - `gateway` -- gateway management
   - `nodes` -- device/node management
   - `agents_list` -- agent listing
   - `memory_search`, `memory_get` -- memory/knowledge base
   - `tts` -- text-to-speech

7. **Plugin Tools:** `resolvePluginTools()` loads dynamically registered plugin tools, filtered by the computed `pluginToolAllowlist`.

### Tool Post-Processing Pipeline

After assembly, tools pass through:

1. **Owner-Only Policy:** `applyOwnerOnlyToolPolicy()` wraps sensitive tools (e.g., `whatsapp_login`) to reject execution when `senderIsOwner` is false.

2. **Tool Policy Pipeline:** `applyToolPolicyPipeline()` applies a stack of allow/deny policies:
   - Profile policy (with `alsoAllow` merged)
   - Provider-specific profile policy
   - Global policy
   - Global provider policy
   - Agent-specific policy
   - Agent provider policy
   - Group/channel policy
   - Sandbox tool policy
   - Subagent tool policy

3. **Schema Normalization:** `normalizeToolParameters()` for each tool.

4. **Hook Wrapping:** `wrapToolWithBeforeToolCallHook()` wraps each tool to allow plugin hooks before execution.

5. **Abort Signal Wrapping:** `wrapToolWithAbortSignal()` connects each tool to the run's abort controller.

---

## Tool Policy

**File:** `pi-tools.policy.ts`

### Policy Resolution

`resolveEffectiveToolPolicy()` computes the layered policy hierarchy:

```typescript
function resolveEffectiveToolPolicy(params: {
  config?: OpenClawConfig;
  sessionKey?: string;
  modelProvider?: string;
  modelId?: string;
}): {
  agentId?: string;
  globalPolicy?: SandboxToolPolicy;
  globalProviderPolicy?: SandboxToolPolicy;
  agentPolicy?: SandboxToolPolicy;
  agentProviderPolicy?: SandboxToolPolicy;
  profile?: string;           // e.g. "minimal", "coding", "messaging", "full"
  providerProfile?: string;
  profileAlsoAllow?: string[];
  providerProfileAlsoAllow?: string[];
}
```

Provider-specific policies are resolved via `resolveProviderToolPolicy()`, which matches against both `provider/modelId` keys and bare `provider` keys in the `byProvider` config section.

### Group/Channel Policy

`resolveGroupToolPolicy()` extracts group context from the session key, resolves the message channel, then queries:
1. Channel dock's `resolveToolPolicy` if available
2. `resolveChannelGroupToolsPolicy()` from config as fallback

### Subagent Policy

`resolveSubagentToolPolicy()` applies a default deny list for subagent sessions:

```typescript
const DEFAULT_SUBAGENT_TOOL_DENY = [
  "sessions_list", "sessions_history", "sessions_send", "sessions_spawn",
  "gateway", "agents_list", "whatsapp_login", "session_status",
  "cron", "memory_search", "memory_get",
];
```

### Tool Groups (tool-policy.ts)

```typescript
const TOOL_GROUPS: Record<string, string[]> = {
  "group:memory":     ["memory_search", "memory_get"],
  "group:web":        ["web_search", "web_fetch"],
  "group:fs":         ["read", "write", "edit", "apply_patch"],
  "group:runtime":    ["exec", "process"],
  "group:sessions":   ["sessions_list", "sessions_history", "sessions_send",
                        "sessions_spawn", "session_status"],
  "group:ui":         ["browser", "canvas"],
  "group:automation": ["cron", "gateway"],
  "group:messaging":  ["message"],
  "group:nodes":      ["nodes"],
  "group:openclaw":   [/* all native tools */],
};
```

### Tool Profiles

```typescript
type ToolProfileId = "minimal" | "coding" | "messaging" | "full";

const TOOL_PROFILES: Record<ToolProfileId, ToolProfilePolicy> = {
  minimal:   { allow: ["session_status"] },
  coding:    { allow: ["group:fs", "group:runtime", "group:sessions", "group:memory", "image"] },
  messaging: { allow: ["group:messaging", "sessions_list", "sessions_history",
                        "sessions_send", "session_status"] },
  full:      {},  // empty = allow all
};
```

### Policy Matching

`makeToolPolicyMatcher()` compiles glob patterns from `allow` and `deny` arrays (with `expandToolGroups()` resolution) and returns a function that checks a tool name against:
1. If denied, reject
2. If no allow list, accept
3. If in allow list, accept
4. Special case: `apply_patch` is allowed if `exec` is allowed

---

## Tool Schema Normalization

**File:** `pi-tools.schema.ts`

### Purpose

Ensures tool JSON Schemas are portable across providers with different schema requirements.

### normalizeToolParameters(tool)

1. **Simple schemas** (already have `type` + `properties`, no `anyOf`): Pass through `cleanSchemaForGemini()` only.

2. **Missing type** (has `properties` or `required` but no `type`): Adds `type: "object"` and cleans for Gemini.

3. **Union schemas** (`anyOf` or `oneOf` at top level):
   - Iterates all object-type variants
   - Merges property schemas across variants via `mergePropertySchemas()`
   - Merges enum values from `enum`, `const`, `anyOf`, `oneOf` sub-schemas
   - Computes `required` as properties required in ALL variants (intersection)
   - Flattens into a single `{ type: "object", properties, required }` schema
   - Cleans for Gemini

### cleanSchemaForGemini(schema)

Delegates to `schema/clean-for-gemini.ts`. Removes JSON Schema keywords unsupported by Gemini (e.g., `$schema`, `$defs`, `format`, `patternProperties`, etc.).

---

## Tool Result Guard

**File:** `session-tool-result-guard.ts`

### installSessionToolResultGuard(sessionManager, opts?)

Monkey-patches `sessionManager.appendMessage` to intercept all message persistence:

#### For `toolResult` messages:
- Extracts the `toolCallId` from the message
- Removes it from the pending tool calls map
- Applies `capToolResultSize()` to enforce `HARD_MAX_TOOL_RESULT_CHARS` (~50K chars)
- Applies optional `transformToolResultForPersistence` hook

#### Truncation Strategy (`capToolResultSize`):
- Calculates total text size across all text content blocks
- If under `HARD_MAX_TOOL_RESULT_CHARS`, returns unchanged
- Proportionally allocates budget across text blocks (minimum 2,000 chars each)
- Cuts at nearest newline boundary (within 80% of budget) for clean breaks
- Appends truncation suffix warning

#### For `assistant` messages:
- Runs `sanitizeToolCallInputs()` to clean tool call inputs
- Extracts tool call IDs and tracks them in a pending map

#### Missing Tool Results:
- When `allowSyntheticToolResults` is true (default), flushes synthetic `"[Tool result not available]"` results for any pending tool calls before:
  - Non-assistant messages arrive
  - New assistant messages with tool calls arrive (old pending are flushed first)
  - Explicit `flushPendingToolResults()` call

Returns `{ flushPendingToolResults, getPendingIds }`.

---

## Model Providers

**Files:** `models-config.ts`, `models-config.providers.ts`

### Provider Architecture

Providers fall into two categories:

#### Implicit Providers (auto-discovered)

Resolved by `resolveImplicitProviders()` based on environment variables and auth profile store:

| Provider               | API                      | Discovery Method                        | Auth Source                           |
|------------------------|--------------------------|-----------------------------------------|---------------------------------------|
| Amazon Bedrock         | `bedrock-converse-stream` | AWS SDK (`discoverBedrockModels`)       | AWS env vars / SDK default chain      |
| GitHub Copilot         | built-in                 | GitHub token exchange                   | `COPILOT_GITHUB_TOKEN` / `GH_TOKEN`  |
| Ollama                 | `ollama`                 | HTTP `/api/tags`                        | `OLLAMA_API_KEY` env / profile        |
| vLLM                   | `openai-completions`     | HTTP `/models`                          | `VLLM_API_KEY` env / profile          |
| MiniMax                | `anthropic-messages`     | Hardcoded catalog                       | `MINIMAX_API_KEY` / profile / OAuth   |
| MiniMax Portal         | `anthropic-messages`     | Hardcoded catalog                       | OAuth profile                         |
| Moonshot               | `openai-completions`     | Hardcoded catalog                       | `MOONSHOT_API_KEY` / profile          |
| Qwen Portal            | `openai-completions`     | Hardcoded catalog                       | OAuth profile                         |
| Xiaomi                 | `anthropic-messages`     | Hardcoded catalog                       | `XIAOMI_API_KEY` / profile            |
| Synthetic              | `anthropic-messages`     | Hardcoded catalog                       | `SYNTHETIC_API_KEY` / profile         |
| Venice                 | `openai-completions`     | HTTP discovery                          | `VENICE_API_KEY` / profile            |
| Together AI            | `openai-completions`     | Hardcoded catalog                       | `TOGETHER_API_KEY` / profile          |
| HuggingFace            | `openai-completions`     | HTTP `/v1/models` or hardcoded catalog  | `HF_TOKEN` / profile                  |
| Qianfan (Baidu)        | `openai-completions`     | Hardcoded catalog                       | `QIANFAN_API_KEY` / profile           |
| NVIDIA NIM             | `openai-completions`     | Hardcoded catalog                       | `NVIDIA_API_KEY` / profile            |
| Cloudflare AI Gateway  | `anthropic-messages`     | Profile metadata                        | Profile / `CLOUDFLARE_AI_GATEWAY_API_KEY` |

#### Explicit Providers (from config)

Defined in `config.models.providers` with:
- `baseUrl`: API endpoint
- `api`: Protocol (`anthropic-messages`, `openai-completions`, `ollama`, `bedrock-converse-stream`)
- `apiKey`: Auth credential (direct value, env var name, or `${ENV_VAR}` syntax)
- `auth`: Auth mode (`api-key`, `aws-sdk`, `oauth`, `token`)
- `models`: Array of `ModelDefinitionConfig`

### Provider Merging

`ensureOpenClawModelsJson()` orchestrates:

1. Resolves implicit providers from environment/profiles
2. Merges with explicit providers from config (explicit wins for same provider key)
3. Merges model lists: explicit models take precedence, implicit models fill gaps (by ID)
4. Normalizes providers: fixes `${ENV_VAR}` apiKey syntax, fills missing apiKeys from env/profiles
5. Normalizes Google model IDs (e.g., `gemini-3-pro` to `gemini-3-pro-preview`)
6. Writes `models.json` to `agentDir` (only if content changed)

---

## Model Selection and Compatibility

### model-selection.ts

#### Key Functions

```typescript
function parseModelRef(raw: string, defaultProvider: string): ModelRef | null
```
Parses `"provider/model"` or bare `"model"` strings into `{ provider, model }`. Applies provider normalization (e.g., `z.ai` to `zai`, `qwen` to `qwen-portal`) and model ID normalization (Anthropic aliases like `opus-4.6` to `claude-opus-4-6`).

```typescript
function resolveConfiguredModelRef(params: {
  cfg: OpenClawConfig;
  defaultProvider: string;
  defaultModel: string;
}): ModelRef
```
Resolves the configured default model from `cfg.agents.defaults.model.primary`, supporting alias lookup via `buildModelAliasIndex()`.

```typescript
function buildAllowedModelSet(params): {
  allowAny: boolean;
  allowedCatalog: ModelCatalogEntry[];
  allowedKeys: Set<string>;
}
```
Computes the set of allowed models from config's `agents.defaults.models` allowlist. If empty, all models are allowed.

### model-compat.ts

`normalizeModelCompat()` normalizes model metadata across providers, ensuring consistent `contextWindow`, `maxTokens`, `input`, `reasoning`, and cost fields.

### model-forward-compat.ts

Handles forward-compatibility for newer model IDs that don't yet exist in the SDK's built-in model list. Creates synthetic model objects from existing templates:

- OpenAI Codex: `gpt-5.3-codex` from `gpt-5.2-codex` template
- Anthropic: `claude-opus-4-6` from `claude-opus-4-5` template
- Z.AI: `glm-5` from `glm-4.7` template
- Antigravity: Various opus-4-6 variants from opus-4-5 templates

### model-fallback.ts

Implements the fallback chain for model failures. When a `FailoverError` is thrown:

1. Reads the configured fallback list from `config.agents.defaults.model.fallbacks`
2. Filters candidates against the configured allowlist
3. Tries each candidate in order, skipping auth profiles in cooldown
4. Propagates image-specific fallback candidates for image-size errors
5. Aborts on `AbortError` (user cancellation), retries on `FailoverError`/timeouts

### model-auth.ts

```typescript
type ResolvedProviderAuth = {
  apiKey?: string;
  profileId?: string;
  source: string;
  mode: "api-key" | "oauth" | "token" | "aws-sdk";
};
```

`getApiKeyForModel()` resolves credentials through a priority chain:
1. Explicit `profileId` parameter
2. `auth: "aws-sdk"` override in provider config
3. Auth profile store (ordered by `resolveAuthProfileOrder()`)
4. Environment variables (provider-specific env var map)
5. Custom provider API key from `models.json`
6. AWS SDK default chain (for Bedrock)

---

## Auth Profiles

**Directory:** `auth-profiles/`

### File Structure

| File                    | Purpose                                                        |
|-------------------------|----------------------------------------------------------------|
| `types.ts`              | Type definitions for credentials and store                     |
| `profiles.ts`           | Core CRUD operations on the profile store                      |
| `store.ts`              | File persistence (read/write `auth-profiles.json`)             |
| `order.ts`              | Profile ordering/rotation logic                                |
| `oauth.ts`              | OAuth flow implementation (token refresh, exchange)            |
| `session-override.ts`   | Per-session auth profile overrides                             |
| `usage.ts`              | Usage tracking (`markAuthProfileUsed`, `markAuthProfileGood`)  |
| `repair.ts`             | Profile ID migration and repair                                |
| `doctor.ts`             | Diagnostics (validate profiles, check connectivity)            |
| `display.ts`            | Human-readable profile display formatting                      |
| `paths.ts`              | File path resolution for auth store                            |
| `constants.ts`          | Cooldown durations, retry limits                               |
| `external-cli-sync.ts`  | Sync profiles from external CLI tools                          |

### Credential Types

```typescript
type ApiKeyCredential = {
  type: "api_key";
  provider: string;
  key?: string;
  email?: string;
  metadata?: Record<string, string>;  // e.g., accountId, gatewayId
};

type TokenCredential = {
  type: "token";
  provider: string;
  token: string;
  expires?: number;
  email?: string;
};

type OAuthCredential = OAuthCredentials & {
  type: "oauth";
  provider: string;
  clientId?: string;
  email?: string;
};

type AuthProfileStore = {
  version: number;
  profiles: Record<string, AuthProfileCredential>;
  order?: Record<string, string[]>;        // per-agent profile order overrides
  lastGood?: Record<string, string>;       // last successful profile per provider
  usageStats?: Record<string, ProfileUsageStats>;
};

type ProfileUsageStats = {
  lastUsed?: number;
  cooldownUntil?: number;
  disabledUntil?: number;
  disabledReason?: AuthProfileFailureReason;
  errorCount?: number;
  failureCounts?: Partial<Record<AuthProfileFailureReason, number>>;
  lastFailureAt?: number;
};
```

### Failure Handling

`markAuthProfileFailure()` records failures with reasons (`"auth"`, `"format"`, `"rate_limit"`, `"billing"`, `"timeout"`, `"unknown"`) and puts profiles into cooldown. `isProfileInCooldown()` checks the cooldown timestamp before profile selection.

---

## Bash Tool System

### File Structure

| File                        | Purpose                                                       |
|-----------------------------|---------------------------------------------------------------|
| `bash-tools.ts`             | Tool definition (`createExecTool`, `createProcessTool`)       |
| `bash-tools.exec.ts`        | Command execution implementation                              |
| `bash-tools.process.ts`     | Background process management (list, kill, send-keys)         |
| `bash-tools.exec-runtime.ts`| Execution runtime (PTY management, timeout, output capture)   |
| `bash-tools.shared.ts`      | Shared utilities                                              |
| `bash-process-registry.ts`  | Global process tracking registry                              |

### Exec Tool

Created via `createExecTool()` with extensive configuration:

- `host`: Execution host config
- `security`: Security policy for command approval
- `ask`: Whether to ask for user approval before execution
- `node`: Node.js-specific execution config
- `pathPrepend`: PATH prepend entries
- `safeBins`: Allowlisted safe binaries
- `cwd`: Working directory
- `allowBackground`: Whether the `process` tool is allowed
- `scopeKey`: Process isolation scope (session or agent level)
- `backgroundMs`: Default background timeout
- `timeoutSec`: Command timeout
- `sandbox`: Docker sandbox configuration (container name, workspace dir, workdir, env)

### Process Tool

Created via `createProcessTool()` with scope-based isolation:

- Lists, kills, and sends keystrokes to background processes
- Scoped by `scopeKey` to prevent cross-session process visibility
- Cleanup timeout configurable via `cleanupMs`

---

## Session Management

### Session Preparation (pi-embedded-runner/session-manager-init.ts)

`prepareSessionManagerForRun()` handles a quirk in pi-coding-agent's `SessionManager`:

- If the session file exists but has no assistant message, SessionManager marks `flushed=true` and will never persist the initial user message
- The fix: reset the file to empty so the first flush includes header + user + assistant in order
- For new files: sets `sessionId` and `cwd` on the header entry

### Session Write Lock (session-write-lock.ts)

`acquireSessionWriteLock()` ensures exclusive write access to a session file during a run, preventing concurrent runs from corrupting the session transcript.

### Compaction (pi-embedded-runner/compact.ts)

```typescript
async function compactEmbeddedPiSessionDirect(
  params: CompactEmbeddedPiSessionParams,
): Promise<EmbeddedPiCompactResult>
```

Compaction reduces session context when it approaches the model's context window limit:

1. Resolves model, auth, workspace, sandbox (same as a normal run)
2. Opens the session file, sanitizes history
3. Calls `session.compact(customInstructions)` which summarizes early turns
4. Runs `before_compaction` and `after_compaction` plugin hooks
5. Returns `{ ok, compacted, result: { summary, firstKeptEntryId, tokensBefore, tokensAfter } }`

The `compactEmbeddedPiSession()` wrapper adds lane queueing. `compactEmbeddedPiSessionDirect()` is used when already inside a lane (from the overflow-compaction path in `runEmbeddedPiAgent`).

Compaction triggers:
- `"overflow"`: Context overflow detected during prompt submission
- `"manual"`: User-initiated `/compact` command
- `"cache_ttl"`: Automatic pruning based on cache TTL strategy
- `"safeguard"`: Preventive compaction before reaching limits

Diagnostic logging includes pre/post message metrics (message count, text chars, tool result chars, estimated tokens, top contributors by size).

### Transcript Repair (session-transcript-repair.ts)

Handles various transcript corruption scenarios:

- `extractToolCallsFromAssistant()`: Extracts tool call IDs/names from assistant messages
- `sanitizeToolCallInputs()`: Cleans malformed tool call inputs in assistant messages
- `sanitizeToolUseResultPairing()`: Ensures every `toolUse` block has a matching `toolResult`, and vice versa
- `makeMissingToolResult()`: Creates synthetic `"[Tool result not available]"` messages

---

## Sandbox

**Directory:** `sandbox/`

### File Structure

| File               | Purpose                                                              |
|--------------------|----------------------------------------------------------------------|
| `context.ts`       | `resolveSandboxContext()` -- main entry point                        |
| `config.ts`        | `resolveSandboxConfigForAgent()` -- config resolution                |
| `docker.ts`        | Docker container creation/management                                 |
| `workspace.ts`     | Sandbox workspace initialization (copy bootstrap, sync skills)       |
| `fs-bridge.ts`     | Filesystem bridge for read/write/edit tools in sandbox               |
| `browser.ts`       | Browser container management (CDP, VNC, noVNC)                       |
| `browser-bridges.ts`| Browser bridge URL resolution                                       |
| `tool-policy.ts`   | Sandbox-specific tool allowlist                                      |
| `registry.ts`      | Container registry/tracking                                          |
| `prune.ts`         | Idle/old container cleanup                                           |
| `manage.ts`        | Container lifecycle management                                       |
| `runtime-status.ts`| Sandbox runtime status resolution                                    |
| `shared.ts`        | Shared utilities (scope key, workspace dir)                          |
| `config-hash.ts`   | Config hashing for container reuse decisions                         |
| `constants.ts`     | Default values                                                       |
| `types.ts`         | Type definitions                                                     |
| `types.docker.ts`  | Docker-specific types                                                |

### resolveSandboxContext()

```typescript
async function resolveSandboxContext(params: {
  config?: OpenClawConfig;
  sessionKey?: string;
  workspaceDir?: string;
}): Promise<SandboxContext | null>
```

Returns `null` when sandboxing is disabled. Otherwise:

1. Checks `resolveSandboxRuntimeStatus()` -- sandbox mode from config
2. Resolves config via `resolveSandboxConfigForAgent()` (agent-specific overrides on global)
3. Prunes stale containers (`maybePruneSandboxes`)
4. Creates/resolves workspace directory (syncs skills to sandbox workspace)
5. Ensures Docker container (`ensureSandboxContainer`)
6. Optionally provisions a browser container (`ensureSandboxBrowser`)
7. Creates filesystem bridge (`createSandboxFsBridge`)

### SandboxContext Return

```typescript
type SandboxContext = {
  enabled: true;
  sessionKey: string;
  workspaceDir: string;
  agentWorkspaceDir: string;
  workspaceAccess: "none" | "ro" | "rw";
  containerName: string;
  containerWorkdir: string;
  docker: SandboxDockerConfig;
  tools: SandboxToolPolicy;
  browserAllowHostControl: boolean;
  browser?: { bridgeUrl; noVncUrl };
  fsBridge?: SandboxFsBridge;
};
```

### Docker Config Resolution

`resolveSandboxDockerConfig()` merges global and agent-specific Docker settings:

```typescript
type SandboxDockerConfig = {
  image: string;               // default: DEFAULT_SANDBOX_IMAGE
  containerPrefix: string;
  workdir: string;             // default: DEFAULT_SANDBOX_WORKDIR
  readOnlyRoot: boolean;       // default: true
  tmpfs: string[];             // default: ["/tmp", "/var/tmp", "/run"]
  network: string;             // default: "none"
  user?: string;
  capDrop: string[];           // default: ["ALL"]
  env: Record<string, string>; // default: { LANG: "C.UTF-8" }
  setupCommand?: string;
  pidsLimit?: number;
  memory?: string;
  memorySwap?: string;
  cpus?: number;
  ulimits?: Record<string, unknown>;
  seccompProfile?: string;
  apparmorProfile?: string;
  dns?: string[];
  extraHosts?: string[];
  binds?: string[];
};
```

### Sandbox Scope

```typescript
type SandboxScope = "session" | "agent" | "shared";
```

- `"session"`: One container per session key
- `"agent"`: One container per agent ID
- `"shared"`: Single container across all sessions

---

## System Prompt Building

**File:** `pi-embedded-runner/system-prompt.ts` and `system-prompt.ts`

### Prompt Modes

```typescript
type PromptMode = "full" | "minimal" | "none";
```

- `"full"`: All sections (main agent)
- `"minimal"`: Reduced sections for subagents (Tooling, Workspace, Runtime only)
- `"none"`: Basic identity line only

### Components

The system prompt is built by `buildEmbeddedSystemPrompt()` from these components:

1. **Base Instructions**: Agent identity, capabilities, behavioral guidelines
2. **Tool Descriptions**: Auto-generated from the tool list, including parameter schemas
3. **Skill Info**: Available skills with descriptions and locations (`## Skills (mandatory)`)
4. **Channel Capabilities**: Telegram inline buttons, Slack threading, etc.
5. **TTS Hints**: Text-to-speech guidance when TTS is enabled
6. **Thinking Hints**: Reasoning tag format guidance for providers that use `<think>` tags
7. **Message Tool Hints**: Channel-specific messaging guidance (actions, formatting)
8. **Bootstrap Context**: Workspace context files (`.openclaw`, `OPENCLAW.md`, etc.)
9. **Sandbox Info**: Sandbox status, workspace mount, browser availability
10. **Custom Overrides**: `extraSystemPrompt` from params
11. **Runtime Info**: Host, OS, arch, Node version, model, shell, channel, capabilities
12. **User Time/Timezone**: Current time and timezone for the user
13. **Model Alias Lines**: Available model aliases for `/model` switching
14. **Memory Section**: Memory tool usage guidance (when memory tools available)
15. **Docs Path**: Path to OpenClaw documentation
16. **Heartbeat Prompt**: Keep-alive guidance for default agent
17. **Reaction Guidance**: Emoji reaction level guidance (Telegram/Signal)
18. **Context Files**: Injected context files from hooks
19. **Memory Citations Mode**: How to cite memory sources

### System Prompt Report

`buildSystemPromptReport()` generates a diagnostic report of the system prompt composition for debugging:

```typescript
type SessionSystemPromptReport = {
  source: "run";
  generatedAt: number;
  sessionId: string;
  sessionKey?: string;
  provider: string;
  model: string;
  workspaceDir: string;
  bootstrapMaxChars: number;
  sandbox: { mode: string; sandboxed: boolean };
  systemPrompt: string;
  bootstrapFiles: BootstrapFile[];
  injectedFiles: EmbeddedContextFile[];
  skillsPrompt?: string;
  tools: AnyAgentTool[];
};
```

---

## Usage and Metrics

**File:** `usage.ts`

### UsageLike (Input Normalization)

```typescript
type UsageLike = {
  input?: number;
  output?: number;
  cacheRead?: number;
  cacheWrite?: number;
  total?: number;
  // Provider-specific alternates
  inputTokens?: number;
  outputTokens?: number;
  promptTokens?: number;
  completionTokens?: number;
  input_tokens?: number;
  output_tokens?: number;
  prompt_tokens?: number;
  completion_tokens?: number;
  cache_read_input_tokens?: number;
  cache_creation_input_tokens?: number;
  totalTokens?: number;
  total_tokens?: number;
  cache_read?: number;
  cache_write?: number;
};
```

### NormalizedUsage (Output)

```typescript
type NormalizedUsage = {
  input?: number;
  output?: number;
  cacheRead?: number;
  cacheWrite?: number;
  total?: number;
};
```

### normalizeUsage(raw)

Maps all provider-specific field names to the canonical `NormalizedUsage` format, filtering out non-finite numbers.

### UsageAccumulator (in run.ts)

Accumulates usage across all API calls in a run. Key insight: `lastCacheRead`/`lastCacheWrite`/`lastInput` track only the most recent call's values for context-window display, because summing `cacheRead` across N tool-call round-trips yields `N * context_size`, which overstates actual context usage.

---

## Complete Request-to-Response Flow

### Step 1: Incoming Request

A request arrives with: session identity, workspace, user prompt, images, config, channel info. This is packaged into `RunEmbeddedPiAgentParams` by the upstream handler (auto-reply, CLI, API endpoint).

### Step 2: Lane Queueing and Workspace Setup

`runEmbeddedPiAgent()` enqueues on session + global lanes, resolves workspace directory, validates the working directory exists.

### Step 3: Model and Provider Resolution

1. `ensureOpenClawModelsJson()` writes the merged provider config
2. `resolveModel()` looks up the model in the registry, with forward-compat fallback
3. `resolveContextWindowInfo()` + `evaluateContextWindowGuard()` validate context size
4. Provider-specific setup: Copilot token exchange, Ollama native streaming, etc.

### Step 4: Auth Profile Resolution

1. `ensureAuthProfileStore()` loads credentials from disk
2. `resolveAuthProfileOrder()` determines candidate order
3. First non-cooldown candidate is selected
4. `getApiKeyForModel()` resolves the API key (env, profile, or config)
5. Key is set on `authStorage` for the SDK

### Step 5: Execution Attempt (`runEmbeddedAttempt`)

Within `runEmbeddedAttempt()`:

1. **Sandbox Resolution**: `resolveSandboxContext()` provisions Docker container if needed
2. **Skill Loading**: `loadWorkspaceSkillEntries()` + `applySkillEnvOverrides()`
3. **Bootstrap Context**: `resolveBootstrapContextForRun()` loads workspace context files
4. **Tool Creation**: `createOpenClawCodingTools()` assembles and filters tools
5. **System Prompt**: `buildEmbeddedSystemPrompt()` constructs the full system prompt
6. **Session Setup**:
   - `repairSessionFileIfNeeded()` fixes corrupted session files
   - `SessionManager.open()` loads the session
   - `guardSessionManager()` wraps with tool result guard
   - `prepareSessionManagerForRun()` normalizes session state
   - `sanitizeSessionHistory()` repairs history for the current provider
   - `validateAnthropicTurns()` / `validateGeminiTurns()` provider-specific validation
   - `limitHistoryTurns()` truncates to configured DM history limit
   - `sanitizeToolUseResultPairing()` repairs orphaned tool results
7. **Agent Session**: `createAgentSession()` from pi-coding-agent SDK

### Step 6: Streaming Subscription

`subscribeEmbeddedPiSession()` sets up the event subscription with:
- Block reply chunking
- Reasoning mode handling
- Messaging tool deduplication
- Compaction coordination
- Usage accumulation

### Step 7: Prompt Submission

1. `before_agent_start` hooks run (may prepend context)
2. `detectAndLoadPromptImages()` scans for image references in the prompt
3. `injectHistoryImagesIntoMessages()` adds images to historical message positions
4. `activeSession.prompt(effectivePrompt, { images })` submits to the LLM

### Step 8: Streaming Execution

The pi-ai SDK streams events through the subscription:
- `message_start` / `message_update` / `message_end`: Assistant text chunks
- `tool_execution_start` / `tool_execution_update` / `tool_execution_end`: Tool calls
- `auto_compaction_start` / `auto_compaction_end`: Automatic compaction events
- `agent_start` / `agent_end`: Agent lifecycle events

During streaming:
- `onBlockReply` callbacks deliver text chunks to the user
- `onReasoningStream` callbacks stream thinking content
- `onToolResult` callbacks deliver tool execution summaries
- Tool results are persisted with size caps and missing-result synthesis

### Step 9: Post-Processing

1. `waitForCompactionRetry()` waits for any pending compaction to complete
2. Cache-TTL timestamp appended (if cache-ttl pruning mode)
3. `agent_end` hooks run (fire-and-forget)
4. Messages snapshot captured for error analysis
5. Subscription unsubscribed, abort controller cleaned up

### Step 10: Response Assembly (in `runEmbeddedPiAgent`)

1. **Usage normalization**: `toNormalizedUsage()` with last-call cache fields
2. **Error classification**: Context overflow, role ordering, image size, auth/billing/rate-limit
3. **Failover decisions**: Auth profile rotation, thinking level fallback, compaction retry
4. **Payload building**: `buildEmbeddedRunPayloads()` formats assistant texts and tool metas
5. **Profile bookkeeping**: `markAuthProfileGood()` + `markAuthProfileUsed()` on success
6. **Result return**: `EmbeddedPiRunResult` with payloads, meta, messaging tool info

The upstream handler then delivers the response through the appropriate channel (Telegram, Slack, Discord, WhatsApp, CLI, API, etc.), applying channel-specific formatting, TTS processing, and reply directive handling.
