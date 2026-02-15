# Web Control UI

## Overview

The OpenClaw Web Control UI is a single-page application that serves as the gateway dashboard for managing and interacting with the OpenClaw system. It provides a chat interface for direct agent interaction, real-time monitoring of connected instances, channel management, configuration editing, cron job scheduling, usage analytics, and more.

### Technology Stack

| Layer | Technology | Details |
|---|---|---|
| **UI Framework** | [Lit](https://lit.dev/) 3.x | Web components with reactive properties via `@state()` decorators |
| **Rendering** | Lit `html` tagged templates | Declarative HTML rendering with automatic DOM updates |
| **State Management** | Lit reactive properties | Centralized state in the `OpenClawApp` element; no external state library |
| **Build Tool** | [Vite](https://vitejs.dev/) 7.x | ES module bundler with HMR during development |
| **Markdown** | [marked](https://marked.js.org/) 17.x | GFM-enabled Markdown parsing for chat messages |
| **Sanitization** | [DOMPurify](https://github.com/cure53/DOMPurify) 3.x | HTML sanitization of all rendered Markdown |
| **Cryptography** | [@noble/ed25519](https://github.com/paulmillr/noble-ed25519) 3.x | Ed25519 key generation and signing for device identity |
| **Testing** | [Vitest](https://vitest.dev/) 4.x | Unit tests (Node) and browser tests (Playwright) |
| **Language** | TypeScript | Strict-typed throughout, no runtime type guards library |

### How It Is Served

The Web Control UI is embedded directly in the gateway server. During a production build, Vite compiles the UI to static assets in `dist/control-ui/`. The gateway HTTP server (`/home/alex/git/openclaw/src/gateway/control-ui.ts`) serves these assets at the configured base path. For SPA routing, any unknown path falls back to `index.html`. The gateway injects runtime configuration (base path, assistant name, assistant avatar) into the HTML via a `<script>` tag before the `</head>` close.

**Source files:**
- `/home/alex/git/openclaw/ui/index.html` -- HTML entry point
- `/home/alex/git/openclaw/ui/src/main.ts` -- JavaScript entry point (imports styles and registers the app)
- `/home/alex/git/openclaw/src/gateway/control-ui.ts` -- Gateway-side static file serving

---

## Architecture

### High-Level Component Architecture

```
index.html
  |
  +-- main.ts  (imports styles.css, imports app.ts)
        |
        +-- OpenClawApp (custom element: <openclaw-app>)
              |
              +-- app-render.ts        (top-level layout: shell, topbar, nav, content)
              +-- app-lifecycle.ts      (connectedCallback, disconnectedCallback, updated)
              +-- app-gateway.ts        (WebSocket connection, event dispatch)
              +-- app-settings.ts       (settings persistence, tab routing, theme)
              +-- app-chat.ts           (chat send/abort/queue logic)
              +-- app-polling.ts        (interval-based polling for nodes, logs, debug)
              +-- app-scroll.ts         (auto-scroll for chat thread and logs)
              +-- app-tool-stream.ts    (real-time tool execution overlay)
              +-- app-channels.ts       (WhatsApp/Nostr channel handlers)
              +-- app-render.helpers.ts (nav tab rendering, theme toggle, chat controls)
              +-- app-render-usage-tab.ts (usage tab delegation)
              |
              +-- controllers/          (gateway RPC wrappers)
              |     +-- chat.ts, config.ts, cron.ts, agents.ts, ...
              |
              +-- views/                (per-tab render functions)
              |     +-- chat.ts, overview.ts, cron.ts, config.ts, ...
              |
              +-- chat/                 (chat rendering subsystem)
              |     +-- grouped-render.ts, message-normalizer.ts, tool-cards.ts, ...
              |
              +-- components/           (reusable custom elements)
                    +-- resizable-divider.ts
```

### Design Principles

1. **Single Root Component** -- The entire UI is a single `OpenClawApp` custom element. There is no component tree of nested custom elements (except for `<resizable-divider>`). Instead, views are plain render functions that return Lit `TemplateResult` values.

2. **Functional Decomposition** -- The large `app.ts` component delegates all behavior to external modules. Each `app-*.ts` file owns one concern (gateway, chat, settings, scroll, polling, etc.) and operates on a typed "host" interface rather than requiring the full `OpenClawApp` class.

3. **Props-Down, Events-Up** -- Each view receives a typed props object with data and callback functions. Views never access the app state directly; they call the provided callbacks.

4. **No Shadow DOM** -- The `OpenClawApp` overrides `createRenderRoot()` to return `this`, meaning all rendering happens in the light DOM. Global CSS stylesheets in `styles/` apply directly. Only the `ResizableDivider` component uses Shadow DOM (via `LitElement` default).

5. **Controller Pattern** -- Each data domain (chat, config, cron, sessions, etc.) has a corresponding controller in `controllers/` that wraps gateway RPC calls and mutates state on the host.

### File Organization

```
ui/
  index.html              -- HTML shell (loads <openclaw-app>)
  package.json            -- Dependencies (lit, marked, dompurify, vite, noble-ed25519)
  vite.config.ts          -- Build configuration
  vitest.config.ts        -- Browser test config
  vitest.node.config.ts   -- Node test config
  public/                 -- Static assets (favicons)
  src/
    main.ts               -- Entry point
    styles.css             -- CSS import aggregator
    styles/                -- All CSS files
      base.css             -- CSS custom properties, reset, typography
      layout.css           -- Shell grid, topbar, nav, content
      layout.mobile.css    -- Responsive breakpoints
      components.css       -- Cards, buttons, forms, pills, chips, etc.
      config.css           -- Config editor specific styles
      chat.css             -- Chat CSS import aggregator
      chat/
        layout.css         -- Chat grid layout
        text.css           -- Chat bubble text styles
        grouped.css        -- Message grouping styles
        tool-cards.css     -- Tool call/result card styles
        sidebar.css        -- Markdown sidebar styles
    ui/
      app.ts               -- Main OpenClawApp component
      app-view-state.ts    -- Full AppViewState type definition
      app-render.ts        -- Top-level render function
      app-render.helpers.ts-- Navigation tabs, theme toggle, chat controls
      app-render-usage-tab.ts -- Usage tab render delegation
      app-lifecycle.ts     -- Lifecycle hooks (connected/disconnected/updated)
      app-gateway.ts       -- Gateway WebSocket client setup and event handling
      app-settings.ts      -- Settings persistence, tab navigation, theme
      app-chat.ts          -- Chat message sending, aborting, queuing
      app-channels.ts      -- WhatsApp and Nostr channel handlers
      app-polling.ts       -- Interval polling (nodes, logs, debug)
      app-scroll.ts        -- Chat and logs auto-scroll management
      app-tool-stream.ts   -- Real-time tool execution stream
      app-events.ts        -- EventLogEntry type
      app-defaults.ts      -- Default form values
      gateway.ts           -- GatewayBrowserClient class
      storage.ts           -- localStorage settings persistence
      navigation.ts        -- Tab definitions, URL routing
      theme.ts             -- Theme resolution (system/light/dark)
      theme-transition.ts  -- Animated theme transitions via View Transitions API
      presenter.ts         -- Data formatting helpers
      format.ts            -- Number/time formatting utilities
      markdown.ts          -- Marked + DOMPurify markdown rendering
      icons.ts             -- Lucide-style SVG icon library
      text-direction.ts    -- RTL/LTR text direction detection
      tool-display.ts      -- Tool display name/icon resolution
      tool-display.json    -- Tool display configuration data
      uuid.ts              -- UUID v4 generation
      types.ts             -- All gateway data types
      ui-types.ts          -- UI-only types (ChatAttachment, CronFormState)
      assistant-identity.ts-- Assistant name/avatar resolution
      device-auth.ts       -- Device auth token localStorage management
      device-identity.ts   -- Ed25519 device identity generation
      types/
        chat-types.ts      -- Chat-specific types (ChatItem, MessageGroup, etc.)
      controllers/
        chat.ts            -- Chat history loading, message sending, event handling
        config.ts          -- Config loading, saving, applying, schema fetching
        cron.ts            -- Cron job CRUD operations
        agents.ts          -- Agent list loading
        agent-files.ts     -- Agent workspace file management
        agent-identity.ts  -- Agent identity (name/avatar) loading
        agent-skills.ts    -- Per-agent skill loading
        assistant-identity.ts -- Assistant identity loading from gateway
        channels.ts        -- Channel status loading
        channels.types.ts  -- Channel type definitions
        debug.ts           -- Debug/status data loading and manual RPC
        devices.ts         -- Device pairing management
        exec-approval.ts   -- Exec approval request/response
        exec-approvals.ts  -- Exec approval file management
        logs.ts            -- Log file loading
        nodes.ts           -- Node list loading
        presence.ts        -- Presence entry loading
        sessions.ts        -- Sessions list loading, patching, deletion
        skills.ts          -- Skills status, toggle, install, API key management
        usage.ts           -- Usage data loading
      chat/
        grouped-render.ts  -- Message group rendering
        message-normalizer.ts -- Message role/content normalization
        message-extract.ts -- Text/thinking content extraction
        tool-cards.ts      -- Tool call/result card extraction and rendering
        tool-helpers.ts    -- Tool output formatting and preview truncation
        copy-as-markdown.ts-- Copy-to-clipboard button for assistant messages
        constants.ts       -- Chat rendering constants
      views/
        chat.ts            -- Chat view (thread, compose, queue, sidebar)
        overview.ts        -- Overview/dashboard view
        cron.ts            -- Cron job management view
        config.ts          -- Configuration editor view
        config-form.ts     -- Schema-driven form renderer
        config-form.render.ts  -- Config form field rendering
        config-form.shared.ts  -- Config form utilities
        config-form.analyze.ts -- Config schema analysis
        config-form.node.ts    -- Config form node renderer
        channels.ts        -- Channels overview view
        channels.shared.ts -- Channel shared utilities
        channels.types.ts  -- Channel view types
        channels.config.ts -- Channel config section
        channels.discord.ts    -- Discord channel card
        channels.googlechat.ts -- Google Chat channel card
        channels.imessage.ts   -- iMessage channel card
        channels.nostr.ts      -- Nostr channel card
        channels.nostr-profile-form.ts -- Nostr profile editor
        channels.signal.ts     -- Signal channel card
        channels.slack.ts      -- Slack channel card
        channels.telegram.ts   -- Telegram channel card
        channels.whatsapp.ts   -- WhatsApp channel card
        agents.ts          -- Agents management view
        agents-panels-status-files.ts -- Agent files, channels, cron panels
        agents-panels-tools-skills.ts -- Agent tools and skills panels
        agents-utils.ts    -- Agent helper utilities
        instances.ts       -- Connected instances view
        sessions.ts        -- Sessions list view
        usage.ts           -- Usage analytics view
        usage-metrics.ts   -- Usage metric calculations and chart rendering
        usage-query.ts     -- Usage search/filter/export utilities
        usage-render-overview.ts  -- Usage overview rendering
        usage-render-details.ts   -- Usage detail panel rendering
        usageTypes.ts      -- Usage type definitions
        usageStyles.ts     -- Usage CSS-in-JS styles
        usage-styles/      -- Split usage style modules
        nodes.ts           -- Nodes and device management view
        nodes-exec-approvals.ts -- Exec approvals panel in nodes
        skills.ts          -- Skills management view
        debug.ts           -- Debug/diagnostics view
        logs.ts            -- Live log viewer
        exec-approval.ts   -- Exec approval dialog overlay
        gateway-url-confirmation.ts -- Gateway URL change confirmation dialog
        markdown-sidebar.ts-- Markdown content sidebar panel
      components/
        resizable-divider.ts -- Draggable split-view divider
```

---

## Main Application Component (`app.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app.ts`

The `OpenClawApp` class extends `LitElement` and is registered as `<openclaw-app>`. It is the single root of the entire application. The class body is intentionally thin -- it declares all reactive state properties and delegates all behavior to external modules via method wrappers.

### Reactive State Properties

The component declares approximately 120+ `@state()` decorated properties organized by domain:

| Domain | Example Properties | Purpose |
|---|---|---|
| **Connection** | `connected`, `hello`, `lastError`, `client` | Gateway WebSocket state |
| **Settings** | `settings`, `password`, `tab`, `theme`, `themeResolved` | User preferences and navigation |
| **Chat** | `chatMessages`, `chatStream`, `chatRunId`, `chatQueue`, `chatAttachments`, `chatSending` | Chat conversation state |
| **Sidebar** | `sidebarOpen`, `sidebarContent`, `splitRatio` | Tool output sidebar |
| **Config** | `configRaw`, `configSchema`, `configForm`, `configFormDirty` | Configuration editor |
| **Channels** | `channelsSnapshot`, `whatsappLoginQrDataUrl`, `nostrProfileFormState` | Channel management |
| **Sessions** | `sessionsResult`, `sessionsFilterActive`, `sessionsFilterLimit` | Session management |
| **Usage** | `usageResult`, `usageCostSummary`, `usageSelectedSessions`, `usageChartMode` | Analytics dashboard |
| **Cron** | `cronJobs`, `cronStatus`, `cronForm` | Cron job management |
| **Agents** | `agentsList`, `agentsPanel`, `agentFilesList`, `agentSkillsReport` | Agent management |
| **Skills** | `skillsReport`, `skillEdits`, `skillMessages` | Skill management |
| **Nodes** | `nodes`, `devicesList`, `execApprovalsSnapshot`, `execApprovalQueue` | Node/device management |
| **Debug** | `debugStatus`, `debugHealth`, `debugCallMethod`, `debugCallResult` | Debug diagnostics |
| **Logs** | `logsEntries`, `logsFilterText`, `logsLevelFilters`, `logsAutoFollow` | Log viewer |

### Lifecycle (lines 347-367)

```typescript
createRenderRoot() { return this; }  // Light DOM rendering -- no Shadow DOM

connectedCallback()    -> handleConnected(this)    // app-lifecycle.ts
firstUpdated()         -> handleFirstUpdated(this)  // app-lifecycle.ts
disconnectedCallback() -> handleDisconnected(this)  // app-lifecycle.ts
updated(changed)       -> handleUpdated(this, changed) // app-lifecycle.ts
```

The `createRenderRoot()` override at line 347 is critical: it returns `this` instead of a shadow root, which means the app renders directly into the light DOM so that global CSS applies.

### Render Delegation (line 568)

```typescript
render() {
  return renderApp(this as unknown as AppViewState);
}
```

All rendering is delegated to `app-render.ts::renderApp()`.

### Method Wrappers (lines 369-571)

The class provides thin wrapper methods that forward to the corresponding external module function. For example:

```typescript
connect()          -> connectGatewayInternal(this)     // app-gateway.ts
handleSendChat()   -> handleSendChatInternal(this)     // app-chat.ts
applySettings()    -> applySettingsInternal(this)       // app-settings.ts
setTab()           -> setTabInternal(this)              // app-settings.ts
setTheme()         -> setThemeInternal(this)            // app-settings.ts
handleAbortChat()  -> handleAbortChatInternal(this)     // app-chat.ts
loadOverview()     -> loadOverviewInternal(this)        // app-settings.ts
loadCron()         -> loadCronInternal(this)            // app-settings.ts
```

### Onboarding Mode (lines 92-103)

If the URL contains `?onboarding=1`, the UI enters onboarding mode which hides the topbar, forces focus mode for chat, and hides the thinking toggle.

---

## AppViewState Type

**File:** `/home/alex/git/openclaw/ui/src/ui/app-view-state.ts`

The `AppViewState` type is a comprehensive interface (285 lines) that represents the full state and capabilities of the application. It includes all reactive properties from `OpenClawApp` plus all method references (callbacks). This type serves as the "contract" between the main component and the render/view functions. Views receive subsets of this state via their own props types.

---

## Gateway Client (`gateway.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/gateway.ts`

The `GatewayBrowserClient` class manages the WebSocket connection to the OpenClaw gateway server.

### Connection Lifecycle

1. **`start()`** (line 77) -- Initiates connection. Sets `closed = false` and calls `connect()`.
2. **`connect()`** (line 93) -- Creates a new `WebSocket` to `opts.url`. Registers handlers for `open`, `message`, `close`, and `error`.
3. **On Open** -- Calls `queueConnect()` which starts a 750ms timer before sending the connect handshake (allows the server to send a `connect.challenge` event with a nonce first).
4. **`sendConnect()`** (line 128) -- Builds the connect request with:
   - Protocol version negotiation (`minProtocol: 3, maxProtocol: 3`)
   - Client info (name `"openclaw-control-ui"`, mode `"webchat"`, platform, version)
   - Authentication (token, password, or device identity)
   - Device identity with Ed25519 signature (only in secure contexts)
   - Role `"operator"` with scopes `["operator.admin", "operator.approvals", "operator.pairing"]`
5. **On Hello-OK** -- Stores any device token from the server, resets backoff, calls `opts.onHello()`.
6. **On Close** -- Flushes pending requests with errors, calls `opts.onClose()`, schedules reconnect.

### Reconnection Strategy (line 112)

Uses exponential backoff starting at 800ms, multiplied by 1.7 on each attempt, capped at 15,000ms:

```typescript
scheduleReconnect() {
  const delay = this.backoffMs;
  this.backoffMs = Math.min(this.backoffMs * 1.7, 15_000);
  window.setTimeout(() => this.connect(), delay);
}
```

Backoff resets to 800ms after a successful hello.

### RPC Request/Response (line 289)

```typescript
request<T>(method: string, params?: unknown): Promise<T>
```

Sends a JSON frame `{ type: "req", id: uuid, method, params }` and returns a promise that resolves when the matching `{ type: "res", id }` response arrives. Pending requests are tracked in a `Map<string, Pending>` and rejected on disconnect.

### Event Handling (line 238)

Incoming frames are dispatched by `type`:
- **`"event"`** -- Forwarded to `opts.onEvent()`. Sequence numbers are tracked for gap detection.
- **`"res"`** -- Matched to pending request by `id` and resolved/rejected.
- **`"event"` with `"connect.challenge"`** -- Captures the nonce and triggers `sendConnect()`.

### Challenge-Response Authentication

The gateway may send a `connect.challenge` event containing a nonce before the client sends its connect request. The client captures this nonce and includes it in the signed device auth payload, proving the signature is fresh.

### Device Identity

**File:** `/home/alex/git/openclaw/ui/src/ui/device-identity.ts`

On first load (in secure HTTPS contexts), the UI generates an Ed25519 keypair using `@noble/ed25519`, derives a device ID from the SHA-256 hash of the public key, and stores it in `localStorage` under key `"openclaw-device-identity-v1"`. On subsequent loads, the stored identity is reused.

During the connect handshake, the client signs a payload containing the device ID, client info, role, scopes, token, and the server's challenge nonce. The signature proves device ownership.

### Device Auth Token

**File:** `/home/alex/git/openclaw/ui/src/ui/device-auth.ts`

After a successful hello, the server may issue a `deviceToken` in the hello-ok response. This token is stored in `localStorage` under `"openclaw.device.auth.v1"` keyed by `(deviceId, role)`. On subsequent connections, the device token is preferred over the shared gateway token. If the device token fails, the client falls back to the shared token and clears the stale device token.

---

## Gateway Event Handling (`app-gateway.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-gateway.ts`

### `connectGateway()` (line 118)

Creates a new `GatewayBrowserClient` and registers four callbacks:

- **`onHello`** -- Sets `connected = true`, applies the snapshot (presence, health, session defaults), resets orphaned chat state, loads agents, nodes, devices, and refreshes the active tab.
- **`onClose`** -- Sets `connected = false`, sets `lastError` (unless code 1012 = service restart).
- **`onEvent`** -- Dispatches to `handleGatewayEvent()`.
- **`onGap`** -- Sets a warning error about sequence gaps.

### `handleGatewayEvent()` (line 180)

Routes gateway events by name:

| Event | Handler |
|---|---|
| `"agent"` | `handleAgentEvent()` -- updates tool stream for real-time tool execution display |
| `"chat"` | `handleChatEvent()` -- processes delta/final/aborted/error states for streaming chat |
| `"presence"` | Updates `presenceEntries` from payload |
| `"cron"` | Reloads cron data if on cron tab |
| `"device.pair.requested"` / `"device.pair.resolved"` | Reloads device list |
| `"exec.approval.requested"` | Adds entry to `execApprovalQueue`; auto-removes on expiry |
| `"exec.approval.resolved"` | Removes entry from `execApprovalQueue` |

All events are also appended to `eventLogBuffer` (capped at 250 entries) for the debug view.

### `applySnapshot()` (line 275)

Applied when the hello-ok arrives. Extracts initial presence, health, and session defaults from the snapshot. Session defaults include the resolved main session key, which may differ from the hardcoded `"main"` (e.g., `"agent:assistant:main"`).

---

## Lifecycle Management (`app-lifecycle.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-lifecycle.ts`

### `handleConnected()` (line 37)

Called from `connectedCallback()`. Performs first-time setup:
1. Infers the base path from `window.__OPENCLAW_CONTROL_UI_BASE_PATH__` or the URL pathname.
2. Applies settings from URL query parameters (token, password, session, gatewayUrl).
3. Syncs the active tab with the current URL path.
4. Resolves and applies the theme.
5. Attaches the theme media query listener for system theme changes.
6. Registers the `popstate` handler for browser back/forward.
7. Connects to the gateway WebSocket.
8. Starts nodes polling (5s interval).
9. Starts logs or debug polling if those tabs are active.

### `handleUpdated()` (line 68)

Called after every reactive property change. Handles two auto-behaviors:
1. **Chat auto-scroll** -- When `chatMessages`, `chatToolMessages`, `chatStream`, `chatLoading`, or `tab` changes while on the chat tab, schedules a scroll-to-bottom (unless the user is scrolled up).
2. **Logs auto-follow** -- When `logsEntries`, `logsAutoFollow`, or `tab` changes while on the logs tab with auto-follow enabled, scrolls to the bottom.

### `handleDisconnected()` (line 58)

Cleanup: removes popstate handler, stops all polling intervals, detaches theme listener, disconnects the topbar resize observer.

---

## Settings & Storage

### Storage (`storage.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/storage.ts`

Settings are persisted to `localStorage` under the key `"openclaw.control.settings.v1"`.

```typescript
type UiSettings = {
  gatewayUrl: string;        // WebSocket URL (default: derived from page protocol/host)
  token: string;             // Gateway auth token
  sessionKey: string;        // Default session key (default: "main")
  lastActiveSessionKey: string; // Last used session key
  theme: ThemeMode;          // "system" | "light" | "dark" (default: "system")
  chatFocusMode: boolean;    // Hide sidebar/header in chat (default: false)
  chatShowThinking: boolean; // Show tool calls/results (default: true)
  splitRatio: number;        // Sidebar split ratio, 0.4-0.7 (default: 0.6)
  navCollapsed: boolean;     // Sidebar collapsed state (default: false)
  navGroupsCollapsed: Record<string, boolean>; // Per-group collapsed state
};
```

`loadSettings()` reads from localStorage with defensive type checking and defaults. `saveSettings()` serializes to JSON and writes to localStorage.

### Settings Management (`app-settings.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-settings.ts`

**`applySettings()`** (line 59) -- Saves to localStorage, syncs theme if changed, updates `applySessionKey`.

**`applySettingsFromUrl()`** (line 84) -- Reads `token`, `password`, `session`, and `gatewayUrl` from URL query parameters or hash. Sensitive values (token, password) are removed from the URL via `replaceState`. The `gatewayUrl` parameter triggers a confirmation dialog rather than auto-applying.

**`setTab()`** (line 149) -- Changes the active tab, manages logs/debug polling lifecycle, refreshes the tab's data, and syncs the URL.

**`refreshActiveTab()`** (line 184) -- Loads the appropriate data for whichever tab is active. Each tab has specific data requirements:

| Tab | Data Loaded |
|---|---|
| overview | channels, presence, sessions, cron status, debug |
| channels | channels (with probe), config schema, config |
| instances | presence |
| sessions | sessions |
| cron | channels, cron status, cron jobs |
| skills | skills |
| agents | agents, config, identities, agent-specific data |
| nodes | nodes, devices, config, exec approvals |
| chat | chat history, sessions, avatar |
| config | config schema, config |
| debug | debug data |
| logs | log entries |

---

## Navigation & Routing (`navigation.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/navigation.ts`

### Tab Definitions

The UI has 12 tabs organized into 4 groups:

```typescript
const TAB_GROUPS = [
  { label: "Chat",     tabs: ["chat"] },
  { label: "Control",  tabs: ["overview", "channels", "instances", "sessions", "usage", "cron"] },
  { label: "Agent",    tabs: ["agents", "skills", "nodes"] },
  { label: "Settings", tabs: ["config", "debug", "logs"] },
];
```

### URL Routing

Each tab maps to a URL path (e.g., `"chat"` -> `/chat`, `"overview"` -> `/overview`). The root path `/` maps to `"chat"`. The chat tab additionally syncs the `?session=` query parameter.

Key functions:
- **`pathForTab(tab, basePath)`** -- Returns the URL path for a tab, respecting the base path.
- **`tabFromPath(pathname, basePath)`** -- Resolves a tab from a URL pathname.
- **`inferBasePathFromPathname(pathname)`** -- Detects the base path prefix from the URL (for deployments behind a reverse proxy at a sub-path).

Browser history is managed via `pushState`/`replaceState` without page reloads. The `popstate` event handler syncs the tab when the user navigates with back/forward buttons.

### Tab Metadata

Each tab has an associated:
- **Icon** (`iconForTab`) -- Lucide-style SVG icon name
- **Title** (`titleForTab`) -- Human-readable title
- **Subtitle** (`subtitleForTab`) -- Descriptive subtitle shown in the page header

---

## Theme System

### Theme Resolution (`theme.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/theme.ts`

Three modes: `"system"`, `"light"`, `"dark"`. System mode resolves via `window.matchMedia("(prefers-color-scheme: dark)")`.

The resolved theme is applied by:
1. Setting `data-theme` on `<html>` (used by CSS selectors)
2. Setting `color-scheme` CSS property on `<html>`

### Theme Transitions (`theme-transition.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/theme-transition.ts`

When the user changes themes, the UI uses the View Transitions API (`document.startViewTransition()`) for a smooth circular reveal animation originating from the click position. The transition origin is set via CSS custom properties `--theme-switch-x` and `--theme-switch-y`.

If the browser doesn't support View Transitions, or the user has `prefers-reduced-motion: reduce`, the theme switches instantly.

### CSS Custom Properties

**File:** `/home/alex/git/openclaw/ui/src/styles/base.css`

The dark theme (default) defines ~90 CSS custom properties under `:root`:

- **Backgrounds:** `--bg`, `--bg-accent`, `--bg-elevated`, `--bg-hover`, `--card`, `--panel`
- **Text:** `--text`, `--text-strong`, `--chat-text`, `--muted`
- **Borders:** `--border`, `--border-strong`, `--border-hover`
- **Accent:** `--accent` (#ff5c5c red), `--accent-hover`, `--accent-2` (teal)
- **Semantic:** `--ok` (green), `--warn` (amber), `--danger` (red), `--info` (blue)
- **Typography:** `--font-body` (Space Grotesk), `--mono` (JetBrains Mono)
- **Shadows:** `--shadow-sm`, `--shadow-md`, `--shadow-lg`, `--shadow-xl`
- **Radii:** `--radius-sm` (6px), `--radius-md` (8px), `--radius-lg` (12px)

The light theme is defined under `[data-theme="light"]` and overrides all color-related properties.

---

## Layout & Shell

**File:** `/home/alex/git/openclaw/ui/src/styles/layout.css`

The shell uses CSS Grid with the following structure:

```
grid-template-columns: var(--shell-nav-width) minmax(0, 1fr);
grid-template-rows: var(--shell-topbar-height) 1fr;
grid-template-areas:
  "topbar topbar"
  "nav content";
```

- **`--shell-nav-width`**: 220px (0px when collapsed or in focus mode)
- **`--shell-topbar-height`**: 56px (0 in onboarding mode)
- Height: `100dvh` (with `100vh` fallback)

### Shell Modes

| CSS Class | Effect |
|---|---|
| `.shell--chat` | Full viewport height, no overflow |
| `.shell--nav-collapsed` | Navigation width collapses to 0px |
| `.shell--chat-focus` | Navigation collapses, content header hidden |
| `.shell--onboarding` | Topbar hidden, no padding |

### Topbar

The topbar contains:
- **Left:** Hamburger menu toggle (nav collapse), brand logo + "OPENCLAW / Gateway Dashboard" text
- **Right:** Health status pill with connection indicator, theme toggle (system/light/dark buttons)

### Navigation Sidebar

The sidebar renders tab groups with collapsible sections. Each group has a label button that toggles the collapsed state (persisted in settings). Each tab is rendered as an `<a>` element with client-side navigation (click handler calls `setTab()` and `preventDefault()`). A "Resources" section at the bottom links to external docs.

---

## Top-Level Rendering (`app-render.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-render.ts`

The `renderApp()` function (line 89) produces the complete page layout:

1. **Shell wrapper** with CSS class modifiers for chat/focus/collapsed/onboarding modes
2. **Topbar** with brand, status pill, theme toggle
3. **Navigation sidebar** with grouped tabs
4. **Main content area** with:
   - Page header (title + subtitle, chat controls when on chat tab)
   - Active view (conditionally rendered based on `state.tab`)
5. **Exec approval dialog** (overlay, always rendered but hidden when no approvals pending)
6. **Gateway URL confirmation dialog** (overlay for URL change from query params)

Each tab's view is rendered only when `state.tab` matches, using Lit's `nothing` for inactive tabs. This means inactive tabs produce no DOM nodes, keeping the page lightweight.

### View Wiring

Each view receives a typed props object constructed inline. Callbacks in the props close over `state` to access controllers and mutate state. For example, the chat view:

```typescript
renderChat({
  sessionKey: state.sessionKey,
  messages: state.chatMessages,
  stream: state.chatStream,
  onSend: () => state.handleSendChat(),
  onAbort: () => void state.handleAbortChat(),
  onDraftChange: (next) => (state.chatMessage = next),
  // ... many more props
})
```

---

## Views

### Chat View (`views/chat.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/chat.ts`

The primary view for direct agent interaction. It consists of:

**Chat Thread** -- A scrollable `<div class="chat-thread">` with `role="log"` and `aria-live="polite"`. Messages are rendered using `lit/directives/repeat.js` for efficient keyed updates.

**Message Processing Pipeline:**
1. `buildChatItems()` takes raw messages and tool messages, limits to 200 most recent history entries, injects compaction dividers, optionally hides tool results, appends streaming/reading indicators
2. `groupMessages()` groups consecutive messages by normalized role into `MessageGroup` objects
3. The repeat directive renders each group via `renderMessageGroup()`, `renderStreamingGroup()`, or `renderReadingIndicatorGroup()`

**Split View Sidebar** -- When a user clicks a tool card, its output opens in a resizable sidebar. The `<resizable-divider>` component handles drag-to-resize. The sidebar renders Markdown content via `renderMarkdownSidebar()`.

**Compose Area** -- A `<textarea>` with:
- Auto-height adjustment on input
- Enter to send (Shift+Enter for newlines)
- IME composition awareness (`e.isComposing || e.keyCode === 229`)
- Image paste handling (clipboard images become `ChatAttachment` objects with data URLs)
- Attachment preview strip with remove buttons
- RTL/LTR direction detection via `detectTextDirection()`

**Message Queue** -- When a message is sent while the agent is processing, it enters a queue displayed below the thread. Queued messages can be individually removed.

**Compaction Indicator** -- Shows "Compacting context..." during context compaction and "Context compacted" for 5 seconds after completion.

**New Messages Indicator** -- A floating "New messages" button appears when new messages arrive while the user is scrolled up.

**Controls** -- Session selector dropdown, refresh button, thinking toggle (brain icon), focus mode toggle (crosshair icon).

### Chat System (`chat/`)

#### Message Normalization (`chat/message-normalizer.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/chat/message-normalizer.ts`

`normalizeMessage()` converts raw message objects (from various gateway/API formats) into a consistent `NormalizedMessage` with `role`, `content` (array of `MessageContentItem`), `timestamp`, and `id`. It detects tool messages by looking for `toolCallId`, `tool_call_id`, `toolName`, or content items with `toolresult`/`tool_result` types.

`normalizeRoleForGrouping()` maps role strings to canonical values: `"user"`, `"assistant"`, `"system"`, or `"tool"` (for toolResult/tool_result/tool/function roles).

#### Grouped Rendering (`chat/grouped-render.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/chat/grouped-render.ts`

Messages are grouped by role. Each group renders:
1. **Avatar** -- "U" circle for user, initial/image for assistant, gear icon for tool
2. **Message bubbles** -- Each message in the group as a chat bubble with:
   - Images extracted from content blocks (base64 or URL)
   - Thinking/reasoning blocks (wrapped in `<div class="chat-thinking">`)
   - Markdown text rendered via `toSanitizedMarkdownHtml()`
   - Tool cards for tool call/result content items
   - Copy-as-markdown button for assistant messages
3. **Footer** -- Sender name ("You" / assistant name / role) and timestamp

**Reading Indicator** -- Three animated dots (`.chat-reading-indicator__dots`) shown when the stream is empty (agent is "thinking").

**Streaming Group** -- Renders the current partial response as a regular message bubble with the `streaming` CSS class.

#### Tool Cards (`chat/tool-cards.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/chat/tool-cards.ts`

`extractToolCards()` parses message content for tool call and tool result items. Each card has:
- **Kind:** `"call"` or `"result"`
- **Name:** Tool name
- **Args:** Tool arguments (parsed from JSON if string)
- **Text:** Tool output text (for results)

`renderToolCardSidebar()` renders each card as a clickable card with:
- Icon (resolved from `tool-display.json` configuration)
- Label (human-readable tool name)
- Detail line (file path, command, etc. extracted from args)
- Preview text (first 2 lines / 100 chars for long output)
- Click handler that opens the full output in the sidebar

Tool display configuration is loaded from `/home/alex/git/openclaw/ui/src/ui/tool-display.json` and resolved via `/home/alex/git/openclaw/ui/src/ui/tool-display.ts`. Special handling exists for common tools like `read` (shows file path with line range) and `write`/`edit` (shows file path).

#### Copy as Markdown (`chat/copy-as-markdown.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/chat/copy-as-markdown.ts`

Renders a copy button on assistant message bubbles. Uses `navigator.clipboard.writeText()` with visual feedback: shows a checkmark for 1.5s on success, error message for 2s on failure.

### Overview View (`views/overview.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/overview.ts`

The dashboard view with two sections:

**Gateway Access Card** -- Form fields for:
- WebSocket URL (defaults to derived from page host)
- Gateway Token (hidden when using trusted-proxy auth)
- Password (not stored, type="password")
- Default Session Key
- Connect and Refresh buttons
- Auth failure hints with links to documentation

**Snapshot Card** -- Displays:
- Connection status (Connected/Disconnected with color)
- Uptime (human-readable duration)
- Tick interval
- Last channels refresh time
- Error messages with contextual auth hints

**Statistics Row** -- Three stat cards showing:
- Instances count (presence beacons)
- Sessions count
- Cron status (Enabled/Disabled) with next wake time

**Notes Section** -- Quick reminders about Tailscale serve, session hygiene, and cron reminders.

### Channels View (`views/channels.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/channels.ts`

Displays cards for each supported channel:
- **WhatsApp** -- Status, QR code login flow, start/stop/logout actions
- **Telegram** -- Status, bot info, webhook probe
- **Discord** -- Status, bot info, probe
- **Google Chat** -- Status, credentials, webhook
- **Slack** -- Status, bot/team info, probe
- **Signal** -- Status, base URL, probe
- **iMessage** -- Status, CLI/DB paths, probe
- **Nostr** -- Status, public key, profile editor

Each channel has its own rendering module (e.g., `channels.discord.ts`, `channels.telegram.ts`). Channels are sorted with enabled/active channels first. The view supports inline configuration editing for channel-specific settings.

### Sessions View (`views/sessions.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/sessions.ts`

Lists all tracked sessions with:
- Filters: active minutes, limit, include global, include unknown
- Per-session details: key, kind, label, surface, timestamps, token counts
- Per-session controls: thinking level select, verbose level select, reasoning level select
- Delete session button
- Provider-specific thinking level options (e.g., binary for Z.AI)

### Cron View (`views/cron.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/cron.ts`

**Scheduler Status** -- Enabled/disabled, job count, next wake time.

**New Job Form** -- Fields for:
- Name, description, agent ID
- Schedule kind: `at` (one-time), `every` (interval), `cron` (expression)
- Session target: `main` or `isolated`
- Wake mode: `next-heartbeat` or `now`
- Payload kind: `systemEvent` or `agentTurn`
- Delivery: `none` or `announce` with channel/recipient selection
- Timeout seconds

**Jobs List** -- Each job displays schedule, payload summary, state (next/last run, status). Actions: toggle enable/disable, run now, remove. Expandable run history per job.

### Config View (`views/config.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/config.ts`

Supports two modes:
- **Form Mode** -- Schema-driven form with sections, search, validation (rendered by `config-form.ts`)
- **Raw Mode** -- Direct JSON editor textarea

Actions: Reload, Save (writes to disk), Apply (hot-reload config), Update (runs `openclaw update`).

The form mode uses the gateway's config schema to render appropriate input types (text, number, boolean, select for union literals). Fields are organized into sections with sidebar navigation.

### Agents View (`views/agents.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/agents.ts`

A tabbed interface for managing agents:
- **Agent Selector** -- Dropdown to switch between agents
- **Overview Panel** -- Agent identity (name, emoji, avatar), model configuration with primary/fallback selection
- **Files Panel** -- Agent workspace files with inline editor, save/reset
- **Tools Panel** -- Tool profile selection, allow/deny lists
- **Skills Panel** -- Per-agent skill enable/disable toggles
- **Channels Panel** -- Channel status for the selected agent
- **Cron Panel** -- Cron jobs filtered to the selected agent

### Nodes View (`views/nodes.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/nodes.ts`

**Exec Approvals** -- Manage per-command approval rules for gateway and nodes, with agent-specific overrides.

**Node Bindings** -- Bind exec nodes to agents and configure default exec targets.

**Device Pairing** -- Lists pending pairing requests (approve/reject) and paired devices (rotate/revoke tokens).

**Nodes List** -- Connected nodes with their capabilities and link status.

### Skills View (`views/skills.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/skills.ts`

Lists all skills grouped by source (workspace, built-in, installed, extra). Each skill shows:
- Name, description, emoji
- Eligibility status (requirements met/missing)
- Enable/disable toggle
- API key input for skills that require one
- Install buttons for skills that need dependencies
- Missing requirements (bins, env vars, config, OS)

Filter input for searching skills by name/description/source.

### Usage View (`views/usage.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/usage.ts`

A comprehensive analytics dashboard with:
- Date range selector (start/end date)
- Cost breakdown (by model, by provider, by agent, by channel)
- Daily chart (tokens or cost, total or by-type)
- Session list with sorting (tokens, cost, recent, messages, errors) and multi-column display
- Per-session details: message counts, tool usage, latency stats, model breakdown, time series
- Session log viewer with role/tool filters
- CSV export for daily and session data
- Query/filter system with suggestions and chips
- Mosaic visualization
- Context weight analysis (system prompt, skills, tools, workspace files)

### Debug View (`views/debug.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/debug.ts`

**Snapshots** -- Status, health, and heartbeat data displayed as formatted JSON. Includes security audit summary.

**Manual RPC** -- Input fields for method name and JSON params, with call button and result/error display.

**Models** -- Catalog from `models.list` displayed as formatted JSON.

**Event Log** -- Scrollable list of recent gateway events with timestamps and formatted payloads.

### Logs View (`views/logs.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/logs.ts`

A live log tail viewer with:
- Text filter input
- Level filter checkboxes (trace, debug, info, warn, error, fatal)
- Auto-follow toggle
- Export button (downloads filtered entries as `.log` file)
- Truncation notice when log output is truncated
- Scrollable log stream with columns: time, level (color-coded), subsystem, message

Logs are polled every 2 seconds when the tab is active.

### Instances View (`views/instances.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/instances.ts`

Lists connected instances/clients with:
- Host name and IP
- Mode, roles, scopes, platform, device family, model identifier, version (as chips)
- Presence age (relative timestamp)
- Last input time
- Reason

### Exec Approval Dialog (`views/exec-approval.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/exec-approval.ts`

A modal overlay that appears when the gateway requests approval for a shell command execution. Shows:
- Command text
- Metadata: host, agent, session, CWD, resolved path, security level, ask reason
- Expiry countdown
- Queue count (when multiple approvals pending)
- Three actions: "Allow once", "Always allow", "Deny"

### Gateway URL Confirmation (`views/gateway-url-confirmation.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/gateway-url-confirmation.ts`

A security dialog that appears when the URL contains a `?gatewayUrl=` parameter. Shows the proposed URL with a warning about trusting external URLs. Actions: Confirm (applies and reconnects) or Cancel.

### Markdown Sidebar (`views/markdown-sidebar.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/views/markdown-sidebar.ts`

A panel for displaying tool output. Renders content as sanitized Markdown with a close button. Includes a "View Raw Text" fallback button if rendering fails.

---

## Chat System Deep Dive

### Message Sending Flow (`app-chat.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-chat.ts`

1. **`handleSendChat()`** (line 159) -- Entry point. Validates message text or attachments exist. Checks for stop commands (`/stop`, `stop`, `esc`, `abort`, `wait`, `exit`). Clears draft and attachments.
2. If the chat is busy (sending or running), the message is **enqueued** via `enqueueChatMessage()`.
3. Otherwise, calls `sendChatMessageNow()`:
   a. Resets tool stream
   b. Calls `sendChatMessage()` controller which sends `"chat.send"` RPC
   c. Optimistically appends user message to `chatMessages`
   d. Sets `chatRunId`, `chatStream = ""`, `chatStreamStartedAt`
   e. On success, schedules chat scroll
   f. On failure, restores draft and adds error message

### Streaming Response

The gateway sends `chat` events with states:
- **`"delta"`** -- Updates `chatStream` with the partial response text
- **`"final"`** -- Clears stream, run ID; triggers history reload and queue flush
- **`"aborted"`** -- Clears stream and run ID
- **`"error"`** -- Clears stream and run ID; sets `lastError`

### Message Queue

When messages are sent while busy, they enter `chatQueue`. After a run completes (final/error/aborted), `flushChatQueue()` sends the next queued message. The queue is displayed in the UI with per-item remove buttons.

### Reset Commands

`/new` and `/reset` are treated as reset commands that trigger a session refresh after the run completes, updating the sessions list.

### Tool Stream (`app-tool-stream.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-tool-stream.ts`

Real-time tool execution events arrive via the `"agent"` gateway event with `stream: "tool"`. Each tool call progresses through phases:
- **`"start"`** -- Creates a new `ToolStreamEntry` with name and args
- **`"update"`** -- Updates with partial result
- **`"result"`** -- Updates with final result

Tool stream entries are maintained in a `Map` (keyed by `toolCallId`) with an ordered list. They are synced to `chatToolMessages` via a throttled timer (80ms). The stream is capped at 50 entries.

Compaction events (`stream: "compaction"`) trigger a compaction indicator toast.

### Chat Avatars

`refreshChatAvatar()` fetches the avatar URL for the current session's agent by calling the gateway's `/avatar/{agentId}?meta=1` endpoint. The avatar is displayed in the chat thread next to assistant messages.

---

## Controllers

Controllers in `controllers/` are stateless functions that accept a host/state object and make gateway RPC calls.

### Chat Controller (`controllers/chat.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/controllers/chat.ts`

- **`loadChatHistory(state)`** -- Calls `chat.history` with session key and limit 200
- **`sendChatMessage(state, message, attachments?)`** -- Calls `chat.send` with session key, message, idempotency key, and optional base64 image attachments
- **`abortChatRun(state)`** -- Calls `chat.abort` with session key and run ID
- **`handleChatEvent(state, payload)`** -- Processes incoming chat events (delta/final/aborted/error)

Image attachments are converted from data URLs to `{ type: "image", mimeType, content: base64 }` format before sending.

### Config Controller (`controllers/config.ts`)

- **`loadConfig(state)`** -- Calls `config.get`; populates raw, form, and schema state
- **`loadConfigSchema(state)`** -- Calls `config.schema`; populates JSON schema and UI hints
- **`saveConfig(state)`** -- Calls `config.save` with the current form/raw content
- **`applyConfig(state)`** -- Calls `config.apply` to hot-reload the configuration
- **`runUpdate(state)`** -- Calls `system.update` to run `openclaw update`
- **`updateConfigFormValue(state, path, value)`** -- Immutably patches the config form at a nested path
- **`removeConfigFormValue(state, path)`** -- Removes a value from the config form at a nested path

### Cron Controller (`controllers/cron.ts`)

- **`loadCronJobs(state)`** -- Calls `cron.list`
- **`loadCronStatus(state)`** -- Calls `cron.status`
- **`addCronJob(state)`** -- Calls `cron.add` with form state converted to a cron job spec
- **`toggleCronJob(state, job, enabled)`** -- Calls `cron.update` to toggle enabled
- **`runCronJob(state, job)`** -- Calls `cron.run` to trigger immediate execution
- **`removeCronJob(state, job)`** -- Calls `cron.remove`
- **`loadCronRuns(state, jobId)`** -- Calls `cron.runs` for run history

### Sessions Controller (`controllers/sessions.ts`)

- **`loadSessions(state, opts?)`** -- Calls `sessions.list` with optional active minutes filter
- **`patchSession(state, key, patch)`** -- Calls `sessions.patch` to update thinking/verbose/reasoning levels
- **`deleteSession(state, key)`** -- Calls `sessions.delete`

### Other Controllers

| Controller | Methods | RPC Methods Called |
|---|---|---|
| `agents.ts` | `loadAgents` | `agents.list` |
| `agent-files.ts` | `loadAgentFiles`, `loadAgentFileContent`, `saveAgentFile` | `agents.files.list`, `agents.files.get`, `agents.files.set` |
| `agent-identity.ts` | `loadAgentIdentity`, `loadAgentIdentities` | `agents.identity` |
| `agent-skills.ts` | `loadAgentSkills` | `agents.skills` |
| `assistant-identity.ts` | `loadAssistantIdentity` | `assistant.identity` |
| `channels.ts` | `loadChannels` | `channels.status` |
| `debug.ts` | `loadDebug`, `callDebugMethod` | `system.status`, `system.health`, `models.list`, `system.heartbeat`, (arbitrary) |
| `devices.ts` | `loadDevices`, `approveDevicePairing`, `rejectDevicePairing`, `rotateDeviceToken`, `revokeDeviceToken` | `devices.list`, `devices.pair.approve`, `devices.pair.reject`, `devices.token.rotate`, `devices.token.revoke` |
| `exec-approval.ts` | `parseExecApprovalRequested`, `addExecApproval`, `removeExecApproval` | (event parsing only) |
| `exec-approvals.ts` | `loadExecApprovals`, `saveExecApprovals`, `updateExecApprovalsFormValue`, `removeExecApprovalsFormValue` | `exec-approvals.get`, `exec-approvals.set` |
| `logs.ts` | `loadLogs` | `logs.tail` |
| `nodes.ts` | `loadNodes` | `nodes.list` |
| `presence.ts` | `loadPresence` | `system-presence` |
| `skills.ts` | `loadSkills`, `updateSkillEnabled`, `saveSkillApiKey`, `updateSkillEdit`, `installSkill` | `skills.status`, `skills.toggle`, `skills.apikey`, `skills.install` |
| `usage.ts` | `loadUsage` | `sessions.usage` |

---

## Components

### Resizable Divider (`components/resizable-divider.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/components/resizable-divider.ts`

A custom element `<resizable-divider>` that enables drag-to-resize split views. This is the only component that uses Shadow DOM (via LitElement default).

**Properties:**
- `splitRatio` (Number, default 0.6) -- Current split position
- `minRatio` (Number, default 0.4) -- Minimum ratio
- `maxRatio` (Number, default 0.7) -- Maximum ratio

**Behavior:**
- On mousedown: records start position and ratio, adds global mousemove/mouseup listeners
- On mousemove: calculates delta ratio from container width, clamps to min/max, dispatches `resize` custom event
- On mouseup: cleans up listeners

**Styling:** 4px wide bar with accent color on hover/drag, invisible 8px padding for easier grab targeting via `::before` pseudo-element.

---

## Presenters (`presenter.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/presenter.ts`

Pure data formatting functions used by views:

- **`formatPresenceSummary(entry)`** -- Combines host, IP, mode, version into a summary string
- **`formatPresenceAge(entry)`** -- Relative timestamp from the entry's `ts` field
- **`formatNextRun(ms)`** -- Absolute timestamp + relative timestamp for cron next run
- **`formatSessionTokens(row)`** -- Token count in `"total / context"` format
- **`formatEventPayload(payload)`** -- JSON.stringify with 2-space indent for debug display
- **`formatCronState(job)`** -- Status, next run, last run summary
- **`formatCronSchedule(job)`** -- Human-readable schedule description (at/every/cron)
- **`formatCronPayload(job)`** -- Payload summary with delivery info

### Format Utilities (`format.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/format.ts`

- **`formatMs(ms)`** -- Locale-formatted date from epoch milliseconds
- **`formatList(values)`** -- Comma-joined list or "none"
- **`clampText(value, max)`** -- Truncates with ellipsis
- **`truncateText(value, max)`** -- Returns `{ text, truncated, total }` for limit display
- **`toNumber(value, fallback)`** -- Safe string-to-number conversion
- **`parseList(input)`** -- Split by comma/newline and trim
- **`stripThinkingTags(value)`** -- Remove reasoning XML tags from text

Also re-exports `formatRelativeTimestamp` and `formatDurationHuman` from shared infra modules.

---

## Markdown Rendering (`markdown.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/markdown.ts`

`toSanitizedMarkdownHtml(markdown)` is the primary rendering function:

1. Truncates input to 140,000 characters (shows truncation notice)
2. For input > 40,000 characters: renders as `<pre>` block (skips markdown parsing for performance)
3. For shorter input: parses with `marked` (GFM mode, breaks enabled)
4. Sanitizes with DOMPurify (allowlist of safe HTML tags and attributes)
5. Caches results in an LRU Map (200 entries, max 50,000 chars per key)

**Security features:**
- Custom `marked.Renderer` that escapes raw HTML in markdown (prevents rendering pasted HTML error pages)
- DOMPurify hook that forces `rel="noreferrer noopener"` and `target="_blank"` on all links
- Strict tag allowlist (no `<script>`, `<style>`, `<iframe>`, etc.)
- Data URI allowed only for `<img>` tags

---

## Icons (`icons.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/icons.ts`

A collection of Lucide-style SVG icons rendered as Lit `html` template results. Icons use `currentColor` for stroke, making them inherit the surrounding text color. The `icons` object provides named access (e.g., `icons.messageSquare`, `icons.settings`, `icons.check`, `icons.x`, `icons.brain`, `icons.copy`, etc.).

---

## Text Direction Detection (`text-direction.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/text-direction.ts`

`detectTextDirection(text)` returns `"rtl"` or `"ltr"` by examining the first significant character using Unicode Script Properties. Supports Hebrew, Arabic, Syriac, Thaana, Nko, Samaritan, Mandaic, Adlam, Phoenician, and Lydian scripts. Whitespace and punctuation are skipped.

This is applied to chat message bubbles and the compose textarea via the `dir` HTML attribute.

---

## Polling (`app-polling.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-polling.ts`

Three independent polling intervals:

| Poll | Interval | Condition | Target |
|---|---|---|---|
| Nodes | 5,000ms | Always (while connected) | `loadNodes({ quiet: true })` |
| Logs | 2,000ms | Only when logs tab active | `loadLogs({ quiet: true })` |
| Debug | 3,000ms | Only when debug tab active | `loadDebug()` |

Each poll has start/stop functions that manage `window.setInterval` handles stored on the host.

---

## Scroll Management (`app-scroll.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/app-scroll.ts`

### Chat Scroll

**`scheduleChatScroll(host, force, smooth)`** -- Waits for Lit render completion (`updateComplete`), then uses `requestAnimationFrame` to scroll the chat thread to the bottom.

Scroll behavior:
- **Near-bottom threshold:** 450px. If the user is within this distance from the bottom, new messages auto-scroll.
- **Force:** Only effective on first auto-scroll (initial page load). After that, user scroll position is respected.
- **Smooth:** Only if `prefers-reduced-motion` is not set to reduce.
- **Retry:** A secondary scroll occurs 120-150ms later to account for late layout shifts.

**`handleChatScroll(host, event)`** -- Tracks whether the user is near the bottom. Clears the "new messages" indicator when they scroll back down.

**`resetChatScroll(host)`** -- Resets auto-scroll state (used on session change).

### Logs Scroll

**`scheduleLogsScroll(host, force)`** -- Scrolls the `.log-stream` container to the bottom. Only scrolls if within 80px of bottom or forced.

### Log Export

**`exportLogs(lines, label)`** -- Creates a Blob from log lines and triggers a download as `openclaw-logs-{label}-{timestamp}.log`.

### Topbar Observer

**`observeTopbar(host)`** -- Uses `ResizeObserver` on the `.topbar` element to set `--topbar-height` CSS variable dynamically.

---

## Build System

### Vite Configuration (`vite.config.ts`)

**File:** `/home/alex/git/openclaw/ui/vite.config.ts`

```typescript
export default defineConfig(() => ({
  base: process.env.OPENCLAW_CONTROL_UI_BASE_PATH || "./",
  publicDir: "public",
  optimizeDeps: { include: ["lit/directives/repeat.js"] },
  build: {
    outDir: "../dist/control-ui",
    emptyOutDir: true,
    sourcemap: true,
  },
  server: {
    host: true,
    port: 5173,
    strictPort: true,
  },
}));
```

Key points:
- **Base path** is configurable via `OPENCLAW_CONTROL_UI_BASE_PATH` env var (defaults to `"./"`).
- **Output** goes to `dist/control-ui/` at the project root.
- **Source maps** are generated for production debugging.
- **Dev server** runs on port 5173.
- `lit/directives/repeat.js` is explicitly pre-bundled to avoid import issues.

### Package Scripts

```json
{
  "build": "vite build",
  "dev": "vite",
  "preview": "vite preview",
  "test": "vitest run --config vitest.config.ts"
}
```

### Testing

Two test configurations:
- **`vitest.config.ts`** -- Browser tests using `@vitest/browser-playwright`
- **`vitest.node.config.ts`** -- Node unit tests

Browser tests include:
- `config-form.browser.test.ts` -- Config form rendering tests with screenshots
- `navigation.browser.test.ts` -- Routing and auto-scroll tests
- `focus-mode.browser.test.ts` -- Focus mode behavior
- `chat-markdown.browser.test.ts` -- Markdown rendering in chat

Node tests include:
- `navigation.test.ts` -- URL routing logic
- `format.test.ts` -- Formatting utilities
- `text-direction.test.ts` -- RTL detection
- `uuid.test.ts` -- UUID generation
- `app-scroll.test.ts` -- Scroll management
- `app-settings.test.ts` -- Settings loading/saving
- `chat/message-normalizer.test.ts` -- Message normalization
- `chat/message-extract.test.ts` -- Text extraction
- `chat/tool-helpers.test.ts` -- Tool helpers
- `controllers/chat.test.ts` -- Chat controller
- `controllers/config.test.ts` -- Config controller
- `views/chat.test.ts` -- Chat view
- `views/cron.test.ts` -- Cron view
- `views/sessions.test.ts` -- Sessions view

---

## Hosting & Serving (`src/gateway/control-ui.ts`)

**File:** `/home/alex/git/openclaw/src/gateway/control-ui.ts`

The gateway serves the Control UI as static files over HTTP.

### `handleControlUiHttpRequest(req, res, opts)`

The main request handler (line 239):

1. **Method check** -- Only GET and HEAD are allowed (405 for others).
2. **Base path matching** -- If a base path is configured, requests must start with `{basePath}/`. A bare `{basePath}` redirects to `{basePath}/`.
3. **Root validation** -- If the UI assets directory is missing or invalid, returns 503 with a helpful error message suggesting `pnpm ui:build`.
4. **Path resolution** -- Strips the base path, resolves relative path, validates against directory traversal attacks.
5. **Static file serving** -- If the file exists, serves it with the correct Content-Type and `no-cache` Cache-Control.
6. **SPA fallback** -- For unknown paths, serves `index.html` (client-side routing).

### Index HTML Injection

When serving `index.html`, the gateway injects a `<script>` tag before `</head>` containing:

```javascript
window.__OPENCLAW_CONTROL_UI_BASE_PATH__ = "/base/path";
window.__OPENCLAW_ASSISTANT_NAME__ = "AgentName";
window.__OPENCLAW_ASSISTANT_AVATAR__ = "emoji_or_url";
```

The assistant identity (name and avatar) is resolved from the gateway configuration for the default agent.

### Avatar Endpoint

`handleControlUiAvatarRequest()` handles requests to `/avatar/{agentId}`:
- With `?meta=1` query: returns JSON `{ avatarUrl: string | null }` -- the URL to the avatar image
- Without meta: serves the actual avatar image file (for local file avatars)

### Security Headers

All Control UI responses include:
- `X-Frame-Options: DENY` -- Prevents embedding in iframes
- `Content-Security-Policy: frame-ancestors 'none'` -- CSP frame protection
- `X-Content-Type-Options: nosniff` -- Prevents MIME type sniffing

### Content Type Resolution

Maps file extensions to MIME types: `.html`, `.js`, `.css`, `.json`, `.map`, `.svg`, `.png`, `.jpg`, `.gif`, `.webp`, `.ico`, `.txt`, with `application/octet-stream` as default.

---

## Styling Architecture

### CSS Organization

The CSS is organized into layered imports:

```
styles.css
  |-- base.css       -- Custom properties, reset, typography, animations
  |-- layout.css     -- Shell grid, topbar, nav, content areas
  |-- layout.mobile.css -- Responsive breakpoints and mobile overrides
  |-- components.css -- Cards, buttons, forms, lists, pills, chips, modals
  |-- config.css     -- Config editor specific styles
```

Chat styles are further split:

```
chat.css
  |-- chat/layout.css     -- Chat grid and split view
  |-- chat/text.css       -- Message text and bubble styles
  |-- chat/grouped.css    -- Message group layout and avatars
  |-- chat/tool-cards.css -- Tool call/result card styles
  |-- chat/sidebar.css    -- Markdown sidebar panel
```

### Design System

**Typography:**
- Body: Space Grotesk (Google Fonts)
- Monospace: JetBrains Mono (Google Fonts)

**Color Palette (Dark Theme):**
- Background: `#12141a` (deep dark blue-gray)
- Cards: `#181b22`
- Accent: `#ff5c5c` (signature red)
- Secondary accent: `#14b8a6` (teal)
- Success: `#22c55e`
- Warning: `#f59e0b`
- Danger: `#ef4444`
- Text: `#e4e4e7`
- Muted: `#71717a`

**Animation:**
- Dashboard enter: 400ms ease-out fade + slide
- Focus mode transition: 200ms ease-out
- Theme transition: View Transitions API circular reveal
- Reading indicator: Pulsing dots animation

**Responsive:**
- `layout.mobile.css` provides breakpoints for smaller screens
- Grid layouts adapt from multi-column to single-column
- Navigation sidebar becomes a hamburger toggle

---

## Assistant Identity

**File:** `/home/alex/git/openclaw/ui/src/ui/assistant-identity.ts`

The UI supports customizing the assistant's display name and avatar:

1. **Injected identity** -- Resolved at startup from `window.__OPENCLAW_ASSISTANT_NAME__` and `window.__OPENCLAW_ASSISTANT_AVATAR__` (set by the gateway during index.html serving).
2. **Dynamic identity** -- Loaded from the gateway via `assistant.identity` RPC call after connection.
3. **Per-agent identity** -- Resolved from the agents list based on the current session key's agent ID.

The `AssistantIdentity` type includes `name` (string), `avatar` (string or null -- can be an emoji, URL, or data URL), and optional `agentId`.

Avatar resolution priority:
1. Agent identity's `avatarUrl` (if it looks like a URL)
2. Avatar meta endpoint (`/avatar/{agentId}?meta=1`)
3. Agent identity's `avatar` field
4. Injected identity from page load
5. Default: letter "A"

---

## UUID Generation (`uuid.ts`)

**File:** `/home/alex/git/openclaw/ui/src/ui/uuid.ts`

Generates UUIDv4 strings using the best available source:
1. `crypto.randomUUID()` (preferred, available in secure contexts)
2. `crypto.getRandomValues()` (fallback, manual UUID construction)
3. `Math.random()` (last resort with XOR time mixing, logs a one-time warning)

Used for RPC request IDs, chat idempotency keys, attachment IDs, and queue item IDs.

---

## Data Flow Summary

```
User Action
    |
    v
OpenClawApp method wrapper  (e.g., handleSendChat)
    |
    v
app-*.ts module function    (e.g., app-chat.ts::handleSendChat)
    |
    v
Controller function          (e.g., controllers/chat.ts::sendChatMessage)
    |
    v
GatewayBrowserClient.request()  -- WebSocket RPC
    |
    v
Gateway server processes request, streams events
    |
    v
GatewayBrowserClient.onEvent callback
    |
    v
app-gateway.ts::handleGatewayEvent()  -- routes by event name
    |
    v
State mutations on OpenClawApp @state() properties
    |
    v
Lit reactive update cycle  -- re-renders affected template parts
    |
    v
renderApp() -> specific view render function
    |
    v
DOM updates (efficient diff via lit-html)
```

---

## Key Design Decisions

1. **No Shadow DOM on root** -- Using light DOM for the main app allows a single global stylesheet to style everything, avoiding the complexity of distributing styles across shadow boundaries. Only the `ResizableDivider` uses Shadow DOM since it is a self-contained reusable component.

2. **Functional render functions over nested components** -- Views are plain functions, not custom elements. This simplifies data flow (props in, callbacks out) and avoids the overhead of many custom element registrations.

3. **External module decomposition** -- The main `app.ts` file is kept minimal by delegating all logic to external modules that operate on typed host interfaces. This enables unit testing of each module in isolation.

4. **Optimistic updates** -- Chat messages are added to the local state immediately on send (before the server confirms), providing instant feedback. Errors are handled by appending error messages.

5. **LRU Markdown cache** -- Frequently re-rendered messages benefit from a 200-entry Markdown cache that avoids re-parsing unchanged content.

6. **Device identity for auth** -- Ed25519 keypairs generated client-side enable device-specific tokens, which means the shared gateway token can be revoked without affecting individual devices.

7. **View Transitions for theme switching** -- The circular reveal animation provides polished theme transitions without any library dependencies, degrading gracefully to instant switching.

8. **Configurable base path** -- The UI can be served at any sub-path (e.g., behind a reverse proxy) thanks to runtime base path injection and dynamic path resolution.
