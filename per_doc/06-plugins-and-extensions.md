# Plugins and Extensions Architecture

This document covers the complete plugin, extension, and skills system in OpenClaw. The plugin system is the primary mechanism for extending OpenClaw's capabilities, supporting everything from messaging channel integrations to voice calling, memory backends, auth providers, and developer tooling.

---

## Plugin System Core (`src/plugins/`)

The plugin system is composed of several cooperating subsystems: discovery, manifest loading, configuration resolution, the plugin registry, the loader, the hook runner, and lifecycle management (install/update/uninstall). Each plugin surfaces capabilities through a registration API that allows it to provide tools, hooks, channels, gateway methods, HTTP handlers, CLI commands, services, providers, and custom pre-LLM commands.

### Plugin Registry (`registry.ts`)

The registry is the central data structure that holds all registrations from all loaded plugins. It is created via `createPluginRegistry()` and populated during the loading phase.

```typescript
type PluginRegistry = {
  plugins: PluginRecord[];                     // All discovered/loaded plugins
  tools: PluginToolRegistration[];             // Agent tools (LLM-callable)
  hooks: PluginHookRegistration[];             // Legacy event hooks
  typedHooks: TypedPluginHookRegistration[];   // Typed lifecycle hooks
  channels: PluginChannelRegistration[];       // Channel integrations
  providers: PluginProviderRegistration[];     // Auth/model providers
  gatewayHandlers: GatewayRequestHandlers;     // Gateway RPC method handlers
  httpHandlers: PluginHttpRegistration[];      // Generic HTTP handlers
  httpRoutes: PluginHttpRouteRegistration[];   // Named HTTP route handlers
  cliRegistrars: PluginCliRegistration[];      // CLI command registrations
  services: PluginServiceRegistration[];       // Long-running services
  commands: PluginCommandRegistration[];       // Pre-LLM slash commands
  diagnostics: PluginDiagnostic[];             // Warnings and errors from loading
};
```

Each loaded plugin is tracked as a `PluginRecord`:

```typescript
type PluginRecord = {
  id: string;                     // Unique plugin identifier (e.g., "telegram", "voice-call")
  name: string;                   // Human-readable name
  version?: string;               // Semver version string
  description?: string;           // Short description
  kind?: PluginKind;              // Currently only "memory" for exclusive-slot plugins
  source: string;                 // Absolute path to the plugin entry file
  origin: PluginOrigin;           // "bundled" | "global" | "workspace" | "config"
  workspaceDir?: string;          // Workspace root if origin is workspace
  enabled: boolean;               // Whether the plugin is active
  status: "loaded" | "disabled" | "error";  // Current state
  error?: string;                 // Error message if status is "error"

  // Tracked registrations for introspection
  toolNames: string[];            // Names of registered tools
  hookNames: string[];            // Names of registered hooks
  channelIds: string[];           // IDs of registered channels
  providerIds: string[];          // IDs of registered providers
  gatewayMethods: string[];       // Registered gateway RPC methods
  cliCommands: string[];          // Registered CLI commands
  services: string[];             // Registered service IDs
  commands: string[];             // Registered pre-LLM commands
  httpHandlers: number;           // Count of HTTP handlers
  hookCount: number;              // Count of typed hooks
  configSchema: boolean;          // Whether a config schema is present
  configUiHints?: Record<string, PluginConfigUiHint>;  // UI rendering hints
  configJsonSchema?: Record<string, unknown>;          // JSON Schema for validation
};
```

The factory function `createPluginRegistry()` returns the registry along with helper functions for each registration type (`registerTool`, `registerHook`, `registerChannel`, etc.) and a `createApi()` function that constructs the `OpenClawPluginApi` for each plugin.

#### Registration Type Details

Each registration type wraps the plugin's contribution with metadata:

- **PluginToolRegistration**: Links a `pluginId`, a `factory` function `(ctx) => AnyAgentTool | AnyAgentTool[] | null`, tool `names[]`, whether the tool is `optional`, and the `source` path.
- **PluginChannelRegistration**: Links a `pluginId` with a `ChannelPlugin` implementation and optional `ChannelDock`.
- **PluginProviderRegistration**: Links a `pluginId` with a `ProviderPlugin` (auth methods, model config).
- **PluginHttpRouteRegistration**: Maps a normalized `path` string to a handler; duplicate paths are rejected with a diagnostic error.
- **PluginServiceRegistration**: Wraps a service with `id`, `start()`, and optional `stop()` methods.
- **PluginCommandRegistration**: Wraps a command definition with `name`, `description`, `handler`, `acceptsArgs`, and `requireAuth`.

#### Duplicate and Conflict Detection

The registry actively prevents conflicts:

- Gateway methods that collide with core handlers or previously registered plugin handlers produce diagnostic errors.
- HTTP routes with duplicate paths are rejected.
- Provider IDs must be unique across all plugins.
- Plugin commands cannot use reserved names (help, status, config, send, bash, etc.) and cannot duplicate other plugin command names.

---

### Plugin Discovery (`discovery.ts`)

Discovery finds plugin candidates on disk. The function `discoverOpenClawPlugins()` scans four sources in this order:

1. **Config-specified paths** (`config.plugins.load.paths[]`) -- scanned with origin `"config"`.
2. **Workspace extensions** (`.openclaw/extensions/` under the workspace root) -- origin `"workspace"`.
3. **Global extensions** (`~/.config/openclaw/extensions/` or equivalent config directory) -- origin `"global"`.
4. **Bundled extensions** (the `extensions/` directory shipped with the OpenClaw package) -- origin `"bundled"`.

For each directory, discovery:

- Reads top-level files with supported extensions (`.ts`, `.js`, `.mts`, `.cts`, `.mjs`, `.cjs`, excluding `.d.ts`).
- For subdirectories, reads `package.json` and looks for an `openclaw.extensions` array listing entry-point files.
- If no `openclaw.extensions` is present, falls back to looking for `index.ts` or `index.js`.
- Derives an `idHint` from the package name (unscoped, e.g., `@openclaw/voice-call` becomes `voice-call`) or from the filename.
- Deduplicates by resolved absolute path using a `seen` set.

The bundled directory is resolved by `resolveBundledPluginsDir()` in `bundled-dir.ts`:

