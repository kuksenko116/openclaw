# Native Apps

## Overview

OpenClaw provides native applications for three platforms -- iOS, macOS, and Android -- that connect to the gateway server as **"node" role** clients over WebSocket. These apps expose device-specific capabilities (camera, location, voice, screen, contacts, calendar, etc.) to the AI agent runtime, and also provide chat and configuration interfaces.

All three platforms share a common communication pattern:

1. Discover or manually configure a gateway endpoint
2. Connect via WebSocket with device identity (cryptographic key pair)
3. Register available commands (capabilities the device can perform)
4. Handle `node.invoke` requests from the gateway (camera snap, location get, screen record, etc.)
5. Send events to the gateway (voice transcripts, agent requests, deep links)
6. Provide a chat UI for direct interaction with the agent

A shared Swift package (**OpenClawKit**) provides the protocol types, gateway session actor, and chat UI components shared between iOS and macOS. Android reimplements the protocol layer in Kotlin.

---

## Shared Libraries

### OpenClawKit (Swift Package)

**Location:** `apps/shared/OpenClawKit/`
**Package definition:** `apps/shared/OpenClawKit/Package.swift`
**Platforms:** iOS 18+, macOS 15+
**Swift tools version:** 6.2 (strict concurrency enabled)

OpenClawKit is a Swift Package Manager library with three targets:

| Target | Description | Dependencies |
|---|---|---|
| `OpenClawProtocol` | Auto-generated gateway protocol types (Codable structs for all RPC params/results) | None |
| `OpenClawKit` | Core gateway communication, device identity, service command types, capabilities | OpenClawProtocol, ElevenLabsKit |
| `OpenClawChatUI` | Shared SwiftUI chat views, message rendering, markdown, transport protocol | OpenClawKit, Textual (markdown) |

#### OpenClawProtocol (`Sources/OpenClawProtocol/`)

Auto-generated Codable types for the gateway WebSocket protocol:

- **`GatewayModels.swift`** -- All request/response types: `ConnectParams`, `HelloOkPayload`, `AgentParams`, `ChatSendParams`, `ChatEvent`, `SessionsListParams`, `CronJob`, `ExecApprovalRequestParams`, `DevicePairApproveParams`, `TickEvent`, `ShutdownEvent`, etc. (~2800 lines)
- **`AnyCodable.swift`** -- Type-erased JSON encoding/decoding for flexible payload handling
- **`WizardHelpers.swift`** -- Onboarding wizard protocol helpers
- **`GatewayFrame`** -- Enum wrapping `req`, `res`, `event`, and `unknown` frame types

#### OpenClawKit (`Sources/OpenClawKit/`)

Core library providing gateway communication and device service abstractions:

**Gateway Communication:**
- **`GatewayNodeSession.swift`** -- Swift `actor` managing WebSocket connections. Handles request/response tracking with UUID-indexed pending map, invoke timeout via latch pattern, event streaming via `AsyncStream`, snapshot waiting for initial connection state. Uses `GatewayChannelActor` for the underlying WebSocket transport.
- **`GatewayChannel.swift`** -- Low-level WebSocket actor wrapping `URLSessionWebSocketTask`. Provides `WebSocketTasking` protocol for testability. Sets 16MB max message size for large payloads.
- **`BridgeFrames.swift`** -- Frame types for the node bridge protocol:
  - `BridgeInvokeRequest` -- `{type: "invoke", id, command, paramsJSON}`
  - `BridgeInvokeResponse` -- `{type: "invoke-res", id, ok, payloadJSON, error?}`
  - `BridgeEventFrame` -- `{type: "event", event, payloadJSON}`
  - `BridgeHello` -- `{type: "hello", nodeId, displayName, platform, version, caps, commands, permissions}`
- **`GatewayTLSPinning.swift`** -- TLS certificate pinning with TOFU (trust on first use) model. Stores SHA-256 fingerprints.
- **`GatewayPush.swift`** -- Push notification token registration with gateway
- **`GatewayEndpointID.swift`** -- Stable endpoint identification
- **`GatewayPayloadDecoding.swift`** -- Helpers for decoding nested JSON payloads
- **`GatewayErrors.swift`** -- Error types for gateway communication

**Device Identity:**
- **`DeviceIdentity.swift`** -- Cryptographic device identity using Curve25519 signing keys (via CryptoKit). Device ID is the SHA-256 hash of the public key. Stored in `~/Library/Application Support/OpenClaw/identity/device.json`. Provides `signPayload()` for authentication and `publicKeyBase64Url()` for the connect handshake.
- **`DeviceAuthStore.swift`** -- Persistent storage for device authentication tokens
- **`InstanceIdentity.swift`** -- Unique instance tracking across sessions

**Capabilities & Commands:**
- **`Capabilities.swift`** -- Enum of device capabilities: `canvas`, `camera`, `screen`, `voiceWake`, `location`, `device`, `photos`, `contacts`, `calendar`, `reminders`, `motion`
- **`CameraCommands.swift`** -- Camera snap/clip command parameter types (`OpenClawCameraSnapParams`, `OpenClawCameraClipParams`)
- **`LocationCommands.swift`** -- Location get params and payload types
- **`ScreenCommands.swift`** -- Screen record command types
- **`CalendarCommands.swift`** -- Calendar event query/add types
- **`ContactsCommands.swift`** -- Contact search/add types
- **`RemindersCommands.swift`** -- Reminders list/add types
- **`MotionCommands.swift`** -- Motion activity and pedometer types
- **`PhotosCommands.swift`** -- Photos library access types
- **`DeviceCommands.swift`** -- Device status/info payload types
- **`SystemCommands.swift`** -- System-level command types
- **`ChatCommands.swift`** -- Chat-related command types
- **`TalkCommands.swift`** -- Voice conversation command types

