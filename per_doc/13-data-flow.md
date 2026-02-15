# Data Flow

## Overview

This document provides end-to-end data flow diagrams and message lifecycle documentation for the OpenClaw system. Each section traces through actual code paths with file and function references, illustrating how data moves from external inputs through internal processing to outbound responses. The diagrams use ASCII art to visualize each flow.

---

## 1. Inbound Message Flow (External Channel to AI Response)

### Full ASCII Flow Diagram

```
[External Message Platform]
  (Telegram webhook / Discord gateway / WhatsApp Web / Signal REST / Slack Socket / iMessage / LINE / IRC / Google Chat)
        |
        v
[Channel Plugin Handler]                          src/channels/plugins/<channel>/
  |-- Normalize platform payload to common format
  |-- Extract: text, sender info, thread/reply context, attachments
  |-- Detect @mentions, chat type (DM vs group vs channel)
  |-- Resolve account ID from channel config
        |
        v
[Inbound Debouncer]                                src/auto-reply/inbound-debounce.ts
  |-- createInboundDebouncer()
  |-- Key = channel:sender (or channel:thread:sender for threaded chats)
  |-- Accumulates fragments within configurable window (cfg.messages.inbound.debounceMs)
  |-- resolveInboundDebounceMs(): per-channel overrides via cfg.messages.inbound.byChannel
  |-- On timeout: flushBuffer() emits combined payload via onFlush callback
        |
        v
[Build MsgContext]                                  src/auto-reply/templating.ts
  |-- SenderId, SenderName, SenderUsername, SenderE164
  |-- WasMentioned (boolean)
  |-- AllowlistMatch (allowlist-match.ts)
  |-- InboundHistory[] (multi-message batch from debouncer)
  |-- ChatType: "direct" | "group" | "channel"
  |-- Control command detection (/reset, /model, /think, /verbose, /status, /compact)
  |-- MediaType, MediaUrl, MediaTypes[] (attachments)
  |-- Body, RawBody, BodyForCommands
  |-- MessageThreadId (for threaded conversations)
  |-- OriginatingChannel, OriginatingTo (for cross-channel routing)
        |
        v
[Gating Pipeline]                                   (checked in order, each can block)
  |
  |-- 1. Allowlist Check                            src/channels/allowlists/
  |     |-- Match sender by: id, name, username, E164, tag, slug, prefixed-id
  |     |-- AllowlistMatchSource: "wildcard"|"id"|"name"|"tag"|"username"|etc.
  |     +-- If not allowed: silently drop message
  |
  |-- 2. Mention Gating                            src/channels/mention-gating.ts
  |     |-- resolveMentionGatingWithBypass()
  |     |-- Groups only: was bot @mentioned?
  |     |-- Bypass: authorized control commands can pass without mention
  |     |-- implicitMention flag for DMs and replies
  |     +-- If shouldSkip: silently drop
  |
  |-- 3. Command Authorization                     src/channels/command-gating.ts
  |     |-- resolveControlCommandGate()
  |     |-- Checks access groups (useAccessGroups flag)
  |     |-- CommandAuthorizer[]: configured + allowed
  |     |-- modeWhenAccessGroupsOff: "allow"|"deny"|"configured"
  |     +-- If shouldBlock: drop with optional error
  |
  +-- 4. DM Policy                                 src/sessions/send-policy.ts
        |-- resolveSendPolicy()
        |-- Modes: allow, block, pairing-only
        +-- Pairing check against pairing-store.ts
        |
        v
[Resolve Agent Route]                               src/routing/resolve-route.ts
  |-- resolveAgentRoute(input)
  |-- Loads bindings from cfg.bindings via listBindings()
  |-- Filters by channel + accountId match
  |-- Evaluates binding tiers in strict precedence order:
  |     1. binding.peer          (exact peer match: kind + id)
  |     2. binding.peer.parent   (thread parent peer inheritance)
  |     3. binding.guild+roles   (guild + Discord role match)
  |     4. binding.guild         (guild-only match)
  |     5. binding.team          (team match, e.g. Slack workspace)
  |     6. binding.account       (specific account, not wildcard)
  |     7. binding.channel       (wildcard account match)
  |-- Falls back to default agent via resolveDefaultAgentId()
  |-- Calls pickFirstExistingAgentId() to validate agent exists in cfg.agents.list
  |
  |-- Build Session Key                             src/routing/session-key.ts
  |     |-- buildAgentPeerSessionKey()
  |     |-- Format: agent:{agentId}:{scope-specific-suffix}
  |     |-- DM scopes (cfg.session.dmScope):
  |     |     "main"                     -> agent:{agentId}:main
  |     |     "per-peer"                 -> agent:{agentId}:direct:{peerId}
  |     |     "per-channel-peer"         -> agent:{agentId}:{channel}:direct:{peerId}
  |     |     "per-account-channel-peer" -> agent:{agentId}:{channel}:{accountId}:direct:{peerId}
  |     |-- Group/channel chats:         -> agent:{agentId}:{channel}:{chatType}:{peerId}
  |     |-- Identity links: resolveLinkedPeerId() collapses cross-channel identities
  |     +-- Thread suffix: resolveThreadSessionKeys() appends :thread:{threadId}
  |
  +-- Returns ResolvedAgentRoute:
        { agentId, channel, accountId, sessionKey, mainSessionKey, matchedBy }
        |
        v
[Record Session]                                     src/config/sessions.ts
  |-- updateSessionStore(): save inbound metadata to session store JSON
  |-- Updates lastRoute: { channel, to } for reply targeting
  |-- Stores session entry with timestamp and model info
        |
        v
[Media Understanding]                                src/media-understanding/
  |-- applyMediaUnderstanding()                      src/media-understanding/apply.ts
  |-- For each attachment in normalizedAttachments:
  |     |-- resolveScopeDecision()                   src/media-understanding/resolve.ts
  |     |     Checks scope config per chatType/channel/sessionKey
  |     |
  |     |-- resolveModelEntries()                    src/media-understanding/resolve.ts
  |     |     Resolves configured model(s) for each capability
  |     |
  |     |-- Image attachments:
  |     |     Provider registry: Anthropic, OpenAI, Google, Groq, Minimax, z.ai
  |     |     -> vision model describes image content
  |     |     -> output appended to MsgContext.Body
  |     |
  |     |-- Audio attachments:
  |     |     Providers: Deepgram (default), OpenAI Whisper, Google
  |     |     -> audio transcribed to text
  |     |     -> transcript prepended as [Audio transcript: ...]
  |     |
  |     |-- Video attachments:
  |     |     -> Frame extraction (ffmpeg or provider)
  |     |     -> Frame descriptions combined
  |     |
  |     +-- PDF/text files:
  |           -> Content extraction
  |           -> Appended as context block
  |
  |-- Concurrency: resolveMaxConcurrency()          src/media-understanding/concurrency.ts
  +-- Error handling: isMediaUnderstandingSkipError() gracefully skips failures
        |
        v
[Link Understanding]                                 src/link-understanding/apply.ts
  |-- applyLinkUnderstanding()
  |-- Detect URLs in message text
  |-- Fetch web pages via Readability
  |-- Extract + summarize content
  +-- Append summary to MsgContext as context block
        |
        v
[Dispatch Inbound Message]                           src/auto-reply/dispatch.ts
  |-- dispatchInboundMessage()
  |-- finalizeInboundContext(): set CommandAuthorized
  |-- Wraps execution with withReplyDispatcher() for cleanup guarantee
  |-- Calls dispatchReplyFromConfig()                src/auto-reply/reply/dispatch-from-config.ts
  |     |-- shouldSkipDuplicateInbound() dedup check
  |     |-- Runs message_received hooks via getGlobalHookRunner()
  |     |-- Checks cross-channel routing (originatingChannel != currentSurface)
  |     |-- tryFastAbortFromMessage() for /stop commands
  |     |-- Calls getReplyFromConfig()               src/auto-reply/reply/get-reply.ts
  |     |     |-- Resolves agentId from session key
  |     |     |-- Resolves default model + provider
  |     |     |-- Ensures agent workspace exists
  |     |     |-- Applies media understanding (if not fast test)
  |     |     |-- Applies link understanding
  |     |     |-- resolveCommandAuthorization()
  |     |     |-- initSessionState(): load or create session
  |     |     |-- resolveReplyDirectives(): /model, /think, /verbose, /elevated
  |     |     |-- handleInlineActions(): inline directive processing
  |     |     +-- runPreparedReply()                  src/auto-reply/reply/get-reply-run.ts
  |     |           -> Calls runEmbeddedPiAgent()
  |     |
  |     |-- Applies TTS to each reply payload:
  |     |     maybeApplyTtsToPayload()               src/tts/tts.ts
  |     |     Modes: "final" (default), "block", "tool"
  |     |     Provider resolution: ElevenLabs, OpenAI, Edge TTS
  |     |
  |     +-- Routes replies: dispatcher.sendFinalReply() or routeReply() for cross-channel
  +-- Returns DispatchFromConfigResult: { queuedFinal, counts }
        |
        v
[AI Agent Execution]                                 src/agents/pi-embedded-runner/run.ts
  |-- runEmbeddedPiAgent()
  |-- Enqueues in session lane + global lane for concurrency control
  |-- resolveModel(): provider + modelId + authStorage
  |-- evaluateContextWindowGuard(): check model context window
  |-- Auth profile rotation:
  |     resolveAuthProfileOrder() -> profileCandidates[]
  |     applyApiKeyInfo() -> set runtime API key
  |     advanceAuthProfile() -> rotate on failure
  |
  |-- Build run payloads:                            src/agents/pi-embedded-runner/run/payloads.ts
  |     buildEmbeddedRunPayloads()
  |
  |-- Run attempt:                                   src/agents/pi-embedded-runner/run/attempt.ts
  |     runEmbeddedAttempt()
  |     |-- acquireSessionWriteLock()
  |     |-- resolveSandboxContext()                   src/agents/sandbox.ts
  |     |-- createOpenClawCodingTools()              src/agents/pi-tools.ts
  |     |     Creates: exec, process, read, write, edit, apply_patch
  |     |     Creates: openclaw tools (message, cron, gateway, browser, canvas, etc.)
  |     |     Creates: channel tools, plugin tools
  |     |-- applyToolPolicyPipeline()                src/agents/tool-policy-pipeline.ts
  |     |     Steps: profile policy -> group policy -> subagent policy -> owner-only
  |     |-- buildEmbeddedSystemPrompt()              src/agents/pi-embedded-runner/system-prompt.ts
  |     |     Incorporates: identity, tools, skills, bootstrap files, context
  |     |-- toClientToolDefinitions()                src/agents/pi-tool-definition-adapter.ts
  |     |-- subscribeEmbeddedPiSession()             src/agents/pi-embedded-subscribe.ts
  |     |     Sets up streaming event handlers:
  |     |     - message_start, text_delta, text_end
  |     |     - tool_execution_start, tool_execution_end
  |     |     - compaction events
  |     |-- createAgentSession() + streamSimple()    @mariozechner/pi-coding-agent
  |     +-- Returns attempt result with assistant texts, tool metas, usage
  |
  |-- Error handling + failover:
  |     |-- Auth errors: rotate auth profile, retry
  |     |-- Context overflow: trigger compaction
  |     |     compactEmbeddedPiSessionDirect()       src/agents/pi-embedded-runner/compact.ts
  |     |-- Billing errors: format billing message
  |     |-- Rate limits: failover to next profile or model fallback
  |     +-- FailoverError: runWithModelFallback() if fallbacks configured
  |
  +-- Returns EmbeddedPiRunResult:
        { assistantTexts, toolMetas, usage, model, provider, sessionId }
        |
        v
[Post-Processing]
  |-- TTS conversion (if auto/tagged)               src/tts/tts.ts
  |     Provider: ElevenLabs / OpenAI / Edge TTS
  |     Modes: "final", "block", "tool"
  |-- Channel-specific formatting
  |-- Messaging tool deduplication                   src/agents/pi-embedded-helpers.ts
  |     isMessagingToolDuplicateNormalized()
  +-- Response prefix template interpolation         src/auto-reply/reply/response-prefix-template.ts
        |
        v
[Deliver Replies]                                    src/auto-reply/reply/reply-dispatcher.ts
  |-- ReplyDispatcher.sendFinalReply() / sendBlockReply() / sendToolResult()
  |-- normalizeReplyPayload(): strip heartbeat markers, apply prefix
  |-- Human delay (if configured): getHumanDelay()
  |-- Text chunking:                                 src/auto-reply/chunk.ts
  |     resolveTextChunkLimit(): per-channel limits
  |       Telegram: 4000, Discord: 2000, default: 4000
  |     chunkText(): respects fence blocks, paragraph boundaries
  |-- Route to channel outbound adapter
  |-- For cross-channel: routeReply()                src/auto-reply/reply/route-reply.ts
  +-- Update message metadata
        |
        v
[Cleanup]
  |-- dispatcher.markComplete()
  |-- dispatcher.waitForIdle(): drain pending queue
  |-- Remove typing indicators
  +-- Release session lane
```