1. Checks the `OPENCLAW_BUNDLED_PLUGINS_DIR` environment variable.
2. Looks for an `extensions/` sibling of the process executable (for compiled binaries).
3. Walks up from the current module to find `extensions/` at the package root (for npm/dev).

The result is a `PluginDiscoveryResult` with `candidates: PluginCandidate[]` and `diagnostics: PluginDiagnostic[]`.

```typescript
type PluginCandidate = {
  idHint: string;              // Derived plugin ID
  source: string;              // Absolute path to entry file
  rootDir: string;             // Plugin root directory
  origin: PluginOrigin;        // Where the plugin was found
  workspaceDir?: string;       // Workspace root for workspace plugins
  packageName?: string;        // From package.json name field
  packageVersion?: string;     // From package.json version field
  packageDescription?: string; // From package.json description field
  packageDir?: string;         // Directory containing package.json
  packageManifest?: OpenClawPackageManifest; // Parsed openclaw metadata
};
```

---

### Plugin Manifest System (`manifest.ts`, `manifest-registry.ts`)

Each plugin must have an `openclaw.plugin.json` manifest file in its root directory. This is separate from `package.json` and serves as the authoritative source for plugin identity and configuration schema.

#### Plugin Manifest Format (`openclaw.plugin.json`)

```typescript
type PluginManifest = {
  id: string;                                    // Required: unique plugin ID
  configSchema: Record<string, unknown>;         // Required: JSON Schema for plugin config
  kind?: PluginKind;                             // Optional: "memory" for exclusive-slot
  channels?: string[];                           // Channel IDs this plugin provides
  providers?: string[];                          // Provider IDs this plugin provides
  skills?: string[];                             // Associated skill IDs
  name?: string;                                 // Display name
  description?: string;                          // Short description
  version?: string;                              // Version string
  uiHints?: Record<string, PluginConfigUiHint>;  // UI rendering hints for config fields
};
```

Example minimal manifest (Telegram):

```json
{
  "id": "telegram",
  "channels": ["telegram"],
  "configSchema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {}
  }
}
```

Example manifest with kind and UI hints (voice-call):

```json
{
  "id": "voice-call",
  "uiHints": {
    "provider": { "label": "Provider", "help": "Use twilio, telnyx, or mock." },
    "telnyx.apiKey": { "label": "Telnyx API Key", "sensitive": true },
    "serve.port": { "label": "Webhook Port" },
    "streaming.enabled": { "label": "Enable Streaming", "advanced": true }
  },
  "configSchema": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "enabled": { "type": "boolean" },
      "provider": { "type": "string", "enum": ["telnyx", "twilio", "plivo", "mock"] },
      "fromNumber": { "type": "string", "pattern": "^\\+[1-9]\\d{1,14}$" }
    }
  }
}
```

#### Plugin Config UI Hints

```typescript
type PluginConfigUiHint = {
  label?: string;        // Human-readable label for the field
  help?: string;         // Tooltip or help text
  advanced?: boolean;    // Whether to hide in basic config view
  sensitive?: boolean;   // Whether to mask the value (API keys, tokens)
  placeholder?: string;  // Placeholder text for input fields
};
```

#### Manifest Registry (`manifest-registry.ts`)

The manifest registry loads and indexes all plugin manifests from discovered candidates. It handles:

- Loading `openclaw.plugin.json` from each candidate's `rootDir`.
- Detecting and resolving duplicate plugin IDs across origins.
- Origin precedence: `config` > `workspace` > `global` > `bundled` (lower rank wins ties).
- Symlink-aware deduplication using `fs.realpathSync()`.
- Short-lived caching (200ms by default, configurable via `OPENCLAW_PLUGIN_MANIFEST_CACHE_MS`).

The result is a `PluginManifestRegistry`:

```typescript
type PluginManifestRecord = {
  id: string;
  name?: string;
  description?: string;
  version?: string;
  kind?: PluginKind;
  channels: string[];
  providers: string[];
  skills: string[];
  origin: PluginOrigin;
  rootDir: string;
  source: string;
  manifestPath: string;
  schemaCacheKey?: string;
  configSchema?: Record<string, unknown>;
  configUiHints?: Record<string, PluginConfigUiHint>;
};
```

---

### Package Manifest Metadata (`package.json` `openclaw` field)

In addition to `openclaw.plugin.json`, each plugin's `package.json` can contain an `openclaw` field with metadata used for onboarding, catalog display, and install configuration.

```typescript
type OpenClawPackageManifest = {
  extensions?: string[];                // Entry-point files (e.g., ["./index.ts"])
  channel?: PluginPackageChannel;       // Channel catalog metadata
  install?: PluginPackageInstall;       // Install configuration
};

type PluginPackageChannel = {
  id?: string;                          // Channel identifier
  label?: string;                       // Display name (e.g., "Telegram")
  selectionLabel?: string;              // Label in channel picker
  detailLabel?: string;                 // Detailed label
  docsPath?: string;                    // Docs URL path (e.g., "/channels/telegram")
  blurb?: string;                       // Short description for catalog
  order?: number;                       // Sort order in lists (default 50)
  aliases?: string[];                   // Alternative names for matching
  preferOver?: string[];                // Channels this one supersedes
  systemImage?: string;                 // Icon name (SF Symbols on macOS)
  quickstartAllowFrom?: boolean;        // Whether to offer in quickstart
  forceAccountBinding?: boolean;        // Require account ID binding
};

type PluginPackageInstall = {
  npmSpec?: string;                     // npm package spec (e.g., "@openclaw/telegram")
  localPath?: string;                   // Local path for dev (e.g., "extensions/telegram")
  defaultChoice?: "npm" | "local";      // Which install method to default to
};
```

---

### Plugin Configuration State (`config-state.ts`)

The configuration state module normalizes the `plugins` section of the OpenClaw config into a consistent structure and resolves enable/disable decisions for each plugin.

#### Normalized Config

```typescript
type NormalizedPluginsConfig = {
  enabled: boolean;         // Global plugin enable flag (default: true)
  allow: string[];          // Allowlist: if non-empty, only these plugins load
  deny: string[];           // Denylist: these plugins are always blocked
  loadPaths: string[];      // Extra paths to scan for plugins
  slots: {
    memory?: string | null; // Selected memory plugin ID, or null for "none"
  };
  entries: Record<string, { enabled?: boolean; config?: unknown }>;
};
```

#### Enable State Resolution

