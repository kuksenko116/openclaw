# Configuration System

## Overview

The configuration system lives in `src/config/` and is responsible for loading, validating, writing, and migrating the OpenClaw configuration file. The config file is stored as JSON5 and validated at runtime using Zod schemas. The system supports environment variable interpolation, config file includes, backward-compatible migrations, group-level policies, and atomic writes with full audit trails.

---

## Config Location and Precedence

Configuration values are resolved in this order (later sources override earlier ones):

1. **Built-in defaults** -- hardcoded in `defaults.ts`
2. **Config file** -- `~/.openclaw/openclaw.json` (JSON5 format)
3. **`$include` directives** -- merged from referenced YAML/JSON/JSON5 files
4. **`.env` files** -- loaded via `loadDotEnv()` from the dotenv module
5. **`config.env.vars`** -- inline env vars defined inside the config itself
6. **`process.env`** -- real environment variables override everything

The config file path can be overridden with:

- `$OPENCLAW_CONFIG_PATH` -- explicit path to the config file
- `$OPENCLAW_STATE_DIR` -- changes the state directory (config lives inside it)
- Default: `~/.openclaw/openclaw.json`

Legacy state directories (`.clawdbot`, `.moldbot`, `.moltbot`) are detected and used if the new `~/.openclaw` directory does not yet exist, as handled in `paths.ts`.

---

## Configuration System (`src/config/`)

### `config.ts` -- Main Barrel Export

`src/config/config.ts` is a re-export barrel that surfaces the public API of the config module:

```typescript
export {
  clearConfigCache,
  createConfigIO,
  loadConfig,
  parseConfigJson5,
  readConfigFileSnapshot,
  readConfigFileSnapshotForWrite,
  resolveConfigSnapshotHash,
  writeConfigFile,
} from "./io.js";
export { migrateLegacyConfig } from "./legacy-migrate.js";
export * from "./paths.js";
export * from "./runtime-overrides.js";
export * from "./types.js";
export {
  validateConfigObject,
  validateConfigObjectRaw,
  validateConfigObjectRawWithPlugins,
  validateConfigObjectWithPlugins,
} from "./validation.js";
export { OpenClawSchema } from "./zod-schema.js";
```

The actual loading logic (file I/O, JSON5 parsing, env var resolution, include merging, validation, default application) lives in `io.ts`.

### `io.ts` -- File I/O with Atomic Write and Audit Records (1,127 lines)

This is the core engine of the configuration system. Key responsibilities:

**Reading:**
- `loadConfig()` -- the primary entry point. Reads the config file, parses JSON5, resolves `$include` directives, substitutes `${ENV_VAR}` references, applies env vars from `config.env`, runs Zod validation, applies defaults (model, agent, session, compaction, logging, message, talk API key), normalizes paths, and warns on miskeys or future-version configs.
- `readConfigFileSnapshot()` -- returns a `ConfigFileSnapshot` containing the raw text, parsed object, validated config, and SHA-256 hash. Used for non-mutating reads.
- `readConfigFileSnapshotForWrite()` -- like `readConfigFileSnapshot` but also captures an `envSnapshotForRestore` for later `${VAR}` reference restoration during writes.
- `parseConfigJson5()` -- thin wrapper around `JSON5.parse` that returns a result type instead of throwing.

**Writing:**
- `writeConfigFile()` -- the central write path. Implements atomic write via rename with a copy-fallback strategy:
  1. Stamps the config with `meta.lastTouchedVersion` and `meta.lastTouchedAt`.
  2. Restores `${VAR}` environment variable references that were resolved during read (using the env snapshot).
  3. Writes to a temp file, then atomically renames it over the target path.
  4. If rename fails (cross-device), falls back to copy + unlink.
  5. Rotates config backups via `backup-rotation.ts`.
  6. Appends a `ConfigWriteAuditRecord` to `~/.openclaw/logs/config-audit.jsonl`.

**Audit Records:**

Every config write produces a JSONL audit record with:

```typescript
type ConfigWriteAuditRecord = {
  ts: string;                    // ISO timestamp
  source: "config-io";
  event: "config.write";
  result: "rename" | "copy-fallback" | "failed";
  configPath: string;
  pid: number;                   // process ID
  ppid: number;                  // parent process ID
  cwd: string;                   // working directory
  argv: string[];                // command-line arguments
  execArgv: string[];            // Node.js exec arguments
  watchMode: boolean;            // whether running in watch mode
  watchSession: string | null;
  watchCommand: string | null;
  existsBefore: boolean;         // whether file existed before write
  previousHash: string | null;   // SHA-256 of previous content
  nextHash: string | null;       // SHA-256 of new content
  previousBytes: number | null;
  nextBytes: number | null;
  changedPathCount: number | null;
  hasMetaBefore: boolean;
  hasMetaAfter: boolean;
  gatewayModeBefore: string | null;
  gatewayModeAfter: string | null;
  suspicious: string[];          // flags like "size-drop", "missing-meta-before-write"
  errorCode?: string;
  errorMessage?: string;
};
```

Suspicious flags are computed by `resolveConfigWriteSuspiciousReasons()` and include:
- `size-drop:N->M` -- config shrank by more than 50% (when previous was >= 512 bytes)
- `missing-meta-before-write` -- the config lacked a `meta` block before the write
- `gateway-mode-removed` -- the `gateway.mode` field was present before but absent after

**Dependency Injection:**

`createConfigIO()` accepts a `ConfigIoDeps` object for testability, allowing injection of custom `fs`, `json5`, `env`, `homedir`, `configPath`, and `logger` implementations.

### `validation.ts` -- Config Validation with Plugins (438 lines)

Provides four validation functions:

- `validateConfigObjectRaw(raw)` -- validates without applying runtime defaults. First checks for legacy config issues; if any are found, returns them as validation failures. Then runs `OpenClawSchema.safeParse(raw)` and checks for duplicate agent directories.
- `validateConfigObject(raw)` -- validates and applies runtime defaults (model defaults, agent defaults, session defaults).
- `validateConfigObjectRawWithPlugins(raw)` -- like `validateConfigObjectRaw` but also validates plugin config sections against their JSON Schema manifests.
- `validateConfigObjectWithPlugins(raw)` -- validates with plugins and applies runtime defaults.

Additional validations:
- **Identity avatar paths** -- must be workspace-relative paths, http(s) URLs, or data URIs. Absolute paths, `~`-prefixed paths, and paths outside the workspace are rejected.
- **Channel normalization** -- channel IDs are normalized via `normalizeChatChannelId`.
- **Plugin config validation** -- each plugin's config section is validated against its manifest JSON Schema using `validateJsonSchemaValue`.
- **Duplicate agent directories** -- `findDuplicateAgentDirs` rejects configs where multiple agents share the same workspace directory.

### `schema.ts` -- Schema Generation (367 lines)

Converts the Zod schema into a JSON Schema representation for use by the Control UI and other consumers. Key exports:

- `ConfigSchemaResponse` -- the response type containing `schema`, `uiHints`, `version`, and `generatedAt`.
- Schema merging logic (`mergeObjectSchema`) for combining base schemas with plugin/channel extensions.
- Plugin and channel UI metadata integration (`PluginUiMetadata`, `ChannelUiMetadata`).
- Extension hint collection to track which UI hints belong to plugin/channel config sections.

### `schema.hints.ts` -- UI Hints for Config Fields

Defines `ConfigUiHint` and `ConfigUiHints` types that provide UI metadata for each config field:

```typescript
type ConfigUiHint = {
  label?: string;       // display name
  help?: string;        // tooltip/description
  group?: string;       // UI grouping category
  order?: number;       // sort order within group
  advanced?: boolean;   // hide in simplified views
  sensitive?: boolean;  // mask in UI
  placeholder?: string; // input placeholder
  itemTemplate?: unknown;
};
```

