# Channels and Routing Architecture

This document covers the complete channel and routing system: how messages enter
the system from various chat platforms, how they are routed to agents, and how
replies flow back out.

---

## 1. Channel Registry

**File:** `src/channels/registry.ts`

The registry is the canonical source of truth for which chat channels exist and
in what order they appear in the UI.

### ChatChannelId Type

A union of string literal types derived from the ordered tuple:

```ts
export const CHAT_CHANNEL_ORDER = [
  "telegram",
  "whatsapp",
  "discord",
  "irc",
  "googlechat",
  "slack",
  "signal",
  "imessage",
] as const;

export type ChatChannelId = (typeof CHAT_CHANNEL_ORDER)[number];
// = "telegram" | "whatsapp" | "discord" | "irc" | "googlechat" | "slack" | "signal" | "imessage"
```

### Constants

| Constant               | Value        | Purpose                                     |
|------------------------|--------------|---------------------------------------------|
| `CHAT_CHANNEL_ORDER`  | 8-element tuple | Canonical ordering for UI + iteration     |
| `DEFAULT_CHAT_CHANNEL`| `"whatsapp"` | Fallback channel when none is specified      |
| `CHANNEL_IDS`         | Spread of `CHAT_CHANNEL_ORDER` | Mutable array form             |

### Aliases

```ts
export const CHAT_CHANNEL_ALIASES: Record<string, ChatChannelId> = {
  imsg: "imessage",
  "internet-relay-chat": "irc",
  "google-chat": "googlechat",
  gchat: "googlechat",
};
```

### Key Functions

| Function                    | Signature                                              | Purpose |
|-----------------------------|--------------------------------------------------------|---------|
| `normalizeChatChannelId()`  | `(raw?: string \| null) => ChatChannelId \| null`      | Normalizes a raw string to a valid `ChatChannelId` via alias resolution + membership check. |
| `normalizeChannelId()`      | `(raw?: string \| null) => ChatChannelId \| null`      | Alias for `normalizeChatChannelId()` preferred in shared code. |
| `normalizeAnyChannelId()`   | `(raw?: string \| null) => ChannelId \| null`          | Extends normalization to the plugin registry (bundled + external channels). Requires initialized plugin runtime. |
| `listChatChannels()`        | `() => ChatChannelMeta[]`                              | Returns metadata for all 8 core channels in `CHAT_CHANNEL_ORDER`. |
| `getChatChannelMeta()`      | `(id: ChatChannelId) => ChatChannelMeta`               | Returns metadata for a single core channel. |
| `listChatChannelAliases()`  | `() => string[]`                                       | Lists alias keys (e.g. `["imsg", "internet-relay-chat", ...]`). |

### ChannelMeta Type

Defined in `src/channels/plugins/types.core.ts`:

```ts
export type ChannelMeta = {
  id: ChannelId;
  label: string;            // "Telegram", "WhatsApp", etc.
  selectionLabel: string;   // "Telegram (Bot API)", "WhatsApp (QR link)", etc.
  docsPath: string;         // "/channels/telegram"
  docsLabel?: string;
  blurb: string;            // Short description for the UI
  order?: number;
  aliases?: string[];
  detailLabel?: string;     // "Telegram Bot", "WhatsApp Web", etc.
  systemImage?: string;     // SF Symbol name for iOS/macOS UI
  selectionDocsPrefix?: string;
  selectionDocsOmitLabel?: boolean;
  selectionExtras?: string[];
  showConfigured?: boolean;
  quickstartAllowFrom?: boolean;
  forceAccountBinding?: boolean;
  preferSessionLookupForAnnounceTarget?: boolean;
  preferOver?: string[];
};
```

---

## 2. Channel Plugin Architecture

**Files:** `src/channels/plugins/types.plugin.ts`, `src/channels/plugins/types.adapters.ts`, `src/channels/plugins/types.core.ts`

### ChannelPlugin Interface

Each channel (bundled or external) implements the `ChannelPlugin` interface.
Adapters are optional except where noted; channels opt in to capabilities.

```ts
export type ChannelPlugin<ResolvedAccount = any, Probe = unknown, Audit = unknown> = {
  id: ChannelId;                              // REQUIRED: "telegram", "whatsapp", etc.
  meta: ChannelMeta;                          // REQUIRED: display metadata
  capabilities: ChannelCapabilities;          // REQUIRED: what this channel supports

  defaults?: { queue?: { debounceMs?: number } };
  reload?: { configPrefixes: string[]; noopPrefixes?: string[] };

  // --- Adapter slots (all optional) ---
  onboarding?: ChannelOnboardingAdapter;      // CLI setup wizard hooks
  config: ChannelConfigAdapter<ResolvedAccount>; // Account config + listing
  configSchema?: ChannelConfigSchema;         // JSON schema for config validation
  setup?: ChannelSetupAdapter;                // Account setup / config application
  pairing?: ChannelPairingAdapter;            // Pairing approval + ID normalization
  security?: ChannelSecurityAdapter<ResolvedAccount>; // DM policy + warnings
  groups?: ChannelGroupAdapter;               // Group mention/tool policy
  mentions?: ChannelMentionAdapter;           // Mention stripping patterns
  outbound?: ChannelOutboundAdapter;          // Delivery: sendText/sendMedia/sendPoll
  status?: ChannelStatusAdapter<ResolvedAccount, Probe, Audit>; // Probe/audit/snapshot
  gatewayMethods?: string[];                  // Gateway HTTP method list
  gateway?: ChannelGatewayAdapter<ResolvedAccount>; // Start/stop/login/logout
  auth?: ChannelAuthAdapter;                  // Login flow
  elevated?: ChannelElevatedAdapter;          // Elevated DM allowFrom fallback
  commands?: ChannelCommandAdapter;           // Command enforcement behavior
  streaming?: ChannelStreamingAdapter;        // Block-streaming coalesce defaults
  threading?: ChannelThreadingAdapter;        // Reply-to mode + tool context builder
  messaging?: ChannelMessagingAdapter;        // Target normalization + formatting
  agentPrompt?: ChannelAgentPromptAdapter;    // Inject channel-specific tool hints
  directory?: ChannelDirectoryAdapter;        // Peer/group/member listing
  resolver?: ChannelResolverAdapter;          // Resolve target names to IDs
  actions?: ChannelMessageActionAdapter;      // Message action dispatch
  heartbeat?: ChannelHeartbeatAdapter;        // Readiness check + recipient resolution
  agentTools?: ChannelAgentToolFactory | ChannelAgentTool[]; // Channel-owned agent tools
};
```