The function `resolveEnableState()` determines whether a plugin should be loaded. The decision cascade:

1. If `config.plugins.enabled` is false, all plugins are disabled.
2. If the plugin ID is in `config.plugins.deny`, it is blocked.
3. If `config.plugins.allow` is non-empty and the ID is absent, it is blocked.
4. If the plugin ID matches `config.plugins.slots.memory`, it is enabled.
5. If `config.plugins.entries[id].enabled` is explicitly true or false, that is honored.
6. Bundled plugins in the `BUNDLED_ENABLED_BY_DEFAULT` set (`device-pair`, `phone-control`, `talk-voice`) are enabled; other bundled plugins are disabled by default.
7. Non-bundled plugins (global, workspace, config) are enabled by default.

#### Memory Slot Resolution

Memory plugins use an exclusive-slot system. Only one `kind: "memory"` plugin can be active at a time. The default slot ID is `"memory-core"`. The function `resolveMemorySlotDecision()` ensures that:

- If the memory slot is set to `null` (config value `"none"`), all memory plugins are disabled.
- If set to a specific ID, only that plugin is enabled for the memory slot.
- If unset, the first discovered memory plugin is selected.

#### Test Environment Behavior

In test environments (`process.env.VITEST` set), `applyTestPluginDefaults()` automatically:

- Disables plugins globally unless the test has explicit plugin configuration.
- Sets the memory slot to `"none"` to avoid loading heavyweight memory backends.

---

### Plugin Loader (`loader.ts`)

The loader is the orchestrator that ties discovery, manifest loading, config resolution, and registration together. The main entry point is `loadOpenClawPlugins()`.

#### Loading Pipeline

```
loadOpenClawPlugins(options)
  |
  +-- applyTestPluginDefaults() -- Adjust config for test environments
  +-- normalizePluginsConfig() -- Normalize config.plugins
  +-- Check registry cache (by workspace dir + normalized config)
  +-- clearPluginCommands() -- Reset previous command registrations
  +-- createPluginRuntime() -- Create the shared runtime object
  +-- createPluginRegistry() -- Create empty registry + helpers
  +-- discoverOpenClawPlugins() -- Find candidates on disk
  +-- loadPluginManifestRegistry() -- Load and index manifests
  +-- Create jiti instance with SDK alias resolution
  |
  +-- For each candidate (matched with manifest record):
  |     +-- Skip if no manifest record found
  |     +-- Skip duplicates (first origin wins)
  |     +-- resolveEnableState() -- Check if plugin should load
  |     +-- Check configSchema exists (required)
  |     +-- jiti(candidate.source) -- Dynamic import via jiti
  |     +-- resolvePluginModuleExport() -- Extract definition/register
  |     +-- resolveMemorySlotDecision() -- Handle memory exclusivity
  |     +-- validatePluginConfig() -- Validate config against JSON schema
  |     +-- createApi(record, config) -- Build plugin API
  |     +-- register(api) -- Call plugin's register function
  |     +-- Collect diagnostics on any errors
  |
  +-- Cache the registry
  +-- setActivePluginRegistry()
  +-- initializeGlobalHookRunner()
  +-- Return PluginRegistry
```

#### Module Resolution

The loader uses `jiti` for dynamic imports, supporting TypeScript files directly without compilation. It configures aliases so that plugins can `import from "openclaw/plugin-sdk"` and `"openclaw/plugin-sdk/account-id"`, resolving these to the appropriate source or dist files based on the environment.

#### Plugin Module Export Format

A plugin module can export either:

1. An `OpenClawPluginDefinition` object (with `register` or `activate` function).
2. A bare function `(api: OpenClawPluginApi) => void`.
3. A default export wrapping either of the above.

```typescript
type OpenClawPluginDefinition = {
  id?: string;
  name?: string;
  description?: string;
  version?: string;
  kind?: PluginKind;                            // "memory" for exclusive-slot
  configSchema?: OpenClawPluginConfigSchema;    // Runtime schema (zod-like)
  register?: (api: OpenClawPluginApi) => void | Promise<void>;
  activate?: (api: OpenClawPluginApi) => void | Promise<void>;  // Alias for register
};
```

Note: If `register()` returns a Promise, a diagnostic warning is emitted -- async registration is not awaited and its side effects are ignored.

#### Load Options

```typescript
type PluginLoadOptions = {
  config?: OpenClawConfig;                                  // Full app config
  workspaceDir?: string;                                    // Workspace root
  logger?: PluginLogger;                                    // Custom logger
  coreGatewayHandlers?: Record<string, GatewayRequestHandler>; // Built-in handlers
  cache?: boolean;                                          // Enable registry cache
  mode?: "full" | "validate";                               // "validate" skips registration
};
```

---

### Plugin API (`OpenClawPluginApi`)

The API object passed to each plugin's `register()` function. This is the plugin's interface to the host system.

```typescript
type OpenClawPluginApi = {
  // Identity
  id: string;                    // Plugin ID
  name: string;                  // Plugin name
  version?: string;              // Plugin version
  description?: string;          // Plugin description
  source: string;                // Absolute path to plugin entry file

  // Configuration
  config: OpenClawConfig;        // Full application config
  pluginConfig?: Record<string, unknown>;  // Plugin-specific config (validated)

  // Runtime
  runtime: PluginRuntime;        // Host runtime services
  logger: PluginLogger;          // Structured logger (info, warn, error, debug)

  // Registration methods
  registerTool(tool, opts?): void;              // Register agent tools
  registerHook(events, handler, opts?): void;   // Register legacy hooks
  registerHttpHandler(handler): void;           // Register generic HTTP handler
  registerHttpRoute({ path, handler }): void;   // Register named HTTP route
  registerChannel(registration): void;          // Register channel plugin
  registerGatewayMethod(method, handler): void; // Register gateway RPC method
  registerCli(registrar, opts?): void;          // Register CLI commands
  registerService(service): void;               // Register long-running service
  registerProvider(provider): void;             // Register auth/model provider
  registerCommand(command): void;               // Register pre-LLM command
  resolvePath(input): string;                   // Resolve ~ and env vars in paths

  // Typed hook registration
  on<K extends PluginHookName>(hookName, handler, opts?): void;
};
```

#### Tool Registration

Tools are functions the LLM agent can call. A plugin can register tools in two forms:

1. **Direct tool object**: An `AnyAgentTool` with name, description, parameters schema, and execute function.
2. **Factory function**: `(ctx: OpenClawPluginToolContext) => AnyAgentTool | AnyAgentTool[] | null`. Called at agent startup with context about the current session.

```typescript
type OpenClawPluginToolContext = {
  config?: OpenClawConfig;
  workspaceDir?: string;
  agentDir?: string;
  agentId?: string;
  sessionKey?: string;
  messageChannel?: string;
  agentAccountId?: string;
  sandboxed?: boolean;
};

type OpenClawPluginToolOptions = {
  name?: string;           // Single tool name
  names?: string[];        // Multiple tool names (for factories returning arrays)
  optional?: boolean;      // If true, tool failure is non-fatal
};
```

Example from `memory-core`:

```typescript
api.registerTool(
  (ctx) => {
    const memorySearchTool = api.runtime.tools.createMemorySearchTool({
      config: ctx.config,
      agentSessionKey: ctx.sessionKey,
    });
    const memoryGetTool = api.runtime.tools.createMemoryGetTool({
      config: ctx.config,
      agentSessionKey: ctx.sessionKey,
    });
    if (!memorySearchTool || !memoryGetTool) return null;
    return [memorySearchTool, memoryGetTool];
  },
  { names: ["memory_search", "memory_get"] },
);
```

#### Provider Registration

Providers add model backends or authentication methods:

```typescript
type ProviderPlugin = {
  id: string;                         // Provider ID (e.g., "google-gemini")
  label: string;                      // Display name
  docsPath?: string;                  // Documentation URL path
  aliases?: string[];                 // Alternative names
  envVars?: string[];                 // Required environment variables
  models?: ModelProviderConfig;       // Model definitions
  auth: ProviderAuthMethod[];         // Authentication methods
  formatApiKey?: (cred) => string;    // Custom API key formatting
  refreshOAuth?: (cred) => Promise<OAuthCredential>;  // OAuth refresh
};

type ProviderAuthMethod = {
  id: string;                         // Auth method ID
  label: string;                      // Display label
  hint?: string;                      // Help text
  kind: "oauth" | "api_key" | "token" | "device_code" | "custom";
  run: (ctx: ProviderAuthContext) => Promise<ProviderAuthResult>;
};
```

#### Command Registration

Pre-LLM commands bypass the AI agent entirely. They are processed before built-in commands and before agent invocation:

```typescript
type OpenClawPluginCommandDefinition = {
  name: string;                // Command name without leading "/" (e.g., "tts")
  description: string;         // Shown in /help and command menus
  acceptsArgs?: boolean;       // Whether the command takes arguments
  requireAuth?: boolean;       // Require authorized sender (default: true)
  handler: PluginCommandHandler;
};
```

Commands are validated against a set of reserved names (help, status, config, send, bash, etc.) and must match the pattern `^[a-z][a-z0-9_-]*$`.

#### Service Registration

Services are long-running background processes:

```typescript
type OpenClawPluginService = {
  id: string;
  start: (ctx: OpenClawPluginServiceContext) => void | Promise<void>;
  stop?: (ctx: OpenClawPluginServiceContext) => void | Promise<void>;
};

type OpenClawPluginServiceContext = {
  config: OpenClawConfig;
  workspaceDir?: string;
  stateDir: string;          // Persistent state directory
  logger: PluginLogger;
};
```

Services are started via `startPluginServices()` in `services.ts`, which iterates through all registered services, calls `start()`, and returns a handle with a `stop()` method that stops them in reverse order.

---

### Hook System (`hooks.ts`)

The hook system allows plugins to participate in lifecycle events. There are two hook mechanisms:

1. **Legacy hooks** via `api.registerHook(events, handler, opts?)` -- string-based event names, backed by the internal hook system.
2. **Typed hooks** via `api.on(hookName, handler, opts?)` -- strongly typed with event/context type pairs.

#### Hook Names

```typescript
type PluginHookName =
  | "before_agent_start"     // Before agent processes a message
  | "agent_end"              // After agent finishes processing
  | "before_compaction"      // Before session compaction
  | "after_compaction"       // After session compaction
  | "before_reset"           // Before session reset (/new, /reset)
  | "message_received"       // Inbound message received
  | "message_sending"        // Outbound message about to be sent
  | "message_sent"           // Outbound message sent
  | "before_tool_call"       // Before a tool is executed
  | "after_tool_call"        // After a tool finishes
  | "tool_result_persist"    // When tool result is written to transcript
  | "session_start"          // Session begins
  | "session_end"            // Session ends
  | "gateway_start"          // Gateway server starts
  | "gateway_stop";          // Gateway server stops
```

#### Hook Execution Modes

The hook runner (`createHookRunner()`) supports two execution patterns:

1. **Void hooks** (fire-and-forget): All handlers run in parallel via `Promise.all()`. Used for observational hooks like `message_received`, `message_sent`, `agent_end`, `session_start`, `session_end`, `gateway_start`, `gateway_stop`, `before_compaction`, `after_compaction`, `before_reset`.

2. **Modifying hooks** (sequential): Handlers run one at a time in priority order (higher priority first). Each handler can return a result that modifies the event. Used for:
   - `before_agent_start` -- can inject `systemPrompt` and `prependContext`
   - `message_sending` -- can modify `content` or set `cancel: true`
   - `before_tool_call` -- can modify `params`, set `block: true` with `blockReason`

3. **Synchronous hooks**: `tool_result_persist` runs synchronously (no async) because it is in a hot path. Each handler receives the message and can return a modified version. Handlers returning Promises get a warning and are skipped.

#### Hook Event Types

Each hook has specific event and context types:

```typescript
// before_agent_start
type PluginHookBeforeAgentStartEvent = { prompt: string; messages?: unknown[] };
type PluginHookBeforeAgentStartResult = { systemPrompt?: string; prependContext?: string };

// before_tool_call
type PluginHookBeforeToolCallEvent = { toolName: string; params: Record<string, unknown> };
type PluginHookBeforeToolCallResult = { params?: Record<string, unknown>; block?: boolean; blockReason?: string };

// message_sending
type PluginHookMessageSendingEvent = { to: string; content: string; metadata?: Record<string, unknown> };
type PluginHookMessageSendingResult = { content?: string; cancel?: boolean };

// tool_result_persist
type PluginHookToolResultPersistEvent = { toolName?: string; toolCallId?: string; message: AgentMessage; isSynthetic?: boolean };
type PluginHookToolResultPersistResult = { message?: AgentMessage };

// before_compaction
type PluginHookBeforeCompactionEvent = {
  messageCount: number;
  compactingCount?: number;
  tokenCount?: number;
  messages?: unknown[];
  sessionFile?: string;    // Path to JSONL transcript on disk
};
```