### Stage-by-Stage Breakdown

**Channel Plugin Handler** -- Each supported platform (Telegram, Discord, Slack, Signal, WhatsApp, iMessage, LINE, IRC, Google Chat) has a dedicated channel plugin registered via the plugin system (`src/channels/plugins/`). The plugin normalizes the platform-specific webhook or polling payload into the common `MsgContext` format. It extracts raw message text, sender identity fields, thread/reply context, chat type classification (DM, group, channel), and any attached media. The plugin registry (`src/channels/registry.ts`) maintains the canonical list of channel IDs with aliases (e.g., `imsg` -> `imessage`, `gchat` -> `googlechat`).

**Inbound Debouncer** (`src/auto-reply/inbound-debounce.ts`) -- Many platforms deliver messages in fragments (Telegram media groups, rapid-fire short messages, etc.). The `createInboundDebouncer<T>()` function accumulates items within a configurable window. The debounce duration is resolved by `resolveInboundDebounceMs()`, which checks three layers: per-call override, per-channel override (`cfg.messages.inbound.byChannel`), and global default (`cfg.messages.inbound.debounceMs`). The debounce key is built by the channel-specific `buildKey` function (typically `channel:sender` or `channel:thread:sender`). When the timer fires, `flushBuffer()` calls `onFlush()` with all accumulated items.

**Build MsgContext** (`src/auto-reply/templating.ts`) -- The normalized message is enriched into a `MsgContext` object containing all metadata for downstream processing: `SenderId`, `SenderName`, `SenderUsername`, `SenderE164`, `WasMentioned`, `AllowlistMatch`, `InboundHistory[]`, control command detection flags, media attachment metadata (`MediaType`, `MediaUrl`, `MediaTypes[]`), and thread context (`MessageThreadId`). The context also carries `OriginatingChannel` and `OriginatingTo` for cross-channel routing scenarios.

**Gating Pipeline** -- Four sequential gates, each capable of blocking the message:

1. **Allowlist Check** (`src/channels/allowlists/`) -- Matches the sender against the channel's allowlist by multiple identity fields: ID, name, username, E164, tag, slug, or prefixed variants. The `AllowlistMatch` type tracks whether the sender was allowed and which match source matched. A wildcard (`*`) allowlist entry matches all senders.

2. **Mention Gating** (`src/channels/mention-gating.ts`) -- In group chats, `resolveMentionGatingWithBypass()` checks whether the bot was @mentioned. DMs skip this gate. An authorized control command can bypass the mention requirement (`shouldBypassMention`). The `implicitMention` flag covers direct replies to the bot.

3. **Command Authorization** (`src/channels/command-gating.ts`) -- `resolveControlCommandGate()` checks whether the sender has permission to use control commands. When `useAccessGroups` is true, at least one `CommandAuthorizer` must be both configured and allowed. The `modeWhenAccessGroupsOff` setting controls behavior when access groups are disabled.

4. **DM Policy** (`src/sessions/send-policy.ts`) -- `resolveSendPolicy()` enforces DM-level access control. Modes include `allow` (open), `block` (deny), and `pairing-only` (requires completed pairing via `src/pairing/pairing-store.ts`).

**Resolve Agent Route** (`src/routing/resolve-route.ts`) -- The `resolveAgentRoute()` function determines which agent handles this message. It loads bindings from `cfg.bindings` via `listBindings()` (`src/routing/bindings.ts`), filters by channel and accountId, then evaluates binding tiers in strict precedence: `peer > peer.parent > guild+roles > guild > team > account > channel > default`. The result includes the resolved `agentId`, `sessionKey`, and `matchedBy` field for debugging.

**Session Key Building** (`src/routing/session-key.ts`) -- `buildAgentPeerSessionKey()` constructs the session key that uniquely identifies the conversation. The format is `agent:{agentId}:{scope-suffix}`. The DM scope (`cfg.session.dmScope`) controls how DM sessions are isolated: `main` collapses all DMs to a single session, `per-peer` creates one session per peer across all channels, `per-channel-peer` adds channel isolation, and `per-account-channel-peer` adds account isolation. Group chats always use `agent:{agentId}:{channel}:{chatType}:{peerId}`. Identity links (`cfg.session.identityLinks`) allow cross-channel identity consolidation via `resolveLinkedPeerId()`. Threaded conversations append `:thread:{threadId}` via `resolveThreadSessionKeys()`.

**Media Understanding** (`src/media-understanding/`) -- When attachments are present, `applyMediaUnderstanding()` processes each one. First, `resolveScopeDecision()` checks whether media understanding is enabled for this context (by chat type, channel, session key). Then `resolveModelEntries()` selects the configured model(s) for each capability (image, audio, video). The provider registry (`src/media-understanding/providers/`) maps provider IDs (anthropic, openai, google, groq, deepgram, minimax, z.ai) to their implementations. Images are described via vision models, audio is transcribed, video undergoes frame extraction and description, and documents have their content extracted. Processing respects concurrency limits (`src/media-understanding/concurrency.ts`) and gracefully skips on errors.

**Link Understanding** (`src/link-understanding/apply.ts`) -- `applyLinkUnderstanding()` detects URLs in the message text, fetches the web pages, parses them with Readability for content extraction, and appends a summarized context block to the message.

**Dispatch** (`src/auto-reply/dispatch.ts`) -- `dispatchInboundMessage()` finalizes the context, wraps execution with `withReplyDispatcher()` for cleanup guarantees, and delegates to `dispatchReplyFromConfig()`. This function checks for duplicate inbound messages, fires `message_received` hooks, handles fast abort for `/stop` commands, and calls `getReplyFromConfig()` to obtain the AI response. Each reply payload passes through `maybeApplyTtsToPayload()` for optional TTS synthesis, then is dispatched via the reply dispatcher or cross-channel routing.

---

## 2. Outbound Response Flow (AI Agent to Channel Delivery)

### ASCII Flow Diagram