### ChannelCapabilities

Declared on each plugin to indicate what features the channel supports:

```ts
export type ChannelCapabilities = {
  chatTypes: Array<ChatType | "thread">;  // "direct" | "group" | "channel" | "thread"
  polls?: boolean;
  reactions?: boolean;
  edit?: boolean;
  unsend?: boolean;
  reply?: boolean;
  effects?: boolean;
  groupManagement?: boolean;
  threads?: boolean;
  media?: boolean;
  nativeCommands?: boolean;    // Slash commands / native bot commands
  blockStreaming?: boolean;    // Channel blocks streaming; must use coalesced delivery
};
```

**Capabilities by channel:**

| Channel      | chatTypes                        | polls | reactions | media | nativeCommands | threads | blockStreaming |
|--------------|----------------------------------|-------|-----------|-------|----------------|---------|---------------|
| Telegram     | direct, group, channel, thread   |       |           |       | yes            |         | yes           |
| WhatsApp     | direct, group                    | yes   | yes       | yes   |                |         |               |
| Discord      | direct, channel, thread          | yes   | yes       | yes   | yes            | yes     |               |
| IRC          | direct, group                    |       |           | yes   |                |         | yes           |
| Google Chat  | direct, group, thread            |       | yes       | yes   |                | yes     | yes           |
| Slack        | direct, channel, thread          |       | yes       | yes   | yes            | yes     |               |
| Signal       | direct, group                    |       | yes       | yes   |                |         |               |
| iMessage     | direct, group                    |       | yes       | yes   |                |         |               |

### Key Adapter Types

**ChannelOutboundAdapter** (`src/channels/plugins/types.adapters.ts`):

```ts
export type ChannelOutboundAdapter = {
  deliveryMode: "direct" | "gateway" | "hybrid";
  chunker?: ((text: string, limit: number) => string[]) | null;
  chunkerMode?: "text" | "markdown";
  textChunkLimit?: number;
  pollMaxOptions?: number;
  resolveTarget?: (params: { cfg?; to?; allowFrom?; accountId?; mode? })
    => { ok: true; to: string } | { ok: false; error: Error };
  sendText?: (ctx: ChannelOutboundContext) => Promise<OutboundDeliveryResult>;
  sendMedia?: (ctx: ChannelOutboundContext) => Promise<OutboundDeliveryResult>;
  sendPayload?: (ctx: ChannelOutboundPayloadContext) => Promise<OutboundDeliveryResult>;
  sendPoll?: (ctx: ChannelPollContext) => Promise<ChannelPollResult>;
};
```

**ChannelConfigAdapter** (`src/channels/plugins/types.adapters.ts`):

```ts
export type ChannelConfigAdapter<ResolvedAccount> = {
  listAccountIds: (cfg: OpenClawConfig) => string[];
  resolveAccount: (cfg: OpenClawConfig, accountId?: string | null) => ResolvedAccount;
  defaultAccountId?: (cfg: OpenClawConfig) => string;
  setAccountEnabled?: (params: { cfg; accountId; enabled }) => OpenClawConfig;
  deleteAccount?: (params: { cfg; accountId }) => OpenClawConfig;
  isEnabled?: (account: ResolvedAccount, cfg: OpenClawConfig) => boolean;
  disabledReason?: (account: ResolvedAccount, cfg: OpenClawConfig) => string;
  isConfigured?: (account: ResolvedAccount, cfg: OpenClawConfig) => boolean | Promise<boolean>;
  unconfiguredReason?: (account: ResolvedAccount, cfg: OpenClawConfig) => string;
  describeAccount?: (account: ResolvedAccount, cfg: OpenClawConfig) => ChannelAccountSnapshot;
  resolveAllowFrom?: (params: { cfg; accountId? }) => string[] | undefined;
  formatAllowFrom?: (params: { cfg; accountId?; allowFrom }) => string[];
};
```

**ChannelGroupAdapter:**

```ts
export type ChannelGroupAdapter = {
  resolveRequireMention?: (params: ChannelGroupContext) => boolean | undefined;
  resolveGroupIntroHint?: (params: ChannelGroupContext) => string | undefined;
  resolveToolPolicy?: (params: ChannelGroupContext) => GroupToolPolicyConfig | undefined;
};
```

**ChannelThreadingAdapter:**

```ts
export type ChannelThreadingAdapter = {
  resolveReplyToMode?: (params: { cfg; accountId?; chatType? }) => "off" | "first" | "all";
  allowExplicitReplyTagsWhenOff?: boolean;
  buildToolContext?: (params: { cfg; accountId?; context; hasRepliedRef? })
    => ChannelThreadingToolContext | undefined;
};
```

---

## 3. Channel Dock

**File:** `src/channels/dock.ts`

The dock layer provides lightweight channel metadata and behavior for shared code
paths without importing the heavyweight plugin implementations (monitors, web
login, etc.).

### ChannelDock Type

```ts
export type ChannelDock = {
  id: ChannelId;
  capabilities: ChannelCapabilities;
  commands?: ChannelCommandAdapter;
  outbound?: { textChunkLimit?: number };
  streaming?: ChannelDockStreaming;
  elevated?: ChannelElevatedAdapter;
  config?: {
    resolveAllowFrom?: (params: { cfg; accountId? }) => Array<string | number> | undefined;
    formatAllowFrom?: (params: { cfg; accountId?; allowFrom }) => string[];
  };
  groups?: ChannelGroupAdapter;
  mentions?: ChannelMentionAdapter;
  threading?: ChannelThreadingAdapter;
  agentPrompt?: ChannelAgentPromptAdapter;
};
```