Groups include: Wizard, Update, Diagnostics, Logging, Gateway, Node Host, Agents, Tools, Bindings, Audio, Models, Messages, Commands, Session, Cron, Hooks, UI, Browser, Talk, Channels, Skills, Plugins, Discovery, Presence, Voice Wake. Each group has an assigned display order.

Sensitive fields are identified via `mapSensitivePaths()` which walks the Zod schema looking for `sensitive()` markers and maps them to config paths.

### `schema.labels.ts` -- Human-Readable Field Labels

A flat `Record<string, string>` mapping dotted config paths to display labels. For example:

```typescript
"gateway.auth.token": "Gateway Token",
"tools.media.image.enabled": "Enable Image Understanding",
"agents.list.*.identity.avatar": "Identity Avatar",
```

Used by the Control UI to render friendly labels for each config field.

### `schema.help.ts` -- Contextual Help Text

A flat `Record<string, string>` mapping config paths to help/description text. For example:

```typescript
"gateway.auth.token": "Required by default for gateway access ...",
"gateway.reload.mode": 'Hot reload strategy for config changes ("hybrid" recommended).',
"discovery.mdns.mode": 'mDNS broadcast mode ("minimal" default, "full" includes cliPath/sshPort, "off" disables mDNS).',
```

---

## Zod Schema Definitions

The Zod schema is the single source of truth for config validation. It is split across multiple files for maintainability:

### `zod-schema.ts` -- Master Schema (666 lines)

Assembles the top-level `OpenClawSchema` by importing and composing sub-schemas:

```typescript
import { ToolsSchema } from "./zod-schema.agent-runtime.js";
import { AgentsSchema, AudioSchema, BindingsSchema, BroadcastSchema } from "./zod-schema.agents.js";
import { ApprovalsSchema } from "./zod-schema.approvals.js";
import { HexColorSchema, ModelsConfigSchema } from "./zod-schema.core.js";
import { HookMappingSchema, HooksGmailSchema, InternalHooksSchema } from "./zod-schema.hooks.js";
import { ChannelsSchema } from "./zod-schema.providers.js";
import { CommandsSchema, MessagesSchema, SessionSchema, SessionSendPolicySchema } from "./zod-schema.session.js";
```

Also defines schemas for: `BrowserSnapshotDefaults`, `NodeHost`, `MemoryQmd` (paths, sessions, update, embeddings), `CronJob`, `Discovery` (mDNS), `CanvasHost`, `Talk`, `GatewayConfig`, `DiagnosticsConfig`, `LoggingConfig`, `UpdateConfig`, `UiConfig`, `SkillsConfig`, `PluginsConfig`, and `WebConfig`.

### `zod-schema.core.ts` -- Core Definitions (492 lines)

Foundational schemas used by multiple other files:

- `ModelApiSchema` -- union of supported API types: `"openai-completions"`, `"openai-responses"`, `"anthropic-messages"`, `"google-generative-ai"`, `"github-copilot"`, `"bedrock-converse-stream"`, `"ollama"`.
- `ModelCompatSchema` -- model compatibility flags (supportsStore, supportsDeveloperRole, supportsReasoningEffort, thinkingFormat, etc.).
- `ModelDefinitionSchema` -- individual model definition (id, name, api, reasoning, input modalities, cost, contextWindow, maxTokens, headers, compat).
- `ModelProviderSchema` -- provider configuration (baseUrl, apiKey, headers, models, etc.).
- `HexColorSchema` -- hex color validation.
- `ModelsConfigSchema` -- top-level models configuration.
- `AllowDenyChannelRulesSchema` -- reusable allow/deny rule schemas (imported from `zod-schema.allowdeny.ts`).

### `zod-schema.agent-defaults.ts` -- Agent Defaults (175 lines)

Schemas for the `agents.defaults` section: default workspace, model preferences, context window sizes, and per-agent override patterns.

### `zod-schema.agent-runtime.ts` -- Agent Runtime / Tools (575 lines)