**Canvas / A2UI:**
- **`CanvasCommands.swift`** -- Canvas present/hide/navigate/eval/snapshot commands
- **`CanvasA2UICommands.swift`** -- A2UI reset/push/pushJSONL commands
- **`CanvasA2UIJSONL.swift`** -- JSONL serialization for A2UI protocol messages
- **`CanvasA2UIAction.swift`** -- Action types for canvas-to-native bridge
- **`CanvasCommandParams.swift`** -- Typed parameters for canvas commands

**Voice:**
- **`TalkPromptBuilder.swift`** -- Builds system prompts for voice conversation models
- **`TalkDirective.swift`** -- Parses TTS directives from agent responses
- **`TalkHistoryTimestamp.swift`** -- Timestamp formatting for talk history
- **`AudioStreamingProtocols.swift`** -- Audio streaming abstractions
- **`TalkSystemSpeechSynthesizer.swift`** -- System TTS wrapper
- **`ElevenLabsKitShim.swift`** -- ElevenLabs TTS integration shim

**Utilities:**
- **`BonjourTypes.swift`** -- mDNS service type constants for gateway discovery
- **`BonjourEscapes.swift`** -- DNS domain name escaping utilities
- **`DeepLinks.swift`** -- `openclaw://` URL scheme parsing
- **`LocationSettings.swift`** -- Location permission level types
- **`AsyncTimeout.swift`** -- Async timeout helper using `Task.sleep`
- **`JPEGTranscoder.swift`** -- JPEG quality/size optimization
- **`NodeError.swift`** -- Error types for node operations
- **`StoragePaths.swift`** -- Platform-specific storage path resolution
- **`ToolDisplay.swift`** -- Tool execution display formatting
- **`OpenClawKitResources.swift`** -- Bundled resources (audio chimes)

#### OpenClawChatUI (`Sources/OpenClawChatUI/`)

Shared SwiftUI chat interface components used by both iOS and macOS:

- **`ChatTransport.swift`** -- Protocol defining the chat transport interface:
  ```swift
  protocol OpenClawChatTransport: Sendable {
      func requestHistory(sessionKey:) async throws -> OpenClawChatHistoryPayload
      func sendMessage(sessionKey:, message:, thinking:, idempotencyKey:, attachments:) async throws -> OpenClawChatSendResponse
      func abortRun(sessionKey:, runId:) async throws
      func listSessions(limit:) async throws -> OpenClawChatSessionsListResponse
      func requestHealth(timeoutMs:) async throws -> Bool
      func events() -> AsyncStream<OpenClawChatTransportEvent>
      func setActiveSessionKey(_:) async throws
  }
  ```
- **`ChatViewModel.swift`** -- Observable view model managing chat state, message history, streaming responses, tool call tracking, session switching
- **`ChatView.swift`** -- Main chat SwiftUI view with message list and compose area
- **`ChatComposer.swift`** -- Message input area with attachment support
- **`ChatMessageViews.swift`** -- Individual message rendering (user, assistant, tool results)
- **`ChatModels.swift`** -- Chat data models (messages, sessions, attachments)
- **`ChatSessions.swift`** -- Session listing and switching UI
- **`ChatSheets.swift`** -- Sheet presentations (session picker, settings)
- **`ChatPayloadDecoding.swift`** -- Decodes chat event payloads from gateway
- **`ChatMarkdownPreprocessor.swift`** -- Preprocesses markdown for rendering
- **`ChatMarkdownRenderer.swift`** -- Renders markdown to attributed strings
- **`AssistantTextParser.swift`** -- Parses assistant response text (strips thinking tags, extracts tool calls)
- **`ChatTheme.swift`** -- Chat UI theming (colors, fonts, spacing)

---

## iOS App

**Location:** `apps/ios/`
**Framework:** SwiftUI with `@Observable` macro (Observation framework)
**Entry point:** `apps/ios/Sources/OpenClawApp.swift`
**Minimum iOS:** 18.0

### Architecture

The iOS app follows an **environment-injected observable model** pattern:

```
OpenClawApp (@main)
 ├── NodeAppModel (@Observable) -- central state & service coordinator
 ├── GatewayConnectionController -- discovery, TLS pinning, connection lifecycle
 └── RootCanvas → RootTabs (TabView)
      ├── ScreenTab -- Canvas/A2UI WKWebView
      ├── VoiceTab -- Voice wake + talk mode controls
      └── SettingsTab -- Gateway connection, device capabilities
```

`NodeAppModel` and `GatewayConnectionController` are injected via SwiftUI `@Environment` and accessed by child views.

### App Entry (`OpenClawApp.swift`)

```swift
@main
struct OpenClawApp: App {
    @State private var appModel: NodeAppModel
    @State private var gatewayController: GatewayConnectionController
}
```

- Bootstraps `GatewaySettingsStore` persistence on init
- Injects `appModel` and `gatewayController` into environment
- Handles `onOpenURL` for deep links (`openclaw://` URLs)
- Tracks `scenePhase` changes (active/background) for connection management

### NodeAppModel (`Sources/Model/NodeAppModel.swift`)

Central `@MainActor @Observable` class managing all app state and services:

**Dual Gateway Connections:**
- **`nodeGateway`** (`GatewayNodeSession`) -- Connects as "node" role for device capabilities (`node.invoke` handling)
- **`operatorGateway`** (`GatewayNodeSession`) -- Connects as "operator" role for chat, config, voice wake

**State Properties:**
- `gatewayStatusText`, `gatewayServerName`, `connectedGatewayID` -- Connection status
- `mainSessionKey`, `selectedAgentId`, `gatewayDefaultAgentId`, `gatewayAgents` -- Agent/session state
- `isBackgrounded`, `screenRecordActive`, `cameraHUDText`, `cameraHUDKind` -- UI state