#### Global Hook Runner (`hook-runner-global.ts`)

A singleton hook runner is initialized when plugins are loaded and made available globally:

```typescript
initializeGlobalHookRunner(registry)   // Called by the loader
getGlobalHookRunner()                  // Access from anywhere
hasGlobalHooks(hookName)               // Quick check for registered hooks
resetGlobalHookRunner()                // Reset for testing
```

The global runner is configured with `catchErrors: true`, so individual hook failures are logged but do not crash the system.

---

### Plugin Runtime (`runtime/types.ts`)

The `PluginRuntime` is a dependency-injection container providing plugins access to host system capabilities. It is created once during loading and shared across all plugins.

Key subsystems exposed:

```typescript
type PluginRuntime = {
  version: string;                              // OpenClaw version
  config: { loadConfig, writeConfigFile };      // Config I/O
  system: { enqueueSystemEvent, runCommandWithTimeout, formatNativeDependencyHint };
  media: { loadWebMedia, detectMime, mediaKindFromMime, isVoiceCompatibleAudio, getImageMetadata, resizeToJpeg };
  tts: { textToSpeechTelephony };               // Text-to-speech for telephony
  tools: { createMemoryGetTool, createMemorySearchTool, registerMemoryCli };
  channel: {
    text: { chunkMarkdownText, chunkText, resolveTextChunkLimit, ... };
    reply: { dispatchReplyFromConfig, finalizeInboundContext, formatAgentEnvelope, ... };
    routing: { resolveAgentRoute };
    pairing: { buildPairingReply, readAllowFromStore, upsertPairingRequest };
    media: { fetchRemoteMedia, saveMediaBuffer };
    activity: { record, get };
    session: { resolveStorePath, readSessionUpdatedAt, recordInboundSession, ... };
    mentions: { buildMentionRegexes, matchesMentionPatterns, ... };
    reactions: { shouldAckReaction, removeAckReactionAfterReply };
    groups: { resolveGroupPolicy, resolveRequireMention };
    debounce: { createInboundDebouncer, resolveInboundDebounceMs };
    commands: { resolveCommandAuthorizedFromAuthorizers, isControlCommandMessage, ... };
    // Channel-specific adapters
    discord: { messageActions, sendMessageDiscord, monitorDiscordProvider, ... };
    slack: { sendMessageSlack, monitorSlackProvider, handleSlackAction, ... };
    telegram: { sendMessageTelegram, monitorTelegramProvider, ... };
    signal: { sendMessageSignal, monitorSignalProvider, ... };
    imessage: { monitorIMessageProvider, sendMessageIMessage, ... };
    whatsapp: { sendMessageWhatsApp, monitorWebChannel, ... };
    line: { sendMessageLine, monitorLineProvider, ... };
  };
  logging: { shouldLogVerbose, getChildLogger };
  state: { resolveStateDir };
};
```

This runtime is passed to plugins as `api.runtime` and allows channel plugins to access messaging, media, routing, and other infrastructure without direct imports from the core codebase.

---

### Plugin SDK (`src/plugin-sdk/`)

The plugin SDK is the public API surface exported as `"openclaw/plugin-sdk"`. Plugins import from this module to get types and utilities.

Files:

- **`index.ts`**: Re-exports types and utilities from across the codebase. Includes channel types (`ChannelPlugin`, `ChannelId`, adapters), plugin types (`OpenClawPluginApi`, `OpenClawPluginService`), gateway types, config types, channel-specific schemas and helpers, media utilities, and more.
- **`account-id.ts`**: Account identity helpers, exported as `"openclaw/plugin-sdk/account-id"`.

Key re-exports include:

- All channel adapter types (messaging, auth, gateway, group, heartbeat, etc.)
- Configuration schemas for each channel (Discord, Slack, Telegram, WhatsApp, etc.)
- Helper functions for allowlists, mention gating, ack reactions, text chunking
- HTTP body utilities with SSRF protection
- Diagnostic event types for telemetry
- Onboarding helpers for channel setup

---

### Plugin Installation (`install.ts`)

The installation system supports multiple source types:

1. **npm packages** (`installPluginFromNpmSpec`): Runs `npm pack` to download the tarball, extracts it, validates, scans for dangerous code, and copies to the global extensions directory.
2. **Local directories** (`installPluginFromDir`): Validates package.json, checks `openclaw.extensions`, and copies to the extensions directory.
3. **Archive files** (`installPluginFromArchive`): Extracts `.tar.gz` or `.tgz` archives, resolves the package root, and delegates to directory installation.
4. **Single files** (`installPluginFromFile`): Copies a standalone `.ts`/`.js` file to the extensions directory.
5. **Smart path resolution** (`installPluginFromPath`): Auto-detects whether the path is a directory, archive, or single file and delegates accordingly.

#### Security Scanning

All installations run a security scan via `skillScanner.scanDirectoryWithSummary()`. Critical findings (e.g., dangerous code patterns like `eval`, `exec`) generate warnings but do not block installation.

#### Install Validation

- `package.json` must have `openclaw.extensions` array.
- Extension entries must not escape the plugin directory (path traversal protection).
- Plugin ID is derived from the unscoped package name.
- Duplicate installs are rejected unless mode is `"update"`.

#### Install Result

```typescript
type InstallPluginResult =
  | { ok: true; pluginId: string; targetDir: string; manifestName?: string; version?: string; extensions: string[] }
  | { ok: false; error: string };
```

---

### Plugin Update (`update.ts`)

The update system handles two scenarios:

1. **npm-installed plugin updates** (`updateNpmInstalledPlugins`): Iterates through plugins with `source: "npm"` in the install records, re-downloads via `npm pack`, and compares versions. Supports dry-run mode.

2. **Update channel sync** (`syncPluginsForUpdateChannel`): Synchronizes between bundled (local dev) and npm-installed versions based on the update channel:
   - `"dev"` channel: Switches to bundled local paths, adds paths to `load.paths`.
   - Non-dev channels: Switches from bundled paths to npm-installed versions, removes bundled paths from `load.paths`.

