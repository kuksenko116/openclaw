# Infrastructure, Logging, Process Management, Daemon, Pairing, Cron & Utilities

This document provides an exhaustive reference for all infrastructure layers in the OpenClaw codebase: bootstrap and entry, logging, process management, daemon mode, device pairing, cron scheduling, auto-reply dispatch, path resolution, environment management, update system, health checks, and every utility module in `src/infra/`.

---

## 1. Overview -- Infrastructure Layers

OpenClaw's infrastructure is organized into these layers, from lowest to highest:

| Layer | Location | Purpose |
|---|---|---|
| **Bootstrap** | `openclaw.mjs`, `src/entry.ts` | Module compile cache, dist resolution, respawn, profile parsing |
| **Runtime Guard** | `src/infra/runtime-guard.ts` | Node.js version enforcement (>=22.12.0) |
| **Logging** | `src/logging/` (8 modules) | Structured file logging, console capture, subsystem loggers, redaction |
| **Process Management** | `src/process/` (6 modules) | Command execution, lane-based queues, signal bridging, spawn utilities |
| **Daemon Mode** | `src/daemon/` (20+ modules) | launchd (macOS), systemd (Linux), schtasks (Windows) service management |
| **Device Pairing** | `src/pairing/`, `src/infra/device-*` | Pairing flow, code exchange, device identity, auth tokens |
| **Cron System** | `src/cron/` (15+ modules) | Job scheduling, recurring/one-shot jobs, isolated agent runs, delivery |
| **Auto-Reply** | `src/auto-reply/` (50+ modules) | Message dispatch, reply generation, templating, command registry |
| **Health & Diagnostics** | `src/infra/diagnostic-*`, `src/infra/heartbeat-*`, `src/infra/system-*` | Heartbeat runner, system presence, diagnostic events |
| **Infrastructure Utilities** | `src/infra/` (100+ modules) | File locking, networking, retry, backoff, crypto, state migrations, updates |
| **Path Resolution** | `src/config/paths.ts`, `src/infra/home-dir.ts`, `src/infra/tmp-openclaw-dir.ts` | State directory, config paths, credentials, legacy migration |
| **Environment** | `src/infra/env.ts`, `src/infra/dotenv.ts`, `src/infra/env-file.ts` | Env var loading, normalization, `.env` file management |
| **Update System** | `src/infra/update-*.ts` | Self-update via git or npm, version checking, channel management |

---

## 2. Bootstrap and Entry

### `openclaw.mjs` -- Bootstrap Script

The top-level entry point. This is the file referenced by `bin` in `package.json`.

```javascript
#!/usr/bin/env node
import module from "node:module";

// Enable Node.js compile cache for faster subsequent startups
if (module.enableCompileCache && !process.env.NODE_DISABLE_COMPILE_CACHE) {
  try { module.enableCompileCache(); } catch { /* ignore */ }
}
```

**Responsibilities:**

1. **Module compile cache**: Calls `module.enableCompileCache()` (Node 22.8+) to cache compiled bytecode, reducing cold-start time on subsequent runs.
2. **Warning filter installation**: Imports and installs the process warning filter from `dist/warning-filter.js` (or `.mjs`) before any application code runs, ensuring ExperimentalWarning and deprecation notices are suppressed from the very start.
3. **Dist resolution**: Attempts to import `./dist/entry.js`, then falls back to `./dist/entry.mjs`. Throws `"openclaw: missing dist/entry.(m)js (build output)."` if neither exists. Only `ERR_MODULE_NOT_FOUND` errors are swallowed; real runtime errors are rethrown.

The bootstrap uses a helper `isModuleNotFoundError(err)` that checks for `err.code === "ERR_MODULE_NOT_FOUND"` and a `tryImport(specifier)` that swallows only module-not-found errors.

### `src/entry.ts` -- Application Entry Point

The compiled entry point loaded by the bootstrap.

**Initialization sequence (synchronous):**

1. `process.title = "openclaw"` -- Sets the process title for `ps` and activity monitor.
2. `installProcessWarningFilter()` -- Installs the warning filter (idempotent, uses a global Symbol key).
3. `normalizeEnv()` -- Normalizes environment variables (currently only `Z_AI_API_KEY` to `ZAI_API_KEY`).
4. `--no-color` handling: If `--no-color` is in argv, sets `NO_COLOR=1` and `FORCE_COLOR=0`.
5. Windows argv normalization: `process.argv = normalizeWindowsArgv(process.argv)`.

**Respawn mechanism (`ensureExperimentalWarningSuppressed`):**

The entry point may respawn itself with additional Node flags. The decision tree:

```
shouldSkipRespawnForArgv(argv)?  --> skip (certain subcommands don't need it)
OPENCLAW_NO_RESPAWN=1?           --> skip
OPENCLAW_NODE_OPTIONS_READY=1?   --> skip (already respawned)
hasExperimentalWarningSuppressed? --> skip (flag already present)
otherwise                        --> respawn with --disable-warning=ExperimentalWarning
```

When respawning:
- Sets `OPENCLAW_NODE_OPTIONS_READY=1` as a recursion guard.
- Spawns `process.execPath` with `[--disable-warning=ExperimentalWarning, ...process.execArgv, ...process.argv.slice(1)]`.
- Uses `stdio: "inherit"` for transparent I/O.
- Attaches a child process bridge (`attachChildProcessBridge`) to forward signals.
- Parent process waits for child exit and propagates exit code.

**Profile parsing (when not respawning):**

```typescript
const parsed = parseCliProfileArgs(process.argv);
if (parsed.profile) {
  applyCliProfileEnv({ profile: parsed.profile });
  process.argv = parsed.argv;  // Strip profile flags before Commander sees them
}
```

Then dynamically imports `./cli/run-main.js` and calls `runCli(process.argv)`.

### `src/infra/warning-filter.ts` -- Process Warning Filter

**Type: `ProcessWarning`**
```typescript
type ProcessWarning = {
  code?: string;
  name?: string;
  message?: string;
};
```

**Suppressed warnings:**
| Code | Message Contains | Reason |
|---|---|---|
| `DEP0040` | `punycode` | Deprecated punycode module (used by dependencies) |
| `DEP0060` | `util._extend` | Deprecated util._extend (used by dependencies) |
| (ExperimentalWarning) | `SQLite is an experimental feature` | Node 22 native SQLite |

**Installation mechanism:** Uses `Symbol.for("openclaw.warning-filter")` on `globalThis` to ensure idempotent installation. Wraps `process.emitWarning` with a filter that normalizes warning arguments (supports Error objects, string+options, and positional string args) and calls `shouldIgnoreWarning()` before forwarding to the original.

---

## 3. Logging Infrastructure (`src/logging/`)

### Architecture Overview

The logging system has four cooperating layers:

```
                    +-----------------+
                    | SubsystemLogger |  (per-module facade)
                    +--------+--------+
                             |
              +--------------+--------------+
              |                             |
      +-------v-------+           +--------v--------+
      | File Logger   |           | Console Output  |
      | (TSLog + JSON)|           | (color, styles) |
      +---------------+           +-----------------+
```

### `src/logging/levels.ts` -- Log Level Definitions

```typescript
const ALLOWED_LOG_LEVELS = ["silent", "fatal", "error", "warn", "info", "debug", "trace"] as const;
type LogLevel = (typeof ALLOWED_LOG_LEVELS)[number];
```

**Level ordering** (TSLog convention): `fatal=0, error=1, warn=2, info=3, debug=4, trace=5, silent=Infinity`.

**Functions:**
- `normalizeLogLevel(level?: string, fallback: LogLevel = "info"): LogLevel` -- Validates and normalizes a string to a valid log level.
- `levelToMinLevel(level: LogLevel): number` -- Converts a level name to its numeric value for TSLog's `minLevel` parameter.

### `src/logging/state.ts` -- Shared Mutable State

A singleton module holding all mutable logging state:

```typescript
const loggingState = {
  cachedLogger: null,                    // TsLogger instance
  cachedSettings: null,                  // ResolvedSettings (level, file)
  cachedConsoleSettings: null,           // ConsoleSettings (level, style)
  overrideSettings: null,               // Test override
  consolePatched: false,                 // Console capture installed?
  forceConsoleToStderr: false,           // RPC/JSON mode
  consoleTimestampPrefix: false,         // Prefix console lines with timestamps
  consoleSubsystemFilter: null as string[] | null,  // Subsystem allowlist
  resolvingConsoleSettings: false,       // Reentrancy guard
  streamErrorHandlersInstalled: false,   // EPIPE handler installed?
  rawConsole: null,                      // Original console methods (pre-patch)
};
```

### `src/logging/config.ts` -- Configuration Loading

```typescript
function readLoggingConfig(): LoggingConfig | undefined;
```

Reads `logging` section from the JSON5 config file at `resolveConfigPath()`. Returns `undefined` if the file is missing or the logging section is absent/invalid. Uses `json5` parser for relaxed JSON syntax (comments, trailing commas).

### `src/logging/logger.ts` -- Core File Logger

**Key constants:**
```typescript
const DEFAULT_LOG_DIR = resolvePreferredOpenClawTmpDir();  // /tmp/openclaw or fallback
const DEFAULT_LOG_FILE = path.join(DEFAULT_LOG_DIR, "openclaw.log");  // legacy path
const LOG_PREFIX = "openclaw";
const LOG_SUFFIX = ".log";
const MAX_LOG_AGE_MS = 24 * 60 * 60 * 1000;  // 24 hours
```

**Types:**
```typescript
type LoggerSettings = {
  level?: LogLevel;
  file?: string;
  consoleLevel?: LogLevel;
  consoleStyle?: ConsoleStyle;
};

type ResolvedSettings = {
  level: LogLevel;
  file: string;
};

type LogTransport = (logObj: LogTransportRecord) => void;
```

**Settings resolution (`resolveSettings`):**
1. Check `loggingState.overrideSettings` (test mode).
2. Check `readLoggingConfig()` (config file).
3. Fall back to `require("../config/config.js").loadConfig().logging`.
4. Default level: `"silent"` during tests (`VITEST=true`), `"info"` otherwise.
5. Default file: rolling daily path (e.g., `/tmp/openclaw/openclaw-2026-02-15.log`).