### DOCKS Record

A `Record<ChatChannelId, ChannelDock>` mapping every core channel ID to its dock
configuration. Key per-channel differences:

| Channel      | textChunkLimit | Streaming coalesce | Notable config/behavior |
|--------------|----------------|-------------------|-------------------------|
| Telegram     | 4000           | --                | `resolveReplyToMode`, forum thread support |
| WhatsApp     | 4000           | --                | `enforceOwnerForCommands`, E164 normalization, group intro hint |
| Discord      | 2000           | minChars=1500, idleMs=1000 | `allowFromFallback` elevated, `resolveReplyToMode` |
| IRC          | 350            | minChars=300, idleMs=1000  | Case-insensitive group ID matching, allowFrom prefix stripping |
| Google Chat  | 4000           | --                | `resolveReplyToMode`, thread support |
| Slack        | 4000           | minChars=1500, idleMs=1000 | `resolveReplyToMode` from account config, `allowExplicitReplyTagsWhenOff` |
| Signal       | 4000           | minChars=1500, idleMs=1000 | E164 normalization in allowFrom |
| iMessage     | 4000           | --                | Group mention gating |

### Dock Lifecycle

- **Core channels** are statically defined in the `DOCKS` record.
- **Plugin channels** (registered externally) get their dock built on-demand via
  `buildDockFromPlugin()`, which extracts the lightweight subset from the full
  `ChannelPlugin`.
- `listChannelDocks()` merges both sets, sorts by `order` / `CHAT_CHANNEL_ORDER`
  index, and returns a unified list.
- `getChannelDock(id)` returns a single dock; checks core docks first, then
  falls back to the plugin registry.

---

## 4. Message Routing

**Directory:** `src/routing/`

### resolve-route.ts -- The Routing Engine

**`resolveAgentRoute(input: ResolveAgentRouteInput): ResolvedAgentRoute`**

This is the main routing function. Given a channel, account, peer, guild, team,
and role information, it determines which agent should handle the message and
what session key to use.

#### Input Type

```ts
export type ResolveAgentRouteInput = {
  cfg: OpenClawConfig;
  channel: string;
  accountId?: string | null;
  peer?: RoutePeer | null;              // { kind: ChatType; id: string }
  parentPeer?: RoutePeer | null;        // Thread parent for binding inheritance
  guildId?: string | null;              // Discord server ID
  teamId?: string | null;              // Slack workspace ID
  memberRoleIds?: string[];             // Discord member role IDs
};
```

#### Output Type

```ts
export type ResolvedAgentRoute = {
  agentId: string;
  channel: string;
  accountId: string;
  sessionKey: string;           // Full session key for persistence
  mainSessionKey: string;       // Collapsed session key (agent:id:main)
  matchedBy:
    | "binding.peer"            // Matched a specific peer binding
    | "binding.peer.parent"     // Matched via parent peer (thread inheritance)
    | "binding.guild+roles"     // Guild + role combination
    | "binding.guild"           // Guild only
    | "binding.team"            // Team (Slack workspace)
    | "binding.account"         // Account-specific binding
    | "binding.channel"         // Channel-wide wildcard binding
    | "default";                // No binding matched, used default agent
};
```

#### Tiered Precedence

Bindings are evaluated in a strict priority order. The first matching tier wins:

```
Tier 1: binding.peer           -- exact peer match (group:123456, direct:user42)
Tier 2: binding.peer.parent    -- parent peer for thread inheritance
Tier 3: binding.guild+roles    -- guild ID + at least one matching role
Tier 4: binding.guild          -- guild ID only (no role constraint)
Tier 5: binding.team           -- team ID (Slack workspace)
Tier 6: binding.account        -- non-wildcard account match
Tier 7: binding.channel        -- wildcard account ("*") match
Fallback: default              -- resolveDefaultAgentId(cfg)
```

Each tier filters bindings pre-matched for the channel+account combination
(cached per config object via WeakMap) and checks scope constraints. The
`choose()` helper builds the full `ResolvedAgentRoute` including session key
construction.

```
                       resolveAgentRoute()
                             |
                    getEvaluatedBindingsForChannelAccount()
                    (cached pre-filter by channel + accountId)
                             |
                    +--------+--------+
                    |  For each tier  |
                    |  in priority    |
                    +--------+--------+
                             |
                    matchesBindingScope()
                    (peer, guild, team, roles)
                             |
                    +-yes----+----no--+
                    |                 |
               choose()        next tier
               (build route)         |
                                  default
```

### bindings.ts

**File:** `src/routing/bindings.ts`

Functions for reading and organizing the `cfg.bindings` array:

```ts
// AgentBinding shape (from src/config/types.agents.ts):
export type AgentBinding = {
  agentId: string;
  match: {
    channel: string;
    accountId?: string;
    peer?: { kind: ChatType; id: string };
    guildId?: string;
    teamId?: string;
    roles?: string[];
  };
};
```

| Function                          | Purpose |
|-----------------------------------|---------|
| `listBindings(cfg)`              | Returns `cfg.bindings` as `AgentBinding[]` (or empty array). |
| `listBoundAccountIds(cfg, channelId)` | Collects distinct non-wildcard account IDs bound to the given channel. |
| `resolveDefaultAgentBoundAccountId(cfg, channelId)` | Returns the first account ID bound to the default agent for a channel. |
| `buildChannelAccountBindings(cfg)` | Builds a `Map<channelId, Map<agentId, accountId[]>>` for cross-referencing. |
| `resolvePreferredAccountId(params)` | Picks the best account ID from bound accounts vs. defaults. |

### session-key.ts