```
[EmbeddedPiRunResult]
  |-- assistantTexts: string[]
  |-- toolMetas: ToolMeta[]
  |-- usage: UsageLike
  |-- messagingToolSentTexts: string[]
        |
        v
[Reply Payload Construction]
  |-- ReplyPayload: { text?, mediaUrl?, poll?, audioAsVoice? }
  |-- For streaming: onBlockReply callbacks fire during execution
  |-- For tool results: onToolResult callbacks fire per tool
  |-- For final: array of ReplyPayload from getReplyFromConfig()
        |
        v
[Messaging Tool Deduplication]                       src/agents/pi-embedded-helpers.ts
  |-- isMessagingToolDuplicateNormalized()
  |-- Compares normalized text of final reply against messaging tool sends
  |-- If agent already sent via messaging tool: suppress duplicate final reply
        |
        v
[TTS Conversion]                                     src/tts/tts.ts
  |-- maybeApplyTtsToPayload()
  |-- Conditions checked:
  |     ttsAuto mode: "off"|"always"|"inbound"|"tagged"
  |     inboundAudio: was the inbound message audio?
  |     channel-specific TTS output format
  |-- Provider selection:
  |     ElevenLabs: API-based, high quality, configurable voice
  |     OpenAI: gpt-4o-mini-tts, multiple voices (alloy, echo, etc.)
  |     Edge TTS: free, uses Microsoft Edge voices
  |-- Output: adds mediaUrl + audioAsVoice to payload
  |-- Telegram: opus format for voice notes
  |-- Default: mp3 format
        |
        v
[Reply Dispatcher]                                   src/auto-reply/reply/reply-dispatcher.ts
  |-- createReplyDispatcher() or createReplyDispatcherWithTyping()
  |-- Three dispatch channels:
  |     sendToolResult(payload)   -> queued with "tool" kind
  |     sendBlockReply(payload)   -> queued with "block" kind
  |     sendFinalReply(payload)   -> queued with "final" kind
  |-- normalizeReplyPayloadInternal():
  |     Strip heartbeat tokens (SILENT_REPLY_TOKEN)
  |     Apply response prefix template
  |     Skip empty payloads
  |-- Human delay: configurable pause between messages for natural rhythm
  |-- Idle tracking: waitForIdle() / markComplete()
        |
        v
[Text Chunking]                                      src/auto-reply/chunk.ts
  |-- resolveTextChunkLimit(cfg, provider, accountId)
  |     Per-channel limits: Telegram 4000, Discord 2000, default 4000
  |     Configurable via cfg.channels.<provider>.textChunkLimit
  |     Per-account overrides via cfg.channels.<provider>.accounts.<accountId>.textChunkLimit
  |-- chunkText():
  |     Respect markdown fence blocks (never split inside fences)
  |     parseFenceSpans() + findFenceSpanAt() + isSafeFenceBreak()
  |     Prefer paragraph boundaries (blank lines)
  |     ChunkMode: "length" (default) or "newline" (paragraph preference)
  +-- Returns string[] of chunks within limit
        |
        v
[Channel Outbound Adapter]
  |-- deliver(payload, { kind }) callback
  |-- Platform-specific API calls:
  |     Telegram: sendMessage / sendVoice / sendPhoto / sendPoll
  |     Discord: channel.send / interaction.reply
  |     WhatsApp: sendMessage with media support
  |     Signal: signal-cli REST API
  |     Slack: chat.postMessage / chat.update
  |     iMessage: imsg bridge
  |-- Thread targeting: reply to specific thread/message
  |-- Media upload: voice notes, images, documents
  +-- Message metadata update
        |
        v
[Cross-Channel Routing]                              src/auto-reply/reply/route-reply.ts
  |-- routeReply(): when originatingChannel != currentSurface
  |-- Resolves target channel plugin
  |-- Sends via target channel's outbound adapter
  +-- Returns { ok, error? }
```

### Chunking Details

The chunking system (`src/auto-reply/chunk.ts`) ensures messages respect platform character limits without breaking formatting. The `resolveTextChunkLimit()` function resolves the limit through a priority chain: per-account override > per-channel override > fallback default (4000). The `chunkText()` function uses fence-aware splitting -- it parses markdown fence spans via `parseFenceSpans()` and never splits inside a fenced code block. When splitting is needed, it prefers paragraph boundaries (blank lines) and falls back to the hard character limit. The `ChunkMode` can be set to `"length"` (split only when exceeding limit) or `"newline"` (prefer breaking on paragraph boundaries).

---

## 3. Gateway WebSocket Request Flow

### ASCII Flow Diagram

```
[Client (Web UI / CLI / Mobile / External)]
        |
        | WebSocket upgrade to ws://host:18789/ws
        v
[HTTP Server]                                        src/gateway/server-http.ts
  |-- Origin check                                   src/gateway/origin-check.ts
  |-- TLS termination (optional)                     src/gateway/server/tls.ts
  +-- WebSocket upgrade handler
        |
        v
[Connection Phase]                                   src/gateway/server-ws-runtime.ts
  |
  |  Step 1: Server sends connect.challenge
  |  +-- { type: "challenge", nonce: <random-hex> }
  |
  |  Step 2: Client sends connect request
  |  +-- { type: "connect",
  |       role: "operator" | "node",
  |       auth: { token?, password?, device? },
  |       client: { id, displayName, platform, version },
  |       scopes: ["operator.admin", ...],
  |       caps: [...],
  |       commands: [...] }
  |
  |  Step 3: Authentication                          src/gateway/auth.ts
  |  |-- Mode resolution: none | token | password | trusted-proxy
  |  |-- isLocalDirectRequest(): bypass for local connections
  |  |-- Token auth: safeEqualSecret() comparison
  |  |-- Password auth: safeEqualSecret() comparison
  |  |-- Tailscale auth: whois lookup + user identity
  |  |-- Device auth:                                src/gateway/device-auth.ts
  |  |     buildDeviceAuthPayload(): v1/v2 signature
  |  |     Signature verification over nonce
  |  |     Pairing status check
  |  +-- Rate limiting:                              src/gateway/auth-rate-limit.ts
  |       createAuthRateLimiter()
  |       Per-IP and per-scope rate limits
  |
  |  Step 4: Server sends HelloOk
  |  +-- { type: "hello",
  |       methods: [...available methods...],
  |       events: [...subscribed events...],
  |       snapshot: { sessions, nodes, health, pending approvals },
  |       stateVersion: { presence, health } }
  |
  |  For role="node": NodeRegistry.register()        src/gateway/node-registry.ts
  |  For role="operator": add to clients set
  +-- Connection established, client receives connId
        |
        v
[Request/Response Loop]
  |
  |  Client sends:
  |  { type: "request", method: "chat.send", id: "req-123", params: {...} }
  |       |
  |       v
  |  [Method Authorization]                          src/gateway/server-methods.ts
  |  |-- authorizeGatewayMethod(method, client)
  |  |-- Scope-based access control:
  |  |     ADMIN_SCOPE: "operator.admin" (all methods)
  |  |     READ_SCOPE: "operator.read" (health, status, list methods)
  |  |     WRITE_SCOPE: "operator.write" (send, chat, invoke methods)
  |  |     APPROVALS_SCOPE: "operator.approvals"
  |  |     PAIRING_SCOPE: "operator.pairing"
  |  |-- NODE_ROLE_METHODS: restricted to role="node"
  |  +-- Returns error if unauthorized
  |       |
  |       v
  |  [Method Dispatch]                               src/gateway/server-methods.ts
  |  |-- coreGatewayHandlers: merged handler map
  |  |-- Handler categories:
  |  |     chatHandlers        -> chat.send, chat.abort, chat.history, chat.inject
  |  |     sessionsHandlers    -> sessions.list, sessions.preview, sessions.patch
  |  |     agentHandlers       -> agent, agent.wait, agent.identity.get
  |  |     agentsHandlers      -> agents.list, agents.mutate
  |  |     modelsHandlers      -> models.list
  |  |     healthHandlers      -> health
  |  |     channelsHandlers    -> channels.status
  |  |     configHandlers      -> config.get, config.patch, config.apply
  |  |     cronHandlers        -> cron.list, cron.status, cron.add, cron.run
  |  |     nodeHandlers        -> node.list, node.invoke, node.invoke.result
  |  |     sendHandlers        -> send
  |  |     skillsHandlers      -> skills.status, skills.update
  |  |     talkHandlers        -> talk.config, talk.mode
  |  |     ttsHandlers         -> tts.enable, tts.disable, tts.convert, tts.status
  |  |     voicewakeHandlers   -> voicewake.get, voicewake.set
  |  |     usageHandlers       -> usage.status, usage.cost
  |  |     logsHandlers        -> logs.tail
  |  |     systemHandlers      -> system-presence, last-heartbeat, status
  |  |     updateHandlers      -> update
  |  |     deviceHandlers      -> device.pair.*, device.token.*
  |  |     connectHandlers     -> connect.*
  |  |     browserHandlers     -> browser.request
  |  |     wizardHandlers      -> wizard.start, wizard.step
  |  |     webHandlers         -> web.search, web.fetch
  |  +-- Plugin gateway handlers merged in
  |       |
  |       v
  |  [Handler Execution]
  |  |-- Each handler receives: { params, respond, client, context }
  |  |-- respond(ok, result, error): sends ResponseFrame
  |  +-- { type: "response", id: "req-123", ok: true, result: {...} }
  |
  +-- Server sends ResponseFrame back to client
        |
        v
[Event Streaming]                                    src/gateway/server-broadcast.ts
  |-- createGatewayBroadcaster()
  |-- broadcastInternal(event, payload, opts, targetConnIds?)
  |-- Frame: { type: "event", event: "...", payload: {...}, seq: N }
  |-- Scope filtering: hasEventScope(client, event)
  |     EVENT_SCOPE_GUARDS: maps events to required scopes
  |-- Slow consumer handling:
  |     If bufferedAmount > MAX_BUFFERED_BYTES:
  |       dropIfSlow: silently skip
  |       else: close with 1008 "slow consumer"
  |-- Event types:
  |     "agent"         -> chat deltas, tool calls, reasoning traces
  |     "presence"      -> client connect/disconnect
  |     "health"        -> system health metrics
  |     "cron"          -> scheduled job status (started/running/finished)
  |     "exec.approval" -> approval request/resolution
  |     "device.pair"   -> pairing events
  |     "node.pair"     -> node pairing events
  |     "skills"        -> skill status changes
  +-- Sequential event sequence numbers (seq field)
```

### Connection Lifecycle Detail

1. **WebSocket Upgrade** -- The gateway HTTP server (`src/gateway/server-http.ts`) handles the WebSocket upgrade request at the `/ws` endpoint. Origin checks (`src/gateway/origin-check.ts`) are applied for CORS enforcement.

2. **Challenge** -- Immediately after upgrade, the server sends a `connect.challenge` frame containing a cryptographic nonce (random hex string). This nonce is used for device identity authentication (v2 signature verification).

3. **Connect Request** -- The client responds with a `connect` frame containing its `role` (`"operator"` or `"node"`), authentication credentials, client metadata (id, displayName, platform, version), requested scopes, capabilities, and available commands.