The `ToolsSchema` covering:
- Tool execution policies (`exec`), including shell, sandbox, Docker, and approval settings.
- Media understanding (image, audio, video) with per-modality config.
- Web search tools and provider selection.
- MCP (Model Context Protocol) server configurations.
- Apply-patch tool for OpenAI models.
- `alsoAllow` for additional tool patterns.

### `zod-schema.providers-core.ts` -- Provider Schemas (979 lines)

Schemas for all messaging channel providers:
- WhatsApp (Web, Business Cloud, Baileys bridge)
- Telegram (bot token, webhook, custom commands, group policies)
- Discord (bot token, guild management, presence, thread policies)
- Slack (bot/app tokens, workspace config)
- Signal (CLI path, database, phone number)
- iMessage (AppleScript bridge, database path)
- IRC (server, port, TLS, channels, SASL)
- Microsoft Teams
- Google Chat
- Matrix (homeserver, credentials)
- Generic HTTP channels

Each provider schema defines its authentication, connection settings, group/DM policies, and channel-specific features.

### `zod-schema.hooks.ts` -- Hooks Schema (165 lines)

Schemas for the hooks system: `HookMappingSchema` (event-to-handler mappings), `HooksGmailSchema` (Gmail integration hooks), and `InternalHooksSchema` (internal lifecycle hooks).

### `zod-schema.session.ts` -- Session Schema (138 lines)

Schemas for session management: `SessionSchema` (TTL, max tokens, compaction), `SessionSendPolicySchema` (who can send messages), `MessagesSchema` (message formatting and limits), and `CommandsSchema` (custom slash commands).

### Supporting Schema Files

- `zod-schema.agents.ts` -- Agent list, bindings, broadcast, and audio schemas.
- `zod-schema.providers.ts` -- Composes all provider schemas into `ChannelsSchema`.
- `zod-schema.providers-whatsapp.ts` -- WhatsApp-specific schema details.
- `zod-schema.allowdeny.ts` -- Reusable allow/deny list schemas for channel rules.
- `zod-schema.approvals.ts` -- Execution approval configuration schemas.
- `zod-schema.sensitive.ts` -- `sensitive()` helper that marks Zod schema nodes as containing sensitive data, used by `schema.hints.ts` for UI masking.

---

## Configuration Features

### `env-substitution.ts` -- `${ENV_VAR}` Interpolation (170 lines)

Resolves `${VAR_NAME}` patterns in string values at config load time:

- Only uppercase env var names are matched: pattern `[A-Z_][A-Z0-9_]*`.
- Escape with `$${}` to output a literal `${}` in the config value.
- Missing env vars throw `MissingEnvVarError` with the var name and config path for debuggability.
- Substitution walks the entire config tree recursively, substituting strings in objects and arrays.

```json5
{
  models: {
    providers: {
      "vercel-gateway": {
        apiKey: "${VERCEL_GATEWAY_API_KEY}"
      }
    }
  }
}
```

### `env-preserve.ts` -- Preserving `${VAR}` References During Write-Back (141 lines)

When config is read, `${VAR}` references are resolved to their runtime values. When writing back, this module detects values that match what a `${VAR}` reference would resolve to and restores the original `${VAR}` reference so env var references survive config round-trips.

A value is restored only if:
1. The pre-substitution config contained a `${VAR}` pattern at that path.
2. Resolving that pattern with the current env produces exactly the incoming value.

If a caller intentionally set a new value (different from what the env var resolves to), the new value is kept as-is. This logic is integrated into `writeConfigFile()` in `io.ts` via the `envSnapshotForRestore` captured at read time.

### `includes.ts` -- Config File Includes/Merging (241 lines)

Supports the `$include` directive for modular config composition:

```json5
{
  "$include": "./base.json5",             // single file
  "$include": ["./a.json5", "./b.json5"]  // merge multiple files
}
```