**File:** `src/routing/session-key.ts`

Builds deterministic, hierarchical session keys used for persistence and
concurrency isolation.

#### Key Constants

```ts
export const DEFAULT_AGENT_ID = "main";
export const DEFAULT_MAIN_KEY = "main";
export const DEFAULT_ACCOUNT_ID = "default";
```

#### Session Key Format

```
agent:<agentId>:<rest>
```

The `<rest>` portion varies by context:

| Context          | Key Format | Example |
|------------------|------------|---------|
| Main (DM, dmScope=main) | `agent:<agentId>:main` | `agent:main:main` |
| Per-peer DM      | `agent:<agentId>:direct:<peerId>` | `agent:main:direct:user123` |
| Per-channel-peer DM | `agent:<agentId>:<channel>:direct:<peerId>` | `agent:main:telegram:direct:user123` |
| Per-account-channel-peer DM | `agent:<agentId>:<channel>:<accountId>:direct:<peerId>` | `agent:main:telegram:default:direct:user123` |
| Group            | `agent:<agentId>:<channel>:group:<peerId>` | `agent:main:discord:channel:123456789` |
| Thread           | `<parentKey>:thread:<threadId>` | `agent:main:slack:channel:C01:thread:ts123` |

#### buildAgentPeerSessionKey()

```ts
export function buildAgentPeerSessionKey(params: {
  agentId: string;
  mainKey?: string;
  channel: string;
  accountId?: string | null;
  peerKind?: ChatType | null;     // "direct" | "group" | "channel"
  peerId?: string | null;
  identityLinks?: Record<string, string[]>;
  dmScope?: "main" | "per-peer" | "per-channel-peer" | "per-account-channel-peer";
}): string;
```

**dmScope behavior for direct messages:**

- `"main"` (default): All DMs collapse to `agent:<id>:main` -- single shared session.
- `"per-peer"`: `agent:<id>:direct:<peerId>` -- separate session per peer across all channels.
- `"per-channel-peer"`: `agent:<id>:<channel>:direct:<peerId>` -- separate per channel+peer.
- `"per-account-channel-peer"`: `agent:<id>:<channel>:<accountId>:direct:<peerId>` -- fully isolated.

**Identity links:** When `dmScope` is not `"main"`, the system checks
`identityLinks` to map a peer ID to a canonical identity before building the key.
This allows the same person messaging from different channels/numbers to share a
session.

#### Other Session Key Functions

| Function | Purpose |
|----------|---------|
| `buildAgentMainSessionKey()` | Builds `agent:<agentId>:main` |
| `resolveAgentIdFromSessionKey()` | Extracts agent ID from a session key |
| `classifySessionKeyShape()` | Returns `"missing"`, `"agent"`, `"legacy_or_alias"`, or `"malformed_agent"` |
| `normalizeAgentId()` | Lowercases, strips invalid chars, max 64 chars |
| `normalizeAccountId()` | Same normalization for account IDs |
| `toAgentStoreSessionKey()` / `toAgentRequestSessionKey()` | Convert between request-level and store-level keys |
| `buildGroupHistoryKey()` | Builds `<channel>:<accountId>:<peerKind>:<peerId>` for group history storage |
| `resolveThreadSessionKeys()` | Appends `:thread:<threadId>` suffix for thread sessions |

---

## 5. Session Management

**Directory:** `src/sessions/`

### session-key-utils.ts

**File:** `src/sessions/session-key-utils.ts`

Low-level session key parsing shared across the codebase:

```ts
export type ParsedAgentSessionKey = {
  agentId: string;
  rest: string;           // Everything after "agent:<agentId>:"
};

export function parseAgentSessionKey(sessionKey: string | undefined | null): ParsedAgentSessionKey | null;
```

Parsing rules:
- Split by `":"`, filter empty parts
- Must have at least 3 parts: `["agent", <agentId>, <rest...>]`
- First part must be `"agent"`

Utility predicates:

| Function | Checks for |
|----------|------------|
| `isSubagentSessionKey()` | Key starts with `subagent:` or rest starts with `subagent:` |
| `isAcpSessionKey()` | Key starts with `acp:` or rest starts with `acp:` |
| `isCronRunSessionKey()` | Rest matches `/^cron:[^:]+:run:[^:]+$/` |
| `resolveThreadParentSessionKey()` | Strips trailing `:thread:*` or `:topic:*` suffix to find parent |

### send-policy.ts

**File:** `src/sessions/send-policy.ts`

Determines whether outbound messages are allowed for a given session:

```ts
export type SessionSendPolicyDecision = "allow" | "deny";

export function resolveSendPolicy(params: {
  cfg: OpenClawConfig;
  entry?: SessionEntry;
  sessionKey?: string;
  channel?: string;
  chatType?: SessionChatType;
}): SessionSendPolicyDecision;
```

**Resolution order:**
1. **Per-session override:** If the session entry has `sendPolicy`, use it.
2. **Rule matching:** Iterate `cfg.session.sendPolicy.rules`, matching on
   `channel`, `chatType`, and `keyPrefix`. First `deny` wins immediately;
   any `allow` is noted.
3. **Default:** Falls back to `cfg.session.sendPolicy.default`, then `"allow"`.

### model-overrides.ts

**File:** `src/sessions/model-overrides.ts`

Applies per-session model/provider overrides:

```ts
export type ModelOverrideSelection = {
  provider: string;
  model: string;
  isDefault?: boolean;
};

export function applyModelOverrideToSessionEntry(params: {
  entry: SessionEntry;
  selection: ModelOverrideSelection;
  profileOverride?: string;
  profileOverrideSource?: "auto" | "user";
}): { updated: boolean };
```

When `isDefault` is true, existing overrides are cleared. Otherwise,
`providerOverride` and `modelOverride` are set on the session entry. Auth
profile overrides are managed in tandem.

### level-overrides.ts

**File:** `src/sessions/level-overrides.ts`

Manages verbose/thinking level per session:

```ts
export function parseVerboseOverride(raw: unknown):
  | { ok: true; value: VerboseLevel | null | undefined }
  | { ok: false; error: string };

export function applyVerboseOverride(entry: SessionEntry, level: VerboseLevel | null | undefined): void;
```

---

## 6. Message Context (MsgContext)

**File:** `src/auto-reply/templating.ts`

The `MsgContext` type is the universal message envelope that flows through the
entire inbound pipeline. Every field is optional to accommodate different
channels and message types.

### Field Categories

**Message Body:**

| Field              | Type                          | Purpose |
|--------------------|-------------------------------|---------|
| `Body`             | `string`                      | Primary message text |
| `BodyForAgent`     | `string`                      | Agent prompt body (may include envelope/history/context) |
| `RawBody`          | `string`                      | Raw body without structural context (legacy alias for `CommandBody`) |
| `CommandBody`      | `string`                      | Preferred for command detection |
| `BodyForCommands`  | `string`                      | Clean text for command parsing (no history/sender context) |
| `CommandArgs`      | `CommandArgs`                 | Parsed command arguments |

**Routing & Identity:**

| Field              | Type     | Purpose |
|--------------------|----------|---------|
| `SessionKey`       | `string` | Resolved session key for this conversation |
| `From`             | `string` | Sender address/JID |
| `To`               | `string` | Recipient address/JID (usually the bot) |
| `AccountId`        | `string` | Provider account ID (multi-account support) |
| `OriginatingChannel` | `ChannelId \| InternalMessageChannel` | Reply routing: send replies to this channel |
| `OriginatingTo`    | `string` | Reply routing: send replies to this target |

**Sender Information:**

| Field              | Type     | Purpose |
|--------------------|----------|---------|
| `SenderId`         | `string` | Unique sender identifier |
| `SenderName`       | `string` | Display name |
| `SenderUsername`    | `string` | Username (without @) |
| `SenderE164`       | `string` | Phone number in E.164 format |
| `SenderTag`        | `string` | User tag (e.g., Discord discriminator) |

**Chat Metadata:**

| Field              | Type               | Purpose |
|--------------------|--------------------|---------|
| `ChatType`         | `string`           | `"direct"`, `"group"`, `"channel"` |
| `MessageSid`       | `string`           | Message ID |
| `MessageSidFull`   | `string`           | Full provider-specific message ID |
| `Timestamp`        | `number`           | Message timestamp |
| `MessageThreadId`  | `string \| number` | Thread identifier (Telegram topic, Matrix thread) |
| `IsForum`          | `boolean`          | Telegram forum supergroup marker |

**Reply Context:**

| Field              | Type     | Purpose |
|--------------------|----------|---------|
| `ReplyToId`        | `string` | ID of the message being replied to |
| `ReplyToIdFull`    | `string` | Full provider-specific reply-to ID |
| `ReplyToBody`      | `string` | Text of the message being replied to |
| `ReplyToSender`    | `string` | Sender of the message being replied to |

**Group Context:**

| Field               | Type     | Purpose |
|---------------------|----------|---------|
| `GroupSubject`      | `string` | Group name/subject |
| `GroupChannel`      | `string` | Human label for channel-like groups (e.g., `#general`) |
| `GroupSpace`        | `string` | Workspace/space identifier |
| `GroupMembers`      | `string` | Serialized member list |
| `GroupSystemPrompt` | `string` | Per-group system prompt |

**Gating & Commands:**

| Field               | Type      | Purpose |
|---------------------|-----------|---------|
| `WasMentioned`      | `boolean` | Whether the bot was @-mentioned |
| `CommandAuthorized`  | `boolean` | Whether the sender can execute control commands |
| `CommandSource`     | `"text" \| "native"` | How the command was invoked |

**Media:**

| Field              | Type       | Purpose |
|--------------------|------------|---------|
| `MediaPath`        | `string`   | Local path to downloaded media |
| `MediaUrl`         | `string`   | Remote URL of media |
| `MediaType`        | `string`   | MIME type |
| `MediaPaths`       | `string[]` | Multiple media paths |
| `MediaUrls`        | `string[]` | Multiple media URLs |
| `MediaTypes`       | `string[]` | Multiple MIME types |

**Understanding & History:**

| Field                   | Type                         | Purpose |
|-------------------------|------------------------------|---------|
| `InboundHistory`        | `Array<{sender, body, timestamp?}>` | Recent chat history for context |
| `ThreadHistoryBody`     | `string`                     | Full thread history for new thread sessions |
| `MediaUnderstanding`    | `MediaUnderstandingOutput[]` | AI-processed media descriptions |
| `MediaUnderstandingDecisions` | `MediaUnderstandingDecision[]` | Decisions about media processing |
| `LinkUnderstanding`     | `string[]`                   | AI-processed link summaries |

### FinalizedMsgContext

After `finalizeInboundContext()`, `CommandAuthorized` is guaranteed to be a
`boolean` (default-deny: `false` if missing):

```ts
export type FinalizedMsgContext = Omit<MsgContext, "CommandAuthorized"> & {
  CommandAuthorized: boolean;
};
```

---

## 7. Security and Gating

### Allowlist Matching

**File:** `src/channels/allowlist-match.ts`

```ts
export type AllowlistMatchSource =
  | "wildcard"       // "*" matches everything
  | "id"             // Direct ID match
  | "name"           // Display name match
  | "tag"            // User tag match
  | "username"       // Username match
  | "prefixed-id"    // "telegram:123456" style
  | "prefixed-user"  // "user:alice" style
  | "prefixed-name"  // "name:Alice" style
  | "slug"           // Slug-normalized match
  | "localpart";     // Email-style localpart match

export type AllowlistMatch<TSource extends string = AllowlistMatchSource> = {
  allowed: boolean;
  matchKey?: string;        // The entry that matched
  matchSource?: TSource;    // How it matched
};
```