4. **Authentication** (`src/gateway/auth.ts`) -- The server resolves the auth mode from config (`gateway.auth.mode`): `none`, `token`, `password`, or `trusted-proxy`. Local direct requests (`isLocalDirectRequest()`) can bypass authentication. Token and password auth use `safeEqualSecret()` for timing-safe comparison. Device authentication (`src/gateway/device-auth.ts`) uses `buildDeviceAuthPayload()` to construct a signed payload including the nonce, then verifies the signature. Tailscale auth performs a whois lookup. Rate limiting (`src/gateway/auth-rate-limit.ts`) prevents brute force attempts.

5. **HelloOk** -- On successful authentication, the server sends a `HelloOk` frame containing the list of available methods, subscribed events, a snapshot of current state (sessions, nodes, health, pending approvals), and state version counters.

### Request/Response Protocol

Each request is a `RequestFrame` with fields: `type` ("request"), `method`, `id` (unique per connection), and `params`. The server validates the request against the method's schema, checks role and scope authorization via `authorizeGatewayMethod()`, dispatches to the handler function, and returns a `ResponseFrame` with the same `id`, `ok` status, and `result` or `error` payload. Multiple requests can be in flight concurrently since request IDs are client-generated.

---

## 4. Chat Send Flow (Web UI to Agent)

### ASCII Flow Diagram

```
[Web UI / CLI Client]
  |
  | chat.send { sessionKey, text, attachments?, parentId? }
  v
[Gateway]  chatHandlers["chat.send"]                 src/gateway/server-methods/chat.ts
  |-- validateChatSendParams(params)
  |-- Parse text + attachments:
  |     parseMessageWithAttachments()                src/gateway/chat-attachments.ts
  |-- Resolve session info:
  |     loadSessionEntry() + resolveSessionModelRef()
  |-- Check for /stop command:
  |     isChatStopCommandText() -> abortChatRunsForSessionKey()
  |-- Build MsgContext for dispatch:
  |     Surface = INTERNAL_MESSAGE_CHANNEL
  |     CommandSource = "native" or "operator"
  |     SessionKey from request
  |-- Create ReplyDispatcher:
  |     deliver callback -> broadcast "agent" event to connected clients
  |-- dispatchInboundMessage()                       src/auto-reply/dispatch.ts
  |     (Same flow as channel inbound, but with internal channel)
  |
  |--- STREAMING ---
  |  Agent execution produces events:
  |  createAgentEventHandler()                       src/gateway/server-chat.ts
  |  onAgentEvent() ->
  |    broadcast("agent", {
  |      sessionKey,
  |      action: "text_delta" | "tool_start" | "tool_end" | "thinking" | "message_end",
  |      text?, toolName?, toolInput?, reasoning?
  |    })
  |
  +-- On completion:
       broadcast("agent", { action: "run_complete", sessionKey, usage })
        |
        v
[Connected Clients]
  |-- Receive "agent" events in real time
  |-- Update message list incrementally
  +-- Show streaming text, tool calls, reasoning
```

---

## 5. Native App Interaction Flow (Mobile Node)

### ASCII Flow Diagram

```
[Mobile App (iOS/Android)]
        |
        v
[Connect as role="node"]                             src/gateway/node-registry.ts
  |-- NodeRegistry.register(client, opts)
  |-- NodeSession created:
  |     nodeId, connId, platform, version
  |     caps: ["camera", "location", "sms", "contacts", ...]
  |     commands: ["take_photo", "get_location", "send_sms", ...]
  |     permissions: { camera: true, location: true, ... }
  |-- Broadcast "presence" event to operators
  +-- Send voicewake configuration to node
        |
        v
[Voice Input Flow]
  |
  |  1. Wake word detection (on-device)
  |     voicewake.get / voicewake.set                src/gateway/server-methods/voicewake.ts
  |     Configuration: wake word(s), sensitivity, audio params
  |
  |  2. Audio recording + transcription
  |     On-device or streaming to transcription service
  |
  |  3. Send voice.transcript or agent.request
  |     -> Gateway processes as chat.send equivalent
  |     -> Agent execution with full tool access
  |
  |  4. Response + TTS
  |     -> Agent text response
  |     -> TTS synthesis (if configured)
  |     -> Audio playback on device
        |
        v
[Node Invocations]                                   src/gateway/server-methods/nodes.ts
  |
  |  During agent execution, agent may call device tools:
  |
  |  Agent: "Take a photo of the whiteboard"
  |       |
  |       v
  |  [Tool: camera]
  |       |
  |       v
  |  Gateway sends node.invoke to mobile app:
  |  { command: "take_photo", params: { ... } }
  |       |
  |       v
  |  [Node Command Policy]                           src/gateway/node-command-policy.ts
  |  |-- Validate command against node's declared commands
  |  |-- Check permissions
  |  +-- Apply approval policy if required
  |       |
  |       v
  |  [Mobile App executes command on-device]
  |  |-- Camera capture / GPS location / etc.
  |       |
  |       v
  |  Mobile sends node.invoke.result:
  |  { ok: true, payload: { imageBase64: "...", ... } }
  |       |
  |       v
  |  [Gateway delivers result to agent]
  |  NodeRegistry.resolveInvoke()
  |  -> Tool result returned to agent session
  |  -> Agent continues execution with device data
        |
        v
[Disconnect]
  |-- NodeRegistry.unregister(connId)
  |-- Clear pending invocations (reject with "node disconnected")
  |-- Broadcast "presence" event (node offline)
  +-- Refresh remote skills eligibility
```

### Node Lifecycle Detail

The `NodeRegistry` (`src/gateway/node-registry.ts`) maintains the mapping of connected mobile nodes. Each `NodeSession` stores: `nodeId`, `connId`, `client` reference, `caps` (capabilities), `commands` (available device commands), and `permissions`. When a node disconnects, `unregister()` cleans up all pending invocations by rejecting their promises with a "node disconnected" error, preventing the agent from hanging.

The invoke flow uses a request/response pattern with timeouts. `NodeRegistry.invoke()` sends a `node.invoke` frame to the mobile app, registers a `PendingInvoke` entry with a timer, and returns a promise. The mobile app processes the command and sends back `node.invoke.result`, which resolves the pending promise via `resolveInvoke()`.

---

## 6. Configuration Loading Flow

### ASCII Flow Diagram

```
[Application Startup / Config Reload]
        |
        v
[Resolve Config Path]                               src/config/paths.ts
  |-- resolveConfigPath(env, stateDir)
  |-- resolveDefaultConfigCandidates(): checks multiple locations
  |     ~/.openclaw/config.json5
  |     ~/.openclaw/config.json
  |     $OPENCLAW_CONFIG_PATH (env override)
  +-- First existing candidate wins
        |
        v
[Load .env File]                                     src/infra/dotenv.ts
  |-- loadDotEnv({ quiet: true })
  |-- Hydrates process.env from .env file
  +-- Only for real process.env (not injected test envs)
        |
        v
[Read Config File]                                   src/config/io.ts :: loadConfig()
  |-- fs.readFileSync(configPath, "utf-8")
  |-- JSON5.parse(raw)
  +-- Returns raw parsed object
        |
        v
[Resolve $include Directives]                        src/config/includes.ts
  |-- resolveConfigIncludes(parsed, configPath, resolver)
  |-- INCLUDE_KEY = "$include"
  |-- Supports single file: "$include": "./base.json5"
  |-- Supports array: "$include": ["./a.json5", "./b.json5"]
  |-- IncludeProcessor:
  |     Tracks visited files (circular detection)
  |     MAX_INCLUDE_DEPTH = 10
  |     Recursive resolution
  |-- deepMerge(): arrays concatenate, objects merge, primitives: source wins
  +-- CircularIncludeError if cycle detected
        |
        v
[Apply Config Env Vars]                              src/config/env-vars.ts
  |-- applyConfigEnvVars(cfg, env)
  |-- Sets process.env from cfg.env section
  +-- Makes config-defined vars available for ${VAR} substitution
        |
        v
[Environment Variable Substitution]                  src/config/env-substitution.ts
  |-- resolveConfigEnvVars(resolvedIncludes, env)
  |-- Pattern: ${VAR_NAME} in string values
  |-- Only uppercase vars: /^[A-Z_][A-Z0-9_]*$/
  |-- Escape with $${VAR} to output literal ${VAR}
  |-- Missing vars throw MissingEnvVarError with config path context
  +-- Captures env snapshot for write-time restoration
        |
        v
[Shell Env Fallback]                                 src/infra/shell-env.ts
  |-- loadShellEnvFallback(): loads env from login shell
  |-- Enabled via OPENCLAW_SHELL_ENV=1 or cfg.env.shellEnv.enabled
  |-- Expected keys: OPENAI_API_KEY, ANTHROPIC_API_KEY, etc.
  +-- Configurable timeout: cfg.env.shellEnv.timeoutMs
        |
        v
[Warn on Miskeys]
  |-- warnOnConfigMiskeys(): e.g., "gateway.token" should be "gateway.auth.token"
        |
        v
[Duplicate Agent Dir Check]                          src/config/agent-dirs.ts
  |-- findDuplicateAgentDirs(): prevent two agents sharing a directory
  +-- DuplicateAgentDirError if conflict found
        |
        v
[Validation]                                         src/config/validation.ts
  |-- validateConfigObjectWithPlugins(resolvedConfig)
  |-- JSON Schema validation with plugin schema extensions
  |-- Returns: { ok, config, issues[], warnings[] }
  +-- Logs errors/warnings; returns empty config on invalid
        |
        v
[Apply Defaults Pipeline]                            src/config/defaults.ts
  |-- applyMessageDefaults(cfg)
  |-- applyLoggingDefaults(cfg)
  |-- applySessionDefaults(cfg)
  |-- applyAgentDefaults(cfg)
  |-- applyContextPruningDefaults(cfg)
  |-- applyCompactionDefaults(cfg)
  |-- applyModelDefaults(cfg)
  +-- Each function fills missing fields with sensible defaults
        |
        v
[Normalize Paths]                                    src/config/normalize-paths.ts
  |-- normalizeConfigPaths(cfg)
  |-- Resolves ~ and relative paths to absolute
        |
        v
[Apply Runtime Overrides]                            src/config/runtime-overrides.ts
  |-- applyConfigOverrides(cfg)
  |-- Environment variable overrides (OPENCLAW_*)
  +-- Returns final OpenClawConfig object
        |
        v
[Config Cache]                                       src/config/io.ts :: loadConfig()
  |-- configCache: { config, configPath, expiresAt }
  |-- shouldUseConfigCache(env): enabled by default
  +-- TTL-based expiry for hot-reload support
```