**Logger construction (`buildLogger`):**
- Creates the log directory recursively.
- Prunes old rolling logs (files matching `openclaw-YYYY-MM-DD.log` older than 24h).
- Instantiates `TsLogger` with `type: "hidden"` (no ANSI in output).
- Attaches a file transport that writes JSON lines: `{ ...logObj, time: ISO8601 }`.
- Attaches all registered external transports.

**Rolling log scheme:**
- Filename format: `openclaw-YYYY-MM-DD.log` (local date).
- Detection: `isRollingPath()` checks prefix, suffix, and exact length.
- Pruning: `pruneOldRollingLogs()` iterates directory entries, checks `stat.mtimeMs` against 24h cutoff.

**Public API:**
```typescript
function getLogger(): TsLogger<LogObj>;
function getChildLogger(bindings?, opts?): TsLogger<LogObj>;
function toPinoLikeLogger(logger, level): PinoLikeLogger;  // Baileys adapter
function registerLogTransport(transport): () => void;       // Returns unsubscribe
function getResolvedLoggerSettings(): LoggerResolvedSettings;
function setLoggerOverride(settings: LoggerSettings | null): void;  // Tests
function resetLogger(): void;                                        // Tests
function isFileLogLevelEnabled(level: LogLevel): boolean;
```

The `toPinoLikeLogger` adapter wraps TSLog in a pino-compatible interface with `level`, `child()`, `trace()`, `debug()`, `info()`, `warn()`, `error()`, `fatal()` methods. Used by the Baileys WhatsApp library.

### `src/logging/console.ts` -- Console Capture and Output

**Console styles:**
```typescript
type ConsoleStyle = "pretty" | "compact" | "json";
```

- `"pretty"`: Includes HH:MM:SS timestamps, colorized subsystem tags and level indicators.
- `"compact"`: No timestamps unless `consoleTimestampPrefix` is enabled. Default for non-TTY.
- `"json"`: Structured JSON output per line.

**Console level resolution:**
- If `isVerbose()`, returns `"debug"`.
- During tests (`VITEST=true`), returns `"silent"` unless `OPENCLAW_TEST_CONSOLE=1`.
- Otherwise normalizes from config with `"info"` fallback.

**Console capture (`enableConsoleCapture`):**

Patches `console.log`, `.info`, `.warn`, `.error`, `.debug`, `.trace` to:
1. Format args via `util.format()`.
2. Check `shouldSuppressConsoleMessage()` (suppresses noisy session open/close messages and slow Discord listener warnings in non-verbose mode).
3. Route to file logger at the appropriate level.
4. If `forceConsoleToStderr` (RPC/JSON mode), write to `process.stderr`.
5. Otherwise call the original console method, optionally prepending a timestamp.
6. EPIPE/EIO errors on stdout/stderr are silently caught (graceful shutdown).

**Subsystem filtering:**
```typescript
function setConsoleSubsystemFilter(filters?: string[] | null): void;
function shouldLogSubsystemToConsole(subsystem: string): boolean;
```
When set, only subsystems matching the filter prefixes are shown on the console. Matching is prefix-based: filter `"discord"` matches `"discord"` and `"discord/webhooks"`.

**Timestamp formatting:**
```typescript
function formatConsoleTimestamp(style: ConsoleStyle): string;
// "pretty": "HH:MM:SS"
// other:    "YYYY-MM-DDTHH:MM:SS.mms+TZ:TZ"
```

### `src/logging/subsystem.ts` -- Subsystem Logger Factory

The primary logging facade used throughout the codebase.

**Type: `SubsystemLogger`**
```typescript
type SubsystemLogger = {
  subsystem: string;
  isEnabled: (level: LogLevel, target?: "any" | "console" | "file") => boolean;
  trace: (message: string, meta?: Record<string, unknown>) => void;
  debug: (message: string, meta?: Record<string, unknown>) => void;
  info:  (message: string, meta?: Record<string, unknown>) => void;
  warn:  (message: string, meta?: Record<string, unknown>) => void;
  error: (message: string, meta?: Record<string, unknown>) => void;
  fatal: (message: string, meta?: Record<string, unknown>) => void;
  raw:   (message: string) => void;
  child: (name: string) => SubsystemLogger;
};
```

**Emit flow (`emit` internal function):**
1. Write to file logger via `logToFile()` (level-filtered by TSLog).
2. Check console level gate (`shouldLogToConsole`).
3. Check subsystem filter (`shouldLogSubsystemToConsole`).
4. Suppress probe session logs in non-verbose mode.
5. Format console line via `formatConsoleLine()`.
6. Write via `writeConsoleLine()`.

**Meta handling:** If `meta` contains a `consoleMessage` key, that string is used for console display while the original `message` goes to the file logger. This allows rich file logging with concise console output.

**Console line formatting:**
- Subsystem prefix stripping: removes redundant prefixes like `gateway`, `channels`, `providers`.
- Channel subsystems show just the channel name.
- Max 2 path segments displayed.
- Color assignment: deterministic hash-based from `SUBSYSTEM_COLORS` (cyan, green, yellow, blue, magenta, red), with overrides (e.g., `gmail-watcher` always blue).
- Redundant tag stripping: `[discord] discord: connected` becomes `[discord] connected`.

**Helper functions:**
```typescript
function createSubsystemLogger(subsystem: string): SubsystemLogger;
function runtimeForLogger(logger, exit?): RuntimeEnv;
function createSubsystemRuntime(subsystem, exit?): RuntimeEnv;
```

### `src/logging/redact.ts` -- Sensitive Data Redaction

**Modes:** `"off"` | `"tools"` (default).

**Default redaction patterns** (16 patterns):
- ENV-style assignments: `KEY|TOKEN|SECRET|PASSWORD = value`
- JSON fields: `"apiKey": "value"`, `"token": "value"`, etc.
- CLI flags: `--api-key value`, `--token value`
- Authorization headers: `Bearer <token>`
- PEM private key blocks
- Common token prefixes: `sk-*`, `ghp_*`, `github_pat_*`, `xox[baprs]-*`, `xapp-*`, `gsk_*`, `AIza*`, `pplx-*`, `npm_*`, Telegram bot tokens

**Token masking:** Tokens shorter than 18 characters become `***`. Longer tokens show first 6 and last 4 characters: `sk-abc1...xyz9`.

**PEM handling:** PEM blocks are replaced with `-----BEGIN...\n...redacted...\n-----END...`.

**Public API:**
```typescript
function redactSensitiveText(text: string, options?: RedactOptions): string;
function redactToolDetail(detail: string): string;  // Only in "tools" mode
function getDefaultRedactPatterns(): string[];
```

### `src/logging/redact-identifier.ts` -- Identifier Redaction

```typescript
function sha256HexPrefix(value: string, len = 12): string;
function redactIdentifier(value: string | undefined, opts?: { len?: number }): string;
// Returns "sha256:abc123def456" or "-" for empty input
```

Used to log identifiers (chat IDs, user IDs) in a privacy-preserving way.

### `src/logging/parse-log-line.ts` -- Log Line Parser

```typescript
type ParsedLogLine = {
  time?: string;
  level?: string;
  subsystem?: string;
  module?: string;
  message: string;
  raw: string;
};

function parseLogLine(raw: string): ParsedLogLine | null;
```

Parses JSON log lines back into structured objects. Extracts metadata from the `_meta` field (TSLog convention): `name` (parsed as JSON containing `subsystem` and `module`), `logLevelName`, `date`. Message is reconstructed from numeric-keyed fields in the log object.

### `src/logging/diagnostic.ts` -- Diagnostic Logging

A specialized logging module for system health monitoring.

**Session state tracking:**
```typescript
type SessionStateValue = "idle" | "processing" | "waiting";
type SessionState = {
  sessionId?: string;
  sessionKey?: string;
  lastActivity: number;
  state: SessionStateValue;
  queueDepth: number;
};
```

**Webhook statistics:** Global counters for received, processed, and error counts.

**Functions:**
```typescript
function logWebhookReceived(params: { channel, updateType?, chatId? }): void;
function logWebhookProcessed(params: { channel, updateType?, chatId?, durationMs? }): void;
function logWebhookError(params: { channel, updateType?, chatId?, error }): void;
function logMessageQueued(params: { sessionId?, sessionKey?, channel?, source }): void;
function logMessageProcessed(params: { channel, messageId?, chatId?, sessionId?, sessionKey?, durationMs?, outcome, reason?, error? }): void;
function logSessionStateChange(params: { sessionId?, sessionKey?, state, reason? }): void;
function logSessionStuck(params: { sessionId?, sessionKey?, state, ageMs }): void;
function logLaneEnqueue(lane, queueSize): void;
function logLaneDequeue(lane, waitMs, queueSize): void;
function logRunAttempt(params: { sessionId?, sessionKey?, runId, attempt }): void;
function logActiveRuns(): void;
```

**Diagnostic heartbeat:** `startDiagnosticHeartbeat()` runs a 30-second interval that:
- Counts active/waiting sessions and total queue depth.
- Suppresses output when idle for >120s with no activity.
- Detects stuck sessions (>120s in "processing" state).
- Emits `diagnostic.heartbeat` events.
- Timer is `unref()`'d so it does not prevent process exit.

### `src/logger.ts` -- Legacy Logger Facade

Convenience functions that auto-detect subsystem prefixes:

```typescript
function logInfo(message: string, runtime?: RuntimeEnv): void;
function logWarn(message: string, runtime?: RuntimeEnv): void;
function logSuccess(message: string, runtime?: RuntimeEnv): void;
function logError(message: string, runtime?: RuntimeEnv): void;
function logDebug(message: string): void;
```

If the message starts with a pattern like `discord: connected`, it extracts `"discord"` as the subsystem and routes to `createSubsystemLogger("discord").info("connected")`.

### `src/logging.ts` -- Barrel Re-exports

Re-exports all logging symbols from `logging/console.ts`, `logging/levels.ts`, `logging/logger.ts`, and `logging/subsystem.ts` as a single import target.

---

## 4. Process Management (`src/process/`)

### `src/process/lanes.ts` -- Command Lane Definitions

```typescript
const enum CommandLane {
  Main = "main",
  Cron = "cron",
  Subagent = "subagent",
  Nested = "nested",
}
```