Key behaviors:
- Maximum include depth: 10 (prevents runaway nesting).
- Circular includes are detected and throw `CircularIncludeError` with the full chain.
- Deep merge semantics: arrays concatenate, objects merge recursively, primitives: source wins.
- Paths are resolved relative to the including file.
- Custom resolver interface (`IncludeResolver`) allows injection of `readFile` and `parseJson` for testing.

### `legacy-migrate.ts` + Legacy Migration Files

Legacy migration handles backward compatibility across OpenClaw versions:

- `legacy-migrate.ts` (19 lines) -- entry point: calls `applyLegacyMigrations(raw)` and then validates the result.
- `legacy.ts` -- detects legacy config issues via `findLegacyConfigIssues()`.
- `legacy.migrations.ts` -- orchestrates all migration passes.
- `legacy.migrations.part-1.ts`, `legacy.migrations.part-2.ts`, `legacy.migrations.part-3.ts` -- migration rules split across multiple files for manageability.
- `legacy.shared.ts` -- shared utilities for migration logic.
- `legacy.rules.ts` -- individual migration rule definitions.

The migration system returns `{ next, changes }` where `next` is the migrated config and `changes` is a list of human-readable descriptions of what was changed.

### `defaults.ts` -- Applying Configuration Defaults (470 lines)

Applies runtime defaults to a validated config. Individual functions:

- `applyModelDefaults()` -- sets default model aliases (opus -> `anthropic/claude-opus-4-6`, sonnet -> `anthropic/claude-sonnet-4-5`, gpt -> `openai/gpt-5.2`, gemini -> `google/gemini-3-pro-preview`), default model costs, input modalities, and max tokens.
- `applyAgentDefaults()` -- sets default agent concurrency limits (`DEFAULT_AGENT_MAX_CONCURRENT`, `DEFAULT_SUBAGENT_MAX_CONCURRENT`), context window tokens.
- `applySessionDefaults()` -- session TTL and compaction defaults.
- `applyCompactionDefaults()` -- context compaction settings.
- `applyContextPruningDefaults()` -- context pruning thresholds.
- `applyLoggingDefaults()` -- logging configuration defaults.
- `applyMessageDefaults()` -- message formatting defaults.
- `applyTalkApiKey()` -- resolves the Talk API key via `resolveTalkApiKey()`.
- `resolveAnthropicDefaultAuthMode()` -- determines whether to default to API key or OAuth based on auth profile configuration.

### `paths.ts` -- Config Path Resolution (275 lines)

Resolves file system paths for config and state directories:

- `resolveConfigPath(env, stateDir)` -- resolves the config file path. Checks `$OPENCLAW_CONFIG_PATH` first, then falls back to `stateDir/openclaw.json`.
- `resolveStateDir(env, homedir)` -- resolves the state directory. Checks `$OPENCLAW_STATE_DIR` / `$CLAWDBOT_STATE_DIR`, then `~/.openclaw`, then legacy dirs (`.clawdbot`, `.moldbot`, `.moltbot`).
- `resolveDefaultConfigCandidates()` -- returns candidate config file paths in priority order.
- `resolveIsNixMode()` -- detects when running under Nix (`OPENCLAW_NIX_MODE=1`), which disables auto-install flows and makes config read-only.
- Legacy state directory detection for migration from previous project names.

### `normalize-paths.ts` -- Path Normalization (69 lines)

Walks the config tree and normalizes tilde-prefixed paths (`~/...`) to absolute paths. Only normalizes values under keys matching `/(dir|path|paths|file|root|workspace)$/i` or the special list keys `paths` and `pathPrepend`. This ensures that relative paths in the config file work regardless of the current working directory.

### `plugin-auto-enable.ts` -- Auto-Enabling Plugins Based on Config (471 lines)

Scans the config to detect when a channel or provider is configured and automatically enables the corresponding plugin. For example:

- If Telegram credentials are present, enables the `telegram` channel plugin.
- If Google Antigravity auth is configured, enables `google-antigravity-auth`.
- Provider plugins like `copilot-proxy`, `qwen-portal-auth`, `minimax-portal-auth` are enabled when their provider config exists.