### Config Write Flow

When writing back to the config file, `writeConfigFile()` (`src/config/io.ts`) performs:
1. Env var reference restoration via `restoreEnvVarRefs()` -- replaces resolved values back to `${VAR}` syntax
2. Version stamping via `stampConfigVersion()` -- updates `meta.lastTouchedVersion` and `meta.lastTouchedAt`
3. Atomic write via rename (with copy fallback)
4. Backup rotation via `rotateConfigBackups()`
5. Audit logging to `config-audit.jsonl`

---

## 7. Plugin Loading Flow

### ASCII Flow Diagram

```
[Gateway Startup / Plugin Reload]
        |
        v
[Plugin Discovery]                                   src/plugins/discovery.ts
  |-- discoverOpenClawPlugins()
  |-- Search locations:
  |     1. Bundled plugins dir                       src/plugins/bundled-dir.ts
  |        (compiled into the distribution)
  |     2. User extensions dir: ~/.openclaw/extensions/
  |     3. Workspace extensions: .openclaw/extensions/
  |     4. npm packages with openclaw manifest
  |-- For each directory:
  |     discoverInDirectory():
  |       Scan for .ts/.js/.mts/.cts/.mjs/.cjs files
  |       Read package.json for metadata
  |       resolvePackageExtensions() from openclaw manifest
  |-- Build PluginCandidate[]:
  |     { idHint, source, rootDir, origin, packageName, packageVersion }
  |-- Dedup by resolved absolute path (seen set)
  +-- Returns PluginDiscoveryResult: { candidates, diagnostics }
        |
        v
[Plugin Configuration]                               src/plugins/config-state.ts
  |-- normalizePluginsConfig(cfg.plugins)
  |-- Per-plugin enable state: resolveEnableState()
  |     Modes: enabled, disabled, auto
  |-- Memory slot decision: resolveMemorySlotDecision()
  +-- Returns NormalizedPluginsConfig
        |
        v
[Plugin Loading]                                     src/plugins/loader.ts
  |-- loadPlugins()
  |-- Cache check: buildCacheKey() -> registryCache lookup
  |-- For each enabled candidate:
  |     1. Create jiti loader (TypeScript-capable import)
  |     2. Import module: resolvePluginModuleExport()
  |        Supports: default export, named "register", or definition object
  |     3. Validate plugin config:
  |        validatePluginConfig() against plugin's JSON schema
  |     4. Create plugin runtime:
  |        createPluginRuntime()                     src/plugins/runtime/index.ts
  |     5. Call plugin.register(api):
  |        Plugin receives OpenClawPluginApi with:
  |          registerTool(), registerHook(), registerChannel(),
  |          registerProvider(), registerCommand(), registerService(),
  |          registerHttpHandler(), registerGatewayHandler()
  +-- Returns assembled PluginRegistry
        |
        v
[Plugin Registry Assembly]                           src/plugins/registry.ts
  |-- createPluginRegistry()
  |-- PluginRegistry contains:
  |     plugins: PluginRecord[]         (all loaded plugins)
  |     tools: PluginToolRegistration[] (tool factories)
  |     hooks: PluginHookRegistration[] (event hooks)
  |     typedHooks: TypedPluginHookRegistration[]
  |     channels: PluginChannelRegistration[]
  |     providers: PluginProviderRegistration[]
  |     gatewayHandlers: Record<string, GatewayRequestHandler>
  |     httpHandlers: PluginHttpRegistration[]
  |     httpRoutes: PluginHttpRouteRegistration[]
  |     cliRegistrars: PluginCliRegistration[]
  |     services: PluginServiceRegistration[]
  |     commands: PluginCommandRegistration[]
  |     diagnostics: PluginDiagnostic[]
        |
        v
[Registry Activation]                                src/plugins/runtime.ts
  |-- setActivePluginRegistry(registry, cacheKey)
  |-- Global singleton via Symbol.for("openclaw.pluginRegistryState")
  |-- requireActivePluginRegistry(): creates empty if missing
        |
        v
[Hook Runner Initialization]                         src/plugins/hook-runner-global.ts
  |-- initializeGlobalHookRunner()
  |-- Wires hooks from registry into event dispatch system
  +-- Available via getGlobalHookRunner()
        |
        v
[Plugin Services Start]                              src/plugins/services.ts
  |-- For each registered service:
  |     service.start() called
  |-- Services run as background tasks
  +-- Stopped on shutdown
```

### Plugin Registration API

When a plugin's `register()` function is called, it receives an `OpenClawPluginApi` object with these registration methods:

- `registerTool(factory)` -- Register a tool factory that creates agent tools
- `registerHook(event, handler, options?)` -- Register a hook for system events
- `registerChannel(plugin, dock?)` -- Register a new messaging channel
- `registerProvider(provider)` -- Register an AI model provider
- `registerCommand(definition)` -- Register a CLI command
- `registerService(service)` -- Register a background service
- `registerHttpHandler(handler)` -- Register an HTTP request handler
- `registerGatewayHandler(method, handler)` -- Register a WebSocket method handler

---

## 8. Agent Execution Flow

### ASCII Flow Diagram

```
[runEmbeddedPiAgent(params)]                         src/agents/pi-embedded-runner/run.ts
        |
        v
[Lane Enqueue]
  |-- resolveSessionLane(sessionKey): per-session serialization
  |-- resolveGlobalLane(lane): global concurrency control
  |-- enqueueCommandInLane(): ensures ordered execution
        |
        v
[Workspace Resolution]
  |-- resolveRunWorkspaceDir(): agent workspace directory
  |-- ensureAgentWorkspace(): create if needed
        |
        v
[Model Resolution]                                   src/agents/pi-embedded-runner/model.ts
  |-- resolveModel(provider, modelId, agentDir, config)
  |-- Loads model catalog:                           src/agents/model-catalog.ts
  |     loadModelCatalog() from ~/.openclaw/agents/<agentId>/models.json
  |-- ensureOpenClawModelsJson(): create/update models file
  |-- Returns: { model, error, authStorage, modelRegistry }
        |
        v
[Context Window Guard]                               src/agents/context-window-guard.ts
  |-- resolveContextWindowInfo(): provider + model + config
  |-- evaluateContextWindowGuard():
  |     shouldWarn: below CONTEXT_WINDOW_WARN_BELOW_TOKENS
  |     shouldBlock: below CONTEXT_WINDOW_HARD_MIN_TOKENS
  +-- Blocks execution if context window too small
        |
        v
[Auth Profile Resolution]                            src/agents/model-auth.ts
  |-- ensureAuthProfileStore(agentDir)
  |-- resolveAuthProfileOrder(cfg, store, provider, preferredProfile)
  |-- Profile candidates: ordered list of auth profiles
  |-- applyApiKeyInfo():
  |     getApiKeyForModel() -> resolves API key
  |     authStorage.setRuntimeApiKey() -> sets for SDK
  |-- For github-copilot: resolveCopilotApiToken() token exchange
        |
        v
[Auth Profile Rotation Loop]
  |-- For each profile candidate:
  |     |-- Check cooldown: isProfileInCooldown()
  |     |-- Apply API key
  |     |-- Run attempt
  |     |-- On auth error: markAuthProfileFailure(), advance to next
  |     |-- On success: markAuthProfileGood(), markAuthProfileUsed()
  |     +-- On rate limit: cooldown + advance
        |
        v
[Build Run Payloads]                                 src/agents/pi-embedded-runner/run/payloads.ts
  |-- buildEmbeddedRunPayloads()
  |-- Constructs the messages array for the AI API call
        |
        v
[Run Attempt]                                        src/agents/pi-embedded-runner/run/attempt.ts
  |-- runEmbeddedAttempt()
  |
  |  [Session Write Lock]
  |  |-- acquireSessionWriteLock(): prevents concurrent writes
  |
  |  [Sandbox Resolution]                            src/agents/sandbox.ts
  |  |-- resolveSandboxContext(): Docker/Podman sandbox if configured
  |
  |  [Tool Creation]                                 src/agents/pi-tools.ts
  |  |-- createOpenClawCodingTools():
  |  |     Coding tools: exec, process, read, write, edit, apply_patch
  |  |     OpenClaw tools: message, cron, gateway, browser, canvas,
  |  |       nodes, sessions_*, memory_*, web_*, agents_list, image
  |  |     Channel tools: listChannelAgentTools()
  |  |     Plugin tools: from PluginToolRegistration factories
  |  |
  |  |  [Tool Policy Pipeline]                       src/agents/tool-policy-pipeline.ts
  |  |  |-- applyToolPolicyPipeline()
  |  |  |-- buildDefaultToolPolicyPipelineSteps():
  |  |  |     1. resolveToolProfilePolicy()          src/agents/tool-policy.ts
  |  |  |        Profiles: "minimal"|"coding"|"messaging"|"full"
  |  |  |        Each profile has allow/deny lists
  |  |  |     2. resolveGroupToolPolicy()             src/agents/pi-tools.policy.ts
  |  |  |        Per-channel/group tool restrictions
  |  |  |     3. resolveSubagentToolPolicy()
  |  |  |        Subagent tool scope restrictions
  |  |  |     4. applyOwnerOnlyToolPolicy()           src/agents/tool-policy.ts
  |  |  |        Owner-only tools (e.g., whatsapp_login)
  |  |  +-- Tool groups:                             src/agents/tool-policy.ts
  |  |       "group:memory", "group:web", "group:fs",
  |  |       "group:runtime", "group:sessions", "group:ui",
  |  |       "group:automation", "group:messaging", "group:nodes",
  |  |       "group:openclaw"
  |  |
  |  |  [Before-Tool-Call Hook]                      src/agents/pi-tools.before-tool-call.ts
  |  |  |-- wrapToolWithBeforeToolCallHook(): plugin pre-execution hooks
  |  |
  |  |  [Abort Signal Wrapper]                       src/agents/pi-tools.abort.ts
  |  |  +-- wrapToolWithAbortSignal(): cancellation support
  |
  |  [System Prompt Construction]                    src/agents/pi-embedded-runner/system-prompt.ts
  |  |-- buildEmbeddedSystemPrompt()
  |  |-- Components:
  |  |     Identity (name, personality, instructions)
  |  |     Current time + timezone
  |  |     Channel capabilities + message tool hints
  |  |     Sandbox info
  |  |     Bootstrap files (OPENCLAW.md, workspace context)
  |  |     Skills prompt
  |  |     TTS hints
  |  |     Heartbeat prompt (if heartbeat run)
  |  +-- buildSystemPromptParams()                   src/agents/system-prompt-params.ts
  |
  |  [Tool Definition Adaptation]                    src/agents/pi-tool-definition-adapter.ts
  |  |-- toClientToolDefinitions(): convert to API format
  |  |-- Provider-specific schema adjustments:
  |  |     cleanToolSchemaForGemini(): Google format
  |  |     patchToolSchemaForClaudeCompatibility(): Anthropic format
  |  +-- normalizeToolParameters()                   src/agents/pi-tools.schema.ts
  |
  |  [Session History Management]
  |  |-- prepareSessionManagerForRun()               src/agents/pi-embedded-runner/session-manager-init.ts
  |  |-- repairSessionFileIfNeeded()                 src/agents/session-file-repair.ts
  |  |-- limitHistoryTurns()                         src/agents/pi-embedded-runner/history.ts
  |  |-- sanitizeSessionHistory()                    src/agents/pi-embedded-runner/google.ts
  |  +-- sanitizeToolUseResultPairing()              src/agents/session-transcript-repair.ts
  |
  |  [AI API Call]
  |  |-- createAgentSession() + streamSimple()       @mariozechner/pi-coding-agent
  |  |-- subscribeEmbeddedPiSession()                src/agents/pi-embedded-subscribe.ts
  |  |
  |  |  [Streaming Event Loop]
  |  |  |-- message_start -> text_delta* -> text_end
  |  |  |     |-- Delta accumulation in deltaBuffer
  |  |  |     |-- Thinking tag detection (<think>/<thinking>)
  |  |  |     |-- <final> tag filtering
  |  |  |     |-- Block chunking: EmbeddedBlockChunker
  |  |  |     +-- onBlockReply callbacks for streaming delivery
  |  |  |
  |  |  |-- tool_use -> tool execution -> tool_result
  |  |  |     |-- onBlockReplyFlush: flush pending text before tool
  |  |  |     |-- Tool execution with result guard
  |  |  |     |-- guardSessionManager()              src/agents/session-tool-result-guard-wrapper.ts
  |  |  |     |     capToolResultSize(): truncate oversized results
  |  |  |     |     HARD_MAX_TOOL_RESULT_CHARS limit
  |  |  |     +-- onToolResult callback for tool summary delivery
  |  |  |
  |  |  |-- compaction events:
  |  |  |     compactionInFlight flag
  |  |  |     pendingCompactionRetry counter
  |  |  |
  |  |  +-- message_end -> accumulate usage
  |  |
  |  +-- Returns EmbeddedRunAttemptResult
  |
  |  [Error Handling + Failover]
  |  |-- isAuthAssistantError() -> rotate auth profile
  |  |-- isBillingAssistantError() -> formatBillingErrorMessage()
  |  |-- isLikelyContextOverflowError():
  |  |     compactEmbeddedPiSessionDirect()          src/agents/pi-embedded-runner/compact.ts
  |  |     truncateOversizedToolResultsInSession()
  |  |     Retry after compaction
  |  |-- isRateLimitAssistantError() -> cooldown + failover
  |  |-- isTimeoutErrorMessage() -> timeout handling
  |  |-- FailoverError: classifyFailoverReason()
  |  |     Reasons: "auth"|"rate_limit"|"billing"|"unknown"
  |  +-- pickFallbackThinkingLevel(): downgrade thinking on failure
  |
  +-- Returns EmbeddedPiRunResult
        |
        v
[Usage Tracking]                                     src/agents/usage.ts
  |-- UsageAccumulator: input, output, cacheRead, cacheWrite, total
  |-- mergeUsageIntoAccumulator(): tracks per-call and total
  |-- lastCacheRead/lastCacheWrite: most recent API call's cache fields
  +-- toNormalizedUsage(): avoids inflating context size from accumulated cache
```

