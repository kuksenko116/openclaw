# CLI Framework and Commands

## Overview

The OpenClaw CLI is built on [Commander.js](https://github.com/tj/commander.js) and provides the primary interface for managing the gateway, channels, models, plugins, agents, cron jobs, devices, and more. The CLI lives in `src/cli/` with command implementations in `src/commands/`. It uses lazy command registration to minimize startup time and a dependency injection pattern for testability.

---

## CLI Framework (`src/cli/`)

### `run-main.ts` -- CLI Entry Point (129 lines)

The top-level entry point for the CLI. `runCli(argv)` orchestrates the entire startup:

1. **Normalize Windows arguments** -- `normalizeWindowsArgv(argv)` handles Windows-specific quoting issues.
2. **Load dotenv** -- `loadDotEnv({ quiet: true })` loads `.env` files.
3. **Normalize environment** -- `normalizeEnv()` ensures consistent env var formatting.
4. **Ensure CLI on PATH** -- `ensureOpenClawCliOnPath()` (skipped for read-only commands like `status`, `health`, `config get`).
5. **Assert supported runtime** -- `assertSupportedRuntime()` verifies Node.js >= 22.12.0.
6. **Try route CLI** -- `tryRouteCli(normalizedArgv)` checks for fast-path routing (some commands bypass Commander entirely).
7. **Enable console capture** -- `enableConsoleCapture()` routes console output to structured logs.
8. **Build program** -- `buildProgram()` creates the Commander program with all registrations.
9. **Register primary command** -- Lazy registration: only the command matching `argv` is fully loaded.
10. **Register plugin commands** -- Plugin-provided CLI commands are registered unless skipped (builtin command or help/version flag).
11. **Parse and execute** -- `program.parseAsync(parseArgv)`.

Helper functions:
- `rewriteUpdateFlagArgv()` -- rewrites `--update` flag to the `update` subcommand.
- `shouldSkipPluginCommandRegistration()` -- skips plugin registration when a builtin command is being invoked.
- `shouldEnsureCliPath()` -- determines whether to ensure the CLI binary is on `$PATH`.

### `program/build-program.ts` -- Commander Program Builder (20 lines)

Creates and configures the Commander `Command` instance:

```typescript
function buildProgram() {
  const program = new Command();
  const ctx = createProgramContext();
  setProgramContext(program, ctx);
  configureProgramHelp(program, ctx);
  registerPreActionHooks(program, ctx.programVersion);
  registerProgramCommands(program, ctx, argv);
  return program;
}
```

### `program/context.ts` -- Program Context (19 lines)

Creates a `ProgramContext` containing:

```typescript
type ProgramContext = {
  programVersion: string;       // from VERSION constant
  channelOptions: string[];     // resolved channel option strings
  messageChannelOptions: string; // pipe-delimited channel list for message commands
  agentChannelOptions: string;   // "last|channel1|channel2|..." for agent commands
};
```

Channel options are resolved dynamically from the channel registry.

### `program/command-registry.ts` -- Command Registration (100+ lines)

Manages lazy registration of core CLI commands. Defines `CoreCliEntry` objects, each with:
- `commands` -- array of `{ name, description }` for each command the entry provides.
- `register` -- async function that dynamically imports and registers the command.

Core entries include:

| Entry | Commands |
|-------|----------|
| Setup | `setup` |
| Onboard | `onboard` |
| Configure | `configure` |
| Config | `config` |
| Maintenance | `doctor`, `dashboard`, `reset`, `uninstall` |
| Message | `message` |
| Memory | `memory` |
| Agent | `agent`, `agents` |
| Status/Health/Sessions | `status`, `health`, `sessions` |

The `shouldRegisterCorePrimaryOnly()` function enables lazy loading: when not showing help/version, only the entry matching the primary command is loaded. `registerCoreCliByName()` is called from `run-main.ts` to register just the needed entry.

### `program/register.subclis.ts` -- Sub-CLI Registration (200+ lines)

Manages lazy registration of sub-CLI command groups. Each `SubCliEntry` has a `name`, `description`, and async `register` function. Sub-CLIs are loaded on demand:

| Sub-CLI | Description |
|---------|-------------|
| `acp` | Agent Control Protocol tools |
| `gateway` | Gateway control (start, stop, dev) |
| `daemon` | Gateway service (legacy alias for systemd management) |
| `logs` | Gateway log viewing |
| `system` | System events, heartbeat, presence |
| `models` | Model configuration and discovery |
| `approvals` | Execution approval management |
| `nodes` | Node host commands (camera, canvas, screen, etc.) |
| `devices` | Device pairing and token management |
| `node` | Node control (daemon, registration) |
| `sandbox` | Sandbox tools |
| `tui` | Terminal UI mode |
| `cron` | Cron scheduler management |
| `dns` | DNS helper utilities |
| `docs` | Documentation helpers |
| `hooks` | Hooks tooling |
| `webhooks` | Webhook helpers |
| `pairing` | Device pairing helpers |
| `plugins` | Plugin management (also registers plugin-provided CLI commands) |
| `channels` | Channel management |
| `browser` | Browser automation CLI |
| `security` | Security settings |
| `skills` | Skills management |
| `completion` | Shell completion generation |
| `update` | Self-update management |
| `directory` | Directory navigation |

Lazy loading can be disabled with `OPENCLAW_DISABLE_LAZY_SUBCOMMANDS=1` for debugging.

### `deps.ts` -- Dependency Injection (59 lines)

Provides `CliDeps` and `createDefaultDeps()` for lazy-loading channel send functions:

```typescript
type CliDeps = {
  sendMessageWhatsApp: typeof sendMessageWhatsApp;
  sendMessageTelegram: typeof sendMessageTelegram;
  sendMessageDiscord: typeof sendMessageDiscord;
  sendMessageSlack: typeof sendMessageSlack;
  sendMessageSignal: typeof sendMessageSignal;
  sendMessageIMessage: typeof sendMessageIMessage;
};
```

Each function is a lazy proxy that dynamically imports the actual implementation on first call. `createOutboundSendDeps()` converts `CliDeps` to the `OutboundSendDeps` interface used by the delivery system.

### `command-options.ts` -- Option Detection (8 lines)

`hasExplicitOptions(command, names)` -- checks whether any of the named options were explicitly provided on the command line (via `getOptionValueSource("cli")`), as opposed to being defaults.

### `argv.ts` -- Argument Parsing (60+ lines)

Low-level argument parsing utilities:

- `hasHelpOrVersion(argv)` -- detects `-h`, `--help`, `-v`, `-V`, `--version` flags.
- `hasFlag(argv, name)` -- checks for a specific flag before the `--` terminator.
- `getFlagValue(argv, name)` -- extracts the value of a `--flag value` or `--flag=value` argument.
- `getPrimaryCommand(argv)` -- extracts the first positional argument (the command name).
- `getCommandPath(argv, depth)` -- extracts up to `depth` positional command segments.
- `buildParseArgv(argv)` -- constructs the argv array for Commander's `parseAsync()`.

### `banner.ts` -- CLI Banners/Branding (40+ lines)

Renders the OpenClaw startup banner with version info, commit hash, and a tagline. Features:
- Grapheme-aware string splitting (via `Intl.Segmenter`) for correct width calculation.
- Suppresses banner when `--json` or `--version` flags are present.
- Configurable via `BannerOptions` (argv, commit, columns, richTty).

### `progress.ts` -- Progress Bar Rendering (50+ lines)

Provides `createCliProgress(options)` returning a `ProgressReporter`:

```typescript
type ProgressReporter = {
  setLabel: (label: string) => void;
  setPercent: (percent: number) => void;
  tick: (delta?: number) => void;
  done: () => void;
};
```

Supports multiple backends:
- **OSC progress** -- terminal escape sequence progress (via `osc-progress` package).
- **Spinner** -- fallback via `@clack/prompts` spinner.
- **Line** / **Log** / **None** -- additional fallback modes.

Only one progress instance is active at a time (guarded by `activeProgress` counter).

### `prompt.ts` -- Interactive Prompts (21 lines)

`promptYesNo(question, defaultYes)` -- simple Y/N interactive prompt using `readline`. Honors the global `--yes` flag (auto-accepts).

### `ports.ts` -- Port Utilities (40+ lines)

Port management for detecting and freeing occupied ports:

- `listPortListeners(port)` -- uses `lsof` to find processes listening on a TCP port.
- `parseLsofOutput(output)` -- parses `lsof -FpFc` output into `PortProcess` objects (pid + command).
- `ForceFreePortResult` -- result type for force-freeing ports (killed processes, wait time, whether SIGKILL was needed).

### `profile.ts` -- CLI Profiles (40+ lines)

Parses `--profile <name>` arguments from the CLI argv, allowing multiple named configuration profiles. `parseCliProfileArgs(argv)` extracts the profile name and returns the remaining argv.

### `respawn-policy.ts` -- Process Respawning (5 lines)

`shouldSkipRespawnForArgv(argv)` -- returns true if help or version flags are present, preventing unnecessary respawning.

### `windows-argv.ts` -- Windows Argument Normalization

Handles Windows-specific argument parsing quirks (quoting, escaping differences between cmd.exe and PowerShell).

---

## CLI Commands (`src/cli/` Subdirectories)

### `gateway-cli/` -- Gateway Start/Stop/Status

Located in `src/cli/gateway-cli/`, contains:

| File | Purpose |
|------|---------|
| `register.ts` | Registers the `gateway` command group with Commander |
| `run.ts` | Gateway start execution |
| `run-loop.ts` | Gateway run loop with restart logic |
| `dev.ts` | Development mode gateway with hot reload |
| `call.ts` | Gateway RPC call utilities |
| `discover.ts` | Gateway instance discovery |
| `shared.ts` | Shared gateway CLI helpers |

Commands: `gateway start`, `gateway stop`, `gateway status`, `gateway dev`, `gateway call`, `gateway discover`.

### `daemon-cli.ts` + `daemon-cli/` -- Daemon Management

Barrel export from `src/cli/daemon-cli/`:

```typescript
export { registerDaemonCli } from "./daemon-cli/register.js";
export {
  runDaemonInstall, runDaemonRestart, runDaemonStart,
  runDaemonStatus, runDaemonStop, runDaemonUninstall,
} from "./daemon-cli/runners.js";
```

Internal structure (`src/cli/daemon-cli/`):

| File | Purpose |
|------|---------|
| `register.ts` | Commander registration |
| `runners.ts` | Command implementations |
| `install.ts` | systemd/launchd service installation |
| `lifecycle-core.ts` | Core lifecycle operations |
| `lifecycle.ts` | Start/stop/restart orchestration |
| `probe.ts` | Health probes for the daemon |
| `status.gather.ts` | Status data collection |
| `status.print.ts` | Status output formatting |
| `status.ts` | Status command implementation |
| `shared.ts` | Shared utilities |
| `types.ts` | Type definitions (DaemonInstallOptions, DaemonStatusOptions, GatewayRpcOpts) |
| `response.ts` | Response formatting |

Commands: `daemon install`, `daemon uninstall`, `daemon start`, `daemon stop`, `daemon status`, `daemon restart`.

### `node-cli/` -- Node Host Management

Located in `src/cli/node-cli/`:

| File | Purpose |
|------|---------|
| `register.ts` | Commander registration |
| `daemon.ts` | Node daemon management |

### `nodes-cli/` -- Nodes Commands (Extended)

Located in `src/cli/nodes-cli/`:

| File | Purpose |
|------|---------|
| `register.ts` | Top-level registration |
| `register.camera.ts` | Camera node commands |
| `register.canvas.ts` | Canvas node commands |
| `register.invoke.ts` | Node invoke commands |
| `register.location.ts` | Location node commands |
| `register.notify.ts` | Notification node commands |
| `register.pairing.ts` | Node pairing commands |
| `register.screen.ts` | Screen capture commands |
| `register.status.ts` | Node status commands |
| `rpc.ts` | Node RPC utilities |
| `types.ts` | Type definitions |
| `cli-utils.ts` | Shared CLI utilities |
| `format.ts` | Output formatting |

### `cron-cli/` -- Cron Job Management

Located in `src/cli/cron-cli/`:

| File | Purpose |
|------|---------|
| `register.ts` | Top-level registration |
| `register.cron-add.ts` | `cron add` command |
| `register.cron-edit.ts` | `cron edit` command |
| `register.cron-simple.ts` | `cron list`, `cron remove`, `cron run` |
| `shared.ts` | Shared utilities |

Commands: `cron list`, `cron add`, `cron remove`, `cron run`, `cron edit`.

### `models-cli.ts` -- Model Listing and Management (80+ lines)

Registers the `models` command group with subcommands:

- `models list` -- list models (configured by default, `--all` for full catalog, `--local` for local only, `--provider` filter).
- `models status` -- show configured model state (`--check` exits non-zero on auth issues, `--probe` tests live auth).
- `models set` -- set the primary model.
- `models set-image` -- set the image model.
- `models scan` -- scan for available models.
- `models aliases list|add|remove` -- manage model aliases.
- `models auth add|login|setup-token|paste-token` -- manage provider authentication.
- `models auth order get|set|clear` -- manage auth profile ordering.
- `models fallbacks list|add|remove|clear` -- manage model fallback chains.
- `models image-fallbacks list|add|remove|clear` -- manage image model fallback chains.
- `github-copilot login` -- GitHub Copilot authentication.

### `channels-cli.ts` -- Channel Setup and Management (80+ lines)

Registers the `channels` command group:

- `channels list` -- list configured channels.
- `channels add` -- add a new channel account (supports many `--*` options for each channel type).
- `channels remove` -- remove a channel account.
- `channels status` -- show channel connection status.
- `channels capabilities` -- show channel capabilities.
- `channels resolve` -- resolve a channel target address.
- `channels logs` -- show channel-specific logs.
- `channels login` -- authenticate with a channel.
- `channels logout` -- revoke channel authentication.

Supported option names for `add` include: `channel`, `account`, `name`, `token`, `botToken`, `appToken`, `signalNumber`, `cliPath`, `dbPath`, `httpUrl`, `webhookUrl`, `homeserver`, `userId`, `accessToken`, `groupChannels`, `dmAllowlist`, and many more.

### `config-cli.ts` -- Configuration Get/Set/Edit (80+ lines)

Registers the `config` command group:

- `config get <path>` -- read a config value by dotted path (supports bracket notation for arrays).
- `config set <path> <value>` -- set a config value (auto-parses JSON5 with `--json`).
- `config edit` -- open the config file in `$EDITOR`.
- `config unset <path>` -- remove a config key.
- `config path` -- print the config file path.

The path parser supports: dot notation (`gateway.auth.token`), bracket notation (`agents.list[0].id`), and escaped dots (`some\.key`).

### `completion-cli.ts` -- Shell Completion Generation

Generates shell completion scripts for bash/zsh/fish.

### `hooks-cli.ts` -- Hook Management

Manage event hooks (listing, testing, adding, removing hook configurations).

### `memory-cli.ts` -- Memory Management (711 lines)

Registers the `memory` command group for managing the AI memory/RAG system:

- `memory search <query>` -- semantic search across memory files and session transcripts.
- `memory list` -- list all memory files and their status.
- `memory get <path>` -- read a specific memory file.
- `memory add <path> <content>` -- add content to memory.
- `memory index` -- build or rebuild the memory search index.
- `memory scan` -- scan memory sources for indexing status.

Supports `--agent` flag to scope operations to a specific agent. Uses `MemorySearchManager` for index operations and `withProgress`/`withProgressTotals` for progress reporting.

### `plugins-cli.ts` -- Plugin Management (720 lines)

Registers the `plugins` command group:

- `plugins list` -- list installed plugins (`--json`, `--enabled`, `--verbose`).
- `plugins info <id>` -- show plugin details.
- `plugins install <spec>` -- install from npm spec or local path (auto-detects archives).
- `plugins uninstall <id>` -- uninstall a plugin (`--keep-files`, `--keep-config`, `--force`, `--dry-run`).
- `plugins update [id]` -- update npm-installed plugins (`--all`, `--dry-run`).
- `plugins enable <id>` -- enable a plugin.
- `plugins disable <id>` -- disable a plugin.
- `plugins select <id>` -- select a plugin for an exclusive slot (e.g., choosing one provider plugin).

Also registers plugin-provided CLI commands via `registerPluginCliCommands()`.

### `exec-approvals-cli.ts` -- Execution Approval Management

Manage the execution approval system (approve/deny pending tool executions).

### `skills-cli.ts` -- Skills Management

List, enable, disable, and inspect skill packs.

### `webhooks-cli.ts` -- Webhook Management

Configure webhook endpoints for external integrations.

### `pairing-cli.ts` -- Device Pairing

Pair devices (mobile apps, nodes) with the gateway using pairing codes/tokens.

### `devices-cli.ts` -- Device Management

Manage paired devices, revoke tokens, list connected devices.

### `update-cli/` -- Self-Update Management

Located in `src/cli/update-cli/`:

| File | Purpose |
|------|---------|
| `update-command.ts` | Update command implementation |
| `wizard.ts` | Interactive update wizard |
| `status.ts` | Update status checking |
| `shared.ts` | Shared update utilities |
| `progress.ts` | Update progress reporting |

### `browser-cli.ts` -- Browser Automation CLI

Browser automation commands with extensive sub-modules:

| File | Purpose |
|------|---------|
| `browser-cli.ts` | Main registration |
| `browser-cli-actions-input.ts` | Input actions (click, type, etc.) |
| `browser-cli-actions-observe.ts` | Observation actions (screenshot, DOM) |
| `browser-cli-debug.ts` | Debug/inspect utilities |
| `browser-cli-examples.ts` | Built-in usage examples |
| `browser-cli-extension.ts` | Browser extension management |
| `browser-cli-inspect.ts` | Page inspection |
| `browser-cli-manage.ts` | Browser profile management |
| `browser-cli-shared.ts` | Shared browser CLI utilities |
| `browser-cli-state.ts` | Browser state management |
| `browser-cli-state.cookies-storage.ts` | Cookie/storage state |

### `logs-cli.ts` -- Log Viewing

View and tail gateway logs with filtering.

### `system-cli.ts` -- System Info

Display system events, heartbeat status, and presence information.

### `security-cli.ts` -- Security Settings

Manage security-related configuration.

### `dns-cli.ts` -- DNS Utilities

DNS diagnostic and configuration helpers.

### `directory-cli.ts` -- Directory Navigation

Navigate and manage workspace directories.

### `sandbox-cli.ts` -- Sandbox Management

Manage sandbox/Docker environments for tool execution.

### `tui-cli.ts` -- Terminal UI Mode

Launch the terminal-based user interface.

---

## Core Commands (`src/commands/`)

The `src/commands/` directory contains the business logic implementations that CLI commands delegate to. These are separated from CLI registration to allow reuse (e.g., from the gateway RPC layer).

### `doctor` -- System Diagnostics and Repair

`src/commands/doctor.ts` -- runs comprehensive health checks and offers quick fixes:

Options:
- `--repair` / `--fix` -- apply recommended repairs without prompting.
- `--force` -- apply aggressive repairs (overwrites custom service config).
- `--non-interactive` -- run without prompts (safe migrations only).
- `--generate-gateway-token` -- generate and configure a gateway token.
- `--deep` -- scan system services for extra gateway installs.
- `--no-workspace-suggestions` -- disable workspace memory system suggestions.

Checks include: config validity, service installation, port availability, channel auth, model provider auth, dependency versions, and workspace configuration.

### `onboard` -- Interactive Setup Wizard

`src/commands/onboard.ts` -- the first-run experience for setting up OpenClaw:

- `runInteractiveOnboarding()` -- guided step-by-step setup.
- `runNonInteractiveOnboarding()` -- headless setup via flags (requires `--accept-risk`).
- Handles auth choice normalization for legacy providers (`claude-cli` -> setup-token, `codex-cli` -> OpenAI Codex OAuth).
- Supports `--reset` to clear existing config before onboarding.
- Warns on Windows platform about potential compatibility issues.

Supporting files:
- `onboard-interactive.ts` -- the interactive flow.
- `onboard-non-interactive.ts` -- the headless flow.
- `onboard-channels.ts` -- channel configuration step.
- `onboard-hooks.ts` -- hooks configuration step.
- `onboard-skills.ts` -- skills configuration step.
- `onboard-helpers.ts` -- shared helpers (DEFAULT_WORKSPACE, handleReset).
- `onboard-custom.ts` -- custom onboarding flows.
- `onboard-remote.ts` -- remote gateway onboarding.
- `onboard-types.ts` -- `OnboardOptions` type.
- `onboard-provider-auth-flags.ts` -- provider auth flag handling.
- `auth-choice-*.ts` -- auth choice implementation for each provider (Anthropic, OpenAI, OpenRouter, GitHub Copilot, Google, HuggingFace, etc.).
- `onboarding/` -- onboarding sub-directory with additional flow components.

### `configure` -- Configuration Management

Registered via `program/register.configure.ts`. Provides an interactive configuration wizard for modifying settings without directly editing JSON.

### `status` -- System Status Display

`src/commands/status.ts` exports:

```typescript
export { statusCommand } from "./status.command.js";
export { getStatusSummary } from "./status.summary.js";
export type { SessionStatus, StatusSummary } from "./status.types.js";
```

Supporting files:
- `status.command.ts` -- main status command logic.
- `status.summary.ts` -- summarizes gateway, channel, and session status.
- `status.format.ts` -- output formatting (text and JSON).
- `status.scan.ts` -- scans running services.
- `status.agent-local.ts` -- local agent status.
- `status.daemon.ts` -- daemon service status.
- `status.gateway-probe.ts` -- probe gateway health.
- `status.link-channel.ts` -- channel link status.
- `status.update.ts` -- update availability status.
- `status.types.ts` -- type definitions.

CLI flags: `--json`, `--all` (full diagnosis), `--usage` (model provider usage), `--deep` (channel probes), `--timeout <ms>`, `--verbose`.

### `gateway` -- Start the Gateway Server

Managed via `gateway-cli/`. The `gateway start` command launches the WebSocket gateway server. `gateway dev` provides a development mode with hot reload. `gateway stop` gracefully shuts down a running gateway.

### `send` -- Send Messages via Channels

The message sending functionality is registered via `program/register.message.ts` which sets up the `message` command group. The `message send` command sends messages through any configured channel:

```
openclaw message send --target +15555550123 --message "Hi"
openclaw message send --target +15555550123 --message "Hi" --media photo.jpg
```

Additional message subcommands:
- `message broadcast` -- send to multiple targets.
- `message poll` -- create channel polls (e.g., Discord polls).
- `message react` / `message unreact` -- add/remove reactions.
- `message read` / `message edit` / `message delete` -- read, edit, or delete messages.
- `message pin` / `message unpin` -- pin/unpin messages.
- `message thread` -- thread operations.
- `message search` -- search messages.
- `message permissions` -- check message permissions.
- `message emoji` / `message sticker` -- emoji and sticker operations.

### Other Commands

| Command | File | Purpose |
|---------|------|---------|
| `dashboard` | `dashboard.ts` | Open the Control UI in a browser |
| `reset` | `reset.ts` | Reset local config/state |
| `uninstall` | `uninstall.ts` | Uninstall gateway service and local data |
| `health` | `health.ts` | Quick health check |
| `sessions` | `sessions.ts` | Session management |
| `setup` | (via `register.setup.ts`) | Setup helpers |
| `agents` | `agents.ts` | Multi-agent management (add, delete, list, identity, config) |
| `agent` | `agent.ts` | Single agent operations |
| `sandbox` | `sandbox.ts` | Sandbox explanation and management |
| `signal-install` | `signal-install.ts` | Signal CLI installation helper |

---

## CLI Utilities

### `cli-utils.ts`

Shared CLI utility functions:
- `runCommandWithRuntime()` -- wraps a command action with runtime setup and error handling.
- `formatErrorMessage()` -- formats errors for CLI output.
- `withManager()` -- helper for commands that need a resource manager with cleanup.

### `cli-name.ts`

Resolves the CLI binary name for use in help text and error messages.

### `help-format.ts`

`formatHelpExamples()` -- formats example usage strings for Commander help text, aligning descriptions.

### `channel-options.ts`

`resolveCliChannelOptions()` -- resolves the list of available channel names for CLI option validation.
`formatCliChannelOptions()` -- formats channel options for display in help text.

### `channel-auth.ts`

`runChannelLogin()` / `runChannelLogout()` -- handle channel-specific authentication flows from the CLI.

### `command-format.ts`

`formatCliCommand()` -- formats command strings for display (with styling).

### `tagline.ts`

`pickTagline()` -- selects a startup tagline for the banner.

### `parse-bytes.ts`

Parses human-readable byte strings (e.g., `"10MB"`, `"1.5GiB"`).

### `parse-duration.ts`

Parses human-readable duration strings (e.g., `"30s"`, `"5m"`, `"2h"`).

### `parse-timeout.ts`

Timeout parsing helper used by various CLI commands.

### `wait.ts`

Waiting/polling utilities for CLI commands that need to wait for conditions.

### `route.ts`

`tryRouteCli()` -- fast-path routing that bypasses Commander for certain commands.

### `gateway-rpc.ts`

RPC call utilities for communicating with a running gateway from the CLI.

### `plugin-registry.ts`

Plugin registry integration for CLI plugin commands.

### `profile-utils.ts`

`isValidProfileName()` -- validates CLI profile names.

### `outbound-send-deps.ts`

Outbound message sending dependency wiring for CLI commands.