Each channel's dock defines `resolveAllowFrom()` and `formatAllowFrom()` to read
the allowlist from config and normalize entries (stripping prefixes, normalizing
phone numbers, etc.).

### Mention Gating

**File:** `src/channels/mention-gating.ts`

Controls whether group messages are processed without a bot mention:

```ts
export function resolveMentionGating(params: MentionGateParams): MentionGateResult;
```

```ts
type MentionGateParams = {
  requireMention: boolean;    // Config: require mention in groups?
  canDetectMention: boolean;  // Does this channel support mention detection?
  wasMentioned: boolean;      // Was the bot actually mentioned?
  implicitMention?: boolean;  // Was there an implicit mention (reply, etc.)?
  shouldBypassMention?: boolean; // Override: bypass the gate?
};

type MentionGateResult = {
  effectiveWasMentioned: boolean;  // true if mentioned, implicit, or bypassed
  shouldSkip: boolean;             // true if message should be dropped
};
```

Logic: `shouldSkip = requireMention && canDetectMention && !effectiveWasMentioned`

The extended variant `resolveMentionGatingWithBypass()` automatically computes
`shouldBypassMention` when a group message has a recognized control command from
an authorized sender:

```ts
shouldBypassMention =
  isGroup &&
  requireMention &&
  !wasMentioned &&
  !hasAnyMention &&
  allowTextCommands &&
  commandAuthorized &&
  hasControlCommand;
```

### Command Gating

**File:** `src/channels/command-gating.ts`

Controls who can execute control commands:

```ts
export type CommandAuthorizer = {
  configured: boolean;    // Is this authorizer configured (e.g., access group exists)?
  allowed: boolean;       // Does the sender pass this authorizer?
};

export function resolveControlCommandGate(params: {
  useAccessGroups: boolean;
  authorizers: CommandAuthorizer[];
  allowTextCommands: boolean;
  hasControlCommand: boolean;
  modeWhenAccessGroupsOff?: CommandGatingModeWhenAccessGroupsOff;
}): { commandAuthorized: boolean; shouldBlock: boolean };
```

**Mode when access groups are off:**
- `"allow"` (default): All senders are authorized.
- `"deny"`: No senders are authorized.
- `"configured"`: Check if any authorizer is configured; if none, allow.

### Sender Identity Validation

**File:** `src/channels/sender-identity.ts`

Validates that group messages carry proper sender identity:

```ts
export function validateSenderIdentity(ctx: MsgContext): string[];
```

Checks:
- Non-direct messages must have at least one of: `SenderId`, `SenderName`,
  `SenderUsername`, `SenderE164`.
- `SenderE164` must match `/^\+\d{3,}$/`.
- `SenderUsername` must not contain `@` or whitespace.
- `SenderId` must not be set-but-empty.

---

## 8. Channel Implementations

### Telegram

**Directory:** `src/telegram/`

- **Framework:** grammY (Bot API)
- **Entry:** `bot.ts` -- Creates `Bot` instance with `sequentialize()` middleware
  for update ordering, `apiThrottler()` for rate limiting.
- **Handlers:** `bot-handlers.ts` -- Registers text, photo, document, sticker,
  voice, video, and native command handlers. Implements debouncing for text
  fragments.
- **Context:** `bot-message-context.ts` -- Builds `MsgContext` from grammY
  update context.
- **Dispatch:** `bot-message-dispatch.ts` -- Dispatches to the agent pipeline.
- **Delivery:** `bot/delivery.ts` -- Sends replies with markdown-to-HTML
  conversion, 4000-char chunking.
- **Deduplication:** `bot-updates.ts` -- `createTelegramUpdateDedupe()` prevents
  processing duplicate updates.
- **Forum threads:** `bot/helpers.ts` -- `resolveTelegramForumThreadId()` maps
  Telegram topic IDs to thread sessions.

### Discord

**Directory:** `src/discord/`

- **Framework:** discord.js
- **Monitor:** `monitor/message-handler.ts` -- Debounces by `channel:author`
  pair. Processes text, embeds, attachments.
- **Preflight:** `monitor/message-handler.preflight.ts` -- Pre-flight checks
  before full processing.
- **Process:** `monitor/message-handler.process.ts` -- Full message processing
  pipeline.
- **Allowlist:** `monitor/allow-list.ts` -- DM allowlist checking.
- **Provider:** `monitor/provider.ts` -- Creates and manages the discord.js
  `Client`.
- **Delivery:** `monitor/reply-delivery.ts` -- Sends replies with 2000-char
  chunks.
- **Sender ID:** `monitor/sender-identity.ts` -- Extracts sender identity from
  Discord messages.
- **Threading:** `monitor/threading.ts` -- Thread creation and message routing.
- **Presence:** `monitor/presence.ts` -- Online/offline presence tracking.
- **Native commands:** `monitor/native-command.ts` -- Slash command registration
  and handling.
- **Gateway:** `monitor/gateway-plugin.ts`, `monitor/gateway-registry.ts` --
  Gateway client integration.

### Slack

**Directory:** `src/slack/`

- **Framework:** Bolt SDK with Socket Mode
- **Monitor:** `monitor/message-handler.ts` -- Debounces by
  `channel:thread:user` triple.
- **Context:** `monitor/context.ts` -- Builds `MsgContext` from Slack events.
- **Thread resolution:** `monitor/thread-resolution.ts` -- Maps Slack threads to
  session keys.
- **Media:** `monitor/media.ts` -- Downloads files from Slack.
- **Provider:** `monitor/provider.ts` -- Creates Bolt app with Socket Mode.
- **Commands:** `monitor/commands.ts` -- Handles text commands.
- **Slash commands:** `monitor/slash.ts` -- Slash command registration/dispatch.
- **Allowlist:** `monitor/allow-list.ts` -- DM allowlist checking.
- **Events:** `monitor/events.ts` -- Event subscription setup.
- **Replies:** `monitor/replies.ts` -- Reply delivery with thread support.
- **Channel config:** `monitor/channel-config.ts` -- Per-channel configuration.