---

## 9. Cron Job Execution Flow

### ASCII Flow Diagram

```
[Cron Configuration]
  |-- cfg.cron: array of job definitions
  |-- Job types: systemEvent, agentTurn
  |-- Schedule types: cron expression, "every" interval
        |
        v
[CronService.start()]                               src/cron/service.ts
  |-- ops.start(state)                              src/cron/service/ops.ts
  |-- ensureLoaded(state): load store from disk
  |-- Clear stale running markers (runningAtMs)
  |-- runMissedJobs(state): catch up on jobs missed during downtime
  |-- recomputeNextRuns(state): calculate next fire times
  |-- persist(state): write store to disk
  |-- armTimer(state): set timer for next wake
  +-- Log: "cron: started" with job count and nextWakeAtMs
        |
        v
[Timer Fires]                                        src/cron/service/timer.ts
  |-- Timer callback fires at nextWakeAtMs
  |-- For each due job (isJobDue()):
  |     executeJob(state, job)
  +-- armTimer(state): rearm for next job
        |
        v
[executeJob()]                                       src/cron/service/timer.ts
  |-- emit(state, { jobId, action: "started" })
  |-- Set job.state.runningAtMs = now
  |-- persist(state)
  |
  |  [Job Type: systemEvent]
  |  |-- Enqueue system message into session
  |  +-- Triggers downstream processing
  |
  |  [Job Type: agentTurn]
  |  |-- runCronIsolatedAgentTurn()                  src/cron/isolated-agent/run.ts
  |  |     |-- Resolve agent: requestedAgentId or defaultAgentId
  |  |     |-- resolveAgentConfig(): per-agent model, workspace
  |  |     |-- resolveCronSession(): build session key
  |  |     |     Format: agent:{agentId}:cron:{jobId}:run:{runId}
  |  |     |-- resolveDeliveryTarget()               src/cron/isolated-agent/delivery-target.ts
  |  |     |-- resolveCronDeliveryPlan()             src/cron/delivery.ts
  |  |     |     mode: "announce" | "none"
  |  |     |     channel: specific channel or "last" (from session store)
  |  |     |     to: specific target or from session lastRoute
  |  |     |
  |  |     |-- Run agent:
  |  |     |     runEmbeddedPiAgent() (same flow as section 8)
  |  |     |     OR runCliAgent() for CLI-provider models
  |  |     |     runWithModelFallback() wraps for failover
  |  |     |
  |  |     |-- Post-run:
  |  |     |     pickSummaryFromPayloads(): extract summary text
  |  |     |     pickLastNonEmptyTextFromPayloads(): last agent output
  |  |     |     isHeartbeatOnlyResponse(): detect heartbeat-only runs
  |  |     |
  |  |     |-- Delivery:
  |  |     |     deliverOutboundPayloads()           src/infra/outbound/deliver.ts
  |  |     |       resolveAgentOutboundIdentity()    src/infra/outbound/identity.ts
  |  |     |       Send to target channel:to
  |  |     |     OR runSubagentAnnounceFlow()        src/agents/subagent-announce.ts
  |  |     |       Post summary to parent session
  |  |     |
  |  |     +-- Returns RunCronAgentTurnResult:
  |  |           { status, summary, outputText, error, sessionId, delivered }
  |
  |-- emit(state, { jobId, action: "running" })
  |-- On completion:
  |     job.state.lastRunAtMs = now
  |     job.state.runningAtMs = undefined
  |     computeJobNextRunAtMs(): schedule next
  |     persist(state)
  |     armTimer(state)
  |-- emit(state, { jobId, action: "finished", result/error })
  |
  |-- One-shot jobs: disable after first run
  +-- "every" jobs: recompute from anchorMs
        |
        v
[Run Log]                                            src/cron/run-log.ts
  |-- Append run result to run log file
  |-- Available via cron.runs gateway method
  +-- Queryable for status/history
```

### Cron Delivery Plan

The `resolveCronDeliveryPlan()` function (`src/cron/delivery.ts`) determines how job output is delivered. The delivery configuration can come from the `job.delivery` object or legacy `job.payload` fields. The resolved plan has:
- `mode`: `"announce"` (send to channel) or `"none"` (silent)
- `channel`: specific channel ID or `"last"` (use last known route from session store)
- `to`: specific chat/recipient ID
- `source`: `"delivery"` or `"payload"` (for legacy compatibility)

---

## 10. Voice Interaction Flow (End-to-End)

### ASCII Flow Diagram