Lanes isolate different execution contexts to prevent interleaving. The main lane serializes user message processing; the cron lane allows parallel cron job execution.

### `src/process/command-queue.ts` -- Lane-Based Command Queue

The core serialization primitive for all command execution.

**Types:**
```typescript
type QueueEntry = {
  task: () => Promise<unknown>;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
  enqueuedAt: number;
  warnAfterMs: number;
  onWait?: (waitMs: number, queuedAhead: number) => void;
};

type LaneState = {
  lane: string;
  queue: QueueEntry[];
  activeTaskIds: Set<number>;
  maxConcurrent: number;
  draining: boolean;
  generation: number;  // Incremented on reset to invalidate stale completions
};
```

**Error type:**
```typescript
class CommandLaneClearedError extends Error {
  constructor(lane?: string);
}
```

**Functions:**
```typescript
function enqueueCommandInLane<T>(lane, task, opts?): Promise<T>;
function enqueueCommand<T>(task, opts?): Promise<T>;  // Main lane shortcut
function setCommandLaneConcurrency(lane, maxConcurrent): void;
function getQueueSize(lane?): number;
function getTotalQueueSize(): number;
function clearCommandLane(lane?): number;  // Returns removed count
function resetAllLanes(): void;  // Post-SIGUSR1 restart recovery
function getActiveTaskCount(): number;
function waitForActiveTasks(timeoutMs): Promise<{ drained: boolean }>;
```

**Drain mechanism:** The `drainLane()` pump loop dequeues entries while `activeTaskIds.size < maxConcurrent`. Each task gets a unique `taskId` and tracks its `generation`. On completion, `completeTask()` verifies the generation matches (stale completions from pre-reset tasks are ignored). The pump re-invokes itself after each task completion to drain queued work.

**Lane reset (`resetAllLanes`):** Used after SIGUSR1 in-process restarts. Bumps `generation`, clears `activeTaskIds`, resets `draining` flag, then re-drains lanes that still have queued entries.

**Wait for active (`waitForActiveTasks`):** Polls at 50ms intervals, tracking which task IDs were active at call time. Resolves when all tracked tasks complete or timeout elapses.

### `src/process/exec.ts` -- Command Execution

**Simple execution:**
```typescript
async function runExec(
  command: string,
  args: string[],
  opts: number | { timeoutMs?: number; maxBuffer?: number } = 10_000,
): Promise<{ stdout: string; stderr: string }>;
```

Uses `execFile` (promisified). Resolves Windows `.cmd` extensions automatically for npm/pnpm/yarn/npx.

**Full execution with spawn:**
```typescript
type SpawnResult = {
  stdout: string;
  stderr: string;
  code: number | null;
  signal: NodeJS.Signals | null;
  killed: boolean;
};

type CommandOptions = {
  timeoutMs: number;
  cwd?: string;
  input?: string;
  env?: NodeJS.ProcessEnv;
  windowsVerbatimArguments?: boolean;
};

async function runCommandWithTimeout(argv: string[], options): Promise<SpawnResult>;
```

Features:
- Automatic `NPM_CONFIG_FUND=false` injection for npm commands.
- Shell mode on Windows for non-.exe commands.
- SIGKILL after timeout.
- Input piping via stdin.
- Environment merging with undefined filtering.

### `src/process/child-process-bridge.ts` -- Signal Forwarding

```typescript
type ChildProcessBridgeOptions = {
  signals?: NodeJS.Signals[];
  onSignal?: (signal: NodeJS.Signals) => void;
};

function attachChildProcessBridge(child: ChildProcess, options?): { detach: () => void };
```

**Default signals:**
- POSIX: `SIGTERM`, `SIGINT`, `SIGHUP`, `SIGQUIT`
- Windows: `SIGTERM`, `SIGINT`, `SIGBREAK`

Registers listeners on the parent process for each signal that forward to `child.kill(signal)`. Auto-detaches on child exit or error. Returns a `detach()` function for manual cleanup.

### `src/process/spawn-utils.ts` -- Spawn Helpers

```typescript
function resolveCommandStdio(params: { hasInput: boolean; preferInherit: boolean }):
  ["pipe" | "inherit" | "ignore", "pipe", "pipe"];

function formatSpawnError(err: unknown): string;

async function spawnWithFallback(params: SpawnWithFallbackParams): Promise<SpawnWithFallbackResult>;
```

`spawnWithFallback` tries the primary spawn options, then iterates through fallback options if the error code is retryable (default: `EBADF`). Each fallback can override `SpawnOptions`. Reports which fallback was used.

### `src/process/restart-recovery.ts` -- Restart Iteration Hook

```typescript
function createRestartIterationHook(onRestart: () => void): () => boolean;
```

Returns a function that returns `false` on first call (initial startup) and `true` on subsequent calls (restarts), invoking `onRestart` each time.

---

## 5. Daemon Mode (`src/daemon/`)

### Architecture

The daemon system abstracts platform-specific service management behind a common `GatewayService` interface.

### `src/daemon/constants.ts` -- Service Constants

```typescript
const GATEWAY_LAUNCH_AGENT_LABEL = "ai.openclaw.gateway";
const GATEWAY_SYSTEMD_SERVICE_NAME = "openclaw-gateway";
const GATEWAY_WINDOWS_TASK_NAME = "OpenClaw Gateway";
const GATEWAY_SERVICE_MARKER = "openclaw";
const GATEWAY_SERVICE_KIND = "gateway";
const NODE_LAUNCH_AGENT_LABEL = "ai.openclaw.node";
const NODE_SYSTEMD_SERVICE_NAME = "openclaw-node";
const NODE_WINDOWS_TASK_NAME = "OpenClaw Node";
```

**Profile-aware naming:** Labels include profile suffix for multi-profile support:
```typescript
function resolveGatewayLaunchAgentLabel(profile?: string): string;
// Default: "ai.openclaw.gateway", with profile: "ai.openclaw.<profile>"

function resolveGatewaySystemdServiceName(profile?: string): string;
// Default: "openclaw-gateway", with profile: "openclaw-gateway-<profile>"

function resolveGatewayWindowsTaskName(profile?: string): string;
// Default: "OpenClaw Gateway", with profile: "OpenClaw Gateway (<profile>)"
```

### `src/daemon/service.ts` -- Unified Service Interface

```typescript
type GatewayService = {
  label: string;            // "LaunchAgent" | "systemd" | "Scheduled Task"
  loadedText: string;       // "loaded" | "enabled" | "registered"
  notLoadedText: string;    // "not loaded" | "disabled" | "missing"
  install: (args: GatewayServiceInstallArgs) => Promise<void>;
  uninstall: (args) => Promise<void>;
  stop: (args) => Promise<void>;
  restart: (args) => Promise<void>;
  isLoaded: (args) => Promise<boolean>;
  readCommand: (env) => Promise<{ programArguments, workingDirectory?, environment?, sourcePath? } | null>;
  readRuntime: (env) => Promise<GatewayServiceRuntime>;
};

function resolveGatewayService(): GatewayService;
```

`resolveGatewayService()` returns the platform-appropriate implementation based on `process.platform`. Throws on unsupported platforms.

### `src/daemon/service-runtime.ts` -- Runtime Status Type

```typescript
type GatewayServiceRuntime = {
  status?: "running" | "stopped" | "unknown";
  state?: string;
  subState?: string;
  pid?: number;
  lastExitStatus?: number;
  lastExitReason?: string;
  lastRunResult?: string;
  lastRunTime?: string;
  detail?: string;
  cachedLabel?: boolean;
  missingUnit?: boolean;
};
```

### `src/daemon/launchd.ts` -- macOS LaunchAgent Integration

**Functions:**
```typescript
async function installLaunchAgent(args): Promise<{ plistPath: string }>;
async function uninstallLaunchAgent(args): Promise<void>;
async function stopLaunchAgent(args): Promise<void>;
async function restartLaunchAgent(args): Promise<void>;
async function isLaunchAgentLoaded(args): Promise<boolean>;
async function readLaunchAgentProgramArguments(env): Promise<{ programArguments, ... } | null>;
async function readLaunchAgentRuntime(env): Promise<GatewayServiceRuntime>;
async function findLegacyLaunchAgents(env): Promise<LegacyLaunchAgent[]>;
async function uninstallLegacyLaunchAgents(args): Promise<LegacyLaunchAgent[]>;
async function repairLaunchAgentBootstrap(args): Promise<{ ok, detail? }>;
```

**Install flow:**
1. Create log directory at `<stateDir>/logs/`.
2. Unload and remove all legacy LaunchAgents (profile migration).
3. Generate plist via `buildLaunchAgentPlist()` with `KeepAlive`, `RunAtLoad`, stdout/stderr log paths.
4. Write plist to `~/Library/LaunchAgents/<label>.plist`.
5. `launchctl bootout` (clear old state) then `launchctl enable` then `launchctl bootstrap` then `launchctl kickstart -k`.

**Runtime status:** Parses `launchctl print gui/<uid>/<label>` output for state, PID, last exit status, and exit reason.

**GUI domain resolution:** Uses `process.getuid()` to resolve the correct launchd domain (e.g., `gui/501`).

### `src/daemon/systemd.ts` -- Linux systemd Integration

**Functions:**
```typescript
async function installSystemdService(args): Promise<{ unitPath: string }>;
async function uninstallSystemdService(args): Promise<void>;
async function stopSystemdService(args): Promise<void>;
async function restartSystemdService(args): Promise<void>;
async function isSystemdServiceEnabled(args): Promise<boolean>;
async function readSystemdServiceRuntime(env): Promise<GatewayServiceRuntime>;
async function readSystemdServiceExecStart(env): Promise<{ programArguments, ... } | null>;
async function isSystemdUserServiceAvailable(): Promise<boolean>;
async function findLegacySystemdUnits(env): Promise<LegacySystemdUnit[]>;
async function uninstallLegacySystemdUnits(args): Promise<LegacySystemdUnit[]>;
```

**Unit file location:** `~/.config/systemd/user/<name>.service`.

**Install flow:**
1. Assert systemd availability (`systemctl --user status`).
2. Generate unit file via `buildSystemdUnit()`.
3. Write to user unit directory.
4. `systemctl --user daemon-reload` then `enable` then `restart`.