### Signal

**Directory:** `src/signal/`

- **Framework:** Signal CLI RPC (REST API)
- **Monitor:** `monitor/event-handler.ts` -- Debounces by `group/peer+sender`
  key.
- **Mentions:** `monitor/mentions.ts` -- Signal mention detection.

### WhatsApp

**Directories:** `src/whatsapp/` + `src/web/`

- **Framework:** Baileys library (WhatsApp Web protocol)
- **Normalization:** `whatsapp/normalize.ts` -- JID parsing for user JIDs
  (`@s.whatsapp.net`, `@lid`) and group JIDs (`@g.us`).
- **Inbound pipeline:** `src/web/inbound/`:
  - `access-control.ts` -- DM policy enforcement and allowlist checking.
  - `dedupe.ts` -- Message deduplication.
  - `media.ts` -- Media download from WhatsApp.
  - `monitor.ts` -- Main inbound handler.
  - `send-api.ts` -- Outbound message sending via Baileys.
  - `extract.ts` -- Message extraction from Baileys protocol buffers.

### iMessage

**Directory:** `src/imessage/`

- **Framework:** BlueBubbles integration
- **Monitor:** `monitor/monitor-provider.ts` -- Polls BlueBubbles for new
  messages.
- **Delivery:** `monitor/deliver.ts` -- Sends replies via BlueBubbles API.
- **Actions:** `src/channels/plugins/bluebubbles-actions.ts` -- Message actions
  (reactions, etc.).

### LINE

**Directory:** `src/line/`

- **Framework:** HTTP webhook handler with signature validation
- **Webhook:** `webhook.ts` -- HTTP request handler, validates
  `X-Line-Signature` header.
- **Signature:** `signature.ts` -- HMAC-SHA256 signature validation.
- **Bot:** `bot.ts` -- Bot initialization, `bot-handlers.ts` -- Event routing.
- **Context:** `bot-message-context.ts` -- Builds `MsgContext` from LINE events.
- **Delivery:** `auto-reply-delivery.ts` -- Reply delivery, `send.ts` --
  Low-level send.
- **Rich content:** `flex-templates.ts`, `template-messages.ts`,
  `markdown-to-line.ts` -- LINE Flex Message formatting.
- **Config:** `accounts.ts`, `config-schema.ts` -- Account configuration.

---

## 9. Inbound Message Flow

The following sequence describes how a message travels from a chat platform to
the agent:

```
  Channel SDK (grammY, discord.js, Bolt, Baileys, etc.)
       |
  [1]  Channel handler receives raw message/event
       |
  [2]  Debouncer combines fragments / batches
       |  (typically ~1500ms window; varies by channel)
       |  Telegram: text fragments
       |  Discord: channel:author debounce
       |  Slack: channel:thread:user debounce
       |  Signal: group/peer+sender debounce
       |
  [3]  Build MsgContext
       |  - Extract sender identity (SenderId, SenderName, SenderUsername, SenderE164)
       |  - Detect mentions (WasMentioned)
       |  - Collect history (InboundHistory)
       |  - Detect commands (hasControlCommand, CommandBody)
       |  - Download/reference media (MediaPath, MediaUrl, MediaType)
       |
  [4]  Gating pipeline (any step can reject the message):
       |
       |  4a. Allowlist check
       |      - resolveAllowFrom() + formatAllowFrom() from channel dock
       |      - AllowlistMatch evaluation
       |
       |  4b. Mention requirement
       |      - resolveMentionGating() / resolveMentionGatingWithBypass()
       |      - Group messages may be skipped if bot not mentioned
       |
       |  4c. Command authorization
       |      - resolveControlCommandGate()
       |      - Unauthorized control commands are blocked
       |
       |  4d. DM policy
       |      - ChannelSecurityAdapter.resolveDmPolicy()
       |      - Open/closed DM policies
       |
  [5]  resolveAgentRoute()
       |  - Match bindings in tiered precedence
       |  - Build sessionKey
       |
  [6]  recordInboundSession()
       |  - Save metadata (lastChannel, lastRoute, etc.)
       |
  [7]  Create reply dispatcher
       |  - Set up typing indicator callbacks
       |
  [8]  dispatchInboundMessage()
       |  - finalizeContext() -> FinalizedMsgContext
       |  - Deliver to agent pipeline
       |
  [9]  Agent processes message
       |  - Streaming: draft updates via channel-specific streams
       |  - Block-streaming channels: coalesce with minChars/idleMs
       |
  [10] deliverReplies()
       |  - Chunk text per channel limits (350-4000 chars)
       |  - Send via channel API (sendText, sendMedia, sendPayload)
       |
  [11] Cleanup
       - Dispose dispatcher
       - Clear typing indicators
```

---

## 10. Outbound Delivery

**Type:** `ChannelOutboundAdapter` (from `src/channels/plugins/types.adapters.ts`)

### Delivery Modes

| Mode       | Description |
|------------|-------------|
| `"direct"` | Channel plugin sends directly via its SDK (most channels). |
| `"gateway"` | Message is sent through the gateway HTTP bridge. |
| `"hybrid"` | Tries direct first; falls back to gateway. |

### Outbound Context

```ts
export type ChannelOutboundContext = {
  cfg: OpenClawConfig;
  to: string;                      // Target JID/channel ID
  text: string;                    // Message text
  mediaUrl?: string;               // Media to attach
  gifPlayback?: boolean;           // Treat media as GIF
  replyToId?: string | null;       // Reply to specific message
  threadId?: string | number | null; // Thread context
  accountId?: string | null;       // Account to send from
  identity?: OutboundIdentity;     // Sender identity for impersonation
  deps?: OutboundSendDeps;         // Runtime dependencies
  silent?: boolean;                // Suppress notifications
};
```

### Text Chunking

Each channel defines a `textChunkLimit` (in the dock or outbound adapter):