---

### Plugin Uninstall (`uninstall.ts`)

Uninstallation removes a plugin from config and optionally deletes installed files:

```typescript
type UninstallActions = {
  entry: boolean;       // Removed from config.plugins.entries
  install: boolean;     // Removed from config.plugins.installs
  allowlist: boolean;   // Removed from config.plugins.allow
  loadPath: boolean;    // Removed source path from config.plugins.load.paths
  memorySlot: boolean;  // Reset memory slot to default
  directory: boolean;   // Deleted installed directory
};
```

Safety rules:

- Linked plugins (`source: "path"`) never have their source directory deleted.
- Install paths are validated against the expected default path to prevent deleting arbitrary directories.
- Config cleanup is performed by `removePluginFromConfig()`, which is a pure function operating on config objects.

---

### Slot System (`slots.ts`)

The slot system manages exclusive-occupancy plugin categories. Currently only one kind exists:

```typescript
const SLOT_BY_KIND: Record<PluginKind, PluginSlotKey> = { memory: "memory" };
const DEFAULT_SLOT_BY_KEY: Record<PluginSlotKey, string> = { memory: "memory-core" };
```

`applyExclusiveSlotSelection()` ensures that when a new plugin is selected for a slot, competing plugins of the same kind are disabled in config.

---

### Plugin Enable/Disable (`enable.ts`)

Simple helper to enable a plugin in config:

1. Returns early if plugins are globally disabled or the ID is in the denylist.
2. Sets `config.plugins.entries[pluginId].enabled = true`.
3. Adds the plugin to the allowlist if an allowlist is active.

---

## Extensions (`extensions/` directory -- 36 total)

Extensions are TypeScript code plugins that run at load time and register capabilities via the plugin API. Each extension lives in its own directory under `extensions/` with:

- `index.ts` -- Entry point exporting an `OpenClawPluginDefinition`
- `openclaw.plugin.json` -- Plugin manifest with ID, config schema, and metadata
- `package.json` -- npm package metadata with `openclaw.extensions` array
- Optional `src/` directory for implementation files

### Channel Extensions (20)

Channel extensions integrate messaging platforms. Each registers a `ChannelPlugin` that implements polling, sending, auth, and configuration for a specific platform.

| Extension | Platform |
|-----------|----------|
| `telegram` | Telegram Bot API |
| `discord` | Discord via bot token |
| `slack` | Slack via Socket Mode or Web API |
| `signal` | Signal via signal-cli |
| `whatsapp` | WhatsApp via web bridge |
| `imessage` | iMessage via AppleScript/macOS |
| `irc` | IRC protocol |
| `matrix` | Matrix protocol |
| `twitch` | Twitch chat |
| `line` | LINE Messaging API |
| `googlechat` | Google Chat |
| `mattermost` | Mattermost |
| `msteams` | Microsoft Teams |
| `nextcloud-talk` | Nextcloud Talk |
| `nostr` | Nostr protocol |
| `bluebubbles` | BlueBubbles (iMessage bridge) |
| `feishu` | Feishu/Lark |
| `tlon` | Tlon/Urbit |
| `zalo` | Zalo Official Account |
| `zalouser` | Zalo User API |

### Memory Extensions (2)

Memory extensions implement the `kind: "memory"` slot for persistent knowledge storage.

**memory-core** (`extensions/memory-core/`):

- Kind: `"memory"` (occupies the exclusive memory slot)
- Registers tools: `memory_search` and `memory_get` via factory functions
- Registers CLI: `memory` subcommand
- Uses file-backed storage via `createMemorySearchTool()` and `createMemoryGetTool()` from the runtime
- Default memory slot winner (ID: `memory-core`)

```typescript
const memoryCorePlugin = {
  id: "memory-core",
  name: "Memory (Core)",
  description: "File-backed memory search tools and CLI",
  kind: "memory",
  configSchema: emptyPluginConfigSchema(),
  register(api) {
    api.registerTool((ctx) => {
      const memorySearchTool = api.runtime.tools.createMemorySearchTool({ ... });
      const memoryGetTool = api.runtime.tools.createMemoryGetTool({ ... });
      return [memorySearchTool, memoryGetTool];
    }, { names: ["memory_search", "memory_get"] });

    api.registerCli(({ program }) => {
      api.runtime.tools.registerMemoryCli(program);
    }, { commands: ["memory"] });
  },
};
```

**memory-lancedb** (`extensions/memory-lancedb/`):

- Kind: `"memory"` (competes with memory-core for the memory slot)
- Uses LanceDB vector database for embedding-based recall
- Supports auto-recall (automatic context injection) and auto-capture

### Auth/Provider Extensions (4)

These extensions register `ProviderPlugin` instances for third-party model providers:

| Extension | Provider | Auth Kind |
|-----------|----------|-----------|
| `google-antigravity-auth` | Google Antigravity | OAuth |
| `google-gemini-cli-auth` | Google Gemini CLI | Device code flow |
| `minimax-portal-auth` | MiniMax Portal | Device code flow |
| `qwen-portal-auth` | Qwen Portal | Device code flow |

### Voice Extensions (2)

**voice-call** (`extensions/voice-call/`):

A comprehensive extension demonstrating the full breadth of the plugin API. Registers:

- **Tools**: `voice_call` -- an agent-callable tool with TypeBox schema supporting actions: `initiate_call`, `continue_call`, `speak_to_user`, `end_call`, `get_status`.
- **Gateway methods**: `voicecall.initiate`, `voicecall.continue`, `voicecall.speak`, `voicecall.end`, `voicecall.status`, `voicecall.start`.
- **CLI**: `voicecall` subcommand.
- **Service**: `voicecall` long-running service that manages the call runtime.
- **Config schema**: Extensive JSON Schema covering providers (Telnyx, Twilio, Plivo, mock), phone numbers, inbound policies, TTS settings, streaming config, tunnel settings, etc.
- **UI hints**: Labeled and annotated config fields with sensitive/advanced flags.

Supports Telnyx, Twilio, and Plivo telephony providers with webhook handling, TTS (OpenAI, ElevenLabs, Edge), STT (OpenAI Realtime), and tunnel configuration (ngrok, Tailscale).

**talk-voice** (`extensions/talk-voice/`):