**Runtime status:** Parses `systemctl --user show <unit> --property ActiveState,SubState,MainPID,ExecMainStatus,ExecMainCode`.

**Linger support:** `enableSystemdUserLinger()` ensures the user's systemd instance persists after logout.

### `src/daemon/paths.ts` -- Daemon Path Resolution

```typescript
function resolveHomeDir(env): string;  // HOME or USERPROFILE, throws if missing
function resolveUserPathWithHome(input, home?): string;  // Expands ~ prefix
function resolveGatewayStateDir(env): string;
// OPENCLAW_STATE_DIR or ~/.openclaw<-profile>
```

### `src/daemon/service-env.ts` -- Service Environment Building

```typescript
function buildServiceEnvironment(params: { env, port, token?, launchdLabel? }):
  Record<string, string | undefined>;

function buildNodeServiceEnvironment(params: { env }):
  Record<string, string | undefined>;

function buildMinimalServicePath(options?): string;
function getMinimalServicePathParts(options?): string[];
function resolveLinuxUserBinDirs(home, env?): string[];
```

**Service PATH construction:** Builds a minimal PATH containing:
- Extra directories (from caller).
- Linux user bin directories: `~/.local/bin`, `~/.npm-global/bin`, `~/bin`, nvm/fnm/volta/asdf/pnpm/bun paths.
- System directories: `/opt/homebrew/bin` (macOS), `/usr/local/bin`, `/usr/bin`, `/bin`.

**Service environment variables:** `HOME`, `PATH`, `OPENCLAW_PROFILE`, `OPENCLAW_STATE_DIR`, `OPENCLAW_CONFIG_PATH`, `OPENCLAW_GATEWAY_PORT`, `OPENCLAW_GATEWAY_TOKEN`, `OPENCLAW_LAUNCHD_LABEL`, `OPENCLAW_SYSTEMD_UNIT`, `OPENCLAW_SERVICE_MARKER`, `OPENCLAW_SERVICE_KIND`, `OPENCLAW_SERVICE_VERSION`.

---

## 6. Device Pairing (`src/pairing/` and `src/infra/device-*`)

### `src/infra/device-identity.ts` -- Device Identity

Each OpenClaw installation has a persistent Ed25519 keypair.

**Type:**
```typescript
type DeviceIdentity = {
  deviceId: string;       // SHA-256 hex of raw public key
  publicKeyPem: string;
  privateKeyPem: string;
};
```

**Storage:** `<stateDir>/identity/device.json` with permissions `0o600`.

**Functions:**
```typescript
function loadOrCreateDeviceIdentity(filePath?): DeviceIdentity;
function signDevicePayload(privateKeyPem, payload): string;  // Base64URL signature
function verifyDeviceSignature(publicKey, payload, signatureBase64Url): boolean;
function deriveDeviceIdFromPublicKey(publicKey): string | null;
function publicKeyRawBase64UrlFromPem(publicKeyPem): string;
function normalizeDevicePublicKeyBase64Url(publicKey): string | null;
```

**Key generation:** Uses `crypto.generateKeyPairSync("ed25519")`. Device ID is the SHA-256 hex of the raw 32-byte public key (stripped of SPKI prefix).

### `src/infra/device-pairing.ts` -- Device Pairing Protocol

**Types:**
```typescript
type DevicePairingPendingRequest = {
  requestId: string;      // UUID
  deviceId: string;
  publicKey: string;
  displayName?: string;
  platform?: string;
  clientId?: string;
  clientMode?: string;
  role?: string;
  roles?: string[];
  scopes?: string[];
  remoteIp?: string;
  silent?: boolean;
  isRepair?: boolean;     // Re-pairing existing device
  ts: number;
};

type DeviceAuthToken = {
  token: string;          // 32-char hex (UUID without dashes)
  role: string;
  scopes: string[];
  createdAtMs: number;
  rotatedAtMs?: number;
  revokedAtMs?: number;
  lastUsedAtMs?: number;
};

type PairedDevice = {
  deviceId: string;
  publicKey: string;
  displayName?: string;
  platform?: string;
  clientId?: string;
  clientMode?: string;
  role?: string;
  roles?: string[];
  scopes?: string[];
  remoteIp?: string;
  tokens?: Record<string, DeviceAuthToken>;
  createdAtMs: number;
  approvedAtMs: number;
};
```

**Pending request TTL:** 5 minutes (`PENDING_TTL_MS`).

**State storage:** Split into two files under `<stateDir>/pairing/devices/`:
- `pending.json`: `Record<requestId, DevicePairingPendingRequest>`
- `paired.json`: `Record<deviceId, PairedDevice>`

**Operations (all use async lock for serialization):**
```typescript
async function listDevicePairing(baseDir?): Promise<DevicePairingList>;
async function getPairedDevice(deviceId, baseDir?): Promise<PairedDevice | null>;
async function requestDevicePairing(req, baseDir?): Promise<{ status, request, created }>;
async function approveDevicePairing(requestId, baseDir?): Promise<{ requestId, device } | null>;
async function rejectDevicePairing(requestId, baseDir?): Promise<{ requestId, deviceId } | null>;
async function updatePairedDeviceMetadata(deviceId, patch, baseDir?): Promise<void>;
async function verifyDeviceToken(params): Promise<{ ok, reason? }>;
async function ensureDeviceToken(params): Promise<DeviceAuthToken | null>;
async function rotateDeviceToken(params): Promise<DeviceAuthToken | null>;
async function revokeDeviceToken(params): Promise<DeviceAuthToken | null>;
```

**Token verification:** Checks device exists, role has a token, token is not revoked, constant-time comparison via `safeEqualSecret`, and scope validation. Updates `lastUsedAtMs` on success.

### `src/pairing/pairing-store.ts` -- Channel Pairing Store

Handles per-channel pairing (WhatsApp, Telegram, etc.) with approval codes.

**Constants:**
```typescript
const PAIRING_CODE_LENGTH = 8;
const PAIRING_CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";  // No 0/O/1/I
const PAIRING_PENDING_TTL_MS = 60 * 60 * 1000;  // 1 hour
const PAIRING_PENDING_MAX = 3;
```

**Types:**
```typescript
type PairingRequest = {
  id: string;
  code: string;        // 8-char human-friendly code
  createdAt: string;
  lastSeenAt: string;
  meta?: Record<string, string>;
};
```

**Storage:** `<credentialsDir>/<channel>-pairing.json` and `<channel>-allowFrom.json`, protected by file locking.

**Operations:**
```typescript
async function listChannelPairingRequests(channel, env?): Promise<PairingRequest[]>;
async function upsertChannelPairingRequest(params): Promise<{ code, created }>;
async function approveChannelPairingCode(params): Promise<{ id, entry? } | null>;
async function readChannelAllowFromStore(channel, env?): Promise<string[]>;
async function addChannelAllowFromStoreEntry(params): Promise<{ changed, allowFrom }>;
async function removeChannelAllowFromStoreEntry(params): Promise<{ changed, allowFrom }>;
```

**Approval flow:**
1. Unknown sender messages `upsertChannelPairingRequest` with their sender ID.
2. System returns an 8-character code.
3. Bot owner runs `openclaw pairing approve <channel> <code>`.
4. `approveChannelPairingCode` matches the code, removes from pending, adds sender to `allowFrom`.

### `src/pairing/pairing-messages.ts` -- Pairing Reply Messages

```typescript
function buildPairingReply(params: { channel, idLine, code }): string;
```

Builds the multi-line reply shown to unapproved senders, including the pairing code and CLI command to approve.

### `src/pairing/pairing-labels.ts` -- Pairing Label Resolution

```typescript
function resolvePairingIdLabel(channel: PairingChannel): string;
```

Returns the human-readable label for the pairing ID (e.g., "userId", "chatId") by querying the channel's pairing adapter.

---

## 7. Cron System (`src/cron/`)

### `src/cron/types.ts` -- Core Types

```typescript
type CronSchedule =
  | { kind: "at"; at: string }                              // One-shot absolute time
  | { kind: "every"; everyMs: number; anchorMs?: number }   // Interval-based
  | { kind: "cron"; expr: string; tz?: string };            // Cron expression with timezone

type CronSessionTarget = "main" | "isolated";
type CronWakeMode = "next-heartbeat" | "now";
type CronDeliveryMode = "none" | "announce";
type CronMessageChannel = ChannelId | "last";

type CronDelivery = {
  mode: CronDeliveryMode;
  channel?: CronMessageChannel;
  to?: string;
  bestEffort?: boolean;
};

type CronPayload =
  | { kind: "systemEvent"; text: string }
  | { kind: "agentTurn"; message: string; model?: string; thinking?: string;
      timeoutSeconds?: number; allowUnsafeExternalContent?: boolean;
      deliver?: boolean; channel?: CronMessageChannel; to?: string;
      bestEffortDeliver?: boolean; };

type CronJobState = {
  nextRunAtMs?: number;
  runningAtMs?: number;
  lastRunAtMs?: number;
  lastStatus?: "ok" | "error" | "skipped";
  lastError?: string;
  lastDurationMs?: number;
  consecutiveErrors?: number;
  scheduleErrorCount?: number;
};

type CronJob = {
  id: string;
  agentId?: string;
  name: string;
  description?: string;
  enabled: boolean;
  deleteAfterRun?: boolean;
  createdAtMs: number;
  updatedAtMs: number;
  schedule: CronSchedule;
  sessionTarget: CronSessionTarget;
  wakeMode: CronWakeMode;
  payload: CronPayload;
  delivery?: CronDelivery;
  state: CronJobState;
};

type CronStoreFile = { version: 1; jobs: CronJob[] };
```

### `src/cron/store.ts` -- Store Persistence

```typescript
const DEFAULT_CRON_DIR = path.join(CONFIG_DIR, "cron");
const DEFAULT_CRON_STORE_PATH = path.join(DEFAULT_CRON_DIR, "jobs.json");

function resolveCronStorePath(storePath?: string): string;
async function loadCronStore(storePath: string): Promise<CronStoreFile>;
async function saveCronStore(storePath: string, store: CronStoreFile): Promise<void>;
```

**Save mechanism:** Atomic write via temp file + rename. Creates a `.bak` backup on every save.

### `src/cron/schedule.ts` -- Schedule Computation

```typescript
function computeNextRunAtMs(schedule: CronSchedule, nowMs: number): number | undefined;
```