**Services (injected as protocol-conforming types):**
- `screen: ScreenController` -- Canvas WebView hosting
- `camera: CameraServicing` -- Photo/video capture
- `screenRecorder: ScreenRecordingServicing` -- Screen recording
- `voiceWake: VoiceWakeManager` -- Wake word detection
- `talkMode: TalkModeManager` -- Bidirectional voice conversation
- `locationService: LocationServicing` -- GPS location
- `photosService: PhotosServicing` -- Photo library access
- `contactsService: ContactsServicing` -- Contact lookup/add
- `calendarService: CalendarServicing` -- Calendar events
- `remindersService: RemindersServicing` -- Reminders access
- `motionService: MotionServicing` -- Accelerometer, pedometer
- `deviceStatus: DeviceStatusServicing` -- Battery, network info
- `notificationCenter: NotificationCentering` -- Local push notifications
- `gatewayHealthMonitor: GatewayHealthMonitor` -- Periodic health checks

**Capability Router:**
- `capabilityRouter: NodeCapabilityRouter` -- Routes incoming `node.invoke` requests to the appropriate service handler by command name
- Built lazily via `buildCapabilityRouter()` which maps command strings to handler closures

### GatewayConnectionController (`Sources/Gateway/GatewayConnectionController.swift`)

Manages discovery and connection lifecycle:

- **Bonjour/mDNS Discovery** -- Listens for `_openclaw._tcp` service broadcasts on the local network
- **Service Resolution** -- Resolves discovered services via DNS SRV/A/AAAA records (`GatewayServiceResolver`)
- **TLS Fingerprint Pinning** -- TOFU model: on first connection, prompts user to trust the TLS certificate. Stores fingerprint for future connections
- **Manual Connection** -- Supports manual IP:port entry as fallback
- **Auto-reconnect** -- Reconnects to last known gateway on app launch
- **Keychain Storage** -- Credentials stored in iOS Keychain (`KeychainStore`)
- **Trust Prompts** -- Shows `GatewayTrustPromptAlert` for new/changed TLS certificates

**Supporting files:**
- `GatewaySettingsStore.swift` -- Persists gateway preferences
- `GatewayDiscoveryModel.swift` -- Discovery state model
- `GatewayConnectConfig.swift` -- Connection configuration
- `GatewayHealthMonitor.swift` -- Periodic gateway health checks
- `GatewayDiscoveryDebugLogView.swift` -- Debug UI for discovery

### Tab Structure (`RootTabs.swift`)

Three-tab `TabView`:

| Tab | View | Description |
|---|---|---|
| Screen | `ScreenTab` | Canvas/A2UI rendering in a `WKWebView` |
| Voice | `VoiceTab` | Voice wake trigger list + talk mode controls |
| Settings | `SettingsTab` | Gateway connection, capabilities toggle, preferences |

**Overlay Elements:**
- `StatusPill` -- Top-left pill showing connection state (connected/connecting/error/disconnected), activity indicators (recording, camera, voice wake, pairing)
- `VoiceWakeToast` -- Toast notification when voice wake triggers

### Service Protocols (`Sources/Services/NodeServiceProtocols.swift`)

Protocol-oriented service abstraction for testability:

```swift
protocol CameraServicing: Sendable {
    func listDevices() async -> [CameraController.CameraDeviceInfo]
    func snap(params: OpenClawCameraSnapParams) async throws -> (format, base64, width, height)
    func clip(params: OpenClawCameraClipParams) async throws -> (format, base64, durationMs, hasAudio)
}

protocol LocationServicing: Sendable {
    func ensureAuthorization(mode:) async -> CLAuthorizationStatus
    func currentLocation(params:, desiredAccuracy:, maxAgeMs:, timeoutMs:) async throws -> CLLocation
}

protocol PhotosServicing: Sendable { ... }
protocol ContactsServicing: Sendable { ... }
protocol CalendarServicing: Sendable { ... }
protocol RemindersServicing: Sendable { ... }
protocol MotionServicing: Sendable { ... }
protocol ScreenRecordingServicing: Sendable { ... }
protocol DeviceStatusServicing: Sendable { ... }
```

Concrete implementations (`CameraController`, `LocationService`, `ScreenRecordService`, etc.) conform to these protocols and are injected into `NodeAppModel`.

### Node Capability Router (`Sources/Capabilities/NodeCapabilityRouter.swift`)

Routes incoming `node.invoke` gateway requests to the correct handler:

```swift
final class NodeCapabilityRouter {
    typealias Handler = (BridgeInvokeRequest) async throws -> BridgeInvokeResponse
    private let handlers: [String: Handler]

    func handle(_ request: BridgeInvokeRequest) async throws -> BridgeInvokeResponse {
        guard let handler = handlers[request.command] else { throw RouterError.unknownCommand }
        return try await handler(request)
    }
}
```

Supported command namespaces: `camera.*`, `location.*`, `screen.*`, `canvas.*`, `a2ui.*`, `device.*`, `photos.*`, `contacts.*`, `calendar.*`, `reminders.*`, `motion.*`, `talk.*`, `chat.*`, `system.*`

### Chat Integration (`Sources/Chat/`)

- **`IOSGatewayChatTransport.swift`** -- Implements `OpenClawChatTransport` protocol, wrapping `GatewayNodeSession` calls with timeouts (send: 30s, list: 15s, history: 15s, abort: 10s). Provides `events()` as `AsyncStream` of chat/agent/health events.
- **`ChatSheet.swift`** -- Bottom sheet presenting the shared `ChatView` from OpenClawChatUI

### Screen / Canvas (`Sources/Screen/`)