- ElevenLabs Talk voice configuration
- Enabled by default (in `BUNDLED_ENABLED_BY_DEFAULT`)

### Utility Extensions (8)

| Extension | Purpose | Registers |
|-----------|---------|-----------|
| `diagnostics-otel` | OpenTelemetry export of diagnostic events | Service |
| `device-pair` | iOS/macOS device pairing for remote access | Enabled by default |
| `phone-control` | Arm/disarm phone control features | Enabled by default |
| `thread-ownership` | Slack thread ownership management | -- |
| `copilot-proxy` | GitHub Copilot proxy integration | -- |
| `llm-task` | JSON-only LLM task runner | -- |
| `lobster` | Workflow pipeline automation | -- |
| `open-prose` | VM skill pack for prose editing | -- |

---

## Skills (`skills/` directory -- 51 total)

Skills are fundamentally different from extensions. They are NOT executable code -- they are documentation packs consisting of a `SKILL.md` file with YAML frontmatter. Skills teach the AI agent how to use external tools, CLIs, and APIs by providing structured knowledge that gets injected into the LLM context at chat time.

### Skill File Format

Each skill lives in `skills/<skill-id>/SKILL.md`:

```markdown
---
name: skill-id
description: One-line description of what this skill does.
homepage: https://optional-homepage-url
metadata:
  openclaw:
    emoji: "icon-emoji"
    requires:
      bins: ["required-binary"]       # All of these must be present
      config: ["config.path"]         # Required config keys
      env: ["ENV_VAR"]                # Required environment variables
      anyBins: ["binary-a", "binary-b"] # At least one must be present
    install:
      - id: brew
        kind: brew
        formula: package-name
        tap: "optional/tap"           # Custom Homebrew tap
        bins: ["binary-name"]
        label: "Install via Homebrew"
      - id: apt
        kind: apt
        package: package-name
        bins: ["binary-name"]
        label: "Install via apt"
    primaryEnv: "ENV_VAR"             # Primary environment variable
    os: ["darwin", "linux"]           # Supported operating systems
---

# Skill Title

Instructions, examples, and documentation that the AI agent reads
to understand how to use the associated tool.

## Usage Examples

```bash
command --flag argument
```
```

### Skill Frontmatter Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Skill identifier, matches directory name |
| `description` | string | One-line description for listings |
| `homepage` | string | Optional URL for tool documentation |
| `metadata.openclaw.emoji` | string | Display emoji for the skill |
| `metadata.openclaw.requires.bins` | string[] | Required CLI binaries (all must exist) |
| `metadata.openclaw.requires.anyBins` | string[] | Alternative binaries (any one suffices) |
| `metadata.openclaw.requires.config` | string[] | Required config keys |
| `metadata.openclaw.requires.env` | string[] | Required environment variables |
| `metadata.openclaw.install` | object[] | Installation instructions with kind, formula/package, label |
| `metadata.openclaw.primaryEnv` | string | Primary environment variable for the tool |
| `metadata.openclaw.os` | string[] | Supported operating systems |

### Skill Categories

**Development and Automation:**
- `coding-agent` -- Coding agent patterns and workflows
- `github` -- GitHub CLI (`gh`) for issues, PRs, CI runs, API queries
- `tmux` -- Terminal multiplexer session management

**Notes and Productivity:**
- `obsidian` -- Obsidian vault management via obsidian-cli
- `apple-notes` -- Apple Notes automation
- `bear-notes` -- Bear note-taking app
- `notion` -- Notion workspace management
- `trello` -- Trello board automation
- `things-mac` -- Things 3 task manager (macOS)
- `apple-reminders` -- Apple Reminders
- `1password` -- 1Password CLI integration

**Media and Image:**
- `camsnap` -- Camera snapshot capture
- `video-frames` -- Video frame extraction
- `openai-image-gen` -- OpenAI image generation (DALL-E)
- `openai-whisper` -- Local OpenAI Whisper transcription
- `openai-whisper-api` -- OpenAI Whisper API transcription
- `nano-banana-pro` -- Banana.dev image generation
- `nano-pdf` -- PDF processing

**Music and Audio:**
- `spotify-player` -- Spotify playback via spogo/spotify_player
- `songsee` -- Song recognition
- `sonoscli` -- Sonos speaker control
- `peekaboo` -- Audio/media inspection
- `sherpa-onnx-tts` -- Local TTS via sherpa-onnx

**Communication:**
- `slack` -- Slack workspace interaction patterns
- `discord` -- Discord bot interaction patterns
- `imsg` -- iMessage via command line
- `bluebubbles` -- BlueBubbles iMessage bridge patterns
- `himalaya` -- Email via himalaya CLI

**Web and Data:**
- `summarize` -- Web page summarization
- `weather` -- Weather via wttr.in and Open-Meteo (no API key required)
- `blogwatcher` -- Blog/RSS monitoring
- `gifgrep` -- GIF search
- `gemini` -- Google Gemini API

**System and Hardware:**
- `blucli` -- Bluetooth CLI control
- `eightctl` -- Eight Sleep mattress control
- `wacli` -- Wallpaper CLI
- `canvas` -- Canvas/drawing operations
- `openhue` -- Philips Hue smart lights

**Management and Ops:**
- `healthcheck` -- System health checks
- `model-usage` -- Model usage tracking and reporting
- `session-logs` -- Session log management
- `skill-creator` -- Meta-skill for creating new skills
- `clawhub` -- ClawHub integration

**Other:**
- `oracle` -- Oracle database queries
- `food-order` -- Food ordering automation
- `goplaces` -- Google Places API
- `gog` -- GOG game platform
- `mcporter` -- Minecraft port forwarding
- `ordercli` -- Order management CLI
- `sag` -- System agent
- `voice-call` -- Voice call skill documentation (companion to the extension)

### Example Skills

**Simple skill (weather):**

```yaml
---
name: weather
description: Get current weather and forecasts (no API key required).
homepage: https://wttr.in/:help
metadata: { "openclaw": { "emoji": "...", "requires": { "bins": ["curl"] } } }
---
```

The body provides usage examples with `curl` commands and format codes.

**Skill with install instructions (github):**

```yaml
---
name: github
description: "Interact with GitHub using the `gh` CLI."
metadata:
  openclaw:
    emoji: "..."
    requires: { bins: ["gh"] }
    install:
      - id: brew
        kind: brew
        formula: gh
        bins: ["gh"]
        label: "Install GitHub CLI (brew)"
      - id: apt
        kind: apt
        package: gh
        bins: ["gh"]
        label: "Install GitHub CLI (apt)"
---
```