Returns `PluginAutoEnableResult` with the modified config and a list of human-readable change descriptions.

### `group-policy.ts` -- Group-Level Policies (238 lines)

Resolves per-group (chat room/channel) policies for each messaging channel:

```typescript
type ChannelGroupConfig = {
  requireMention?: boolean;     // whether the bot must be @mentioned
  tools?: GroupToolPolicyConfig; // tool allow/deny rules for this group
  toolsBySender?: GroupToolPolicyBySenderConfig; // per-sender tool policies
};

type ChannelGroupPolicy = {
  allowlistEnabled: boolean;    // whether an allowlist is active
  allowed: boolean;             // whether this group is allowed
  groupConfig?: ChannelGroupConfig;
  defaultConfig?: ChannelGroupConfig;
};
```

Supports:
- Wildcard groups (`*`) as the default policy.
- Case-insensitive group ID matching.
- Per-sender tool policies resolved via `resolveToolsBySender()`, which normalizes sender identifiers (removing `@` prefixes, lowercasing).

### `merge-patch.ts` -- RFC 7396 Merge Patch (26 lines)

Implements the RFC 7396 JSON Merge Patch algorithm for applying partial config updates:

```typescript
function applyMergePatch(base: unknown, patch: unknown): unknown
```

- `null` values in the patch delete the corresponding key.
- Object values are recursively merged.
- All other values replace the base.

Used internally by `io.ts` when computing minimal diffs between config versions.

---

## Type System

### `types.ts` -- Master Type Barrel (33 lines)

Re-exports all type definition files:

```typescript
export * from "./types.agent-defaults.js";
export * from "./types.agents.js";
export * from "./types.approvals.js";
export * from "./types.auth.js";
export * from "./types.base.js";
export * from "./types.browser.js";
export * from "./types.channels.js";
export * from "./types.openclaw.js";
export * from "./types.cron.js";
export * from "./types.discord.js";
export * from "./types.googlechat.js";
export * from "./types.gateway.js";
export * from "./types.hooks.js";
export * from "./types.imessage.js";
export * from "./types.irc.js";
export * from "./types.messages.js";
export * from "./types.models.js";
export * from "./types.node-host.js";
export * from "./types.msteams.js";
export * from "./types.plugins.js";
export * from "./types.queue.js";
export * from "./types.sandbox.js";
export * from "./types.signal.js";
export * from "./types.skills.js";
export * from "./types.slack.js";
export * from "./types.telegram.js";
export * from "./types.tts.js";
export * from "./types.tools.js";
export * from "./types.whatsapp.js";
export * from "./types.memory.js";
```

### `types.openclaw.ts` -- The Master Config Type

Defines `OpenClawConfig`, the top-level configuration interface:

```typescript
type OpenClawConfig = {
  meta?: { lastTouchedVersion?: string; lastTouchedAt?: string };
  auth?: AuthConfig;
  env?: { shellEnv?: { enabled?: boolean; timeoutMs?: number }; vars?: Record<string, string>; ... };
  wizard?: { lastRunAt?: string; lastRunVersion?: string; ... };
  diagnostics?: DiagnosticsConfig;
  logging?: LoggingConfig;
  update?: { channel?: "stable" | "beta" | "dev"; checkOnStart?: boolean };
  browser?: BrowserConfig;
  ui?: { seamColor?: string; assistant?: { name?: string; avatar?: string } };
  skills?: SkillsConfig;
  plugins?: PluginsConfig;
  models?: ModelsConfig;
  nodeHost?: NodeHostConfig;
  agents?: AgentsConfig;
  tools?: ToolsConfig;
  bindings?: AgentBinding[];
  broadcast?: BroadcastConfig;
  audio?: AudioConfig;
  messages?: MessagesConfig;
  commands?: CommandsConfig;
  approvals?: ApprovalsConfig;
  session?: SessionConfig;
  web?: WebConfig;
  channels?: ChannelsConfig;
  cron?: CronConfig;
  hooks?: HooksConfig;
  discovery?: DiscoveryConfig;
  canvasHost?: CanvasHostConfig;
  talk?: TalkConfig;
  gateway?: GatewayConfig;
  memory?: MemoryConfig;
};
```

