# OpenClaw Architecture Overview

## What is OpenClaw?

OpenClaw is a **multi-platform AI assistant gateway** that connects various messaging services (WhatsApp, Telegram, Slack, Discord, Signal, iMessage, Microsoft Teams, Google Chat, and more) to a local AI runtime running on Node.js with TypeScript. It is designed as a **local-first** system that runs on the user's own hardware with optional cloud deployment.

## Key Facts

- **Version**: 2026.2.13
- **Primary Language**: TypeScript (ESM-only)
- **Runtime**: Node.js >= 22.12.0
- **Package Manager**: pnpm v10.23.0
- **Test Framework**: Vitest (1,111 test files)
- **Build Tool**: tsdown (TypeScript bundler)
- **Linter/Formatter**: oxlint / oxfmt

## Repository Structure

```
openclaw/
├── src/                      # Core TypeScript source (~52 modules, ~1,816 files)
├── apps/                     # Native platform applications
│   ├── android/             # Android app (Kotlin/Gradle)
│   ├── ios/                 # iOS app (SwiftUI)
│   ├── macos/               # macOS menu bar app (SwiftUI)
│   └── shared/              # Shared Swift code (OpenClawKit)
├── extensions/              # 37 runtime plugin extensions
├── skills/                  # 49 skill documentation packs
├── ui/                      # Web Control UI (Lit.js)
├── packages/                # Workspace packages (clawdbot, moltbot)
├── docs/                    # Documentation (Mintlify)
├── test/                    # Shared test fixtures
├── scripts/                 # Build and utility scripts
├── Swabble/                 # Swift package for iOS
└── [config files]           # tsconfig, vitest, docker, fly.toml, etc.
```

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      NATIVE APPS                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                  │
│  │  macOS   │  │   iOS    │  │ Android  │                   │
│  │ SwiftUI  │  │ SwiftUI  │  │  Kotlin  │                   │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘                  │
│       └──────────────┼─────────────┘                         │
│                      │ WebSocket (node role)                 │
│                      ▼                                       │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              GATEWAY SERVER (WebSocket)                │   │
│  │  ┌─────────┐ ┌──────────┐ ┌─────────┐ ┌──────────┐  │   │
│  │  │ Auth &  │ │ Session  │ │Protocol │ │Broadcast │  │   │
│  │  │  Rate   │ │  Mgmt    │ │  AJV    │ │  System  │  │   │
│  │  │ Limit   │ │          │ │ Schemas │ │          │  │   │
│  │  └─────────┘ └──────────┘ └─────────┘ └──────────┘  │   │
│  │  ┌──────────────┐  ┌────────────┐  ┌─────────────┐  │   │
│  │  │  80+ RPC     │  │ Health &   │  │ Maintenance │  │   │
│  │  │  Methods     │  │ Presence   │  │   Timers    │  │   │
│  │  └──────────────┘  └────────────┘  └─────────────┘  │   │
│  └───────────────────────┬──────────────────────────────┘   │
│                          │                                   │
│  ┌───────────────────────┼──────────────────────────────┐   │
│  │            CHANNEL ADAPTERS                           │   │
│  │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐        │   │
│  │  │Telegram│ │WhatsApp│ │Discord │ │ Slack  │        │   │
│  │  └────────┘ └────────┘ └────────┘ └────────┘        │   │
│  │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐        │   │
│  │  │ Signal │ │iMessage│ │  LINE  │ │MS Teams│  ...   │   │
│  │  └────────┘ └────────┘ └────────┘ └────────┘        │   │
│  └───────────────────────┬──────────────────────────────┘   │
│                          │                                   │
│  ┌───────────────────────┼──────────────────────────────┐   │
│  │            AI AGENT RUNTIME                           │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │  Model   │ │  Tool    │ │  Session  │             │   │
│  │  │ Providers│ │  System  │ │  Manager  │             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │ Sandbox  │ │  Media   │ │  TTS     │             │   │
│  │  │ (Docker) │ │ Underst. │ │ Pipeline │             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              PLUGIN SYSTEM                            │   │
│  │  37 Extensions (channels, memory, voice, auth, ...)   │   │
│  │  49 Skills (coding-agent, github, notion, ...)        │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌──────────────────┐  ┌────────────────┐                   │
│  │   Web Control UI │  │  CLI (commander)│                  │
│  │   (Lit.js)       │  │  20+ commands   │                  │
│  └──────────────────┘  └────────────────┘                   │
└─────────────────────────────────────────────────────────────┘
```

## Core Design Principles

1. **Local-First**: Runs on user's own hardware; gateway is the central control plane
2. **ESM-Only**: Pure ES modules throughout (no CommonJS)
3. **Strict TypeScript**: `strict: true`, `noImplicitAny: true`, Zod-validated configs
4. **Plugin Architecture**: Extensible via workspace packages + extension SDK
5. **Multi-Channel**: Unified interface across 15+ messaging platforms
6. **Modular Separation**: Clear boundaries between gateway, channels, agents, CLI, and UI

## Runtime Lifecycle

```
1. Bootstrap (openclaw.mjs)
   → Module compile cache, warning filters, dist resolution

2. Entry (src/entry.ts)
   → Environment loading, respawn handling, profile parsing

3. Index (src/index.ts)
   → Dependency setup, program building, CLI execution

4. Commands (src/commands/)
   → Gateway, agent, send, onboard, etc.

5. Gateway (src/gateway/)
   → WebSocket server, session management, channel dispatch

6. Agents (src/agents/)
   → Pi runtime, tool execution, message processing
```

## Document Index

| Document | Description |
|----------|-------------|
| [01-gateway.md](./01-gateway.md) | WebSocket gateway server, protocol, methods, events |
| [02-channels-and-routing.md](./02-channels-and-routing.md) | Channel adapters, message routing, session keys |
| [03-agents-and-ai-runtime.md](./03-agents-and-ai-runtime.md) | Agent execution, model providers, tool system |
| [04-configuration.md](./04-configuration.md) | Configuration system, Zod schemas, migrations |
| [05-cli.md](./05-cli.md) | CLI framework, all commands |
| [06-plugins-and-extensions.md](./06-plugins-and-extensions.md) | Plugin SDK, extensions, skills |
| [07-web-ui.md](./07-web-ui.md) | Web Control UI (Lit.js) |
| [08-native-apps.md](./08-native-apps.md) | iOS, macOS, Android applications |
| [09-media-and-content.md](./09-media-and-content.md) | Media understanding, link extraction, TTS, browser |
| [10-infrastructure.md](./10-infrastructure.md) | Logging, process management, daemon, pairing, cron |
| [11-testing-and-ci.md](./11-testing-and-ci.md) | Testing strategy, CI/CD, deployment |
| [12-security.md](./12-security.md) | Authentication, allowlists, sandbox, device pairing |
| [13-data-flow.md](./13-data-flow.md) | End-to-end message flow diagrams |