**Skill with alternative binaries (spotify-player):**

```yaml
---
name: spotify-player
description: Terminal Spotify playback/search via spogo (preferred) or spotify_player.
metadata:
  openclaw:
    emoji: "..."
    requires: { anyBins: ["spogo", "spotify_player"] }
    install:
      - id: brew
        kind: brew
        formula: spogo
        tap: "steipete/tap"
        bins: ["spogo"]
        label: "Install spogo (brew)"
      - id: brew
        kind: brew
        formula: spotify_player
        bins: ["spotify_player"]
        label: "Install spotify_player (brew)"
---
```

---

## Key Architectural Concepts

### Extensions vs Skills: Comparison

| Aspect | Extensions | Skills |
|--------|-----------|--------|
| **Nature** | Executable TypeScript code | Markdown documentation with YAML frontmatter |
| **Location** | `extensions/*/index.ts` | `skills/*/SKILL.md` |
| **Loaded** | At startup by the plugin loader | At chat time, read into LLM context |
| **Registration** | Via `register(api)` function | Via YAML `metadata.openclaw` |
| **Execution** | Runs in the Node.js process | Read by the LLM as context, not executed |
| **Purpose** | Deep system integration (channels, tools, hooks) | Knowledge transfer to the AI agent |
| **Dependencies** | Can import npm packages, use TypeScript | Only requires external CLI tools/APIs |
| **Config schema** | JSON Schema in `openclaw.plugin.json` | Requirements in YAML frontmatter |
| **Count** | 36 | 51 |
| **Security** | Code-scanned during install | No code execution risk |

### Plugin Lifecycle

```
                    DISCOVERY
                       |
                       v
                  MANIFEST LOAD
                       |
                       v
              CONFIG STATE RESOLUTION
              (enable/disable, slots)
                       |
                       v
                DYNAMIC IMPORT (jiti)
                       |
                       v
              MODULE EXPORT RESOLUTION
              (definition + register)
                       |
                       v
              CONFIG VALIDATION (JSON Schema)
                       |
                       v
                  REGISTRATION
              (api.registerTool, etc.)
                       |
                       v
                 REGISTRY CACHING
                       |
                       v
             GLOBAL HOOK RUNNER INIT
                       |
                       v
                 SERVICE STARTUP
              (startPluginServices)
```

### Data Flow: Tool Registration to Agent Invocation

1. Plugin calls `api.registerTool(factory, { names: ["tool_name"] })`.
2. The factory function and names are stored in `registry.tools[]`.
3. When an agent session starts, the tool factory is called with a `PluginToolContext`.
4. The factory returns an `AnyAgentTool` (or array, or null).
5. The tool is added to the agent's available tools.
6. The LLM can invoke the tool by name during conversation.

### Data Flow: Hook Lifecycle

1. Plugin calls `api.on("before_tool_call", handler, { priority: 10 })`.
2. The handler is stored in `registry.typedHooks[]`.
3. When a tool call occurs, the hook runner's `runBeforeToolCall()` is invoked.
4. Handlers are sorted by priority (descending) and run sequentially.
5. A handler can modify params or block the call entirely.
6. Errors in handlers are caught and logged (with `catchErrors: true`).

### Data Flow: Channel Plugin

1. Channel extension calls `api.registerChannel({ plugin, dock? })`.
2. The `ChannelPlugin` provides adapters for messaging, auth, gateway, setup, etc.
3. The gateway polls the channel via the gateway adapter.
4. Incoming messages are routed through the agent.
5. Outgoing messages are sent via the channel's outbound adapter.
6. The dock (if provided) adds additional channel-specific capabilities.

### Plugin Config Flow

```
config.yaml
  plugins:
    enabled: true
    allow: [...]
    deny: [...]
    slots:
      memory: "memory-core"
    entries:
      voice-call:
        enabled: true
        config:
          provider: "telnyx"
          fromNumber: "+15550001234"
          telnyx:
            apiKey: "KEY_..."

        |
        v  normalizePluginsConfig()
        |
        v  resolveEnableState()
        |
        v  resolveMemorySlotDecision()
        |
        v  validatePluginConfig() against openclaw.plugin.json schema
        |
        v  Passed to plugin as api.pluginConfig
```

---

## Key Source Files Reference

| File | Purpose |
|------|---------|
| `src/plugins/registry.ts` | Plugin registry types and creation |
| `src/plugins/types.ts` | All plugin type definitions (API, hooks, providers, commands) |
| `src/plugins/discovery.ts` | Plugin candidate discovery on disk |
| `src/plugins/manifest.ts` | Plugin manifest loading (`openclaw.plugin.json`) |
| `src/plugins/manifest-registry.ts` | Manifest registry with caching and dedup |
| `src/plugins/loader.ts` | Main plugin loading orchestrator |
| `src/plugins/config-state.ts` | Config normalization and enable/disable resolution |
| `src/plugins/hooks.ts` | Hook runner creation and execution |
| `src/plugins/hook-runner-global.ts` | Global singleton hook runner |
| `src/plugins/commands.ts` | Plugin command registry and execution |
| `src/plugins/services.ts` | Plugin service lifecycle management |
| `src/plugins/slots.ts` | Exclusive slot system for plugin kinds |
| `src/plugins/enable.ts` | Helper for enabling plugins in config |
| `src/plugins/install.ts` | Plugin installation from various sources |
| `src/plugins/uninstall.ts` | Plugin removal from config and disk |
| `src/plugins/update.ts` | Plugin update and channel sync |
| `src/plugins/bundled-dir.ts` | Bundled extensions directory resolution |
| `src/plugins/config-schema.ts` | Empty config schema helper |
| `src/plugins/http-path.ts` | HTTP path normalization |
| `src/plugins/runtime.ts` | Active registry setter |
| `src/plugins/runtime/types.ts` | PluginRuntime type with all host capabilities |
| `src/plugin-sdk/index.ts` | Public SDK surface (`"openclaw/plugin-sdk"`) |
| `src/plugin-sdk/account-id.ts` | Account identity helpers |
| `src/compat/legacy-names.ts` | Project name constants (`MANIFEST_KEY = "openclaw"`) |