```
[Mobile Node / Web Client]
        |
        v
[Wake Word Detection]
  |-- On-device wake word engine
  |-- Configuration via voicewake.get/set             src/gateway/server-methods/voicewake.ts
  |-- Parameters: wake word(s), sensitivity, audio pipeline
  +-- Wake detected -> start recording
        |
        v
[Audio Recording]
  |-- On-device audio capture
  |-- Streaming to transcription service (optional)
  +-- Audio buffer ready
        |
        v
[Transcription]
  |-- Provider options:
  |     Deepgram: streaming or batch                 src/media-understanding/providers/deepgram/
  |     OpenAI Whisper: batch transcription           src/media-understanding/providers/openai/
  |     Google: streaming or batch                    src/media-understanding/providers/google/
  |-- Audio preflight:                                src/media-understanding/audio-preflight.ts
  |     Validate format, duration, size
  |-- Output: transcribed text
  +-- Send as voice.transcript or text message
        |
        v
[Agent Processing]
  |-- Same flow as Inbound Message (section 1)
  |-- inboundAudio flag set to true
  |-- Agent receives transcribed text as input
  |-- Full tool access during execution
  +-- Agent produces text response
        |
        v
[TTS Synthesis]                                      src/tts/tts.ts
  |-- Triggered when:
  |     ttsAuto = "always" (always synthesize)
  |     ttsAuto = "inbound" AND inboundAudio = true
  |     ttsAuto = "tagged" AND response has TTS tags
  |     Session-level ttsAuto override from session store
  |
  |-- Text preparation:
  |     stripMarkdown(): remove formatting for natural speech
  |     summarizeText(): condense long text (if configured)
  |     parseTtsDirectives(): extract voice/model overrides from text
  |     Max text length: DEFAULT_MAX_TEXT_LENGTH (4096)
  |
  |-- Provider selection:
  |     ElevenLabs:                                  src/tts/tts-core.ts :: elevenLabsTTS()
  |       API: POST /v1/text-to-speech/{voiceId}
  |       Voice settings: stability, similarityBoost, style, speed
  |       Model: eleven_multilingual_v2 (default)
  |     OpenAI:                                      src/tts/tts-core.ts :: openaiTTS()
  |       API: POST /v1/audio/speech
  |       Model: gpt-4o-mini-tts (default)
  |       Voices: alloy, echo, fable, onyx, nova, shimmer
  |     Edge TTS:                                    src/tts/tts-core.ts :: edgeTTS()
  |       Free Microsoft Edge TTS
  |       Voice: en-US-MichelleNeural (default)
  |
  |-- Channel-specific output format:
  |     Telegram: opus format for voice notes (audioAsVoice=true)
  |     Default: mp3 format
  |
  |-- Model overrides:                               src/tts/tts.ts :: ResolvedTtsModelOverrides
  |     allowText, allowProvider, allowVoice, allowModelId
  |     Per-agent or per-session overrides
  |
  +-- Output: { mediaUrl, audioAsVoice }
        |
        v
[Audio Delivery]
  |-- mediaUrl added to ReplyPayload
  |-- Delivered via channel adapter:
  |     Telegram: sendVoice (opus voice note)
  |     WhatsApp: sendMessage with audio
  |     Gateway: broadcast with audio URL
  |-- For mobile nodes: audio stream to device
  +-- Device plays audio response
```

---

## 11. Device Pairing Flow

### ASCII Flow Diagram

```
[New Device / Mobile App]
        |
        v
[Pairing Request]
  |-- Device connects to gateway
  |-- Sends pairing request with device metadata
  |-- Gateway generates pairing code:                src/pairing/pairing-store.ts
  |     PAIRING_CODE_LENGTH = 8
  |     PAIRING_CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
  |     (excludes ambiguous characters: 0, O, 1, I)
  |-- PairingRequest stored:
  |     { id, code, createdAt, lastSeenAt, meta }
  |-- PAIRING_PENDING_TTL_MS = 60 minutes
  |-- PAIRING_PENDING_MAX = 3 concurrent requests
  +-- Pairing code displayed to user
        |
        v
[Pairing Store]                                      src/pairing/pairing-store.ts
  |-- Storage: ~/.openclaw/credentials/<channel>-pairing.json
  |-- File locking: withPathLock() for concurrent safety
  |-- PairingStore: { version: 1, requests: PairingRequest[] }
  |-- AllowFromStore: { version: 1, allowFrom: string[] }
  +-- resolveAllowFromPath(): per-channel allow-from list
        |
        v
[User Approves Pairing]
  |-- Via Web UI: device.pair.approve
  |-- Via CLI: openclaw pair approve
  |-- Via another connected device: operator action
  |
  |-- Approval methods:
  |     device.pair.approve { code }                 src/gateway/server-methods/devices.ts
  |     node.pair.approve { code }                   src/gateway/server-methods/nodes.ts
  |-- Validates code against pending requests
  |-- Moves device to approved list
  +-- Broadcasts device.pair.resolved event
        |
        v
[Pairing Complete]
  |-- Device receives approved status
  |-- Device token issued (for future auth)
  |-- Device can now:
  |     Connect as authenticated node
  |     Execute commands
  |     Receive agent events
  +-- Token rotation: device.token.rotate
        |
        v
[Pairing Messages]                                   src/pairing/pairing-messages.ts
  |-- Channel-specific pairing prompts
  |-- Pairing labels                                 src/pairing/pairing-labels.ts
  +-- User-facing instructions per channel
```

---

## 12. Execution Approval Flow

### ASCII Flow Diagram

```
[Agent Requests Tool Execution]
  |-- exec tool with ask mode or risky command
  |-- Security policy: cfg.tools.exec.security
  |-- Ask mode: cfg.tools.exec.ask
        |
        v
[ExecApprovalManager.create()]                       src/gateway/exec-approval-manager.ts
  |-- ExecApprovalRequestPayload:
  |     { command, cwd, host, security, ask, agentId, resolvedPath, sessionKey }
  |-- ExecApprovalRecord:
  |     { id: UUID, request, createdAtMs, expiresAtMs }
  |-- requestedByConnId, requestedByDeviceId, requestedByClientId
        |
        v
[Broadcast Approval Request]
  |-- broadcast("exec.approval.requested", record)
  |-- Scope-filtered: only clients with APPROVALS_SCOPE
  |-- Multiple clients can see the request
        |
        v
[ExecApprovalManager.register()]
  |-- Registers record and returns promise
  |-- Timeout timer set: resolves with null on expiry
  |-- RESOLVED_ENTRY_GRACE_MS = 15s (keep entry for late calls)
  +-- Agent execution pauses, waiting for decision
        |
        v
[User Decision]
  |-- Via Web UI: approve/deny button
  |-- Via Mobile: notification + action
  |-- Via CLI: approval prompt
  |
  |  exec.approval.resolve { id, decision }          src/gateway/server-methods/exec-approval.ts
  |  |-- decision: "allow" | "deny" | "allow_once" | "always_allow"
  |  +-- ExecApprovalManager.resolve(id, decision, resolvedBy)
  |       |-- Clears timeout timer
  |       |-- Sets: resolvedAtMs, decision, resolvedBy
  |       |-- Resolves pending promise with decision
  |       +-- Keeps entry briefly for late calls
        |
        v
[Decision Applied]
  |-- broadcast("exec.approval.resolved", { id, decision })
  |-- Agent receives decision:
  |     "allow" / "allow_once": execute command
  |     "deny": skip with error message
  |     "always_allow": execute + remember for future
  |-- Approval running notice:                       cfg.tools.exec.approvalRunningNoticeMs
  |     Notify user when approved command is still running
  +-- Agent continues execution
        |
        v
[Timeout Path]
  |-- If no decision within timeoutMs:
  |-- Promise resolves with null
  |-- Agent receives timeout -> treats as deny
  +-- Entry cleaned up after RESOLVED_ENTRY_GRACE_MS
```

---

## 13. Session Management Flow

### ASCII Flow Diagram

```
[Session Key Construction]                           src/routing/session-key.ts
  |
  |-- Format: agent:{agentId}:{scope-suffix}
  |
  |-- DM Session Scopes (cfg.session.dmScope):
  |     "main":
  |       agent:{agentId}:main
  |       All DMs share one session regardless of sender/channel
  |
  |     "per-peer":
  |       agent:{agentId}:direct:{peerId}
  |       One session per peer across all channels
  |
  |     "per-channel-peer":
  |       agent:{agentId}:{channel}:direct:{peerId}
  |       One session per peer per channel
  |
  |     "per-account-channel-peer":
  |       agent:{agentId}:{channel}:{accountId}:direct:{peerId}
  |       Full isolation by account + channel + peer
  |
  |-- Group Sessions:
  |     agent:{agentId}:{channel}:{chatType}:{peerId}
  |     chatType = "group" or "channel"
  |
  |-- Thread Sessions:
  |     {baseSessionKey}:thread:{threadId}
  |     resolveThreadSessionKeys(): appends thread suffix
  |
  |-- Subagent Sessions:
  |     agent:{agentId}:subagent:{parentSessionKey}:{subagentId}
  |     isSubagentSessionKey(): prefix check
  |
  |-- ACP Sessions:
  |     agent:{agentId}:acp:{...}
  |     isAcpSessionKey(): prefix check
  |
  |-- Cron Sessions:
  |     agent:{agentId}:cron:{jobId}:run:{runId}
  |     isCronRunSessionKey(): regex check
  |
  +-- Identity Links:
        cfg.session.identityLinks: { canonical: [id1, id2, ...] }
        resolveLinkedPeerId(): collapses cross-channel identities
        Scoped candidates: both bare peerId and channel:peerId
        |
        v
[Session Store]                                      src/config/sessions.ts
  |-- JSON file per agent: ~/.openclaw/agents/<agentId>/sessions.json
  |-- Session entries keyed by sessionKey (lowercase)
  |-- Each entry tracks:
  |     lastRoute: { channel, to } (for reply targeting)
  |     model: current model override
  |     ttsAuto: TTS auto mode override
  |     timestamps
  |-- updateSessionStore(): atomic read-modify-write
  |-- loadSessionStore(): read current state
  +-- resolveStorePath(): per-agent store location
        |
        v
[Session Transcript]
  |-- JSONL file per session
  |-- resolveSessionFilePath(): ~/.openclaw/agents/<agentId>/sessions/<sessionKey>.jsonl
  |-- SessionManager from @mariozechner/pi-coding-agent
  |-- Messages: user, assistant, toolResult
  |-- Version header: { type: "session", version, id, timestamp, cwd }
  +-- repairSessionFileIfNeeded(): fix corrupted transcripts
        |
        v
[Model Overrides]                                    src/sessions/model-overrides.ts
  |-- Per-session model override via /model directive
  |-- Stored in session entry
  |-- Precedence:
  |     1. Session-level override (from /model command)
  |     2. Agent-level config (cfg.agents.list[].model)
  |     3. Global default (cfg.agents.defaults.model)
  +-- resolveSessionModelRef(): resolves final model
        |
        v
[Level Overrides]                                    src/sessions/level-overrides.ts
  |-- /think directive: set thinking level per session
  |-- /verbose directive: set verbose mode per session
  |-- /elevated directive: toggle elevated mode
  +-- Stored in session store, applied at run time
        |
        v
[Session Write Lock]                                 src/agents/session-write-lock.ts
  |-- acquireSessionWriteLock(sessionKey)
  |-- Prevents concurrent writes to same session
  |-- Lock held during agent execution
  +-- Released on completion or error
```