- **`ScreenController.swift`** -- Manages `WKWebView` for canvas rendering. Handles deep links (`openclaw://`) and A2UI action messages bridged from JavaScript.
- **`ScreenTab.swift`** -- SwiftUI tab hosting the canvas WebView
- **`ScreenWebView.swift`** -- `UIViewRepresentable` wrapper for WKWebView
- **`ScreenRecordService.swift`** -- ReplayKit-based screen recording

### Voice (`Sources/Voice/`)

- **`VoiceWakeManager.swift`** -- On-device speech recognition for configurable trigger phrases. Uses `SFSpeechRecognizer` for continuous listening. Sends recognized commands via `agent.request` event to gateway. Configurable pause duration.
- **`VoiceWakePreferences.swift`** -- UserDefaults-backed preferences (enabled, locale, model). Syncs with gateway via `VoiceWakeGlobalSettingsSync`.
- **`TalkModeManager.swift`** -- Bidirectional voice conversation. Push-to-talk (PTT) support. Pauses voice wake to prevent mic conflicts. Manages listening/speaking state transitions.
- **`VoiceTab.swift`** -- Voice settings UI
- **`TalkOrbOverlay.swift`** -- Visual indicator during talk mode

### Other Services

- **`LocationService.swift`** -- CLLocationManager wrapper with authorization handling
- **`CameraController.swift`** -- AVCaptureSession management for photos and video clips
- **`PhotoLibraryService.swift`** -- PHPhotoLibrary access for recent photos
- **`ContactsService.swift`** -- CNContactStore search and add
- **`CalendarService.swift`** -- EKEventStore queries and event creation
- **`RemindersService.swift`** -- EKReminder access
- **`MotionService.swift`** -- CMMotionActivityManager and CMPedometer
- **`DeviceStatusService.swift`** -- UIDevice battery, ProcessInfo thermal state
- **`NetworkStatusService.swift`** -- NWPathMonitor for connectivity info
- **`NotificationService.swift`** -- UNUserNotificationCenter for local notifications
- **`NodeDisplayName.swift`** -- Device name resolution

---

## macOS App

**Location:** `apps/macos/`
**Framework:** SwiftUI with AppKit integration, `@Observable` macro
**Entry point:** `apps/macos/Sources/OpenClaw/MenuBar.swift`
**Minimum macOS:** 15.0

### Architecture

The macOS app is a **menu bar application** with optional floating canvas windows:

```
OpenClawApp (@main, MenuBarExtra)
 ├── AppState (@Observable) -- 100+ UserDefaults-backed properties
 ├── GatewayProcessManager -- local gateway subprocess lifecycle
 ├── GatewayConnectivityCoordinator -- connection mode (local/remote/unconfigured)
 ├── ControlChannel -- operator gateway RPC
 ├── CanvasManager -- multi-window canvas hosting
 └── MenuContent
      ├── CritterStatusLabel -- animated status icon
      ├── Session list (injected via MenuSessionsInjector)
      ├── Cost/usage bar
      └── Settings window (tab-based)
```

### App Entry (`MenuBar.swift`)

```swift
@main
struct OpenClawApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var state: AppState
    private let gatewayManager = GatewayProcessManager.shared
    private let controlChannel = ControlChannel.shared
    private let connectivityCoordinator = GatewayConnectivityCoordinator.shared
}
```

The app uses `MenuBarExtra` with `.menu` style. The menu bar icon is a `CritterStatusLabel` with animated states:
- Paused, sleeping, working, ear-boost (listening), celebration (send success)
- Gateway status indicators (running, stopped, failed, attached)
- Icon animations configurable via preferences

### AppState (`AppState.swift`)

`@MainActor @Observable` class with 100+ properties, all backed to `UserDefaults`:

**Connection Mode:**
```swift
enum ConnectionMode: String {
    case unconfigured  // First launch, needs setup
    case local         // Running gateway as subprocess
    case remote        // Connecting to remote gateway
}
```

**Key Properties:**
- `isPaused`, `launchAtLogin`, `onboardingSeen` -- App lifecycle
- `swabbleEnabled`, `swabbleTriggerWords` -- Voice wake configuration
- `voiceWakeTriggerChime`, `voiceWakeSendChime` -- Audio feedback settings
- `debugPaneEnabled` -- Developer tools
- `isWorking`, `earBoostActive`, `blinkTick`, `sendCelebrationTick` -- UI animation state
- `iconAnimationsEnabled`, `iconState` -- Menu bar icon customization

### GatewayProcessManager (`GatewayProcessManager.swift`)

Singleton managing the local gateway subprocess:

```swift
enum Status: Equatable {
    case stopped
    case starting
    case running(details: String?)
    case attachedExisting(details: String?)  // Found running gateway, attached
    case failed(String)
}
```

**Responsibilities:**
- Launches `openclaw gateway` as a child process
- Detects existing gateway processes (avoids duplicates)
- Sets up environment variables (API keys, config paths)
- Manages log output (capped at 20KB in-memory)
- Integrates with launchd for launch-at-login (`GatewayLaunchAgentManager`)
- Health checking via periodic RPC calls
- Handles process restart on crash

### GatewayConnectivityCoordinator (`GatewayConnectivityCoordinator.swift`)

Manages the connection strategy:

- **Local mode** -- Starts gateway subprocess, connects to localhost
- **Remote mode** -- Connects to a configured remote gateway
  - SSH tunnel support (`RemoteTunnelManager`) for secure remote connections
  - Tailscale integration (`TailscaleService`) for mesh networking
  - Direct connection with TLS
- **Unconfigured** -- Shows onboarding wizard
- Connection retries with exponential backoff

### Canvas System

Multi-window canvas rendering with A2UI support:

- **`CanvasManager.swift`** -- Manages multiple `CanvasWindowController` instances, one per session key. Handles window creation, layout, and lifecycle.
- **`CanvasWindowController.swift`** -- `NSWindowController` hosting a `WKWebView`:
  - Custom URL scheme handlers (`canvas://`, `openclaw-canvas://`) serving local session files
  - JavaScript bridge for A2UI action messages
  - File watcher for hot reload during development
  - `HoverChromeContainerView` for window controls
- **`CanvasSchemeHandler.swift`** -- Intercepts custom scheme requests, serves files from session directory
- **`CanvasA2UIActionMessageHandler.swift`** -- Bridges `a2uiaction` DOM events to native code
- **`CanvasFileWatcher.swift`** -- FSEvents monitoring for live reload
- **`CanvasWindow.swift`** -- Custom `NSWindow` subclass
- **`CanvasChromeContainerView.swift`** -- Floating chrome/controls overlay

### Settings UI

Tab-based `SettingsRootView` with sections:

| Tab | Description |
|---|---|
| General | Gateway mode, connection, launch at login, Anthropic auth |
| Channels | Per-channel configuration (Slack, Discord, Telegram, WhatsApp, iMessage, Signal, etc.) |
| Cron | Scheduled job management with editor |
| Instances | Node pairing and device approval |
| Sessions | Session defaults, chat settings |
| Skills | Skill availability, API key injection |
| Permissions | Microphone, camera, screen recording permissions |
| About | Version info, update check (via Sparkle) |

### Channel Management

- **`ChannelsStore.swift`** -- Channel status polling, account linking workflows
- **`ChannelsSettings.swift`** -- Main channels settings view
- **`ChannelConfigForm.swift`** -- Per-channel configuration form rendering
- **`ChannelsStore+Config.swift`** -- Config read/write operations
- **`ChannelsStore+Lifecycle.swift`** -- Channel start/stop operations
- **`ChannelsSettings+ChannelSections.swift`** -- Per-channel UI sections
- **`ChannelsSettings+ChannelState.swift`** -- Channel state tracking

### Configuration

- **`ConfigFileWatcher.swift`** -- FSEvents monitoring of `~/.openclaw/openclaw.json`, reloads config on changes
- **`ConfigStore.swift`** -- Gateway config snapshot fetching, schema caching
- **`ConfigSettings.swift`** -- Config editor UI (JSON schema-driven form)
- **`ConfigSchemaSupport.swift`** -- Schema parsing and UI hint extraction

### Voice & Audio

- **`VoiceWakeRuntime`** (in shared singletons) -- Manages on-device speech recognition with configurable mic input device selection (`AudioInputDeviceObserver`). Supports multi-locale recognition and chime effects on trigger/send.
- **`TalkModeController`** -- Bidirectional voice. Speech recognition input, TTS output, audio ducking.
- **`HoverHUD`** -- Floating HUD overlay showing voice wake status, talk mode status

### Other macOS Features

- **`DockIconManager.swift`** -- Show/hide dock icon (`NSApplication.setActivationPolicy`)
- **`CLIInstaller.swift`** -- Installs `openclaw` CLI to `/usr/local/bin` via symlink
- **`ExecApprovals.swift` / `ExecApprovalsSocket.swift`** -- Execution approval prompts and gateway subscription
- **`DevicePairingApprovalPrompter.swift`** -- Device pairing approval dialogs
- **`HeartbeatStore.swift`** -- Heartbeat event tracking
- **`HealthStore.swift`** -- Gateway health state caching
- **`AgentEventStore.swift`** -- Agent execution event history
- **`CostUsageMenuView.swift`** -- Token/cost display in menu bar
- **`ContextUsageBar.swift`** -- Context window utilization bar
- **`NotifyOverlay.swift`** -- Notification overlay for important events
- **`PortGuardian.swift`** -- Port conflict detection
- **`DiagnosticsFileLog.swift`** -- File-based diagnostic logging
- **`AnthropicOAuth.swift`** -- Anthropic OAuth flow integration
- **`ModelCatalogLoader.swift`** -- Model discovery and catalog loading

---

## Android App

**Location:** `apps/android/`
**Language:** Kotlin with Jetpack Compose
**Build:** Gradle with Kotlin 2.0.x, Compose compiler plugin
**Target SDK:** 36 (min SDK 31)
**Entry point:** `apps/android/app/src/main/java/ai/openclaw/android/MainActivity.kt`

### Architecture

The Android app follows an **MVVM pattern** with Kotlin Coroutines and StateFlow:

```
NodeApp (Application)
 └── NodeRuntime -- central coordinator (CoroutineScope)
      ├── SecurePrefs -- encrypted SharedPreferences
      ├── DeviceAuthStore -- device token persistence
      ├── GatewayDiscovery -- mDNS/Bonjour service discovery
      ├── GatewaySession -- WebSocket client
      ├── ChatController -- chat state management
      ├── InvokeDispatcher -- command routing
      ├── VoiceWakeManager -- wake word detection
      ├── TalkModeManager -- voice conversation
      ├── CameraCaptureManager -- photo/video
      ├── LocationCaptureManager -- GPS
      ├── ScreenRecordManager -- screen recording
      ├── SmsManager -- SMS read/send
      └── CanvasController -- WebView A2UI

MainActivity (ComponentActivity)
 └── MainViewModel (AndroidViewModel)
      └── Exposes NodeRuntime StateFlows to Compose UI
           └── RootScreen (Composable)
                ├── Canvas area
                ├── SettingsSheet
                ├── ChatSheet
                └── StatusPill overlay
```

### Package Organization