Also defines `ConfigValidationIssue` (path + message) and `LegacyConfigIssue` types.

### Domain-Specific Type Files

Each `types.*.ts` file defines the TypeScript interfaces for one config domain:

| File | Key Types |
|------|-----------|
| `types.agents.ts` | `AgentsConfig`, `AgentEntry`, `AgentBinding` |
| `types.auth.ts` | `AuthConfig`, `AuthProfile`, `AuthOrder` |
| `types.base.ts` | `DiagnosticsConfig`, `LoggingConfig`, `SessionConfig`, `WebConfig` |
| `types.browser.ts` | `BrowserConfig` |
| `types.channels.ts` | `ChannelsConfig` (union of all channel configs) |
| `types.cron.ts` | `CronConfig`, `CronJob` |
| `types.discord.ts` | Discord-specific channel config |
| `types.gateway.ts` | `GatewayConfig`, `DiscoveryConfig`, `TalkConfig`, `CanvasHostConfig` |
| `types.hooks.ts` | `HooksConfig`, hook mappings |
| `types.imessage.ts` | iMessage channel config |
| `types.irc.ts` | IRC channel config |
| `types.memory.ts` | `MemoryConfig` |
| `types.messages.ts` | `MessagesConfig`, `AudioConfig`, `BroadcastConfig`, `CommandsConfig` |
| `types.models.ts` | `ModelsConfig`, `ModelDefinitionConfig`, `ModelProviderConfig` |
| `types.msteams.ts` | Microsoft Teams config |
| `types.node-host.ts` | `NodeHostConfig` |
| `types.plugins.ts` | `PluginsConfig` |
| `types.queue.ts` | Queue-related types |
| `types.sandbox.ts` | Sandbox/Docker config |
| `types.signal.ts` | Signal channel config |
| `types.skills.ts` | `SkillsConfig` |
| `types.slack.ts` | Slack channel config |
| `types.telegram.ts` | Telegram channel config |
| `types.tools.ts` | `ToolsConfig`, `GroupToolPolicyConfig` |
| `types.tts.ts` | Text-to-speech config |
| `types.whatsapp.ts` | WhatsApp channel config |
| `types.agent-defaults.ts` | Agent default settings |
| `types.approvals.ts` | Execution approval settings |
| `types.googlechat.ts` | Google Chat config |

---

## Supporting Modules

### `backup-rotation.ts`

Rotates config file backups on each write to prevent data loss.

### `cache-utils.ts`

Caching utilities for config read results.

### `agent-dirs.ts`

Validates agent workspace directories, detects duplicates via `findDuplicateAgentDirs()`, and formats error messages with `formatDuplicateAgentDirError()`.

### `agent-limits.ts`

Constants for agent concurrency limits: `DEFAULT_AGENT_MAX_CONCURRENT` and `DEFAULT_SUBAGENT_MAX_CONCURRENT`.

### `channel-capabilities.ts`

Resolves channel capability metadata from the config.

### `commands.ts`

Config-level command definitions (custom slash commands).

### `env-vars.ts`

`applyConfigEnvVars()` -- applies `config.env.vars` to the process environment before `${VAR}` substitution runs.

### `runtime-overrides.ts`

`applyConfigOverrides()` -- applies runtime-only overrides that are not persisted to disk.

### `sessions/` subdirectory

Session-related config utilities including `sessions.ts` and session path resolution.

### `talk.ts`

`resolveTalkApiKey()` -- resolves the Talk (voice) API key from config or environment.

### `telegram-custom-commands.ts`

Telegram-specific custom command configuration handling.

### `version.ts`

`compareOpenClawVersions()` -- semver comparison used to detect when a config was written by a newer version.

### `includes-scan.ts`

Scanning for `$include` directives without resolving them (for UI/diagnostic purposes).