### Binding Precedence for Agent Selection

The routing system uses strict precedence when matching bindings. Given a message context with channel, accountId, peer, guildId, teamId, and memberRoleIds, the tiers are evaluated in order:

1. **binding.peer** -- Exact peer match (kind + id). Most specific: routes a specific user to a specific agent.
2. **binding.peer.parent** -- Thread parent peer match. Used for binding inheritance when the thread peer does not match directly.
3. **binding.guild+roles** -- Guild (server) + Discord role match. Routes users with specific roles to agents.
4. **binding.guild** -- Guild-only match (no role constraint).
5. **binding.team** -- Team match (e.g., Slack workspace).
6. **binding.account** -- Specific account binding (accountId is not wildcard `*`).
7. **binding.channel** -- Channel-level binding with wildcard account.
8. **default** -- Falls back to `resolveDefaultAgentId()`.

---

## 14. Media Understanding Flow

### ASCII Flow Diagram

```
[Inbound Message with Attachments]
        |
        v
[Normalize Attachments]                              src/media-understanding/attachments.ts
  |-- normalizeAttachments(ctx):
  |     Extract from MsgContext: MediaUrl, MediaType, MediaTypes
  |     Build MediaAttachment[]: { url, type, filename, size }
  |-- selectAttachments(): filter by capability
  |-- MediaAttachmentCache: cache downloaded files
        |
        v
[Scope Decision]                                     src/media-understanding/scope.ts
  |-- resolveMediaUnderstandingScope():
  |     Check scope config per:
  |       chatType: "direct"|"group"|"channel"
  |       channel: telegram, discord, etc.
  |       sessionKey: specific session patterns
  |-- Returns "allow" or "deny"
  +-- If "deny": skip media understanding entirely
        |
        v
[Provider Registry]                                  src/media-understanding/providers/index.ts
  |-- buildMediaUnderstandingRegistry()
  |-- Registered providers:
  |     anthropic:  image description                src/media-understanding/providers/anthropic/
  |     openai:     image + audio (Whisper)           src/media-understanding/providers/openai/
  |     google:     image + audio + video             src/media-understanding/providers/google/
  |     groq:       image description                 src/media-understanding/providers/groq/
  |     deepgram:   audio transcription               src/media-understanding/providers/deepgram/
  |     minimax:    image description                 src/media-understanding/providers/minimax/
  |     zai:        image description                 src/media-understanding/providers/zai/
  |-- Each provider declares:
  |     capabilities: ["image", "audio", "video"]
  |     execute(params): process attachment
  +-- CLI providers: external command execution
        |
        v
[Model Entry Resolution]                             src/media-understanding/resolve.ts
  |-- resolveModelEntries(cfg, capability):
  |     Check cfg.tools.media.<capability>.models[]
  |     Check cfg.tools.media.<capability>.provider
  |     Auto-detect from available API keys:
  |       AUTO_IMAGE_KEY_PROVIDERS
  |       AUTO_AUDIO_KEY_PROVIDERS
  |       AUTO_VIDEO_KEY_PROVIDERS
  |-- Each entry: { provider, model?, command?, type: "provider"|"cli" }
  +-- Fallback: DEFAULT_IMAGE_MODELS
        |
        v
[Per-Capability Processing]
  |
  |  [Image Processing]
  |  |-- Provider execute():
  |  |     Send image data + prompt to vision model
  |  |     resolvePrompt(): "Describe this image..."
  |  |     resolveMaxChars(): limit description length
  |  |     resolveMaxBytes(): limit input size
  |  |-- Provider-specific:
  |  |     Anthropic: messages API with image content block
  |  |     OpenAI: chat completions with image_url
  |  |     Google: generateContent with inline data     src/media-understanding/providers/google/inline-data.ts
  |  |     Groq: chat completions with image support
  |  +-- Output: { text: description, provider, model }
  |
  |  [Audio Processing]
  |  |-- Deepgram:                                   src/media-understanding/providers/deepgram/audio.ts
  |  |     POST /v1/listen with audio data
  |  |     Returns transcript text
  |  |-- OpenAI Whisper:                             src/media-understanding/providers/openai/audio.ts
  |  |     POST /v1/audio/transcriptions
  |  |     Model: whisper-1
  |  |-- Google:                                     src/media-understanding/providers/google/audio.ts
  |  |     generateContent with audio content
  |  +-- Output: { text: transcript, provider }
  |
  |  [Video Processing]                              src/media-understanding/video.ts
  |  |-- Frame extraction (ffmpeg if available)
  |  |-- Google native video:                        src/media-understanding/providers/google/video.ts
  |  |     Upload video to Google API
  |  |     generateContent with video reference
  |  +-- Output: combined frame descriptions
  |
  +-- [CLI Processing]
       External command execution
       runCliEntry(): spawn process with attachment path
       Parse stdout as description
        |
        v
[Apply to Context]                                   src/media-understanding/apply.ts
  |-- applyMediaUnderstanding(ctx, cfg, agentDir, activeModel)
  |-- For each capability + attachment:
  |     Run provider(s) to get output
  |     Format output:                               src/media-understanding/format.ts
  |       Image: [Image: <description>]
  |       Audio: [Audio transcript: <text>]
  |       Video: [Video: <description>]
  |-- Append formatted output to MsgContext.Body
  |-- Concurrency control:                           src/media-understanding/concurrency.ts
  |     DEFAULT_MEDIA_CONCURRENCY limit
  +-- Error recovery: isMediaUnderstandingSkipError() -> graceful skip
```

---

## Summary of Key Data Paths

| Flow | Entry Point | Core Processing | Output |
|------|-------------|-----------------|--------|
| Channel Inbound | Channel plugin handler | Debounce -> Gate -> Route -> Dispatch -> Agent | Reply payload to channel |
| Gateway Request | WebSocket connect | Auth -> Method dispatch -> Handler | Response frame + events |
| Chat Send | `chat.send` method | Build context -> Agent run -> Stream events | Agent events to clients |
| Node Invoke | Agent tool call | Gateway -> node.invoke -> device | Tool result to agent |
| Config Load | `loadConfig()` | Read -> Include -> Env sub -> Validate -> Defaults | OpenClawConfig object |
| Plugin Load | `loadPlugins()` | Discover -> Load -> Register -> Activate | PluginRegistry |
| Agent Run | `runEmbeddedPiAgent()` | Model -> Auth -> Tools -> Prompt -> Stream -> Result | EmbeddedPiRunResult |
| Cron Job | Timer fires | executeJob -> agent run -> delivery | Announcement to channel |
| Voice | Wake word -> record | Transcribe -> Agent -> TTS | Audio response |
| Pairing | Device connect | Code generation -> approval -> token | Authenticated connection |
| Exec Approval | Tool requests exec | Broadcast -> user decision -> apply | Allow/deny decision |
| Session | Route resolution | Key build -> store update -> transcript | Persisted conversation |
| Media | Attachment detected | Provider selection -> process -> format | Context enrichment |

### File Reference Index

| Component | Primary Files |
|-----------|--------------|
| Channel Registry | `src/channels/registry.ts`, `src/channels/plugins/` |
| Inbound Debounce | `src/auto-reply/inbound-debounce.ts` |
| Gating Pipeline | `src/channels/allowlist-match.ts`, `src/channels/mention-gating.ts`, `src/channels/command-gating.ts`, `src/sessions/send-policy.ts` |
| Routing | `src/routing/resolve-route.ts`, `src/routing/bindings.ts`, `src/routing/session-key.ts` |
| Session Keys | `src/sessions/session-key-utils.ts` |
| Dispatch | `src/auto-reply/dispatch.ts`, `src/auto-reply/reply/dispatch-from-config.ts` |
| Reply System | `src/auto-reply/reply/get-reply.ts`, `src/auto-reply/reply/reply-dispatcher.ts` |
| Chunking | `src/auto-reply/chunk.ts` |
| Agent Runner | `src/agents/pi-embedded-runner/run.ts`, `src/agents/pi-embedded-runner/run/attempt.ts` |
| Streaming | `src/agents/pi-embedded-subscribe.ts` |
| Tools | `src/agents/pi-tools.ts`, `src/agents/tool-policy.ts`, `src/agents/tool-policy-pipeline.ts` |
| System Prompt | `src/agents/pi-embedded-runner/system-prompt.ts`, `src/agents/system-prompt-params.ts` |
| Model Auth | `src/agents/model-auth.ts`, `src/agents/auth-profiles.ts` |
| Gateway Server | `src/gateway/server.impl.ts`, `src/gateway/server-methods.ts` |
| Gateway Auth | `src/gateway/auth.ts`, `src/gateway/device-auth.ts` |
| Gateway Broadcast | `src/gateway/server-broadcast.ts` |
| Node Registry | `src/gateway/node-registry.ts` |
| Exec Approval | `src/gateway/exec-approval-manager.ts` |
| Config IO | `src/config/io.ts`, `src/config/env-substitution.ts`, `src/config/includes.ts`, `src/config/defaults.ts`, `src/config/validation.ts` |
| Plugin System | `src/plugins/discovery.ts`, `src/plugins/loader.ts`, `src/plugins/registry.ts`, `src/plugins/runtime.ts` |
| Cron Service | `src/cron/service.ts`, `src/cron/service/ops.ts`, `src/cron/isolated-agent/run.ts`, `src/cron/delivery.ts` |
| TTS | `src/tts/tts.ts`, `src/tts/tts-core.ts` |
| Media Understanding | `src/media-understanding/runner.ts`, `src/media-understanding/resolve.ts`, `src/media-understanding/apply.ts` |
| Pairing | `src/pairing/pairing-store.ts`, `src/pairing/pairing-messages.ts` |
| Session Tool Guard | `src/agents/session-tool-result-guard.ts` |
| Usage Tracking | `src/agents/usage.ts` |