```
ai.openclaw.android
 ├── MainActivity.kt          -- Activity lifecycle, permissions
 ├── MainViewModel.kt         -- ViewModel exposing StateFlows
 ├── NodeRuntime.kt            -- Central coordinator
 ├── NodeApp.kt                -- Application class
 ├── NodeForegroundService.kt  -- Foreground service for background ops
 ├── SecurePrefs.kt            -- Android Keystore encrypted prefs
 ├── SessionKey.kt             -- Session key utilities
 ├── DeviceNames.kt            -- Device name resolution
 ├── LocationMode.kt           -- Location permission levels
 ├── VoiceWakeMode.kt          -- Voice wake mode enum
 ├── WakeWords.kt              -- Wake word configuration
 ├── CameraHudState.kt         -- Camera overlay state
 ├── PermissionRequester.kt    -- Runtime permission handling
 ├── ScreenCaptureRequester.kt -- MediaProjection permission
 ├── InstallResultReceiver.kt  -- APK install result handler
 │
 ├── gateway/
 │    ├── GatewaySession.kt       -- WebSocket client with reconnect
 │    ├── GatewayDiscovery.kt     -- NSD (mDNS) service discovery
 │    ├── GatewayEndpoint.kt      -- Discovered endpoint model
 │    ├── GatewayProtocol.kt      -- Protocol constants
 │    ├── GatewayTls.kt           -- TLS fingerprint probing
 │    ├── DeviceAuthStore.kt      -- Device token storage
 │    ├── DeviceIdentityStore.kt  -- Curve25519 key generation/storage
 │    └── BonjourEscapes.kt       -- DNS name escaping
 │
 ├── node/
 │    ├── InvokeDispatcher.kt      -- Command routing (central switch)
 │    ├── ConnectionManager.kt     -- Connection lifecycle
 │    ├── GatewayEventHandler.kt   -- Gateway event processing
 │    ├── CameraCaptureManager.kt  -- CameraX photo/video
 │    ├── CameraHandler.kt         -- Camera command handler
 │    ├── LocationCaptureManager.kt -- FusedLocationProvider
 │    ├── LocationHandler.kt        -- Location command handler
 │    ├── ScreenRecordManager.kt   -- MediaProjection recording
 │    ├── ScreenHandler.kt         -- Screen command handler
 │    ├── SmsManager.kt            -- SMS content provider access
 │    ├── SmsHandler.kt            -- SMS command handler
 │    ├── CanvasController.kt      -- WebView management
 │    ├── A2UIHandler.kt           -- A2UI protocol handling
 │    ├── DebugHandler.kt          -- Debug commands
 │    ├── AppUpdateHandler.kt      -- APK self-update
 │    ├── JpegSizeLimiter.kt       -- JPEG quality optimization
 │    └── NodeUtils.kt             -- Utility functions
 │
 ├── chat/
 │    ├── ChatController.kt       -- Chat state, history, streaming
 │    ├── ChatModels.kt           -- Message, session, tool call types
 │    ├── ChatMessage.kt          -- Message model
 │    ├── ChatPendingToolCall.kt  -- Tool call tracking
 │    ├── ChatSessionEntry.kt    -- Session entry model
 │    └── OutgoingAttachment.kt  -- File attachment model
 │
 ├── voice/
 │    ├── VoiceWakeManager.kt          -- SpeechRecognizer wake detection
 │    ├── VoiceWakeCommandExtractor.kt -- Command extraction from speech
 │    ├── TalkModeManager.kt           -- Voice conversation
 │    ├── TalkDirectiveParser.kt       -- TTS directive parsing
 │    └── StreamingMediaDataSource.kt  -- Audio streaming data source
 │
 ├── protocol/
 │    ├── OpenClawCanvasA2UIAction.kt  -- A2UI action types
 │    ├── OpenClawProtocolConstants.kt -- Protocol constants
 │    ├── OpenClawCanvasCommand.kt     -- Canvas command enums
 │    ├── OpenClawCameraCommand.kt     -- Camera command enums
 │    ├── OpenClawLocationCommand.kt   -- Location command enums
 │    ├── OpenClawScreenCommand.kt     -- Screen command enums
 │    └── OpenClawSmsCommand.kt        -- SMS command enums
 │
 ├── ui/
 │    ├── RootScreen.kt           -- Main Compose root
 │    ├── SettingsSheet.kt        -- Settings bottom sheet
 │    ├── ChatSheet.kt            -- Chat bottom sheet
 │    ├── StatusPill.kt           -- Connection status overlay
 │    ├── TalkOrbOverlay.kt       -- Talk mode visual indicator
 │    ├── CameraHudOverlay.kt     -- Camera activity overlay
 │    ├── OpenClawTheme.kt        -- Material3 theme
 │    └── chat/
 │         ├── ChatSheetContent.kt     -- Chat content layout
 │         ├── ChatMessageViews.kt     -- Message rendering
 │         ├── ChatMessageListCard.kt  -- Message list card
 │         ├── ChatComposer.kt         -- Input composer
 │         ├── ChatMarkdown.kt         -- Markdown rendering
 │         ├── ChatSessionsDialog.kt   -- Session picker
 │         └── SessionFilters.kt       -- Session filtering
 │
 └── tools/
      └── ToolDisplay.kt          -- Tool execution display
```

### NodeRuntime (`NodeRuntime.kt`)

Central coordinator managing all subsystems:

```kotlin
class NodeRuntime(context: Context) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    val prefs = SecurePrefs(appContext)
    val canvas = CanvasController()
    val camera = CameraCaptureManager(appContext)
    val location = LocationCaptureManager(appContext)
    val screenRecorder = ScreenRecordManager(appContext)
    val sms = SmsManager(appContext)
    private val discovery = GatewayDiscovery(appContext, scope)
    private val voiceWake: VoiceWakeManager
    private val talkMode: TalkModeManager
    private val chatController: ChatController
    ...
}
```

