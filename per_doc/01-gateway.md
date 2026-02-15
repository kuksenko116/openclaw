# OpenClaw Gateway Architecture

The OpenClaw Gateway is a WebSocket server that serves as the central communication hub
for all OpenClaw clients (Control UI, mobile apps, CLI, nodes). It multiplexes RPC
method calls, broadcasts real-time events, manages chat sessions, orchestrates
sidecar services, and exposes HTTP-compatible API endpoints.

All source code lives under `src/gateway/`.

---

## Table of Contents

1. [File Map](#file-map)
2. [Startup Sequence](#startup-sequence)
3. [Runtime State](#runtime-state)
4. [Configuration Resolution](#configuration-resolution)
5. [WebSocket Connection Lifecycle](#websocket-connection-lifecycle)
6. [Protocol Framing](#protocol-framing)
7. [Method Routing and Handler Registry](#method-routing-and-handler-registry)
8. [Authorization Model](#authorization-model)
9. [RPC Methods (Complete List)](#rpc-methods-complete-list)
10. [Event Broadcasting](#event-broadcasting)
11. [Server-Broadcast Events (Complete List)](#server-broadcast-events-complete-list)
12. [Chat Run State Machine](#chat-run-state-machine)
13. [Node Registry and Invocation](#node-registry-and-invocation)
14. [Node Subscriptions (Pub/Sub)](#node-subscriptions-pubsub)
15. [Node Event Processing](#node-event-processing)
16. [Cron Service Integration](#cron-service-integration)
17. [Maintenance Timers](#maintenance-timers)
18. [Sidecar Services](#sidecar-services)
19. [Authentication](#authentication)
20. [Rate Limiting](#rate-limiting)
21. [Exec Approval Manager](#exec-approval-manager)
22. [Control UI Hosting](#control-ui-hosting)
23. [OpenAI-Compatible HTTP Endpoints](#openai-compatible-http-endpoints)
24. [Protocol Schema Layer](#protocol-schema-layer)
25. [Shutdown and Cleanup](#shutdown-and-cleanup)
26. [Key Data Structures](#key-data-structures)
27. [Constants and Tuning](#constants-and-tuning)
28. [Config Hot-Reload](#config-hot-reload)

---

## File Map

### Core Server

| File | Purpose |
|------|---------|
| `server.impl.ts` | Main gateway entry point (`startGatewayServer`). Orchestrates the full startup sequence (2000+ lines). Wires all subsystems together and returns a `GatewayServer` handle with a `close()` method. |
| `server.ts` | Public re-export of the gateway server. |
| `server-runtime-state.ts` | Factory that creates the runtime state bundle: HTTP server(s), WebSocketServer, client set, broadcaster, chat run state, abort controllers, dedupe map, tool event recipients. |
| `server-runtime-config.ts` | Resolves the `GatewayRuntimeConfig` from config file + CLI options + environment: bind host, auth mode, Control UI, OpenAI endpoints, Tailscale, hooks, canvas host. |
| `server-constants.ts` | Numeric constants governing protocol limits and timer intervals. |
| `server-shared.ts` | Shared types (`DedupeEntry`). |
| `server-utils.ts` | Error formatting utilities. |
| `server-session-key.ts` | Session key resolution for agent runs. |

### WebSocket Layer

| File | Purpose |
|------|---------|
| `server-ws-runtime.ts` | Thin adapter that calls `attachGatewayWsConnectionHandler` with the context. |
| `server/ws-connection.ts` | Connection handler: generates `connId`, runs handshake (challenge/connect/hello-ok), registers client, attaches message handler, handles close with presence update. |
| `server/ws-connection/message-handler.ts` | Per-message routing: parses JSON frame, validates `RequestFrame`, dispatches to `handleGatewayRequest`, sends `ResponseFrame`. |
| `server/ws-types.ts` | `GatewayWsClient` type definition. |

### Method Handlers

| File | Purpose |
|------|---------|
| `server-methods.ts` | Central handler registry. Merges all handler groups into `coreGatewayHandlers`. Contains `authorizeGatewayMethod()` for scope/role enforcement and `handleGatewayRequest()` for dispatch. |
| `server-methods-list.ts` | Canonical list of all gateway methods and events. `listGatewayMethods()` merges base methods with channel-plugin methods. |
| `server-methods/types.ts` | Type definitions: `GatewayClient`, `RespondFn`, `GatewayRequestContext`, `GatewayRequestOptions`, `GatewayRequestHandler`, `GatewayRequestHandlers`. |
| `server-methods/agent.ts` | `agent`, `agent.identity.get`, `agent.wait` handlers. |
| `server-methods/agent-job.ts` | Agent job execution logic. |
| `server-methods/agent-timestamp.ts` | Agent timestamp utilities. |
| `server-methods/agents.ts` | `agents.list`, `agents.create`, `agents.update`, `agents.delete`, `agents.files.*` handlers. |
| `server-methods/browser.ts` | `browser.request` handler. |
| `server-methods/channels.ts` | `channels.status`, `channels.logout` handlers. |
| `server-methods/chat.ts` | `chat.send`, `chat.abort`, `chat.history` handlers. |
| `server-methods/config.ts` | `config.get`, `config.set`, `config.apply`, `config.patch`, `config.schema` handlers. |
| `server-methods/connect.ts` | `connect` handshake handler. |
| `server-methods/cron.ts` | `cron.list`, `cron.status`, `cron.add`, `cron.update`, `cron.remove`, `cron.run`, `cron.runs` handlers. |
| `server-methods/devices.ts` | `device.pair.list`, `device.pair.approve`, `device.pair.reject`, `device.token.rotate`, `device.token.revoke` handlers. |
| `server-methods/exec-approval.ts` | `exec.approval.request`, `exec.approval.waitDecision`, `exec.approval.resolve` handlers. |
| `server-methods/exec-approvals.ts` | `exec.approvals.get`, `exec.approvals.set`, `exec.approvals.node.get`, `exec.approvals.node.set` handlers. |
| `server-methods/health.ts` | `health`, `status` handlers. |
| `server-methods/logs.ts` | `logs.tail` handler. |
| `server-methods/models.ts` | `models.list` handler. |
| `server-methods/nodes.ts` | `node.list`, `node.describe`, `node.invoke`, `node.invoke.result`, `node.event`, `node.pair.*`, `node.rename` handlers. |
| `server-methods/nodes.helpers.ts` | Node handler utilities (JSON parsing). |
| `server-methods/send.ts` | `send` handler (message delivery). |
| `server-methods/sessions.ts` | `sessions.list`, `sessions.preview`, `sessions.patch`, `sessions.reset`, `sessions.delete`, `sessions.compact` handlers. |
| `server-methods/skills.ts` | `skills.status`, `skills.bins`, `skills.install`, `skills.update` handlers. |
| `server-methods/system.ts` | `system-presence`, `system-event`, `last-heartbeat`, `set-heartbeats`, `wake` handlers. |
| `server-methods/talk.ts` | `talk.config`, `talk.mode` handlers. |
| `server-methods/tts.ts` | `tts.status`, `tts.providers`, `tts.enable`, `tts.disable`, `tts.convert`, `tts.setProvider` handlers. |
| `server-methods/update.ts` | `update.run` handler. |
| `server-methods/usage.ts` | `usage.status`, `usage.cost` handlers. |
| `server-methods/voicewake.ts` | `voicewake.get`, `voicewake.set` handlers. |
| `server-methods/web.ts` | `web.login` and related web handlers. |
| `server-methods/wizard.ts` | `wizard.start`, `wizard.next`, `wizard.cancel`, `wizard.status` handlers. |

### Broadcasting and Events

| File | Purpose |
|------|---------|
| `server-broadcast.ts` | `createGatewayBroadcaster()` -- iterates the client set, applies scope guards, drops slow consumers, serializes event frames with monotonic sequence numbers. |
| `server-chat.ts` | Chat run state machine: `ChatRunRegistry` (queue per sessionId), `ChatRunState` (buffers, deltaSentAt, abortedRuns), `ToolEventRecipientRegistry`, `createAgentEventHandler()`. |
| `chat-abort.ts` | `ChatAbortControllerEntry` and `abortChatRunById()`. |

### Node System

| File | Purpose |
|------|---------|
| `node-registry.ts` | `NodeRegistry` class: register/unregister nodes, invoke commands with timeout, handle invoke results. |
| `server-node-subscriptions.ts` | `NodeSubscriptionManager`: bidirectional node-to-session pub/sub for mobile nodes. |
| `server-node-events.ts` | `handleNodeEvent()`: processes `voice.transcript`, `agent.request`, `chat.subscribe`, `chat.unsubscribe`, `exec.started/finished/denied`. |
| `server-node-events-types.ts` | `NodeEventContext` and `NodeEvent` type definitions. |
| `server-mobile-nodes.ts` | `hasConnectedMobileNode()` utility. |
| `node-command-policy.ts` | Command policy enforcement for node invocations. |
| `node-invoke-sanitize.ts` | Input sanitization for node invoke payloads. |
| `node-invoke-system-run-approval.ts` | System run approval logic for node invocations. |

### Subsystem Integration

| File | Purpose |
|------|---------|
| `server-cron.ts` | `buildGatewayCronService()`: creates `CronService` with agent resolution, heartbeat hooks, isolated agent job runner, and event broadcasting. |
| `server-maintenance.ts` | `startGatewayMaintenanceTimers()`: tick, health refresh, dedupe cleanup, chat abort expiry. |
| `server-startup.ts` | `startGatewaySidecars()`: browser control, Gmail watcher, internal hooks, channels, plugin services, memory backend, restart sentinel. |
| `server-channels.ts` | `createChannelManager()`: lifecycle management for messaging channels (Telegram, Discord, etc.). |
| `server-lanes.ts` | Lane concurrency configuration. |
| `server-discovery.ts` / `server-discovery-runtime.ts` | mDNS/Bonjour and wide-area discovery. |
| `server-tailscale.ts` | Tailscale serve/funnel exposure. |
| `server-browser.ts` | Browser control server startup. |
| `server-plugins.ts` | `loadGatewayPlugins()`: loads plugin registry, merges plugin gateway handlers. |
| `server-model-catalog.ts` | Model catalog loading and caching. |
| `server-startup-log.ts` | Structured startup log output. |
| `server-startup-memory.ts` | QMD memory backend initialization. |
| `server-wizard-sessions.ts` | Wizard session tracking. |
| `server-reload-handlers.ts` | Handlers for config reload events. |
| `server-restart-sentinel.ts` | Restart sentinel file for coordinated restarts. |
| `server-close.ts` | `createGatewayCloseHandler()`: orderly shutdown of all subsystems. |

### Authentication

| File | Purpose |
|------|---------|
| `auth.ts` | `resolveGatewayAuth()`, `authorizeGatewayConnect()`, Tailscale identity verification, trusted-proxy auth. |
| `auth-rate-limit.ts` | `createAuthRateLimiter()`: sliding-window rate limiter with per-scope counters, lockout, loopback exemption. |
| `device-auth.ts` | Device identity and token management. |

### HTTP Endpoints

| File | Purpose |
|------|---------|
| `server-http.ts` | HTTP server creation and request routing. |
| `openai-http.ts` | `POST /v1/chat/completions` -- OpenAI Chat Completions API compatibility endpoint with SSE streaming. |
| `openresponses-http.ts` | `POST /v1/responses` -- OpenResponses protocol implementation with SSE streaming. |
| `open-responses.schema.ts` | TypeBox schema definitions for OpenResponses request/response types. |
| `http-common.ts` | Shared HTTP utilities (JSON body reading, SSE headers, auth failure responses). |
| `http-utils.ts` | Bearer token extraction, agent ID resolution, session key resolution for HTTP requests. |
| `control-ui.ts` | Control UI static file serving, SPA fallback, avatar handling, security headers. |
| `control-ui-shared.ts` | Control UI path normalization and avatar URL resolution. |
| `server/http-listen.ts` | HTTP server listen logic. |
| `server/hooks.ts` | Hooks HTTP request handler. |
| `server/plugins-http.ts` | Plugin HTTP request handler. |
| `server/tls.ts` | TLS runtime loading (cert/key). |

### Protocol and Schema

| File | Purpose |
|------|---------|
| `protocol/index.ts` | AJV-compiled validators for all protocol types (370+ validators). Re-exports all schema types. |
| `protocol/schema.ts` | Schema re-export barrel. |
| `protocol/client-info.ts` | Client info parsing. |
| `protocol/schema/frames.ts` | `ConnectParams`, `HelloOk`, `RequestFrame`, `ResponseFrame`, `EventFrame`, `ErrorShape`, `GatewayFrame`. |
| `protocol/schema/snapshot.ts` | `PresenceEntry`, `Snapshot`, `StateVersion`, `SessionDefaults`. |
| `protocol/schema/primitives.ts` | Primitive schemas (`NonEmptyString`, `GatewayClientId`, `GatewayClientMode`). |
| `protocol/schema/types.ts` | Shared type definitions. |
| `protocol/schema/error-codes.ts` | `ErrorCodes` enum and `errorShape()` factory. |
| `protocol/schema/agent.ts` | Agent event and identity schemas. |
| `protocol/schema/agents-models-skills.ts` | Agents, models, skills CRUD schemas. |
| `protocol/schema/channels.ts` | Channel status/logout schemas. |
| `protocol/schema/config.ts` | Config get/set/apply/patch schemas. |
| `protocol/schema/cron.ts` | Cron job CRUD schemas. |
| `protocol/schema/devices.ts` | Device pairing and token schemas. |
| `protocol/schema/exec-approvals.ts` | Exec approval request/resolve schemas. |
| `protocol/schema/logs-chat.ts` | Log tail and chat event schemas. |
| `protocol/schema/nodes.ts` | Node list/describe/invoke/event schemas. |
| `protocol/schema/protocol-schemas.ts` | Protocol-level schemas. |
| `protocol/schema/sessions.ts` | Session CRUD schemas. |
| `protocol/schema/wizard.ts` | Wizard flow schemas. |

### Other

| File | Purpose |
|------|---------|
| `boot.ts` | Gateway boot entrypoint. |
| `client.ts` | Gateway client library. |
| `call.ts` | Gateway RPC call utility. |
| `probe.ts` | Gateway probe/health check. |
| `origin-check.ts` | WebSocket origin validation. |
| `hooks.ts` | Hook config resolution. |
| `hooks-mapping.ts` | Hook event mapping. |
| `config-reload.ts` | `startGatewayConfigReloader()` -- file watcher for config hot-reload. |
| `agent-prompt.ts` | Agent prompt construction from conversation entries. |
| `assistant-identity.ts` | Assistant name/avatar resolution. |
| `chat-sanitize.ts` | Chat message sanitization. |
| `chat-attachments.ts` | Chat attachment handling. |
| `sessions-patch.ts` | Session patch logic. |
| `sessions-resolve.ts` | Session resolution. |
| `session-utils.ts` | Session store loading and manipulation. |
| `session-utils.fs.ts` | Session filesystem utilities. |
| `session-utils.types.ts` | Session utility types. |
| `tools-invoke-http.ts` | HTTP tool invocation. |
| `live-image-probe.ts` | Live image probing. |
| `net.ts` | Network utilities (bind host resolution, loopback detection, trusted proxy checks, client IP resolution). |
| `ws-log.ts` | WebSocket frame logging. |
| `ws-logging.ts` | WebSocket logging configuration. |

---

## Startup Sequence

`startGatewayServer(port, opts)` in `server.impl.ts` executes the following steps in order:

```
 1. CONFIG LOADING & VALIDATION
    |-- readConfigFileSnapshot()
    |-- Auto-migrate legacy config entries (migrateLegacyConfig)
    |-- Re-read and validate config
    |-- applyPluginAutoEnable() -- auto-enable plugins based on env vars
    |-- loadConfig() => cfgAtStart
    |
 2. CORE INITIALIZATION
    |-- Start diagnostic heartbeat if diagnostics enabled
    |-- Set SIGUSR1 restart policy
    |-- Set pre-restart deferral check (queue + pending replies + embedded runs)
    |-- initSubagentRegistry()
    |-- Resolve default agent ID and workspace directory
    |-- listGatewayMethods() => base method list
    |-- loadGatewayPlugins() => plugin registry + merged gateway methods
    |-- Create per-channel loggers and runtime envs
    |-- Merge channel plugin methods into gateway methods
    |
 3. RUNTIME CONFIG RESOLUTION
    |-- resolveGatewayRuntimeConfig() =>
    |     bindHost, controlUiEnabled, openAiChatCompletionsEnabled,
    |     openResponsesEnabled, resolvedAuth, tailscaleConfig, hooksConfig,
    |     canvasHostEnabled
    |-- Create auth rate limiter (if configured)
    |-- Resolve Control UI root (build assets if needed)
    |-- Load TLS runtime (if gateway.tls configured)
    |
 4. RUNTIME STATE CREATION
    |-- createGatewayRuntimeState() =>
    |     httpServer(s), WebSocketServer, client set, broadcaster,
    |     chat run state, abort controllers, dedupe map,
    |     tool event recipients, canvas host handler
    |
 5. SUBSYSTEM INITIALIZATION
    |-- new NodeRegistry()
    |-- createNodeSubscriptionManager()
    |-- applyGatewayLaneConcurrency()
    |-- buildGatewayCronService() => CronService
    |-- createChannelManager()
    |-- startGatewayDiscovery() => mDNS/Bonjour/wide-area
    |-- Prime remote skills cache
    |-- Register skills change listener
    |-- new ExecApprovalManager()
    |-- createWizardSessionTracker()
    |
 6. MAINTENANCE TIMERS
    |-- startGatewayMaintenanceTimers() =>
    |     tickInterval (30s), healthInterval (60s),
    |     dedupeCleanup (60s, includes chat abort expiry)
    |-- Prime health cache (initial refresh)
    |
 7. EVENT SUBSCRIPTIONS
    |-- onAgentEvent(createAgentEventHandler(...)) => agent bus listener
    |-- onHeartbeatEvent(...) => heartbeat event broadcaster
    |-- startHeartbeatRunner() => scheduled heartbeat execution
    |-- cron.start() => begin cron scheduling
    |-- Recover pending outbound deliveries
    |
 8. WEBSOCKET HANDLER ATTACHMENT
    |-- attachGatewayWsHandlers() =>
    |     wss.on("connection", ...) with full context
    |     (methods, events, auth, broadcast, requestContext)
    |
 9. SIDECAR SERVICES
    |-- startGatewaySidecars() =>
    |     browser control server, Gmail watcher,
    |     internal hooks, channels, plugin services,
    |     memory backend, restart sentinel
    |
10. TAILSCALE & DISCOVERY
    |-- startGatewayTailscaleExposure()
    |-- scheduleGatewayUpdateCheck()
    |
11. CONFIG RELOAD LISTENER
    |-- startGatewayConfigReloader() => file watcher for hot-reload
    |
12. RETURN GatewayServer
    |-- { close: createGatewayCloseHandler(...) }
```

---

## Runtime State

`createGatewayRuntimeState()` in `server-runtime-state.ts` builds the core runtime
bundle. It returns:

```typescript
{
  canvasHost: CanvasHostHandler | null;       // Canvas host HTTP handler
  httpServer: HttpServer;                     // Primary HTTP server
  httpServers: HttpServer[];                  // All bound HTTP servers (multi-bind)
  httpBindHosts: string[];                    // Bound host addresses
  wss: WebSocketServer;                       // WebSocket server (noServer mode)
  clients: Set<GatewayWsClient>;             // Connected WebSocket clients
  broadcast: (event, payload, opts?) => void; // Broadcast to all clients
  broadcastToConnIds: (event, payload, connIds, opts?) => void;  // Targeted broadcast
  agentRunSeq: Map<string, number>;           // Per-run sequence counters
  dedupe: Map<string, DedupeEntry>;           // Request deduplication cache
  chatRunState: ChatRunState;                 // Full chat run state machine
  chatRunBuffers: Map<string, string>;        // In-progress assistant text
  chatDeltaSentAt: Map<string, number>;       // Throttle timestamps for deltas
  addChatRun: (sessionId, entry) => void;     // Register a chat run
  removeChatRun: (sessionId, clientRunId, sessionKey?) => ChatRunEntry | undefined;
  chatAbortControllers: Map<string, ChatAbortControllerEntry>;
  toolEventRecipients: ToolEventRecipientRegistry;
}
```

The HTTP server is created with `createGatewayHttpServer()` and supports multiple
bind hosts (e.g., both `127.0.0.1` and `::1`). The WebSocket server is created in
`noServer` mode and attached via HTTP upgrade handlers.

---

## Configuration Resolution

`resolveGatewayRuntimeConfig()` in `server-runtime-config.ts` produces:

```typescript
type GatewayRuntimeConfig = {
  bindHost: string;                    // Resolved bind address
  controlUiEnabled: boolean;           // Serve the browser Control UI
  openAiChatCompletionsEnabled: boolean;  // Serve /v1/chat/completions
  openResponsesEnabled: boolean;       // Serve /v1/responses
  openResponsesConfig?: GatewayHttpResponsesConfig;
  controlUiBasePath: string;           // URL prefix for Control UI
  controlUiRoot?: string;              // Override for Control UI asset root
  resolvedAuth: ResolvedGatewayAuth;   // Resolved authentication config
  authMode: ResolvedGatewayAuthMode;   // "none" | "token" | "password" | "trusted-proxy"
  tailscaleConfig: GatewayTailscaleConfig;
  tailscaleMode: "off" | "serve" | "funnel";
  hooksConfig: ReturnType<typeof resolveHooksConfig>;
  canvasHostEnabled: boolean;
};
```

Safety validations enforced:

- Tailscale funnel requires `auth.mode=password`
- Tailscale serve/funnel requires `bind=loopback`
- Non-loopback bind requires a shared secret (token or password) unless `trusted-proxy`
- `trusted-proxy` mode requires non-loopback bind and `trustedProxies` array
- Auth mode assertions (token mode requires token, password mode requires password, etc.)

---

## WebSocket Connection Lifecycle

Defined in `server/ws-connection.ts`:

```
CLIENT                              GATEWAY
  |                                    |
  |--- WebSocket upgrade request ----->|
  |                                    |-- Generate connId (UUID)
  |                                    |-- Extract headers (host, origin, user-agent,
  |                                    |   x-forwarded-for, x-real-ip)
  |                                    |-- Set handshake timeout (10s default)
  |                                    |
  |<-- EventFrame: connect.challenge --|  { nonce }
  |                                    |
  |--- RequestFrame: connect --------->|
  |    { minProtocol, maxProtocol,     |-- Validate ConnectParams against schema
  |      client: { id, version,        |-- Check protocol version compatibility
  |        platform, mode },           |-- Validate role ("operator" | "node")
  |      role?, scopes?,               |-- Authenticate:
  |      auth?: { token?, password? }, |     a) device identity (signature + nonce)
  |      device?: { id, publicKey,     |     b) trusted-proxy (header-based)
  |        signature, signedAt } }     |     c) token / password (shared secret)
  |                                    |     d) Tailscale identity (whois)
  |                                    |     e) loopback = auth-free
  |                                    |-- Rate-limit check
  |                                    |
  |<-- ResponseFrame: hello-ok --------|
  |    { protocol, server: { version,  |-- Add client to clients Set
  |        host, connId },             |-- Register node (if role=node)
  |      features: { methods, events },|-- Upsert presence entry
  |      snapshot: { presence[],       |-- Build snapshot (presence, health,
  |        health, stateVersion,       |   session defaults, auth mode)
  |        uptimeMs, configPath,       |-- Broadcast presence update
  |        sessionDefaults, authMode },|-- Clear handshake timeout
  |      policy: { maxPayload,         |
  |        maxBufferedBytes,           |
  |        tickIntervalMs },           |
  |      canvasHostUrl?, auth? }       |
  |                                    |
  |=== Bidirectional RPC + Events =====|
  |                                    |
  |--- RequestFrame: { method } ------>|-- Parse JSON
  |                                    |-- Validate RequestFrame schema
  |                                    |-- authorizeGatewayMethod(method, client)
  |                                    |-- Dispatch to handler
  |<-- ResponseFrame: { ok, payload }--|
  |                                    |
  |<-- EventFrame: { event, payload }--|  (server-initiated broadcasts)
  |                                    |
  |--- close ------------------------->|
  |                                    |-- Remove from clients Set
  |                                    |-- Unregister node (if role=node)
  |                                    |-- Remove presence entry
  |                                    |-- Broadcast presence update
  |                                    |-- Log close with diagnostics
```

### Handshake Timeout

The gateway enforces a 10-second handshake timeout (`DEFAULT_HANDSHAKE_TIMEOUT_MS`).
If the client does not complete the `connect` handshake within this window, the
socket is closed.

---

## Protocol Framing

All messages are JSON-encoded. Three frame types form a discriminated union on the
`type` field:

```typescript
// Client -> Server: RPC request
type RequestFrame = {
  type: "req";
  id: string;        // Client-generated request ID (for correlation)
  method: string;    // RPC method name
  params?: unknown;  // Method-specific parameters
};

// Server -> Client: RPC response
type ResponseFrame = {
  type: "res";
  id: string;        // Echoed request ID
  ok: boolean;       // Success/failure
  payload?: unknown; // Method-specific result
  error?: ErrorShape;
};

// Server -> Client: Async event
type EventFrame = {
  type: "event";
  event: string;            // Event name
  payload?: unknown;        // Event-specific data
  seq?: number;             // Monotonically increasing sequence (broadcast only)
  stateVersion?: {          // For optimistic concurrency
    presence: number;
    health: number;
  };
};

// Error detail shape
type ErrorShape = {
  code: string;       // One of ErrorCodes
  message: string;    // Human-readable message
  details?: unknown;  // Optional structured details
  retryable?: boolean;
  retryAfterMs?: number;
};
```

### Error Codes

```typescript
const ErrorCodes = {
  NOT_LINKED: "NOT_LINKED",
  NOT_PAIRED: "NOT_PAIRED",
  AGENT_TIMEOUT: "AGENT_TIMEOUT",
  INVALID_REQUEST: "INVALID_REQUEST",
  UNAVAILABLE: "UNAVAILABLE",
};
```

### Payload Size Limits

```
MAX_PAYLOAD_BYTES     = 8 MiB    // Incoming WebSocket frame cap
MAX_BUFFERED_BYTES    = 16 MiB   // Per-connection send buffer limit (2x max payload)
```

---

## Method Routing and Handler Registry

`server-methods.ts` defines the dispatch pipeline:

```
RequestFrame
    |
    v
authorizeGatewayMethod(method, client)
    |-- Check role (node vs operator)
    |-- Check scopes (admin, read, write, approvals, pairing)
    |-- Return ErrorShape or null
    |
    v (if authorized)
handler = extraHandlers[method] ?? coreGatewayHandlers[method]
    |
    v (if found)
handler({ req, params, client, isWebchatConnect, respond, context })
    |
    v
respond(ok, payload?, error?)  -->  ResponseFrame sent to client
```

The `coreGatewayHandlers` object is assembled by spreading 27 handler group objects:

```typescript
const coreGatewayHandlers: GatewayRequestHandlers = {
  ...connectHandlers,
  ...logsHandlers,
  ...voicewakeHandlers,
  ...healthHandlers,
  ...channelsHandlers,
  ...chatHandlers,
  ...cronHandlers,
  ...deviceHandlers,
  ...execApprovalsHandlers,
  ...webHandlers,
  ...modelsHandlers,
  ...configHandlers,
  ...wizardHandlers,
  ...talkHandlers,
  ...ttsHandlers,
  ...skillsHandlers,
  ...sessionsHandlers,
  ...systemHandlers,
  ...updateHandlers,
  ...nodeHandlers,
  ...sendHandlers,
  ...usageHandlers,
  ...agentHandlers,
  ...agentsHandlers,
  ...browserHandlers,
};
```

Plugin handlers are merged as `extraHandlers` and take priority over core handlers.

### Handler Signature

```typescript
type GatewayRequestHandler = (opts: {
  req: RequestFrame;
  params: Record<string, unknown>;
  client: GatewayClient | null;
  isWebchatConnect: (params: ConnectParams | null | undefined) => boolean;
  respond: RespondFn;
  context: GatewayRequestContext;
}) => Promise<void> | void;
```

### Request Context

Every handler receives a `GatewayRequestContext` providing access to all subsystems:

```typescript
type GatewayRequestContext = {
  deps: CliDeps;
  cron: CronService;
  cronStorePath: string;
  execApprovalManager?: ExecApprovalManager;
  loadGatewayModelCatalog: () => Promise<ModelCatalogEntry[]>;
  getHealthCache: () => HealthSummary | null;
  refreshHealthSnapshot: (opts?) => Promise<HealthSummary>;
  logHealth: { error: (message: string) => void };
  logGateway: SubsystemLogger;
  incrementPresenceVersion: () => number;
  getHealthVersion: () => number;
  broadcast: (event, payload, opts?) => void;
  broadcastToConnIds: (event, payload, connIds, opts?) => void;
  nodeSendToSession: (sessionKey, event, payload) => void;
  nodeSendToAllSubscribed: (event, payload) => void;
  nodeSubscribe: (nodeId, sessionKey) => void;
  nodeUnsubscribe: (nodeId, sessionKey) => void;
  nodeUnsubscribeAll: (nodeId) => void;
  hasConnectedMobileNode: () => boolean;
  nodeRegistry: NodeRegistry;
  agentRunSeq: Map<string, number>;
  chatAbortControllers: Map<string, ChatAbortControllerEntry>;
  chatAbortedRuns: Map<string, number>;
  chatRunBuffers: Map<string, string>;
  chatDeltaSentAt: Map<string, number>;
  addChatRun: (sessionId, entry) => void;
  removeChatRun: (sessionId, clientRunId, sessionKey?) => ChatRunEntry | undefined;
  registerToolEventRecipient: (runId, connId) => void;
  dedupe: Map<string, DedupeEntry>;
  wizardSessions: Map<string, WizardSession>;
  findRunningWizard: () => string | null;
  purgeWizardSession: (id) => void;
  getRuntimeSnapshot: () => ChannelRuntimeSnapshot;
  startChannel: (channel, accountId?) => Promise<void>;
  stopChannel: (channel, accountId?) => Promise<void>;
  markChannelLoggedOut: (channelId, cleared, accountId?) => void;
  wizardRunner: (...) => Promise<void>;
  broadcastVoiceWakeChanged: (triggers) => void;
};
```

---

## Authorization Model

Authorization is enforced per-method in `authorizeGatewayMethod()`:

### Role-Based Access

```
+----------+-----------------------------+
| Role     | Allowed Methods             |
+----------+-----------------------------+
| "node"   | NODE_ROLE_METHODS only:     |
|          |   node.invoke.result        |
|          |   node.event                |
|          |   skills.bins               |
+----------+-----------------------------+
| "operator"| Everything else (scoped)   |
+----------+-----------------------------+
```

### Scope-Based Access (Operator Role)

Operators must have appropriate scopes. An empty scopes array means no permissions.
The `operator.admin` scope grants access to everything.

```
+---------------------+-----------------------------------------------+
| Scope               | Grants Access To                              |
+---------------------+-----------------------------------------------+
| operator.admin      | ALL methods (superuser)                       |
+---------------------+-----------------------------------------------+
| operator.read       | health, logs.tail, channels.status, status,   |
|                     | usage.status, usage.cost, tts.status,         |
|                     | tts.providers, models.list, agents.list,      |
|                     | agent.identity.get, skills.status,            |
|                     | voicewake.get, sessions.list,                 |
|                     | sessions.preview, cron.list, cron.status,     |
|                     | cron.runs, system-presence, last-heartbeat,   |
|                     | node.list, node.describe, chat.history,       |
|                     | config.get, talk.config                       |
+---------------------+-----------------------------------------------+
| operator.write      | send, agent, agent.wait, wake, talk.mode,     |
|                     | tts.enable, tts.disable, tts.convert,         |
|                     | tts.setProvider, voicewake.set, node.invoke,   |
|                     | chat.send, chat.abort, browser.request        |
|                     | (also grants operator.read methods)           |
+---------------------+-----------------------------------------------+
| operator.approvals  | exec.approval.request,                        |
|                     | exec.approval.waitDecision,                   |
|                     | exec.approval.resolve                         |
+---------------------+-----------------------------------------------+
| operator.pairing    | node.pair.*, device.pair.*, device.token.*,   |
|                     | node.rename                                   |
+---------------------+-----------------------------------------------+
```

Methods not in the read/write/approvals/pairing sets require `operator.admin`:
- All `config.*` (except config.get), `wizard.*`, `update.*`
- `channels.logout`, `agents.create/update/delete`, `skills.install/update`
- `cron.add/update/remove/run`, `sessions.patch/reset/delete/compact`
- `exec.approvals.*` (admin-level approval config)

### Event Scope Guards

Broadcast events are filtered per-client:

```typescript
const EVENT_SCOPE_GUARDS = {
  "exec.approval.requested": ["operator.approvals"],
  "exec.approval.resolved":  ["operator.approvals"],
  "device.pair.requested":   ["operator.pairing"],
  "device.pair.resolved":    ["operator.pairing"],
  "node.pair.requested":     ["operator.pairing"],
  "node.pair.resolved":      ["operator.pairing"],
};
```

---

## RPC Methods (Complete List)

Organized by handler file / functional group:

### Agent (`server-methods/agent.ts`, `agents.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `agent` | write | Run an agent turn |
| `agent.identity.get` | read | Get resolved assistant identity |
| `agent.wait` | write | Wait for a running agent to complete |
| `agents.list` | read | List configured agents |
| `agents.create` | admin | Create a new agent |
| `agents.update` | admin | Update an agent's configuration |
| `agents.delete` | admin | Delete an agent |
| `agents.files.list` | admin | List files for an agent |
| `agents.files.get` | admin | Read an agent file |
| `agents.files.set` | admin | Write an agent file |

### Chat (`server-methods/chat.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `chat.send` | write | Send a message and start a chat run |
| `chat.abort` | write | Abort a running chat |
| `chat.history` | read | Retrieve chat history for a session |

### Config (`server-methods/config.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `config.get` | read | Get current config |
| `config.set` | admin | Replace entire config |
| `config.apply` | admin | Apply a partial config merge |
| `config.patch` | admin | Apply JSON patch operations |
| `config.schema` | admin | Get config JSON schema |

### Sessions (`server-methods/sessions.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `sessions.list` | read | List all sessions |
| `sessions.preview` | read | Preview a session's recent messages |
| `sessions.patch` | admin | Modify session metadata |
| `sessions.reset` | admin | Reset a session (clear history) |
| `sessions.delete` | admin | Delete a session |
| `sessions.compact` | admin | Compact session history |

### Cron (`server-methods/cron.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `cron.list` | read | List cron jobs |
| `cron.status` | read | Get cron service status |
| `cron.add` | admin | Add a cron job |
| `cron.update` | admin | Update a cron job |
| `cron.remove` | admin | Remove a cron job |
| `cron.run` | admin | Manually trigger a cron job |
| `cron.runs` | read | Get cron run history |

### Nodes (`server-methods/nodes.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `node.list` | read | List connected nodes |
| `node.describe` | read | Describe a specific node |
| `node.invoke` | write | Invoke a command on a node |
| `node.invoke.result` | node | Report invoke result (node role only) |
| `node.event` | node | Report a node event (node role only) |
| `node.pair.request` | pairing | Request node pairing |
| `node.pair.list` | pairing | List pending pair requests |
| `node.pair.approve` | pairing | Approve a pair request |
| `node.pair.reject` | pairing | Reject a pair request |
| `node.pair.verify` | pairing | Verify a paired node |
| `node.rename` | pairing | Rename a node |

### Devices (`server-methods/devices.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `device.pair.list` | pairing | List pending device pair requests |
| `device.pair.approve` | pairing | Approve a device pair request |
| `device.pair.reject` | pairing | Reject a device pair request |
| `device.token.rotate` | pairing | Rotate a device token |
| `device.token.revoke` | pairing | Revoke a device token |

### Channels (`server-methods/channels.ts`, `talk.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `channels.status` | read | Get channel status |
| `channels.logout` | admin | Logout from a channel |
| `talk.config` | read | Get talk/voice config |
| `talk.mode` | write | Set talk mode |

### Skills (`server-methods/skills.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `skills.status` | read | Get installed skills status |
| `skills.bins` | node | Get skill binaries (node role) |
| `skills.install` | admin | Install a skill |
| `skills.update` | admin | Update a skill |

### TTS (`server-methods/tts.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `tts.status` | read | Get TTS status |
| `tts.providers` | read | List TTS providers |
| `tts.enable` | write | Enable TTS |
| `tts.disable` | write | Disable TTS |
| `tts.convert` | write | Convert text to speech |
| `tts.setProvider` | write | Set TTS provider |

### Health & Status (`server-methods/health.ts`, `usage.ts`, `logs.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `health` | read | Get health summary |
| `status` | read | Get gateway status |
| `logs.tail` | read | Tail recent log entries |
| `usage.status` | read | Get API usage status |
| `usage.cost` | read | Get cost breakdown |

### Models (`server-methods/models.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `models.list` | read | List available models |

### Exec Approvals (`server-methods/exec-approval.ts`, `exec-approvals.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `exec.approval.request` | approvals | Request exec approval |
| `exec.approval.waitDecision` | approvals | Wait for approval decision |
| `exec.approval.resolve` | approvals | Resolve (approve/deny) an exec request |
| `exec.approvals.get` | admin | Get exec approval policy |
| `exec.approvals.set` | admin | Set exec approval policy |
| `exec.approvals.node.get` | admin | Get node exec approval policy |
| `exec.approvals.node.set` | admin | Set node exec approval policy |

### System (`server-methods/system.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `system-presence` | read | Get system presence entries |
| `system-event` | - | Emit a system event |
| `last-heartbeat` | read | Get last heartbeat result |
| `set-heartbeats` | - | Configure heartbeat schedule |
| `wake` | write | Wake the agent |

### Voicewake (`server-methods/voicewake.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `voicewake.get` | read | Get voice wake triggers |
| `voicewake.set` | write | Set voice wake triggers |

### Other (`server-methods/send.ts`, `browser.ts`, `web.ts`, `update.ts`, `wizard.ts`, `connect.ts`)

| Method | Scope | Description |
|--------|-------|-------------|
| `send` | write | Send a message to the agent |
| `browser.request` | write | Request browser interaction |
| `web.login` | - | Initiate web login |
| `update.run` | admin | Run an update |
| `wizard.start` | admin | Start onboarding wizard |
| `wizard.next` | admin | Advance wizard step |
| `wizard.cancel` | admin | Cancel wizard |
| `wizard.status` | admin | Get wizard status |

---

## Event Broadcasting

`createGatewayBroadcaster()` in `server-broadcast.ts` provides two broadcast functions:

### `broadcast(event, payload, opts?)`

Broadcasts to ALL connected clients:

1. Serializes `{ type: "event", event, payload, seq, stateVersion }` to JSON
2. Assigns a monotonically increasing `seq` number
3. Iterates all clients in the `clients` Set
4. Checks `hasEventScope(client, event)` -- applies EVENT_SCOPE_GUARDS
5. Checks `socket.bufferedAmount > MAX_BUFFERED_BYTES`:
   - If slow and `dropIfSlow=true`: skip this client
   - If slow and not droppable: close the socket with code 1008 "slow consumer"
6. Sends the frame via `socket.send()`

### `broadcastToConnIds(event, payload, connIds, opts?)`

Same logic but only targets clients whose `connId` is in the provided Set.
Used for tool events (sent only to clients that registered for a run).

---

## Server-Broadcast Events (Complete List)

Defined in `server-methods-list.ts`:

| Event | Payload Description |
|-------|-------------------|
| `connect.challenge` | `{ nonce }` -- sent immediately on connection open |
| `agent` | Agent run events (lifecycle, assistant text, tool calls) with `runId`, `stream`, `seq`, `data` |
| `chat` | Chat state updates: `{ runId, sessionKey, seq, state: "delta"|"final"|"error", message? }` |
| `presence` | Updated presence array after client connect/disconnect |
| `tick` | `{ ts }` -- periodic keepalive (every 30s) |
| `talk.mode` | Voice/talk mode change notification |
| `shutdown` | `{ reason, restartExpectedMs? }` -- server shutting down |
| `health` | Full health snapshot refresh |
| `heartbeat` | Heartbeat run events |
| `cron` | Cron job events (started, finished, error) |
| `node.pair.requested` | New node pairing request |
| `node.pair.resolved` | Node pairing resolved (approved/rejected) |
| `node.invoke.request` | Node invoke request dispatched to a node |
| `device.pair.requested` | New device pairing request |
| `device.pair.resolved` | Device pairing resolved (approved/rejected) |
| `voicewake.changed` | Voice wake triggers updated |
| `exec.approval.requested` | New exec approval request (scoped to `operator.approvals`) |
| `exec.approval.resolved` | Exec approval resolved (scoped to `operator.approvals`) |

---

## Chat Run State Machine

`server-chat.ts` manages the lifecycle of chat runs (initiated by `chat.send`):

```
chat.send (from client)
    |
    v
addChatRun(sessionId, { sessionKey, clientRunId })
    |-- Stored in ChatRunRegistry (queue per sessionId)
    |
    v
agentCommand() starts an agent run (produces AgentEventPayload events)
    |
    v
createAgentEventHandler() processes each event:
    |
    |-- stream="assistant" + data.text
    |       |
    |       v
    |   emitChatDelta(sessionKey, clientRunId, seq, text)
    |       |-- Buffer text in chatRunBuffers[clientRunId]
    |       |-- Throttle: skip if < 150ms since last delta
    |       |-- Broadcast "chat" event with state="delta"
    |       |-- Forward to node subscribers via nodeSendToSession
    |
    |-- stream="lifecycle" + phase="end"
    |       |
    |       v
    |   emitChatFinal(sessionKey, clientRunId, seq, "done")
    |       |-- Read final text from chatRunBuffers
    |       |-- Clean up buffers and deltaSentAt
    |       |-- Broadcast "chat" event with state="final"
    |
    |-- stream="lifecycle" + phase="error"
    |       |
    |       v
    |   emitChatFinal(sessionKey, clientRunId, seq, "error", error)
    |       |-- Broadcast "chat" event with state="error"
    |
    |-- stream="tool"
            |
            v
        broadcastToConnIds("agent", toolPayload, recipients)
            |-- Only sent to registered tool event recipients
            |-- Verbose level controls result inclusion
```

### Chat Abort Flow

```
chat.abort (from client)
    |
    v
abortChatRunById()
    |-- Record in chatAbortedRuns
    |-- Clean up buffers and delta timestamps
    |-- Remove from ChatRunRegistry
    |-- Broadcast "chat" event with state="final" (aborted)
```

### Heartbeat Suppression

Chat broadcasts for heartbeat-triggered runs can be suppressed based on the
`heartbeat.visibility.webchat.showOk` configuration. This prevents routine
heartbeat activity from cluttering the chat UI.

### Tool Event Recipients

The `ToolEventRecipientRegistry` tracks which WebSocket connections should receive
tool stream events for a given run. Entries have a 10-minute TTL and a 30-second
grace period after finalization.

---

## Node Registry and Invocation

`NodeRegistry` in `node-registry.ts` manages connected node sessions:

```typescript
type NodeSession = {
  nodeId: string;           // Device ID or client ID
  connId: string;           // WebSocket connection ID
  client: GatewayWsClient;  // WebSocket client reference
  displayName?: string;
  platform?: string;
  version?: string;
  coreVersion?: string;
  uiVersion?: string;
  deviceFamily?: string;
  modelIdentifier?: string;
  remoteIp?: string;
  caps: string[];            // Capabilities (e.g., "exec", "tts")
  commands: string[];        // Supported commands
  permissions?: Record<string, boolean>;
  pathEnv?: string;
  connectedAtMs: number;
};
```

### Node Invocation Protocol

```
Operator Client          Gateway            Node Client
    |                      |                     |
    |-- node.invoke ------>|                     |
    |   { nodeId,          |-- node.invoke.request -->|
    |     command,         |   { id, nodeId,     |
    |     params }         |     command,         |
    |                      |     paramsJSON,      |
    |                      |     timeoutMs }      |
    |                      |                     |
    |                      |<-- node.invoke.result --|
    |                      |   { id, nodeId,     |
    |                      |     ok, payload,    |
    |                      |     error }         |
    |<-- ResponseFrame ----|                     |
    |   { ok, payload }    |                     |
```

The invoke uses a promise-based timeout mechanism:
- Default timeout: 30 seconds
- Pending invokes are tracked by request ID
- On disconnect, all pending invokes for that node are rejected
- On timeout, the invoke resolves with `{ ok: false, code: "TIMEOUT" }`

---

## Node Subscriptions (Pub/Sub)

`NodeSubscriptionManager` in `server-node-subscriptions.ts` provides a bidirectional
mapping between nodes and session keys:

```typescript
type NodeSubscriptionManager = {
  subscribe(nodeId, sessionKey): void;     // Node subscribes to a session
  unsubscribe(nodeId, sessionKey): void;   // Node unsubscribes from a session
  unsubscribeAll(nodeId): void;            // Remove all subscriptions for a node
  sendToSession(sessionKey, event, payload, sendEvent?): void;
  sendToAllSubscribed(event, payload, sendEvent?): void;
  sendToAllConnected(event, payload, listConnected?, sendEvent?): void;
  clear(): void;
};
```

Internally maintains two maps:
- `nodeSubscriptions`: `Map<nodeId, Set<sessionKey>>` -- what sessions a node follows
- `sessionSubscribers`: `Map<sessionKey, Set<nodeId>>` -- what nodes follow a session

Used to forward chat events, agent events, and tick/health events to mobile nodes
that are subscribed to specific sessions.

---

## Node Event Processing

`handleNodeEvent()` in `server-node-events.ts` handles events reported by nodes via
the `node.event` RPC method:

| Event | Behavior |
|-------|----------|
| `voice.transcript` | Extracts text from payload, resolves session key, creates/updates session store entry, registers a chat run, starts an agent turn with the transcribed text. |
| `agent.request` | Parses a deep-link payload (`message`, `sessionKey`, `thinking`, `deliver`, `to`, `channel`, `timeoutSeconds`), resolves session, starts an agent turn. |
| `chat.subscribe` | Calls `nodeSubscribe(nodeId, sessionKey)` to register the node for events. |
| `chat.unsubscribe` | Calls `nodeUnsubscribe(nodeId, sessionKey)` to remove the subscription. |
| `exec.started` | Enqueues a system event with exec start details, triggers heartbeat. |
| `exec.finished` | Enqueues a system event with exit code/output, triggers heartbeat. |
| `exec.denied` | Enqueues a system event with denial reason, triggers heartbeat. |

---

## Cron Service Integration

`buildGatewayCronService()` in `server-cron.ts` creates a `CronService` with:

- **Store path**: Resolved from `cron.store` config
- **Agent resolution**: Dynamically resolves the target agent at run time (falls back to default agent)
- **Session key resolution**: Per-agent session store paths
- **System event integration**: Enqueues events tied to agent-specific sessions
- **Heartbeat integration**: Can trigger heartbeat runs
- **Isolated agent jobs**: Runs cron jobs via `runCronIsolatedAgentTurn()`
- **Event broadcasting**: All cron events broadcast to clients as `"cron"` events
- **Run logging**: Finished runs are appended to a per-job run log file

```typescript
type GatewayCronState = {
  cron: CronService;
  storePath: string;
  cronEnabled: boolean;
};
```

---

## Maintenance Timers

`startGatewayMaintenanceTimers()` in `server-maintenance.ts` starts three interval
timers:

### Tick Timer (30s -- `TICK_INTERVAL_MS`)

```
Every 30 seconds:
  broadcast("tick", { ts: Date.now() }, { dropIfSlow: true })
  nodeSendToAllSubscribed("tick", { ts })
```

### Health Refresh Timer (60s -- `HEALTH_REFRESH_INTERVAL_MS`)

```
Every 60 seconds:
  refreshGatewayHealthSnapshot({ probe: true })
    -> broadcasts "health" event with updated snapshot
```

Also primes the health cache immediately on startup so the first connecting client
gets a snapshot without delay.

### Dedupe Cleanup Timer (60s)

Runs three cleanup passes every 60 seconds:

1. **Dedupe map**: Remove entries older than `DEDUPE_TTL_MS` (5 min). If the map
   exceeds `DEDUPE_MAX` (1000), evict oldest entries.

2. **Chat abort controllers**: Expire controllers past `expiresAtMs` by calling
   `abortChatRunById()` with `stopReason: "timeout"`.

3. **Aborted runs map**: Remove entries older than 1 hour. Clean up associated
   buffers and delta timestamps.

---

## Sidecar Services

`startGatewaySidecars()` in `server-startup.ts` launches auxiliary services:

1. **Browser Control Server**: `startBrowserControlServerIfEnabled()` -- local
   Chromium/browser automation server (can be disabled via config).

2. **Gmail Watcher**: `startGmailWatcher()` -- watches a configured Gmail account
   for incoming messages (hooks.gmail.account). Validates the hooks model against
   the model catalog.

3. **Internal Hooks**: `loadInternalHooks()` -- loads hook handlers from config
   and directory discovery. Fires `gateway:startup` hook event after a 250ms delay.

4. **Channels**: `startChannels()` -- launches configured messaging channels
   (Telegram, Discord, etc.). Can be skipped via `OPENCLAW_SKIP_CHANNELS=1`.

5. **Plugin Services**: `startPluginServices()` -- starts background services
   registered by plugins.

6. **Memory Backend**: `startGatewayMemoryBackend()` -- initializes QMD memory
   persistence.

7. **Restart Sentinel**: Checks for and processes restart sentinel file for
   coordinated restarts.

---

## Authentication

`auth.ts` implements the full authentication stack:

### Auth Modes

```typescript
type ResolvedGatewayAuthMode = "none" | "token" | "password" | "trusted-proxy";

type ResolvedGatewayAuth = {
  mode: ResolvedGatewayAuthMode;
  token?: string;           // From config or OPENCLAW_GATEWAY_TOKEN env
  password?: string;        // From config or OPENCLAW_GATEWAY_PASSWORD env
  allowTailscale: boolean;  // Allow Tailscale identity as auth
  trustedProxy?: GatewayTrustedProxyConfig;
};
```

### Auth Resolution Order

`resolveGatewayAuth()` determines the mode:
1. Explicit `authConfig.mode` if set
2. Password present -> `"password"`
3. Token present -> `"token"`
4. Otherwise -> `"none"`

### Connection Authentication Flow (`authorizeGatewayConnect`)

```
1. If mode = "trusted-proxy":
   |-- Verify remote address is in trustedProxies list
   |-- Check required headers
   |-- Extract user from userHeader
   |-- Validate against allowUsers list
   |-- Return { ok: true, method: "trusted-proxy", user }

2. Rate limit check (if limiter configured):
   |-- Check if IP is currently blocked
   |-- If blocked: return { ok: false, rateLimited: true, retryAfterMs }

3. If allowTailscale and NOT local-direct:
   |-- Read Tailscale user headers
   |-- Verify loopback proxy request pattern
   |-- Perform Tailscale whois lookup on client IP
   |-- Compare whois login with header login
   |-- On success: reset rate limit, return { ok: true, method: "tailscale" }

4. If mode = "token":
   |-- Compare connectAuth.token with configured token (timing-safe)
   |-- On mismatch: record failure for rate limiting
   |-- On match: reset rate limit, return { ok: true, method: "token" }

5. If mode = "password":
   |-- Compare connectAuth.password with configured password (timing-safe)
   |-- On mismatch: record failure for rate limiting
   |-- On match: reset rate limit, return { ok: true, method: "password" }

6. Otherwise: record failure, return { ok: false, reason: "unauthorized" }
```

### Local Direct Requests

`isLocalDirectRequest()` returns true when:
- Client IP is loopback (127.0.0.1, ::1)
- Host header is localhost/127.0.0.1/::1 or *.ts.net
- No forwarded headers (unless remote is a trusted proxy)

Local direct requests bypass shared-secret authentication (auth mode="none"
effectively grants access on loopback).

### Device Authentication

Device identity authentication (for mobile clients) uses public-key cryptography:
- Device provides `{ id, publicKey, signature, signedAt, nonce }`
- Gateway verifies the signature against the registered public key
- This is handled separately from shared-secret auth in the connect handler

---

## Rate Limiting

`auth-rate-limit.ts` provides an in-memory sliding-window rate limiter:

```typescript
interface RateLimitConfig {
  maxAttempts?: number;     // Default: 10
  windowMs?: number;        // Default: 60,000 (1 min)
  lockoutMs?: number;       // Default: 300,000 (5 min)
  exemptLoopback?: boolean; // Default: true
}

interface AuthRateLimiter {
  check(ip, scope?): RateLimitCheckResult;
  recordFailure(ip, scope?): void;
  reset(ip, scope?): void;
  size(): number;
  prune(): void;
  dispose(): void;
}
```

### Design

- **Scoped counters**: Independent rate limits for `shared-secret` vs `device-token`
  auth, keyed as `${scope}:${ip}`
- **Loopback exempt**: localhost (127.0.0.1 / ::1) is never rate-limited
- **Sliding window**: Failed attempts within `windowMs` are counted; attempts
  outside the window are pruned
- **Lockout**: After `maxAttempts` failures, the IP is locked for `lockoutMs`
- **Periodic cleanup**: Stale entries pruned every 60 seconds
- **Timer cleanup**: `pruneTimer.unref()` ensures the timer does not prevent
  Node.js process exit

---

## Exec Approval Manager

`ExecApprovalManager` in `exec-approval-manager.ts` coordinates command execution
approvals between nodes and operator clients:

```typescript
type ExecApprovalRecord = {
  id: string;
  request: ExecApprovalRequestPayload;  // { command, cwd, host, security, ask, agentId, ... }
  createdAtMs: number;
  expiresAtMs: number;
  requestedByConnId?: string;
  requestedByDeviceId?: string;
  requestedByClientId?: string;
  resolvedAtMs?: number;
  decision?: ExecApprovalDecision;
  resolvedBy?: string;
};
```

### Flow

```
Node/Agent                    Gateway                    Operator UI
    |                           |                            |
    |-- exec.approval.request ->|                            |
    |                           |-- broadcast                |
    |                           |   exec.approval.requested  |
    |                           |                            |
    |                           |<- exec.approval.resolve ---|
    |                           |   { id, decision }         |
    |                           |                            |
    |<- exec.approval.         |-- broadcast                |
    |   waitDecision result    |   exec.approval.resolved   |
```

- `create()`: Creates a record with timeout
- `register()`: Returns a Promise that resolves when a decision is made (or timeout)
- `resolve()`: Resolves a pending approval with a decision
- `awaitDecision()`: Wait on an already-registered approval
- Grace period of 15 seconds keeps resolved entries for late readers

---

## Control UI Hosting

`control-ui.ts` serves the browser-based Control UI as a static SPA:

### Features

- **Static file serving**: Serves HTML, JS, CSS, images from the resolved UI root
- **SPA fallback**: Unknown paths serve `index.html` (client-side routing)
- **Config injection**: Injects `__OPENCLAW_CONTROL_UI_BASE_PATH__`,
  `__OPENCLAW_ASSISTANT_NAME__`, and `__OPENCLAW_ASSISTANT_AVATAR__` via script tag
- **Avatar handling**: Per-agent avatar resolution (local file, remote URL, data URL)
- **Security headers**: `X-Frame-Options: DENY`, `Content-Security-Policy: frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`
- **Base path support**: Configurable URL prefix (`gateway.controlUi.basePath`)
- **Root override**: Custom asset directory (`gateway.controlUi.root`)
- **Auto-build**: Attempts to build UI assets if not found (`pnpm ui:build`)

### Root State Resolution

```typescript
type ControlUiRootState =
  | { kind: "resolved"; path: string }   // Assets found
  | { kind: "invalid"; path: string }    // Override path doesn't exist
  | { kind: "missing" };                 // No assets found
```

---

## OpenAI-Compatible HTTP Endpoints

### Chat Completions (`openai-http.ts`)

- **Endpoint**: `POST /v1/chat/completions`
- **Auth**: Bearer token (same as gateway token/password)
- **Format**: OpenAI Chat Completions API compatible
- **Streaming**: SSE (`data: {...}\n\n`) with `stream: true`
- **Session resolution**: Uses `user` field or query params for session key
- **Agent resolution**: Supports `model` field mapping to agent IDs

### OpenResponses (`openresponses-http.ts`)

- **Endpoint**: `POST /v1/responses`
- **Auth**: Bearer token
- **Format**: OpenResponses protocol (open-responses.com)
- **Streaming**: SSE with structured events
- **Features**: Supports input items (text, images, files, PDFs), tool definitions,
  agent selection, session management
- **Media limits**: Configurable file/image/PDF size limits
- **Schema**: Defined in `open-responses.schema.ts` using TypeBox

---

## Protocol Schema Layer

The `protocol/` directory defines all gateway types using TypeBox schemas, compiled
to AJV validators in `protocol/index.ts`:

### Schema Files (17 files)

| File | Types Defined |
|------|--------------|
| `primitives.ts` | `NonEmptyString`, `GatewayClientIdSchema`, `GatewayClientModeSchema` |
| `frames.ts` | `ConnectParams`, `HelloOk`, `RequestFrame`, `ResponseFrame`, `EventFrame`, `ErrorShape`, `GatewayFrame` |
| `snapshot.ts` | `PresenceEntry`, `Snapshot`, `StateVersion`, `SessionDefaults`, `HealthSnapshot` |
| `types.ts` | Common shared types |
| `error-codes.ts` | `ErrorCodes` enum, `errorShape()` factory |
| `agent.ts` | `AgentEvent`, `AgentSummary`, `AgentIdentityParams/Result` |
| `agents-models-skills.ts` | `AgentsList`, `AgentsCreate/Update/Delete`, `AgentsFiles*`, `ModelsList`, `SkillsStatus/Install/Update` |
| `channels.ts` | `ChannelsStatus`, `ChannelsLogout`, `TalkConfig`, `TalkMode` |
| `config.ts` | `ConfigGet/Set/Apply/Patch/Schema` params and results |
| `cron.ts` | `CronJob`, `CronAdd/Update/Remove/Run/Runs` |
| `devices.ts` | `DevicePairList/Approve/Reject`, `DeviceTokenRotate/Revoke` |
| `exec-approvals.ts` | `ExecApprovalRequest/WaitDecision/Resolve`, `ExecApprovalsGet/Set` |
| `logs-chat.ts` | `LogsTail`, `ChatSend/Abort/History`, `ChatEvent` |
| `nodes.ts` | `NodeList/Describe/Invoke/InvokeResult/Event`, `NodePair*` |
| `sessions.ts` | `SessionsList/Preview/Patch/Reset/Delete/Compact` |
| `wizard.ts` | `WizardStart/Next/Cancel/Status` |
| `protocol-schemas.ts` | Protocol-level validation schemas |

### Validation

`protocol/index.ts` compiles all schemas using AJV (Ajv) with strict mode. The
compiled validators are used throughout the gateway to validate incoming requests
and outgoing responses.

---

## Shutdown and Cleanup

`createGatewayCloseHandler()` in `server-close.ts` returns an async function that
performs orderly shutdown:

```
close({ reason?, restartExpectedMs? })
    |
    1. Stop Bonjour/mDNS discovery
    2. Stop Tailscale exposure
    3. Close canvas host handler
    4. Close canvas host server
    5. Stop all messaging channels
    6. Stop plugin services
    7. Stop Gmail watcher
    8. Stop cron service
    9. Stop heartbeat runner
   10. Clear node presence timers
   11. Broadcast "shutdown" event to all clients
   12. Clear maintenance timers (tick, health, dedupe)
   13. Unsubscribe from agent events
   14. Unsubscribe from heartbeat events
   15. Clear chat run state
   16. Close all WebSocket connections (code 1012 "service restart")
   17. Clear clients set
   18. Stop config reloader
   19. Stop browser control server
   20. Close WebSocket server
   21. Close all HTTP servers (with closeIdleConnections)
```

---

## Key Data Structures

### GatewayWsClient

```typescript
// server/ws-types.ts
type GatewayWsClient = {
  socket: WebSocket;         // Raw WebSocket connection
  connect: ConnectParams;    // Handshake parameters from the client
  connId: string;            // Unique connection ID (UUID)
  presenceKey?: string;      // Key used in the presence system
  clientIp?: string;         // Resolved client IP address
};
```

### ConnectParams (from client)

```typescript
// protocol/schema/frames.ts
type ConnectParams = {
  minProtocol: number;
  maxProtocol: number;
  client: {
    id: string;              // Client identifier
    displayName?: string;
    version: string;
    platform: string;        // "macos", "linux", "ios", "android", "web"
    deviceFamily?: string;   // "iPhone", "iPad", etc.
    modelIdentifier?: string;
    mode: string;            // "cli", "control-ui", "node", "mobile"
    instanceId?: string;
  };
  caps?: string[];           // Client capabilities
  commands?: string[];       // Supported commands (for nodes)
  permissions?: Record<string, boolean>;
  pathEnv?: string;
  role?: string;             // "operator" | "node"
  scopes?: string[];         // Authorization scopes
  device?: {                 // Device identity (for mobile auth)
    id: string;
    publicKey: string;
    signature: string;
    signedAt: number;
    nonce?: string;
  };
  auth?: {                   // Shared secret auth
    token?: string;
    password?: string;
  };
  locale?: string;
  userAgent?: string;
};
```

### Snapshot (sent to client on connect)

```typescript
// protocol/schema/snapshot.ts
type Snapshot = {
  presence: PresenceEntry[];
  health: HealthSummary;          // Any (dynamic shape)
  stateVersion: {
    presence: number;             // Monotonic counter
    health: number;               // Monotonic counter
  };
  uptimeMs: number;
  configPath?: string;
  stateDir?: string;
  sessionDefaults?: {
    defaultAgentId: string;
    mainKey: string;
    mainSessionKey: string;
    scope?: string;
  };
  authMode?: "none" | "token" | "password" | "trusted-proxy";
};
```

### PresenceEntry

```typescript
type PresenceEntry = {
  host?: string;
  ip?: string;
  version?: string;
  platform?: string;
  deviceFamily?: string;
  modelIdentifier?: string;
  mode?: string;
  lastInputSeconds?: number;
  reason?: string;
  tags?: string[];
  text?: string;
  ts: number;
  deviceId?: string;
  roles?: string[];
  scopes?: string[];
  instanceId?: string;
};
```

### HelloOk (server handshake response)

```typescript
type HelloOk = {
  type: "hello-ok";
  protocol: number;
  server: {
    version: string;
    commit?: string;
    host?: string;
    connId: string;
  };
  features: {
    methods: string[];     // All available RPC methods
    events: string[];      // All available event types
  };
  snapshot: Snapshot;
  canvasHostUrl?: string;
  auth?: {                 // Returned when device auth is used
    deviceToken: string;
    role: string;
    scopes: string[];
    issuedAtMs?: number;
  };
  policy: {
    maxPayload: number;       // MAX_PAYLOAD_BYTES (8 MiB)
    maxBufferedBytes: number; // MAX_BUFFERED_BYTES (16 MiB)
    tickIntervalMs: number;   // TICK_INTERVAL_MS (30s)
  };
};
```

### ChatRunEntry

```typescript
type ChatRunEntry = {
  sessionKey: string;     // Session this run belongs to
  clientRunId: string;    // Client-assigned run ID
};
```

### ChatRunState

```typescript
type ChatRunState = {
  registry: ChatRunRegistry;           // Queue of runs per sessionId
  buffers: Map<string, string>;        // Accumulated assistant text per run
  deltaSentAt: Map<string, number>;    // Last delta broadcast timestamp per run
  abortedRuns: Map<string, number>;    // Aborted run IDs -> abort timestamp
  clear: () => void;
};
```

### NodeSession

```typescript
type NodeSession = {
  nodeId: string;
  connId: string;
  client: GatewayWsClient;
  displayName?: string;
  platform?: string;
  version?: string;
  coreVersion?: string;
  uiVersion?: string;
  deviceFamily?: string;
  modelIdentifier?: string;
  remoteIp?: string;
  caps: string[];
  commands: string[];
  permissions?: Record<string, boolean>;
  pathEnv?: string;
  connectedAtMs: number;
};
```

### NodeInvokeResult

```typescript
type NodeInvokeResult = {
  ok: boolean;
  payload?: unknown;
  payloadJSON?: string | null;
  error?: { code?: string; message?: string } | null;
};
```

### ExecApprovalRequestPayload

```typescript
type ExecApprovalRequestPayload = {
  command: string;
  cwd?: string | null;
  host?: string | null;
  security?: string | null;
  ask?: string | null;
  agentId?: string | null;
  resolvedPath?: string | null;
  sessionKey?: string | null;
};
```

---

## Constants and Tuning

Defined in `server-constants.ts`:

```typescript
MAX_PAYLOAD_BYTES           = 8 * 1024 * 1024;    // 8 MiB - incoming WebSocket frame cap
MAX_BUFFERED_BYTES          = 16 * 1024 * 1024;   // 16 MiB - per-connection send buffer limit
MAX_CHAT_HISTORY_BYTES      = 6 * 1024 * 1024;    // 6 MiB - chat history response cap
DEFAULT_HANDSHAKE_TIMEOUT_MS = 10_000;             // 10s handshake deadline
TICK_INTERVAL_MS            = 30_000;              // 30s keepalive tick
HEALTH_REFRESH_INTERVAL_MS  = 60_000;              // 60s health refresh
DEDUPE_TTL_MS               = 5 * 60_000;          // 5 min dedupe entry TTL
DEDUPE_MAX                  = 1000;                // Max dedupe entries
```

Rate limiter defaults (in `auth-rate-limit.ts`):

```typescript
DEFAULT_MAX_ATTEMPTS = 10;       // Max failed attempts before lockout
DEFAULT_WINDOW_MS    = 60_000;   // 1 minute sliding window
DEFAULT_LOCKOUT_MS   = 300_000;  // 5 minute lockout
PRUNE_INTERVAL_MS    = 60_000;   // Prune stale entries every minute
```

Tool event recipient defaults (in `server-chat.ts`):

```typescript
TOOL_EVENT_RECIPIENT_TTL_MS        = 10 * 60 * 1000;  // 10 min TTL
TOOL_EVENT_RECIPIENT_FINAL_GRACE_MS = 30 * 1000;       // 30s after finalization
```

---

## Config Hot-Reload

`startGatewayConfigReloader()` in `config-reload.ts` watches the config file for
changes and triggers a reload cycle:

- Uses file system watcher for change detection
- On change, re-reads and validates the config
- `createGatewayReloadHandlers()` in `server-reload-handlers.ts` handles the reload
  by updating subsystems:
  - Heartbeat runner config
  - Cron service config
  - Lane concurrency settings
  - Hooks config
  - Channel restarts if needed
  - Plugin service restarts if needed
- Graceful handling of config changes during active chat runs
- The reload is debounced to avoid rapid-fire reloads from file system events