**Schedule kinds:**
- `"at"`: Returns the absolute time if in the future; `undefined` if past. Handles legacy `atMs` number field.
- `"every"`: Computes next interval boundary from anchor point. `steps = ceil((elapsed + everyMs - 1) / everyMs)`.
- `"cron"`: Uses the `croner` library. Floors `nowMs` to the current second boundary. Requests the next occurrence strictly after the floored time to prevent duplicate fires (fix for #14164).

### `src/cron/service.ts` -- CronService Class

```typescript
class CronService {
  constructor(deps: CronServiceDeps);
  async start(): Promise<void>;
  stop(): void;
  async status(): Promise<CronStatusSummary>;
  async list(opts?: { includeDisabled?: boolean }): Promise<CronJob[]>;
  async add(input: CronJobCreate): Promise<CronJob>;
  async update(id: string, patch: CronJobPatch): Promise<CronJob>;
  async remove(id: string): Promise<CronRemoveResult>;
  async run(id: string, mode?: "due" | "force"): Promise<CronRunResult>;
  wake(opts: { mode: "now" | "next-heartbeat"; text: string }): void;
}
```

### `src/cron/service/state.ts` -- Service State

```typescript
type CronServiceDeps = {
  nowMs?: () => number;
  log: Logger;
  storePath: string;
  cronEnabled: boolean;
  cronConfig?: CronConfig;
  defaultAgentId?: string;
  resolveSessionStorePath?: (agentId?) => string;
  sessionStorePath?: string;
  enqueueSystemEvent: (text, opts?) => void;
  requestHeartbeatNow: (opts?) => void;
  runHeartbeatOnce?: (opts?) => Promise<HeartbeatRunResult>;
  runIsolatedAgentJob: (params) => Promise<{ status, summary?, outputText?, error?, sessionId?, sessionKey?, delivered? }>;
  onEvent?: (evt: CronEvent) => void;
};

type CronServiceState = {
  deps: CronServiceDepsInternal;
  store: CronStoreFile | null;
  timer: NodeJS.Timeout | null;
  running: boolean;
  op: Promise<unknown>;
  warnedDisabled: boolean;
  storeLoadedAtMs: number | null;
  storeFileMtimeMs: number | null;
};

type CronEvent = {
  jobId: string;
  action: "added" | "updated" | "removed" | "started" | "finished";
  runAtMs?: number;
  durationMs?: number;
  status?: "ok" | "error" | "skipped";
  error?: string;
  summary?: string;
  sessionId?: string;
  sessionKey?: string;
  nextRunAtMs?: number;
};
```

### Internal Service Modules (`src/cron/service/`)

| Module | Purpose |
|---|---|
| `state.ts` | State types, factory, result types |
| `ops.ts` | Public operations (start, stop, status, list, add, update, remove, run, wakeNow) |
| `store.ts` | Store loading/persisting with mtime-based staleness detection |
| `jobs.ts` | Job creation, patching, next-run computation, schedule error isolation |
| `timer.ts` | Timer arm/stop, job execution, catch-up logic, event emission |
| `locked.ts` | Operation serialization (queue-based, ensures one-at-a-time) |
| `normalize.ts` | Schedule normalization |

### Additional Cron Modules

| Module | Purpose |
|---|---|
| `isolated-agent.ts` | Runs agent jobs in isolated sessions |
| `delivery.ts` | Delivers cron job output to configured channels |
| `schedule.ts` | Cron expression parsing and next-run calculation |
| `parse.ts` | Schedule string parsing |
| `normalize.ts` | Schedule normalization |
| `run-log.ts` | Run history logging |
| `session-reaper.ts` | Cleans up stale isolated sessions |
| `payload-migration.ts` | Migrates legacy payload formats |
| `validate-timestamp.ts` | Timestamp validation |

---

## 8. Health Checks and Diagnostics

### `src/infra/system-presence.ts` -- System Presence

Tracks active OpenClaw instances on the network.

**Type:**
```typescript
type SystemPresence = {
  host?: string;
  ip?: string;
  version?: string;
  platform?: string;
  deviceFamily?: string;
  modelIdentifier?: string;
  lastInputSeconds?: number;
  mode?: string;
  reason?: string;
  deviceId?: string;
  roles?: string[];
  scopes?: string[];
  instanceId?: string;
  text: string;
  ts: number;
};
```

**Constants:** TTL 5 minutes, max 200 entries.

**Self-presence:** Initialized at module load with hostname, primary IPv4, version, platform (including macOS version via `sw_vers`), model identifier (macOS `sysctl -n hw.model`, Linux `os.arch()`).

**Functions:**
```typescript
function updateSystemPresence(payload): SystemPresenceUpdate;
function upsertPresence(key, presence): void;
function listSystemPresence(): SystemPresence[];
```

`listSystemPresence()` prunes expired entries (>5 min TTL), enforces LRU max size, touches self-presence, and returns sorted by timestamp (newest first).

### `src/infra/system-events.ts` -- System Event Queue

Lightweight in-memory per-session event queue.

```typescript
type SystemEvent = { text: string; ts: number };

function enqueueSystemEvent(text, options: { sessionKey, contextKey? }): void;
function drainSystemEvents(sessionKey): string[];
function drainSystemEventEntries(sessionKey): SystemEvent[];
function peekSystemEvents(sessionKey): string[];
function hasSystemEvents(sessionKey): boolean;
function isSystemEventContextChanged(sessionKey, contextKey?): boolean;
```

**Constraints:** Max 20 events per session. Consecutive duplicates are deduplicated. Events are session-keyed and ephemeral (no persistence).

### `src/infra/diagnostic-events.ts` -- Diagnostic Event Bus

```typescript
type DiagnosticEventPayload =
  | DiagnosticUsageEvent
  | DiagnosticWebhookReceivedEvent
  | DiagnosticWebhookProcessedEvent
  | DiagnosticWebhookErrorEvent
  | DiagnosticMessageQueuedEvent
  | DiagnosticMessageProcessedEvent
  | DiagnosticSessionStateEvent
  | DiagnosticSessionStuckEvent
  | DiagnosticLaneEnqueueEvent
  | DiagnosticLaneDequeueEvent
  | DiagnosticRunAttemptEvent
  | DiagnosticHeartbeatEvent;

function emitDiagnosticEvent(event: DiagnosticEventInput): void;
function onDiagnosticEvent(listener): () => void;
function isDiagnosticsEnabled(config?): boolean;
```

Each event is enriched with `seq` (monotonically increasing) and `ts` (milliseconds).

### `src/infra/diagnostic-flags.ts` -- Diagnostic Feature Flags

```typescript
function resolveDiagnosticFlags(cfg?, env?): string[];
function matchesDiagnosticFlag(flag, enabledFlags): boolean;
function isDiagnosticFlagEnabled(flag, cfg?, env?): boolean;
```

Flags are resolved from both `config.diagnostics.flags[]` and `OPENCLAW_DIAGNOSTICS` env var. Supports glob patterns: `"*"`, `"webhook.*"`, `"session*"`.

### `src/infra/heartbeat-runner.ts` -- Heartbeat Execution

The heartbeat runner manages the agent's periodic self-initiated activity cycles. It:
- Resolves per-agent heartbeat configuration (interval, prompt, model override, active hours).
- Checks active hours constraints.
- Resolves delivery targets (channel, recipient).
- Invokes `getReplyFromConfig()` with the heartbeat prompt.
- Delivers outbound payloads via the outbound delivery system.
- Emits heartbeat events for monitoring.
- Respects `ackMaxChars` limits.

### `src/infra/unhandled-rejections.ts` -- Unhandled Rejection Handler

```typescript
function installUnhandledRejectionHandler(): void;
function registerUnhandledRejectionHandler(handler): () => void;
function isAbortError(err): boolean;
function isTransientNetworkError(err): boolean;
```

**Fatal error codes** (immediate exit): `ERR_OUT_OF_MEMORY`, `ERR_SCRIPT_EXECUTION_TIMEOUT`, `ERR_WORKER_OUT_OF_MEMORY`, `ERR_WORKER_UNCAUGHT_EXCEPTION`, `ERR_WORKER_INITIALIZATION_FAILED`.

**Config error codes** (immediate exit): `INVALID_CONFIG`, `MISSING_API_KEY`, `MISSING_CREDENTIALS`.

**Transient network codes** (logged, not fatal): `ECONNRESET`, `ECONNREFUSED`, `ENOTFOUND`, `ETIMEDOUT`, `ESOCKETTIMEDOUT`, `ECONNABORTED`, `EPIPE`, `EHOSTUNREACH`, `ENETUNREACH`, `EAI_AGAIN`, plus various `UND_ERR_*` codes from undici.

**AbortError:** Recognized by name `"AbortError"` or message `"This operation was aborted"`. Logged as warning, not fatal.

**Cause chain traversal:** Error codes are checked on the error itself, its `.cause`, and recursively through `AggregateError.errors`.

---

## 9. Auto-Reply System (`src/auto-reply/`)

### `src/auto-reply/templating.ts` -- MsgContext Type and Templating

**`MsgContext`** is the central message context type with 60+ fields:

```typescript
type MsgContext = {
  // Message body variants
  Body?: string;                     // Raw message body
  BodyForAgent?: string;             // Agent prompt body (with context/history)
  RawBody?: string;                  // Legacy alias for CommandBody
  CommandBody?: string;              // Prefer for command detection
  BodyForCommands?: string;          // Clean text for command parsing
  CommandArgs?: CommandArgs;

  // History
  InboundHistory?: Array<{ sender: string; body: string; timestamp?: number }>;

  // Sender information
  From?: string;
  To?: string;
  SenderName?: string;
  SenderId?: string;
  SenderUsername?: string;
  SenderTag?: string;
  SenderE164?: string;

  // Session context
  SessionKey?: string;
  AccountId?: string;
  ParentSessionKey?: string;

  // Message IDs
  MessageSid?: string;
  MessageSidFull?: string;
  MessageSids?: string[];
  MessageSidFirst?: string;
  MessageSidLast?: string;

  // Reply context
  ReplyToId?: string;
  ReplyToIdFull?: string;
  ReplyToBody?: string;
  ReplyToSender?: string;
  ReplyToIsQuote?: boolean;

  // Forwarded message
  ForwardedFrom?: string;
  ForwardedFromType?: string;
  ForwardedFromId?: string;
  ForwardedFromUsername?: string;
  ForwardedFromTitle?: string;
  ForwardedFromSignature?: string;
  ForwardedFromChatType?: string;
  ForwardedFromMessageId?: number;
  ForwardedDate?: number;

  // Thread context
  ThreadStarterBody?: string;
  ThreadHistoryBody?: string;
  IsFirstThreadTurn?: boolean;
  ThreadLabel?: string;

  // Media
  MediaPath?: string;
  MediaUrl?: string;
  MediaType?: string;
  MediaDir?: string;
  MediaPaths?: string[];
  MediaUrls?: string[];
  MediaTypes?: string[];
  Sticker?: StickerMetadata;
  OutputDir?: string;
  OutputBase?: string;
  MediaRemoteHost?: string;
  Transcript?: string;
  MediaUnderstanding?: MediaUnderstandingOutput[];
  MediaUnderstandingDecisions?: MediaUnderstandingDecision[];
  LinkUnderstanding?: string[];

  // Chat/group context
  Prompt?: string;
  MaxChars?: number;
  ChatType?: string;
  ConversationLabel?: string;
  GroupSubject?: string;
  GroupChannel?: string;
  GroupSpace?: string;
  GroupMembers?: string;
  GroupSystemPrompt?: string;
  UntrustedContext?: string[];
  OwnerAllowFrom?: Array<string | number>;

  // Routing
  Timestamp?: number;
  Provider?: string;
  Surface?: string;
  WasMentioned?: boolean;
  CommandAuthorized?: boolean;
  CommandSource?: "text" | "native";
  CommandTargetSessionKey?: string;
  GatewayClientScopes?: string[];
  MessageThreadId?: string | number;
  IsForum?: boolean;
  OriginatingChannel?: OriginatingChannelType;
  OriginatingTo?: string;
  HookMessages?: string[];
};
```

**`FinalizedMsgContext`**: Same as `MsgContext` but with `CommandAuthorized: boolean` (always set, default-deny false).

**`TemplateContext`**: Extends `MsgContext` with `BodyStripped?`, `SessionId?`, `IsNewSession?`.

**Template interpolation:**
```typescript
function applyTemplate(str: string | undefined, ctx: TemplateContext): string;
// Replaces {{Placeholder}} with ctx[Placeholder]
// Arrays joined with commas; objects return empty string
```

### `src/auto-reply/types.ts` -- Reply Types

```typescript
type ReplyPayload = {
  text?: string;
  mediaUrl?: string;
  mediaUrls?: string[];
  replyToId?: string;
  replyToTag?: boolean;
  replyToCurrent?: boolean;
  audioAsVoice?: boolean;
  isError?: boolean;
  channelData?: Record<string, unknown>;
};

type GetReplyOptions = {
  runId?: string;
  abortSignal?: AbortSignal;
  images?: ImageContent[];
  onAgentRunStart?: (runId: string) => void;
  onReplyStart?: () => Promise<void> | void;
  onTypingCleanup?: () => void;
  onTypingController?: (typing: TypingController) => void;
  isHeartbeat?: boolean;
  heartbeatModelOverride?: string;
  onPartialReply?: (payload: ReplyPayload) => Promise<void> | void;
  onReasoningStream?: (payload: ReplyPayload) => Promise<void> | void;
  onBlockReply?: (payload: ReplyPayload, context?: BlockReplyContext) => Promise<void> | void;
  onToolResult?: (payload: ReplyPayload) => Promise<void> | void;
  onModelSelected?: (ctx: ModelSelectedContext) => void;
  disableBlockStreaming?: boolean;
  blockReplyTimeoutMs?: number;
  skillFilter?: string[];
  hasRepliedRef?: { value: boolean };
};
```

### `src/auto-reply/dispatch.ts` -- Dispatch Logic

```typescript
async function dispatchInboundMessage(params: {
  ctx: MsgContext | FinalizedMsgContext;
  cfg: OpenClawConfig;
  dispatcher: ReplyDispatcher;
  replyOptions?: Omit<GetReplyOptions, "onToolResult" | "onBlockReply">;
  replyResolver?: typeof getReplyFromConfig;
}): Promise<DispatchInboundResult>;

async function dispatchInboundMessageWithBufferedDispatcher(params): Promise<DispatchInboundResult>;
async function dispatchInboundMessageWithDispatcher(params): Promise<DispatchInboundResult>;
```

**Dispatch flow:**
1. `finalizeInboundContext(ctx)` -- Sets `CommandAuthorized` to boolean (default false).
2. `withReplyDispatcher({ dispatcher, run, onSettled })` -- Ensures dispatcher reservations are always released, even on error.
3. `dispatchReplyFromConfig({ ctx, cfg, dispatcher, replyOptions })` -- Core dispatch from config.

### `src/auto-reply/reply.ts` -- Reply Barrel Exports

Re-exports from the `reply/` subdirectory:
- `extractElevatedDirective`, `extractReasoningDirective`, `extractThinkDirective`, `extractVerboseDirective` -- Directive extraction from message text.
- `getReplyFromConfig` -- The main reply generation function.
- `extractExecDirective` -- Shell execution directives.
- `extractQueueDirective` -- Queue management directives.
- `extractReplyToTag` -- Reply-to tag extraction.

---

## 10. Path Resolution

### `src/infra/home-dir.ts` -- Home Directory Resolution

```typescript
function resolveEffectiveHomeDir(env?, homedir?): string | undefined;
function resolveRequiredHomeDir(env?, homedir?): string;
function expandHomePrefix(input, opts?): string;
```

**Resolution order:**
1. `OPENCLAW_HOME` (supports `~` prefix expansion).
2. `HOME` environment variable.
3. `USERPROFILE` (Windows).
4. `os.homedir()` fallback.

### `src/infra/tmp-openclaw-dir.ts` -- Temporary Directory

```typescript
const POSIX_OPENCLAW_TMP_DIR = "/tmp/openclaw";

function resolvePreferredOpenClawTmpDir(options?): string;
```

**Resolution:**
1. Check if `/tmp/openclaw` exists, is a real directory (not symlink), is writable+executable, and owned by current user with no group/other write permissions.
2. If not, try to create it with mode `0o700`.
3. Fallback: `os.tmpdir()/openclaw-<uid>` (or `openclaw` if uid unavailable).

### `src/infra/openclaw-root.ts` -- Package Root

```typescript
async function resolveOpenClawPackageRoot(opts: { cwd?, argv1?, moduleUrl? }): Promise<string | null>;
function resolveOpenClawPackageRootSync(opts): string | null;
```

Walks up from candidate directories searching for a `package.json` with `name: "openclaw"`. Candidate sources: `import.meta.url`, `process.argv[1]` (including symlink resolution for version managers), and CWD.

### `src/infra/state-migrations.ts` -- Legacy State Migration

**Types:**
```typescript
type LegacyStateDetection = {
  targetAgentId: string;
  targetMainKey: string;
  targetScope?: SessionScope;
  stateDir: string;
  oauthDir: string;
  sessions: { legacyDir, legacyStorePath, targetDir, targetStorePath, hasLegacy, legacyKeys };
  agentDir: { legacyDir, targetDir, hasLegacy };
  whatsappAuth: { legacyDir, targetDir, hasLegacy };
  preview: string[];
};
```

**Functions:**
```typescript
async function autoMigrateLegacyStateDir(params): Promise<StateDirMigrationResult>;
async function autoMigrateLegacyState(params): Promise<{ migrated, skipped, changes, warnings }>;
async function detectLegacyStateMigrations(params): Promise<LegacyStateDetection>;
async function runLegacyStateMigrations(params): Promise<{ changes, warnings }>;
```

**State directory migration:** Moves from legacy paths (e.g., `~/.pi-ai/`) to `~/.openclaw/`, creating a symlink at the old location. Handles Windows junctions, symlink chains (max depth 2), and rollback on failure.

**Session key canonicalization:** Migrates legacy session keys to the `agent:<agentId>:<channel>:<kind>:<id>` format. Merges session entries preferring the most recently updated.

**Agent directory migration:** Moves `<stateDir>/agent/` to `<stateDir>/agents/<agentId>/agent/`.

**WhatsApp auth migration:** Moves legacy auth files from `<oauthDir>/` to `<oauthDir>/whatsapp/default/`.

---

## 11. Infrastructure Utilities (`src/infra/`)

### Environment and Configuration

#### `env.ts` -- Environment Variable Handling

```typescript
type AcceptedEnvOption = { key: string; description: string; value?: string; redact?: boolean };

function logAcceptedEnvOption(option: AcceptedEnvOption): void;
function normalizeZaiEnv(): void;
function isTruthyEnvValue(value?: string): boolean;
function normalizeEnv(): void;
```

Logs each accepted env var once (deduped by key). Values >160 chars are truncated. Secrets are redacted.

#### `dotenv.ts` -- `.env` File Loading

```typescript
function loadDotEnv(opts?: { quiet?: boolean }): void;
```

Loads from CWD first (dotenv default), then from `<configDir>/.env` as a global fallback (without overriding existing vars).

#### `env-file.ts` -- `.env` File Mutation

```typescript
function upsertSharedEnvVar(params: { key, value, env? }): { path, updated, created };
```

Reads the shared `.env` file, replaces existing entries (preserving `export` prefix), or appends new ones. Writes with `0o600` permissions.

### File System

#### `fs-safe.ts` -- Safe File Opening

```typescript
class SafeOpenError extends Error {
  code: "invalid-path" | "not-found";
}

type SafeOpenResult = { handle: FileHandle; realPath: string; stat: Stats };

async function openFileWithinRoot(params: { rootDir, relativePath }): Promise<SafeOpenResult>;
```

Opens files within a root directory with symlink protection:
- Resolves `rootDir` to real path.
- Checks resolved path stays within root.
- Opens with `O_RDONLY | O_NOFOLLOW` (POSIX only).
- Verifies lstat is not a symlink, realpath stays within root, is a regular file, and inode/device match.

#### `json-file.ts` -- JSON File I/O

```typescript
function loadJsonFile(pathname: string): unknown;
function saveJsonFile(pathname: string, data: unknown): void;
```

`saveJsonFile` creates directories with `0o700`, writes prettified JSON, and sets `0o600` permissions.

#### `file-lock.ts` -- File-Based Locking

```typescript
type FileLockOptions = {
  retries: { retries: number; factor: number; minTimeout: number; maxTimeout: number; randomize?: boolean };
  stale: number;
};

async function acquireFileLock(filePath, options): Promise<FileLockHandle>;
async function withFileLock<T>(filePath, options, fn): Promise<T>;
```

**Implementation:**
- Lock file: `<filePath>.lock` opened with `wx` (exclusive create).
- Lock payload: `{ pid, createdAt }` for stale detection.
- Stale detection: PID liveness check via `process.kill(pid, 0)`, plus age check.
- Reentrant: Same-process acquisitions increment a counter instead of blocking.
- Uses global `Symbol.for("openclaw.fileLockHeldLocks")` for cross-module state.

#### `gateway-lock.ts` -- Gateway Instance Lock

```typescript
class GatewayLockError extends Error { cause?: unknown }

type GatewayLockHandle = { lockPath: string; configPath: string; release: () => Promise<void> };

async function acquireGatewayLock(opts?): Promise<GatewayLockHandle | null>;
```

Ensures only one gateway instance per config path. Lock file: `<lockDir>/gateway.<sha1-8chars>.lock`.

**Linux-specific PID validation:** Reads `/proc/<pid>/cmdline` to verify the lock holder is actually a gateway process (not a recycled PID). Also reads `/proc/<pid>/stat` field 22 (start time) for precise process identity.

**Bypass:** Returns `null` when `OPENCLAW_ALLOW_MULTI_GATEWAY=1` or in test environments.

### Networking

#### `fetch.ts` -- Fetch Wrapper

```typescript
function wrapFetchWithAbortSignal(fetchImpl): typeof fetch;
function resolveFetch(fetchImpl?): typeof fetch | undefined;
```

Wraps fetch to handle cross-realm `AbortSignal` objects (e.g., from different Node.js contexts) by creating a local `AbortController` and relaying abort events. Also sets `duplex: "half"` for request bodies (required by Node's undici).

#### `bonjour.ts` / `bonjour-discovery.ts` / `bonjour-ciao.ts` / `bonjour-errors.ts`

mDNS/Bonjour service discovery for local network OpenClaw instances using the Ciao library.

#### `ssh-config.ts` / `ssh-tunnel.ts`

SSH configuration parsing and tunnel management for remote media access.

#### `tailscale.ts` / `tailnet.ts` / `widearea-dns.ts`

Tailscale integration for wide-area networking, VPN device discovery, and DNS-based service discovery.

#### `ports.ts` / `ports-inspect.ts` / `ports-lsof.ts` / `ports-format.ts` / `ports-types.ts`

Port availability checking, inspection (`lsof`-based), and formatting utilities.

### Concurrency Primitives

#### `dedupe.ts` -- Deduplication Cache

```typescript
type DedupeCache = {
  check: (key: string | undefined | null, now?: number) => boolean;
  clear: () => void;
  size: () => number;
};

function createDedupeCache(options: { ttlMs: number; maxSize: number }): DedupeCache;
```

LRU cache with TTL. `check()` returns `true` if the key was already seen (and refreshes it), `false` if new.

#### `retry.ts` -- Retry with Backoff

```typescript
type RetryOptions = {
  attempts?: number;
  minDelayMs?: number;
  maxDelayMs?: number;
  jitter?: number;           // 0-1
  label?: string;
  shouldRetry?: (err, attempt) => boolean;
  retryAfterMs?: (err) => number | undefined;
  onRetry?: (info: RetryInfo) => void;
};

async function retryAsync<T>(fn, attemptsOrOptions?, initialDelayMs?): Promise<T>;
function resolveRetryConfig(defaults?, overrides?): Required<RetryConfig>;
```

Supports both simple (`retryAsync(fn, 3, 300)`) and options-based invocation. Exponential backoff with configurable jitter and a `retryAfterMs` hook for server-specified retry-after headers.

#### `backoff.ts` -- Exponential Backoff

```typescript
type BackoffPolicy = { initialMs: number; maxMs: number; factor: number; jitter: number };

function computeBackoff(policy, attempt): number;
async function sleepWithAbort(ms, abortSignal?): Promise<void>;
```

### Error Handling

#### `errors.ts` -- Error Utilities

```typescript
function extractErrorCode(err: unknown): string | undefined;
function isErrno(err): err is NodeJS.ErrnoException;
function hasErrnoCode(err, code): boolean;
function formatErrorMessage(err: unknown): string;
function formatUncaughtError(err: unknown): string;
```

`formatUncaughtError` includes the stack trace for Error instances, except for `INVALID_CONFIG` errors which show only the message.

### Crypto and Security

#### `device-identity.ts` -- (See Section 6)

Ed25519 keypair generation, signing, and verification.

#### `device-auth-store.ts` -- Device Auth Token Store

Token storage for paired devices, integrated with the device pairing system.

### Platform and Runtime

#### `runtime-guard.ts` -- Node.js Version Guard

```typescript
const MIN_NODE: Semver = { major: 22, minor: 12, patch: 0 };

function detectRuntime(): RuntimeDetails;
function runtimeSatisfies(details): boolean;
function assertSupportedRuntime(runtime?, details?): void;
function parseSemver(version): Semver | null;
function isAtLeast(version, minimum): boolean;
function isSupportedNodeVersion(version): boolean;
```

`assertSupportedRuntime()` prints an error message and calls `exit(1)` if the Node version is below 22.12.0.

#### `wsl.ts` -- WSL Detection

WSL (Windows Subsystem for Linux) environment detection.

#### `machine-name.ts` / `os-summary.ts`

Machine name resolution and OS environment summary generation.

#### `shell-env.ts` -- Shell Environment

Shell environment variable resolution for service contexts.

#### `path-env.ts` -- PATH Management

PATH environment variable manipulation utilities.

### Restart and Recovery

#### `restart-sentinel.ts` -- Restart State Persistence

```typescript
type RestartSentinelPayload = {
  kind: "config-apply" | "config-patch" | "update" | "restart";
  status: "ok" | "error" | "skipped";
  ts: number;
  sessionKey?: string;
  deliveryContext?: { channel?, to?, accountId? };
  threadId?: string;
  message?: string | null;
  doctorHint?: string | null;
  stats?: RestartSentinelStats | null;
};

function resolveRestartSentinelPath(env?): string;
async function writeRestartSentinel(payload, env?): Promise<string>;
async function readRestartSentinel(env?): Promise<RestartSentinel | null>;
async function consumeRestartSentinel(env?): Promise<RestartSentinel | null>;
function formatRestartSentinelMessage(payload): string;
```

Persists restart state to `<stateDir>/restart-sentinel.json` so the gateway can report the outcome after restart. `consumeRestartSentinel` reads and deletes the file atomically.

#### `restart.ts` -- Restart Utilities

General restart helper functions.

### Update System

#### `update-check.ts` -- Update Status Checking

```typescript
type UpdateCheckResult = {
  root: string | null;
  installKind: "git" | "package" | "unknown";
  packageManager: "pnpm" | "bun" | "npm" | "unknown";
  git?: GitUpdateStatus;
  deps?: DepsStatus;
  registry?: RegistryStatus;
};

async function checkUpdateStatus(params): Promise<UpdateCheckResult>;
async function checkGitUpdateStatus(params): Promise<GitUpdateStatus>;
async function checkDepsStatus(params): Promise<DepsStatus>;
async function fetchNpmLatestVersion(params?): Promise<RegistryStatus>;
async function fetchNpmTagVersion(params): Promise<NpmTagStatus>;
function compareSemverStrings(a, b): number | null;
```

Git update checking: branch, SHA, tag, upstream, dirty status, ahead/behind counts, optional fetch.
Dependencies: lockfile vs install marker mtime comparison for staleness detection.
Registry: fetches version from `https://registry.npmjs.org/openclaw/<tag>`.

#### `update-runner.ts` -- Update Execution

```typescript
type UpdateRunResult = {
  status: "ok" | "error" | "skipped";
  mode: "git" | "pnpm" | "bun" | "npm" | "unknown";
  root?: string;
  reason?: string;
  before?: { sha?, version? };
  after?: { sha?, version? };
  steps: UpdateStepResult[];
  durationMs: number;
};
```

Executes multi-step update sequences (git pull + install + build, or npm global install).

#### `update-channels.ts` -- Update Channel Management

Manages update channels (stable, beta) and maps them to npm tags.

#### `update-startup.ts` / `update-global.ts`

Startup update checks and global npm package update execution.

### Observability

#### `agent-events.ts` -- Agent Event Streaming

Real-time agent event system with per-run sequence numbers and lifecycle tracking.

#### `channel-activity.ts` / `channel-summary.ts` / `channels-status-issues.ts`

Per-channel activity metrics, summary generation, and issue tracking (disconnects, auth failures).

#### `heartbeat-events.ts` / `heartbeat-active-hours.ts` / `heartbeat-visibility.ts` / `heartbeat-wake.ts` / `heartbeat-events-filter.ts`

Heartbeat event emission, active hours enforcement, visibility control, wake triggers, and cron/exec event filtering.

### Service and Process

#### `system-run-command.ts` -- System Command Execution

System-level command execution with logging.

#### `exec-approvals.ts` / `exec-approvals-allowlist.ts` / `exec-approvals-analysis.ts` / `exec-approval-forwarder.ts`

Shell command approval system: allowlist management, safety analysis, and approval forwarding.

#### `exec-host.ts` / `exec-safety.ts`

Execution host abstraction and safety checking.

### Networking and Discovery

#### `bonjour.ts` / `bonjour-discovery.ts` / `bonjour-ciao.ts` / `bonjour-errors.ts`

mDNS service discovery for LAN-based OpenClaw instances.

#### `transport-ready.ts`

Transport readiness checking for channel connections.

### External Tool Integration

#### `binaries.ts` -- Binary Availability

Checks if external binaries (ffmpeg, whisper-cli, etc.) exist on the system PATH.

#### `brew.ts` -- Homebrew Integration

Homebrew package manager detection and dependency checking.

#### `clipboard.ts` -- Clipboard Access

System clipboard read/write operations using platform-specific tools.

#### `archive.ts` -- Archive Handling

Compression and extraction for zip/tar archives.

#### `voicewake.ts` -- Voice Wake Detection

Voice activation detection integration.

### Formatting and Parsing

#### `format-time/` -- Time Formatting

- `format-datetime.ts` -- Date/time formatting
- `format-duration.ts` -- Duration formatting (e.g., "2m 30s")
- `format-relative.ts` -- Relative time formatting (e.g., "5 minutes ago")

#### `http-body.ts` -- HTTP Body Parsing

HTTP request body parsing and content-type handling.

#### `infra-parsing.test.ts` / `infra-runtime.test.ts` / `infra-store.test.ts`

Test files for parsing, runtime, and store modules.

### Provider Usage Tracking

#### `provider-usage.ts` and related modules

A comprehensive provider usage tracking system:

| Module | Purpose |
|---|---|
| `provider-usage.ts` | Main orchestration |
| `provider-usage.types.ts` | Type definitions |
| `provider-usage.shared.ts` | Shared utilities |
| `provider-usage.load.ts` | Usage data loading |
| `provider-usage.format.ts` | Usage formatting |
| `provider-usage.auth.ts` | Auth key normalization |
| `provider-usage.fetch.ts` | Usage fetching orchestration |
| `provider-usage.fetch.claude.ts` | Claude usage fetching |
| `provider-usage.fetch.codex.ts` | Codex usage fetching |
| `provider-usage.fetch.copilot.ts` | Copilot usage fetching |
| `provider-usage.fetch.gemini.ts` | Gemini usage fetching |
| `provider-usage.fetch.minimax.ts` | MiniMax usage fetching |
| `provider-usage.fetch.zai.ts` | Zai usage fetching |
| `provider-usage.fetch.antigravity.ts` | Antigravity usage fetching |
| `provider-usage.fetch.shared.ts` | Shared fetch utilities |
| `session-cost-usage.ts` | Per-session cost tracking |
| `session-cost-usage.types.ts` | Session cost types |

### Miscellaneous Infrastructure

| Module | Purpose |
|---|---|
| `control-ui-assets.ts` | Serves static assets for the browser control UI |
| `canvas-host-url.ts` | Canvas host URL resolution |
| `git-commit.ts` | Git commit information extraction |
| `install-package-dir.ts` | Package installation directory resolution |
| `is-main.ts` | Main module detection (`if __name__ == "__main__"` equivalent) |
| `node-pairing.ts` | Node host pairing utilities |
| `node-shell.ts` | Node shell execution utilities |
| `npm-registry-spec.ts` | NPM registry interaction for plugin installation |
| `pairing-files.ts` | File I/O helpers for pairing state |
| `retry-policy.ts` | Retry policy definitions |
| `run-node.test.ts` | Node runner tests |
| `session-maintenance-warning.ts` | Session maintenance warning display |
| `skills-remote.ts` | Remote skills loading |
| `tls/` | TLS certificate management |
| `ws.ts` | WebSocket utilities |

---

## 12. Environment Management

### Loading Order

1. **Bootstrap** (`openclaw.mjs`): Module compile cache enabled.
2. **Entry** (`src/entry.ts`): `normalizeEnv()` called (normalizes `Z_AI_API_KEY`).
3. **CLI startup**: `loadDotEnv()` called from gateway/CLI initialization.
4. **Config loading**: Config file parsed, logging config extracted.

### `loadDotEnv()` (from `src/infra/dotenv.ts`)

```
1. dotenv.config({ quiet })           -- CWD/.env
2. dotenv.config({ path: globalEnvPath, override: false })  -- ~/.openclaw/.env
```

Global fallback never overrides existing environment variables.

### `upsertSharedEnvVar()` (from `src/infra/env-file.ts`)

Writes to `<configDir>/.env`, preserving existing entries and `export` prefixes. Used by the wizard and CLI commands to persist API keys and configuration.

### `normalizeEnv()` (from `src/infra/env.ts`)

Currently only normalizes `Z_AI_API_KEY` to `ZAI_API_KEY` (vendor alias consolidation).

---

## 13. Update System

### Version Resolution (`src/version.ts`)

```typescript
const VERSION =
  __OPENCLAW_VERSION__              // Build-time injection
  || process.env.OPENCLAW_BUNDLED_VERSION  // Bundled builds
  || resolveVersionFromModuleUrl(import.meta.url)  // package.json or build-info.json
  || "0.0.0";
```

Searches up to 3 parent directories for `package.json` (with name validation) or `build-info.json`.

### Update Channels (`src/infra/update-channels.ts`)

Maps update channels to npm tags. Default channel is `"stable"` (maps to `"latest"` npm tag). Beta channel maps to `"beta"` tag.

### Check Flow

1. Detect install kind (git vs. npm package).
2. For git: check branch, SHA, dirty status, ahead/behind counts, optional fetch.
3. For npm: check lockfile vs install marker staleness.
4. Optionally fetch latest version from npm registry.
5. Compare versions with semver comparison.

### Update Execution

For git installs: `git pull` + dependency install + build.
For npm: global install with the appropriate package manager.
Results are tracked in `UpdateRunResult` with per-step timing and log tails.

---

## 14. Maintenance Timers

### Diagnostic Heartbeat

- **Interval:** 30 seconds.
- **Purpose:** Logs active/waiting sessions, webhook stats, queue depth.
- **Auto-suppression:** Skips when idle >120s with no activity.
- **Stuck detection:** Warns on sessions in "processing" state for >120s.
- **Timer:** `unref()`'d to not prevent process exit.

### Log Rotation

- **Scheme:** Daily rolling files (`openclaw-YYYY-MM-DD.log`).
- **Pruning:** Files older than 24 hours are deleted on logger initialization.
- **Location:** `resolvePreferredOpenClawTmpDir()` (typically `/tmp/openclaw/`).

### Cron Session Reaper

- **Module:** `src/cron/session-reaper.ts`.
- **Purpose:** Cleans up stale isolated cron sessions based on retention configuration.

### System Presence TTL

- **TTL:** 5 minutes.
- **Max entries:** 200 (LRU eviction).
- **Pruning:** On every `listSystemPresence()` call.

---

## 15. Key Types and Interfaces

### Core Types Summary

| Type | Module | Purpose |
|---|---|---|
| `LogLevel` | `logging/levels.ts` | `"silent" \| "fatal" \| "error" \| "warn" \| "info" \| "debug" \| "trace"` |
| `SubsystemLogger` | `logging/subsystem.ts` | Per-module logging facade |
| `ConsoleStyle` | `logging/console.ts` | `"pretty" \| "compact" \| "json"` |
| `CommandLane` | `process/lanes.ts` | `Main \| Cron \| Subagent \| Nested` |
| `SpawnResult` | `process/exec.ts` | `{ stdout, stderr, code, signal, killed }` |
| `GatewayService` | `daemon/service.ts` | Platform-agnostic service management |
| `GatewayServiceRuntime` | `daemon/service-runtime.ts` | `{ status, state, pid, ... }` |
| `DeviceIdentity` | `infra/device-identity.ts` | `{ deviceId, publicKeyPem, privateKeyPem }` |
| `PairedDevice` | `infra/device-pairing.ts` | Paired device with tokens |
| `DeviceAuthToken` | `infra/device-pairing.ts` | Per-role auth token |
| `PairingRequest` | `pairing/pairing-store.ts` | Channel pairing request with code |
| `CronJob` | `cron/types.ts` | Scheduled job definition |
| `CronSchedule` | `cron/types.ts` | `at \| every \| cron` schedule |
| `CronPayload` | `cron/types.ts` | `systemEvent \| agentTurn` |
| `CronServiceDeps` | `cron/service/state.ts` | Dependency injection for cron |
| `CronEvent` | `cron/service/state.ts` | Job lifecycle event |
| `MsgContext` | `auto-reply/templating.ts` | Inbound message context (60+ fields) |
| `FinalizedMsgContext` | `auto-reply/templating.ts` | MsgContext with boolean CommandAuthorized |
| `ReplyPayload` | `auto-reply/types.ts` | Outbound reply |
| `GetReplyOptions` | `auto-reply/types.ts` | Reply generation options |
| `SystemPresence` | `infra/system-presence.ts` | Network presence entry |
| `SystemEvent` | `infra/system-events.ts` | `{ text, ts }` |
| `DiagnosticEventPayload` | `infra/diagnostic-events.ts` | Union of 12 event types |
| `ProcessWarning` | `infra/warning-filter.ts` | `{ code?, name?, message? }` |
| `RestartSentinelPayload` | `infra/restart-sentinel.ts` | Restart state persistence |
| `BackoffPolicy` | `infra/backoff.ts` | `{ initialMs, maxMs, factor, jitter }` |
| `RetryOptions` | `infra/retry.ts` | `{ attempts, minDelayMs, maxDelayMs, jitter, ... }` |
| `DedupeCache` | `infra/dedupe.ts` | TTL + size bounded cache |
| `FileLockOptions` | `infra/file-lock.ts` | Retry and stale parameters |
| `GatewayLockHandle` | `infra/gateway-lock.ts` | Lock with config path |
| `SafeOpenResult` | `infra/fs-safe.ts` | `{ handle, realPath, stat }` |
| `UpdateCheckResult` | `infra/update-check.ts` | Full update status |

### Interface Contracts

**RuntimeEnv** (from `src/runtime.ts`):
```typescript
type RuntimeEnv = {
  log: (message: string) => void;
  error: (message: string) => void;
  exit: (code: number) => void;
};
```

Used throughout for dependency injection of console output and process exit behavior, enabling testability.

**Logger (cron service)**:
```typescript
type Logger = {
  debug: (obj: unknown, msg?: string) => void;
  info: (obj: unknown, msg?: string) => void;
  warn: (obj: unknown, msg?: string) => void;
  error: (obj: unknown, msg?: string) => void;
};
```

**PinoLikeLogger** (Baileys adapter):
```typescript
type PinoLikeLogger = {
  level: string;
  child: (bindings?) => PinoLikeLogger;
  trace: (...args) => void;
  debug: (...args) => void;
  info:  (...args) => void;
  warn:  (...args) => void;
  error: (...args) => void;
  fatal: (...args) => void;
};
```