Key behaviors:
- Manages dual gateway sessions (node + operator, similar to iOS)
- Voice wake sends `agent.request` events with the recognized command
- Discovery uses Android NSD (Network Service Discovery) for mDNS
- Connection state exposed as `StateFlow` for Compose reactivity

### InvokeDispatcher (`node/InvokeDispatcher.kt`)

Central command routing with capability/permission checks:

```kotlin
suspend fun handleInvoke(command: String, paramsJson: String?): InvokeResult {
    // Foreground check for canvas/camera/screen commands
    // Camera enabled check
    // Location enabled check

    return when (command) {
        // Canvas: present, hide, navigate, eval, snapshot
        // A2UI: reset, push, pushJSONL
        // Camera: snap, clip
        // Location: get
        // Screen: record
        // SMS: send
        // Debug: ed25519, logs
        // App: update
        else -> InvokeResult.error("INVALID_REQUEST", "unknown command")
    }
}
```

### Gateway Communication

- **`GatewaySession.kt`** -- OkHttp-based WebSocket client. Handles frame parsing, request/response correlation, event distribution, automatic reconnection with backoff.
- **`GatewayDiscovery.kt`** -- Android NSD (Network Service Discovery) wrapper for mDNS. Resolves service endpoints, probes TLS fingerprints.
- **`GatewayTls.kt`** -- TLS fingerprint probing and TOFU trust management
- **`DeviceIdentityStore.kt`** -- Curve25519 key generation using Android crypto primitives. Stores identity in encrypted SharedPreferences.
- **`DeviceAuthStore.kt`** -- Device authentication token management

### Chat System

- **`ChatController.kt`** -- Manages chat state: message history, streaming assistant responses, pending tool calls, session list. Uses `StateFlow` for reactive updates.
- **`ChatModels.kt`** -- Data classes for `ChatMessage`, `ChatSessionEntry`, `ChatPendingToolCall`, `OutgoingAttachment`
- UI views in `ui/chat/` render messages, compose input, session picker, and markdown content

### Voice

- **`VoiceWakeManager.kt`** -- `SpeechRecognizer`-based wake word detection. Configurable trigger phrases. Sends recognized text as `agent.request` event.
- **`VoiceWakeCommandExtractor.kt`** -- Extracts command text after trigger phrase match
- **`TalkModeManager.kt`** -- Bidirectional voice conversation with speech recognition input and TTS output
- **`TalkDirectiveParser.kt`** -- Parses `[[tts]]` directives from assistant responses

### Android-Specific Features

- **`NodeForegroundService.kt`** -- Android foreground service for persistent background operation with notification
- **`SecurePrefs.kt`** -- Encrypted SharedPreferences using Android Keystore for key encryption
- **`ScreenCaptureRequester.kt`** -- MediaProjection API permission handling
- **`PermissionRequester.kt`** -- Runtime permission orchestration (camera, location, notification, nearby devices)
- **`AppUpdateHandler.kt`** -- Self-update via APK download and install
- **`JpegSizeLimiter.kt`** -- Progressive JPEG quality reduction to meet size limits

---

## Cross-Platform Communication Protocol

All native apps communicate with the gateway using the same WebSocket frame protocol:

### Connection Flow

```
App                              Gateway
 │                                  │
 │  ─── WebSocket connect ───────>  │
 │                                  │
 │  <── connect.challenge ────────  │  (nonce for replay protection)
 │                                  │
 │  ─── connect request ─────────>  │  (device identity, platform, commands, caps)
 │       {                          │
 │         role: "node",            │
 │         device: {                │
 │           id: SHA256(pubkey),    │
 │           publicKey: base64url,  │
 │           signature: signed(nonce+ts),
 │           signedAt: timestamp    │
 │         },                       │
 │         platform: "ios",         │
 │         commands: ["camera.snap", "location.get", ...],
 │         caps: ["canvas", "camera", "voiceWake", ...]
 │       }                          │
 │                                  │
 │  <── hello-ok ─────────────────  │  (snapshot, features, deviceToken, policy)
 │                                  │
 │  === Connected ================  │
 │                                  │
 │  <── node.invoke.request ──────  │  (command: "camera.snap", params)
 │  ─── node.invoke.result ───────> │  (ok: true, payload: {base64, format})
 │                                  │
 │  ─── node.event ───────────────> │  (event: "agent.request", message)
 │  ─── node.event ───────────────> │  (event: "voice.transcript", text)
 │                                  │
 │  <── voicewake.changed ────────  │  (trigger configuration update)
 │  <── tick ──────────────────────  │  (keepalive)
 │  <── shutdown ──────────────────  │  (graceful disconnect)
```

### Device Identity Authentication

All platforms use the same cryptographic identity:

| Platform | Key Type | Storage |
|---|---|---|
| iOS | Curve25519 (CryptoKit) | Application Support / Keychain |
| macOS | Curve25519 (CryptoKit) | Application Support / Keychain |
| Android | Curve25519 (Android crypto) | Encrypted SharedPreferences |

Device ID = SHA-256 hash of public key (hex-encoded, stable across sessions)

### Capabilities Exposed by Platform

| Capability | iOS | macOS | Android |
|---|---|---|---|
| `canvas` (A2UI rendering) | WKWebView | WKWebView | Android WebView |
| `camera` (photo/video) | AVCaptureSession | AVCaptureSession | CameraX |
| `screen` (recording) | ReplayKit | ScreenCaptureKit | MediaProjection |
| `location` (GPS) | CLLocationManager | CLLocationManager | FusedLocationProvider |
| `voiceWake` (trigger phrases) | SFSpeechRecognizer | SFSpeechRecognizer | SpeechRecognizer |
| `device` (battery, network) | UIDevice | ProcessInfo | Android APIs |
| `photos` (library) | PHPhotoLibrary | -- | -- |
| `contacts` (address book) | CNContactStore | -- | -- |
| `calendar` (events) | EKEventStore | -- | -- |
| `reminders` | EKReminder | -- | -- |
| `motion` (sensors) | CMMotionActivity | -- | -- |
| SMS | -- | -- | ContentProvider |
| Talk (voice conversation) | SFSpeechRecognizer + TTS | SFSpeechRecognizer + TTS | SpeechRecognizer + TTS |