| Channel      | Chunk Limit | Notes |
|--------------|-------------|-------|
| Telegram     | 4000        | Markdown converted to HTML for delivery |
| WhatsApp     | 4000        | |
| Discord      | 2000        | |
| IRC          | 350         | Very short due to IRC line limits |
| Google Chat  | 4000        | |
| Slack        | 4000        | Thread support via `threadId` |
| Signal       | 4000        | |
| iMessage     | 4000        | |

Channels can provide a custom `chunker` function or use the default
text/markdown splitter. The `chunkerMode` (`"text"` or `"markdown"`) controls
whether splitting is syntax-aware.

### Block Streaming

Channels that set `blockStreaming: true` (Telegram, IRC, Google Chat) cannot
receive token-by-token streaming. Instead, the system coalesces output into
blocks using `blockStreamingCoalesceDefaults`:

| Channel      | minChars | idleMs | Behavior |
|--------------|----------|--------|----------|
| Discord      | 1500     | 1000   | Edit previous message with accumulated text |
| IRC          | 300      | 1000   | Send accumulated text as new message |
| Slack        | 1500     | 1000   | Edit previous message in thread |
| Signal       | 1500     | 1000   | Send accumulated text as new message |

---

## 11. Normalization Functions

Each channel normalizes target identifiers for consistent routing and delivery:

### WhatsApp (`src/whatsapp/normalize.ts`)

```ts
export function normalizeWhatsAppTarget(value: string): string | null;
```

- Strips `whatsapp:` prefix
- Group JIDs: validates `<digits>-<digits>@g.us` format
- User JIDs: extracts phone from `<digits>:<digits>@s.whatsapp.net` or `<digits>@lid`
- Phone numbers: normalizes to E.164 via `normalizeE164()`
- Rejects unrecognized `@`-containing strings

Helper predicates:
- `isWhatsAppGroupJid(value)` -- checks for `@g.us` suffix
- `isWhatsAppUserTarget(value)` -- checks for `@s.whatsapp.net` or `@lid` suffix

### Per-Channel AllowFrom Normalization (in dock config)

Each channel's dock `formatAllowFrom` normalizes allowlist entries:

| Channel     | Normalization |
|-------------|---------------|
| Telegram    | Strip `telegram:` / `tg:` prefix, lowercase |
| WhatsApp    | `normalizeWhatsAppTarget()` for JIDs, pass-through for `*` |
| Discord     | Lowercase |
| IRC         | Strip `irc:` / `user:` prefix, lowercase |
| Google Chat | Strip `googlechat:` / `google-chat:` / `gchat:` / `user:` / `users/` prefix, lowercase |
| Slack       | Lowercase |
| Signal      | Strip `signal:` prefix, `normalizeE164()` for phone numbers, pass-through for `*` |
| iMessage    | Trim whitespace |

---

## 12. ChatType

**File:** `src/channels/chat-type.ts`

```ts
export type ChatType = "direct" | "group" | "channel";

export function normalizeChatType(raw?: string): ChatType | undefined;
```

Normalization: `"dm"` maps to `"direct"`. The `"channel"` type is used by
Discord and Slack for public/private channel conversations (as distinct from
group DMs). The `"thread"` value appears only in `ChannelCapabilities.chatTypes`
as a capability flag, not as a routing `ChatType`.

---

## 13. File Reference Index

| Component | File Path |
|-----------|-----------|
| Channel Registry | `src/channels/registry.ts` |
| Channel Dock | `src/channels/dock.ts` |
| ChatType | `src/channels/chat-type.ts` |
| Plugin types (barrel) | `src/channels/plugins/types.ts` |
| Plugin interface | `src/channels/plugins/types.plugin.ts` |
| Core types | `src/channels/plugins/types.core.ts` |
| Adapter types | `src/channels/plugins/types.adapters.ts` |
| Allowlist match | `src/channels/allowlist-match.ts` |
| Mention gating | `src/channels/mention-gating.ts` |
| Command gating | `src/channels/command-gating.ts` |
| Sender identity | `src/channels/sender-identity.ts` |
| Group/mention policies | `src/channels/plugins/group-mentions.ts` |
| Route resolution | `src/routing/resolve-route.ts` |
| Bindings | `src/routing/bindings.ts` |
| Session key building | `src/routing/session-key.ts` |
| Session key parsing | `src/sessions/session-key-utils.ts` |
| Send policy | `src/sessions/send-policy.ts` |
| Model overrides | `src/sessions/model-overrides.ts` |
| Level overrides | `src/sessions/level-overrides.ts` |
| MsgContext / templating | `src/auto-reply/templating.ts` |
| Agent binding type | `src/config/types.agents.ts` |
| Telegram bot | `src/telegram/bot.ts` |
| Telegram handlers | `src/telegram/bot-handlers.ts` |
| Telegram context | `src/telegram/bot-message-context.ts` |
| Telegram dispatch | `src/telegram/bot-message-dispatch.ts` |
| Telegram delivery | `src/telegram/bot/delivery.ts` |
| Discord handler | `src/discord/monitor/message-handler.ts` |
| Discord allowlist | `src/discord/monitor/allow-list.ts` |
| Discord provider | `src/discord/monitor/provider.ts` |
| Slack handler | `src/slack/monitor/message-handler.ts` |
| Slack provider | `src/slack/monitor/provider.ts` |
| Signal handler | `src/signal/monitor/event-handler.ts` |
| WhatsApp normalize | `src/whatsapp/normalize.ts` |
| WhatsApp inbound | `src/web/inbound/monitor.ts` |
| WhatsApp access control | `src/web/inbound/access-control.ts` |
| WhatsApp send | `src/web/inbound/send-api.ts` |
| iMessage monitor | `src/imessage/monitor/monitor-provider.ts` |
| LINE webhook | `src/line/webhook.ts` |
| LINE signature | `src/line/signature.ts` |
| LINE bot handlers | `src/line/bot-handlers.ts` |