### Dual Gateway Pattern

Both iOS and Android maintain **two simultaneous gateway connections**:

1. **Node connection** (role: `"node"`) -- Handles `node.invoke` commands from the gateway. Used for device capabilities (camera, location, screen, etc.). Commands may block for hardware access (permission prompts, sensor reads).

2. **Operator connection** (role: `"operator"`) -- Handles chat, configuration, voice wake sync, agent events. Never blocked by slow hardware operations.

This separation prevents a slow camera capture from blocking chat message delivery.

---

## Build & Distribution

### iOS

- **Build tool:** Xcode with XcodeGen for project generation
- **Swift version:** 6.2 with strict concurrency
- **Dependencies:** OpenClawKit (local SPM), ElevenLabsKit, Textual
- **Distribution:** TestFlight / App Store
- **Code quality:** SwiftLint + SwiftFormat (configs at repo root)

### macOS

- **Build tool:** Xcode with XcodeGen
- **Swift version:** 6.2 with strict concurrency
- **Dependencies:** OpenClawKit (local SPM), MenuBarExtraAccess, Sparkle (auto-update)
- **Distribution:** Direct download with notarization, Sparkle auto-updates
- **Login item:** LaunchAgent plist for launch-at-login

### Android

- **Build tool:** Gradle with Kotlin 2.0.x
- **Compose:** Jetpack Compose with Material3
- **Target/Min SDK:** 36 / 31
- **ABI support:** arm64-v8a, armeabi-v7a, x86, x86_64
- **JDK:** 21
- **Dependencies:** OkHttp (WebSocket), kotlinx.serialization, CameraX, MediaProjection, NSD
- **Distribution:** APK with self-update mechanism
- **Minification:** R8/ProGuard enabled

### Shared (OpenClawKit)

- **Build tool:** Swift Package Manager
- **Platforms:** iOS 18+, macOS 15+
- **Strict concurrency:** Enabled via `StrictConcurrency` upcoming feature flag
- **Testing:** `OpenClawKitTests` target with Swift Testing framework

---

## File Reference Index

| Path | Description |
|---|---|
| `apps/ios/Sources/OpenClawApp.swift` | iOS app entry point |
| `apps/ios/Sources/Model/NodeAppModel.swift` | iOS central state model |
| `apps/ios/Sources/Gateway/GatewayConnectionController.swift` | iOS gateway discovery & connection |
| `apps/ios/Sources/Capabilities/NodeCapabilityRouter.swift` | iOS command routing |
| `apps/ios/Sources/Services/NodeServiceProtocols.swift` | iOS service protocol definitions |
| `apps/ios/Sources/RootTabs.swift` | iOS tab structure |
| `apps/ios/Sources/Chat/IOSGatewayChatTransport.swift` | iOS chat transport |
| `apps/ios/Sources/Voice/VoiceWakeManager.swift` | iOS voice wake |
| `apps/ios/Sources/Voice/TalkModeManager.swift` | iOS talk mode |
| `apps/ios/Sources/Screen/ScreenController.swift` | iOS canvas controller |
| `apps/macos/Sources/OpenClaw/MenuBar.swift` | macOS app entry point |
| `apps/macos/Sources/OpenClaw/AppState.swift` | macOS central state |
| `apps/macos/Sources/OpenClaw/GatewayProcessManager.swift` | macOS gateway subprocess |
| `apps/macos/Sources/OpenClaw/GatewayConnectivityCoordinator.swift` | macOS connection mode |
| `apps/macos/Sources/OpenClaw/CanvasWindowController.swift` | macOS canvas window |
| `apps/macos/Sources/OpenClaw/ControlChannel.swift` | macOS operator gateway RPC |
| `apps/android/app/src/main/java/ai/openclaw/android/NodeRuntime.kt` | Android central coordinator |
| `apps/android/app/src/main/java/ai/openclaw/android/MainViewModel.kt` | Android MVVM view model |
| `apps/android/app/src/main/java/ai/openclaw/android/node/InvokeDispatcher.kt` | Android command routing |
| `apps/android/app/src/main/java/ai/openclaw/android/gateway/GatewaySession.kt` | Android WebSocket client |
| `apps/android/app/src/main/java/ai/openclaw/android/gateway/GatewayDiscovery.kt` | Android mDNS discovery |
| `apps/shared/OpenClawKit/Package.swift` | Shared Swift package definition |
| `apps/shared/OpenClawKit/Sources/OpenClawKit/GatewayNodeSession.swift` | Shared gateway actor |
| `apps/shared/OpenClawKit/Sources/OpenClawKit/BridgeFrames.swift` | Shared frame types |
| `apps/shared/OpenClawKit/Sources/OpenClawKit/DeviceIdentity.swift` | Shared device identity (Curve25519) |
| `apps/shared/OpenClawKit/Sources/OpenClawKit/Capabilities.swift` | Capability enum |
| `apps/shared/OpenClawKit/Sources/OpenClawChatUI/ChatTransport.swift` | Chat transport protocol |
| `apps/shared/OpenClawKit/Sources/OpenClawChatUI/ChatViewModel.swift` | Shared chat view model |
| `apps/shared/OpenClawKit/Sources/OpenClawProtocol/GatewayModels.swift` | Auto-generated protocol types |
