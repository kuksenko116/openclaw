# Media & Content Processing

This document covers the media understanding pipeline, link understanding, text-to-speech, browser automation, markdown processing, the media pipeline, and the canvas host subsystem.

---

## Media Understanding (`src/media-understanding/`)

The media understanding subsystem provides automatic analysis of inbound media attachments -- images, audio, and video -- by routing them through a multi-provider pipeline. The system supports both cloud API providers and local CLI-based tools (whisper-cli, sherpa-onnx, Gemini CLI).

### Core Modules

#### `types.ts` -- Type Definitions

Defines the foundational types for the entire media understanding pipeline:

```typescript
// The three capabilities the system can process
type MediaUnderstandingCapability = "image" | "audio" | "video";

// Output kinds map 1:1 with capabilities
type MediaUnderstandingKind =
  | "audio.transcription"
  | "video.description"
  | "image.description";

// A media attachment extracted from an inbound message
type MediaAttachment = {
  path?: string;       // Local file path
  url?: string;        // Remote URL
  mime?: string;       // MIME type hint
  index: number;       // Position in the attachment list
  alreadyTranscribed?: boolean; // Set by audio-preflight to prevent double-processing
};

// The result of processing a single attachment
type MediaUnderstandingOutput = {
  kind: MediaUnderstandingKind;
  attachmentIndex: number;
  text: string;
  provider: string;
  model?: string;
};

// Decision tracking: records what happened during processing
type MediaUnderstandingDecisionOutcome =
  | "success" | "skipped" | "disabled" | "no-attachment" | "scope-deny";

type MediaUnderstandingModelDecision = {
  provider?: string;
  model?: string;
  type: "provider" | "cli";
  outcome: "success" | "skipped" | "failed";
  reason?: string;
};

// Provider interface -- each provider implements one or more of these
type MediaUnderstandingProvider = {
  id: string;
  capabilities?: MediaUnderstandingCapability[];
  transcribeAudio?: (req: AudioTranscriptionRequest) => Promise<AudioTranscriptionResult>;
  describeVideo?: (req: VideoDescriptionRequest) => Promise<VideoDescriptionResult>;
  describeImage?: (req: ImageDescriptionRequest) => Promise<ImageDescriptionResult>;
};
```

#### `runner.ts` -- Media Analysis Orchestration

The main orchestrator. Exports `runCapability()`, which processes a single capability (image, audio, or video) across all matching attachments:

1. **Check enabled**: If the capability is explicitly disabled in config (`enabled: false`), returns a `disabled` decision immediately.
2. **Select attachments**: Filters attachments by type (image/audio/video) using the attachment policy. Supports `prefer` ordering (`first`, `last`, `path`, `url`) and `maxAttachments` limits (default: 1).
3. **Scope check**: Evaluates scope rules (channel, chat type, session key prefix) to allow or deny processing.
4. **Vision skip**: For image capability, checks whether the primary agent model supports vision natively. If it does, skips separate image description since the model will see the image directly in context.
5. **Resolve model entries**: Tries explicit config entries first, then falls back to auto-resolution:
   - Active model entry (the model currently assigned to the agent)
   - Local CLI tools (sherpa-onnx > whisper-cli > whisper for audio; gemini CLI for all)
   - API key probing (checks which providers have valid API keys configured)
6. **Execute entries**: Iterates through resolved entries, calling either `runCliEntry()` or `runProviderEntry()`, stopping on the first success.
7. **Return outputs and decisions**: Produces `RunCapabilityResult` with outputs and a full decision trace.

Key auto-resolution provider priority:
- **Audio**: openai, groq, deepgram, google
- **Image**: openai, anthropic, google, minimax, zai
- **Video**: google (only)

Default image models by provider:
```typescript
{ openai: "gpt-5-mini", anthropic: "claude-opus-4-6", google: "gemini-3-flash-preview",
  minimax: "MiniMax-VL-01", zai: "glm-4.6v" }
```

Also exports `resolveAutoImageModel()` for determining which image model is available without running processing, and utility functions `buildProviderRegistry()`, `normalizeMediaAttachments()`, and `createMediaAttachmentCache()`.

#### `runner.entries.ts` -- Analysis Entry Points

Contains the execution logic for individual model entries:

- **`runProviderEntry()`**: Resolves API key, timeout, prompt, and max bytes from config cascade (entry > capability config > global config > defaults). For images, calls `provider.describeImage()` or the generic `describeImageWithModel()` fallback. For audio, calls `provider.transcribeAudio()`. For video, estimates base64 payload size and enforces a 70 MB cap before calling `provider.describeVideo()`. Supports per-provider query options (e.g., Deepgram's `detect_language`, `punctuate`, `smart_format`).

- **`runCliEntry()`**: Executes a CLI command with template-expanded arguments. Template variables include `{{MediaPath}}`, `{{MediaDir}}`, `{{OutputDir}}`, `{{OutputBase}}`, `{{Prompt}}`, `{{MaxChars}}`. Resolves output by checking file outputs (whisper-cli writes `.txt` files, whisper uses `--output_dir`), parsing Gemini JSON responses, or extracting sherpa-onnx JSON text fields. Cleanup of temp directories happens in a `finally` block.

- **`buildModelDecision()`**: Constructs a decision record for logging.
- **`formatDecisionSummary()`**: Produces a human-readable one-line summary like `"image: success (1/1) via openai/gpt-5-mini"`.

Default limits:
```typescript
DEFAULT_MAX_BYTES = { image: 10 MB, audio: 20 MB, video: 50 MB }
DEFAULT_TIMEOUT_SECONDS = { image: 60, audio: 60, video: 120 }
DEFAULT_MAX_CHARS = { image: 500, audio: undefined, video: 500 }
DEFAULT_PROMPT = { image: "Describe the image.", audio: "Transcribe the audio.",
                   video: "Describe the video." }
```

#### `resolve.ts` -- Media Type Resolution & Configuration

Resolves configuration values from the layered config cascade and determines which model entries apply:

- **`resolveModelEntries()`**: Merges capability-specific models with shared models (`cfg.tools.media.models`). For shared models, checks `capabilities` on the entry or infers them from the provider registry. Filters to only entries that support the requested capability.
- **`resolveScopeDecision()`**: Delegates to `scope.ts` to evaluate scope rules against session context.
- **`resolveConcurrency()`**: Returns the configured concurrency limit (default: 2).
- **`resolveEntriesWithActiveFallback()`**: Falls back to the active agent model when no explicit entries are configured but the capability is explicitly enabled.
- **`resolveTimeoutMs()`**, **`resolvePrompt()`**, **`resolveMaxChars()`**, **`resolveMaxBytes()`**: Standard resolution with config cascade and defaults.

#### `attachments.ts` -- Attachment Handling

Manages the lifecycle of media attachment data:

- **`normalizeAttachments()`**: Extracts `MediaAttachment[]` from `MsgContext`. Handles `MediaPaths`/`MediaUrls` arrays or single `MediaPath`/`MediaUrl` fields. Normalizes `file://` URIs to filesystem paths.
- **`resolveAttachmentKind()`**: Determines the attachment kind (image/audio/video/document/unknown) from MIME type first, then file extension.
- **`selectAttachments()`**: Filters and orders attachments for a given capability. Respects `prefer` ordering and `maxAttachments` policy. Skips already-transcribed audio attachments to prevent double-processing after audio preflight.
- **`MediaAttachmentCache`**: Caches buffer reads and temp file paths to avoid re-downloading or re-reading attachments across capabilities:
  - `getBuffer()`: Returns a `Buffer` with MIME detection, enforcing `maxBytes` limits. Reads from local path if available, otherwise fetches from URL with timeout.
  - `getPath()`: Returns a filesystem path, writing to a temp file if only URL/buffer is available.
  - `cleanup()`: Removes all temp files created during processing.

#### `video.ts` -- Video Processing

Utility functions for video file handling:

```typescript
// Estimates base64-encoded size from raw byte count
function estimateBase64Size(bytes: number): number;

// Resolves the maximum allowed base64 payload (capped at 70 MB)
function resolveVideoMaxBase64Bytes(maxBytes: number): number;
```

#### `audio-preflight.ts` -- Audio Validation & Pre-Transcription

The `transcribeFirstAudio()` function runs audio transcription BEFORE mention checking in group chats. This allows voice-note-only messages in groups with `requireMention: true` to be processed -- the transcript is used for mention detection.

Flow:
1. Check audio config is enabled
2. Normalize attachments from context
3. Find first non-transcribed audio attachment
4. Run the full `runCapability("audio", ...)` pipeline
5. Extract transcript text
6. Mark the attachment as `alreadyTranscribed = true` to prevent double-processing
7. Return transcript (or `undefined` on failure -- non-blocking)

#### `scope.ts` -- Scope Policy Evaluation

Evaluates scope rules that control which contexts (channels, chat types, session key prefixes) allow media understanding:

```typescript
function resolveMediaUnderstandingScope(params: {
  scope?: MediaUnderstandingScopeConfig;
  sessionKey?: string;
  channel?: string;
  chatType?: string;
}): "allow" | "deny";
```

Rules are evaluated in order; the first matching rule wins. If no rule matches, the `default` action applies (defaults to `"allow"`).

#### `concurrency.ts` -- Concurrency Control

```typescript
async function runWithConcurrency<T>(tasks: Array<() => Promise<T>>, limit: number): Promise<T[]>;
```

Worker-pool pattern: spawns `limit` workers that pull tasks from a shared index. Errors are caught and logged but do not prevent other tasks from running.

#### `format.ts` -- Output Formatting

Formats media understanding outputs into the message body:

- **`formatMediaUnderstandingBody()`**: Produces structured sections like `[Audio]\nTranscript:\n...` or `[Image]\nDescription:\n...`. Handles multiple outputs with numbering (`Audio 1/2`, `Audio 2/2`). Extracts user text from `<media:...>` placeholder tokens.
- **`formatAudioTranscripts()`**: Formats audio outputs for the `Transcript` context field.
- **`extractMediaUserText()`**: Strips `<media:...>` placeholders and returns the remaining user text.

#### `apply.ts` -- Context Application

The top-level `applyMediaUnderstanding()` function orchestrates the full pipeline:

1. Extract user text from the original message body
2. Normalize attachments from `MsgContext`
3. Build provider registry and attachment cache
4. Run all three capabilities (image, audio, video) through `runWithConcurrency()` with the configured concurrency limit (default: 2)
5. Collect outputs and decisions
6. If outputs exist:
   - Format body with `formatMediaUnderstandingBody()`
   - Set `ctx.Transcript` for audio outputs
   - Restore `ctx.CommandBody`/`ctx.RawBody` to user text (not transcript)
   - Store outputs in `ctx.MediaUnderstanding`
7. Extract file content from non-media attachments (PDFs, text files, CSV, JSON, YAML, XML, etc.):
   - Detects text-like content via UTF-8/UTF-16/CP1252 heuristic analysis
   - Applies MIME allowlist filtering (configurable or default set)
   - Extracts content using `extractFileContentFromSource()` (handles PDF page rendering)
   - Wraps extracted text in `<file name="..." mime="...">` XML blocks with injection-safe escaping
   - Respects `maxBytes`, `maxChars`, `maxRedirects`, and PDF-specific limits (`maxPages`, `maxPixels`, `minTextChars`)
8. Finalize inbound context to ensure body updates propagate

Returns `ApplyMediaUnderstandingResult` with outputs, decisions, and boolean flags for each applied type.

#### `defaults.ts` -- Default Configuration Values

Centralizes all default values:

```typescript
DEFAULT_MEDIA_CONCURRENCY = 2;
CLI_OUTPUT_MAX_BUFFER = 5 MB;
DEFAULT_MAX_CHARS = 500;
DEFAULT_AUDIO_MODELS = { groq: "whisper-large-v3-turbo", openai: "gpt-4o-mini-transcribe",
                          deepgram: "nova-3" };
AUTO_AUDIO_KEY_PROVIDERS = ["openai", "groq", "deepgram", "google"];
AUTO_IMAGE_KEY_PROVIDERS = ["openai", "anthropic", "google", "minimax", "zai"];
AUTO_VIDEO_KEY_PROVIDERS = ["google"];
```

### Multi-Provider Support (`providers/`)

The `providers/index.ts` module builds a registry of all supported providers and normalizes provider IDs (e.g., `"gemini"` maps to `"google"`).

Seven providers are registered:

| Provider | ID | Audio | Image | Video |
|---|---|---|---|---|
| **Google (Gemini)** | `google` | Transcription via Gemini, inline-data upload, video description | Via Gemini vision models | Video description via Gemini |
| **OpenAI** | `openai` | Whisper-compatible transcription endpoint | Via vision models (gpt-5-mini, etc.) | -- |
| **Anthropic** | `anthropic` | -- | Via Claude vision models | -- |
| **Groq** | `groq` | Whisper-large-v3-turbo transcription | -- | -- |
| **Deepgram** | `deepgram` | Nova-3 transcription with configurable query params (detect_language, punctuate, smart_format) | -- | -- |
| **MiniMax** | `minimax` | -- | MiniMax-VL-01 vision | -- |
| **ZAI** | `zai` | -- | GLM-4.6v vision | -- |

Provider capabilities (image/audio/video) are declared on each provider and used for auto-detection when shared model entries do not specify explicit capabilities.

### Capabilities Summary

- **Image description**: Sends image buffer to a vision model with a prompt (default: "Describe the image."). Skipped when the primary agent model supports vision natively (image is injected directly into agent context instead).
- **Audio transcription**: Sends audio buffer to a speech-to-text service. Supports multiple backends: cloud APIs (OpenAI Whisper, Groq, Deepgram, Google), local CLIs (whisper-cli, whisper, sherpa-onnx), and the Gemini CLI.
- **Video description**: Sends video buffer (base64-encoded, capped at 70 MB) to a vision model with a prompt (default: "Describe the video."). Currently only Google/Gemini supports this capability.
- **File content extraction**: Non-media attachments (PDFs, text files, structured data) are extracted into `<file>` XML blocks appended to the message body.

### End-to-End Flow

```
1. Inbound message arrives with attachments
   |
2. normalizeAttachments(ctx) -> MediaAttachment[]
   |
3. For each capability (image, audio, video):
   a. Check enabled/disabled
   b. selectAttachments() - filter by type, apply ordering/limits
   c. resolveScopeDecision() - check channel/chatType/keyPrefix rules
   d. Check vision skip (image only: skip if primary model has vision)
   e. resolveModelEntries() -> try config entries
   f. resolveAutoEntries() -> fallback: active model, local CLI, API key probing
   g. runAttachmentEntries() -> iterate entries, first success wins
      - CLI entry: template args, exec, parse output
      - Provider entry: resolve API key, call provider, trim output
   |
4. Collect outputs across all capabilities
   |
5. formatMediaUnderstandingBody() -> structured sections in message body
   |
6. Extract file content from document attachments
   |
7. finalizeInboundContext() -> propagate body updates
```

---

## Link Understanding (`src/link-understanding/`)

The link understanding subsystem detects URLs in inbound messages and runs configurable CLI tools to extract context from those URLs.

### Core Modules

#### `runner.ts` -- Link Extraction & Analysis

`runLinkUnderstanding()` is the main entry point:

1. Check if link understanding is enabled in config (`tools.links.enabled`)
2. Evaluate scope policy (reuses media understanding scope logic)
3. Extract links from message text using `detect.ts`
4. If no CLI models are configured, return the detected URLs without outputs
5. For each detected URL, run CLI entries in order until one produces output
6. CLI entries use the same `runExec()` + template expansion pattern as media understanding, with `{{LinkUrl}}` as the URL template variable

```typescript
type LinkUnderstandingResult = {
  urls: string[];      // All detected URLs
  outputs: string[];   // CLI tool outputs (one per successfully processed URL)
};
```

#### `detect.ts` -- Link Detection

`extractLinksFromMessage()` finds URLs in message text:

1. Strip markdown link syntax `[text](url)` to avoid duplicates
2. Match bare `https?://` URLs
3. Filter out blocked hosts: loopback, private IPs, link-local addresses, cloud metadata endpoints
4. Deduplicate
5. Respect `maxLinks` limit (default from `DEFAULT_MAX_LINKS`)

SSRF protection is integrated via `isBlockedHostname()` and `isPrivateIpAddress()` from `infra/net/ssrf.ts`.

#### `format.ts` -- Link Formatting

```typescript
function formatLinkUnderstandingBody(params: { body?: string; outputs: string[] }): string;
```

Appends link understanding outputs to the message body, separated by double newlines.

#### `apply.ts` -- Context Injection

`applyLinkUnderstanding()` ties it all together:

1. Run link understanding
2. Store outputs in `ctx.LinkUnderstanding`
3. Update `ctx.Body` with formatted link context
4. Call `finalizeInboundContext()` to propagate changes

---

## Text-to-Speech (`src/tts/`)

The TTS subsystem converts agent response text into audio, with support for three providers and extensive configuration via both config files and inline directives.

### Providers

| Provider | Implementation | Default Model/Voice | Notes |
|---|---|---|---|
| **OpenAI** | `openaiTTS()` in `tts-core.ts` | `gpt-4o-mini-tts` / `alloy` | Supports custom endpoints via `OPENAI_TTS_BASE_URL` (e.g., Kokoro, LocalAI). Models: `gpt-4o-mini-tts`, `tts-1`, `tts-1-hd`. 14 voices. |
| **ElevenLabs** | `elevenLabsTTS()` in `tts-core.ts` | `eleven_multilingual_v2` / `pMsXgVXv3BLzUgSXRplE` | Full voice settings control (stability, similarity boost, style, speed, speaker boost). Supports seed, text normalization, language code. |
| **Edge** (local) | `edgeTTS()` in `tts-core.ts` | `en-US-MichelleNeural` / `en-US` | Uses Microsoft Edge TTS (no API key needed). Configurable output format, pitch, rate, volume, proxy. Fallback format: `audio-24khz-48kbitrate-mono-mp3`. |

Provider fallback: The system tries the primary provider first, then falls back through the others in order. Provider selection priority: user prefs > config > auto-detect (OpenAI key > ElevenLabs key > Edge).

### Configuration

```typescript
type ResolvedTtsConfig = {
  auto: "off" | "always" | "inbound" | "tagged";  // When TTS activates
  mode: "final" | "partial";                       // Apply to final reply only or all blocks
  provider: "openai" | "elevenlabs" | "edge";
  providerSource: "config" | "default";
  summaryModel?: string;                           // Model for long-text summarization
  modelOverrides: ResolvedTtsModelOverrides;        // Inline directive permissions
  elevenlabs: {
    apiKey?: string;
    baseUrl: string;       // default: "https://api.elevenlabs.io"
    voiceId: string;       // default: "pMsXgVXv3BLzUgSXRplE"
    modelId: string;       // default: "eleven_multilingual_v2"
    seed?: number;
    applyTextNormalization?: "auto" | "on" | "off";
    languageCode?: string;
    voiceSettings: {
      stability: number;        // 0-1, default 0.5
      similarityBoost: number;  // 0-1, default 0.75
      style: number;            // 0-1, default 0.0
      useSpeakerBoost: boolean; // default true
      speed: number;            // 0.5-2, default 1.0
    };
  };
  openai: {
    apiKey?: string;
    model: string;   // default: "gpt-4o-mini-tts"
    voice: string;   // default: "alloy"
  };
  edge: {
    enabled: boolean;
    voice: string;             // default: "en-US-MichelleNeural"
    lang: string;              // default: "en-US"
    outputFormat: string;      // default: "audio-24khz-48kbitrate-mono-mp3"
    outputFormatConfigured: boolean;
    pitch?: string;
    rate?: string;
    volume?: string;
    saveSubtitles: boolean;
    proxy?: string;
    timeoutMs?: number;
  };
  prefsPath?: string;
  maxTextLength: number;   // default: 4096
  timeoutMs: number;       // default: 30000
};
```

User preferences are stored in a JSON file (default: `~/.openclaw/settings/tts.json`) and override config values for auto mode, provider, max length, and summarization.

### Inline Directives

Agents can embed TTS control directives in their response text:

| Directive | Description |
|---|---|
| `[[tts]]` | Trigger TTS (used with `auto: "tagged"`) |
| `[[tts:provider=openai]]` | Override provider |
| `[[tts:voice=coral]]` | Override OpenAI voice |
| `[[tts:voiceId=abc123...]]` | Override ElevenLabs voice ID |
| `[[tts:model=tts-1-hd]]` | Override model |
| `[[tts:stability=0.8]]` | Override voice settings |
| `[[tts:speed=1.5]]` | Override speed |
| `[[tts:seed=12345]]` | Set ElevenLabs seed |
| `[[tts:text]]...[[/tts:text]]` | Specify exact text to speak (different from displayed text) |

Each directive type can be individually allowed or denied via `modelOverrides`:

```typescript
type ResolvedTtsModelOverrides = {
  enabled: boolean;
  allowText: boolean;
  allowProvider: boolean;
  allowVoice: boolean;
  allowModelId: boolean;
  allowVoiceSettings: boolean;
  allowNormalization: boolean;
  allowSeed: boolean;
};
```

### Pipeline

```
1. Parse [[tts:...]] directives from response text
   |
2. Check auto mode:
   - "off": skip
   - "always": proceed
   - "inbound": proceed only if inbound had audio
   - "tagged": proceed only if [[tts]] directive present
   |
3. Check mode ("final" vs "partial") against reply kind
   |
4. Skip if text < 10 chars, has media URLs, or contains "MEDIA:"
   |
5. If text > maxLength (default 1500):
   a. If summarization enabled: call LLM to summarize
   b. Else: truncate to maxLength
   |
6. Strip markdown formatting (### -> plain text)
   |
7. Call provider (with fallback chain):
   - Edge: write to temp file, schedule 5-minute cleanup
   - OpenAI/ElevenLabs: buffer response, write to temp file
   |
8. Return audio file path in payload
   - Telegram: use .opus format, mark as voice-compatible
   - Other channels: use .mp3 format
```

Channel-specific output formats:
- **Telegram**: Opus @ 48kHz/64kbps (voice-note optimized, marked as `audioAsVoice`)
- **Default**: MP3 @ 44.1kHz/128kbps
- **Telephony**: PCM raw audio (OpenAI: 24kHz, ElevenLabs: 22.05kHz)

---

## Browser Automation (`src/browser/`)

A comprehensive browser control system built on Playwright and Chrome DevTools Protocol (CDP), providing AI-friendly page interaction capabilities.

### Architecture

The browser subsystem is organized into several layers:

#### Playwright AI Layer (`pw-ai.ts`)

The main export module that re-exports all browser control functions. Key capabilities include:

**Snapshot modes** (for AI consumption):
- `snapshotAiViaPlaywright()` -- Semantic AI snapshot (structured page representation)
- `snapshotAriaViaPlaywright()` -- ARIA accessibility tree snapshot
- `snapshotRoleViaPlaywright()` -- Role-based accessibility snapshot

**Interactions**:
- `clickViaPlaywright()`, `hoverViaPlaywright()`, `dragViaPlaywright()`
- `fillFormViaPlaywright()`, `typeViaPlaywright()`, `pressKeyViaPlaywright()`
- `selectOptionViaPlaywright()`, `setInputFilesViaPlaywright()`
- `navigateViaPlaywright()`, `scrollIntoViewViaPlaywright()`

**Screenshots**:
- `takeScreenshotViaPlaywright()` -- Standard screenshot
- `screenshotWithLabelsViaPlaywright()` -- Screenshot with labeled elements

**Activity tracking**:
- `getConsoleMessagesViaPlaywright()` -- Capture console output
- `getNetworkRequestsViaPlaywright()` -- Capture network requests
- `getPageErrorsViaPlaywright()` -- Capture page errors

**Page management**:
- `createPageViaPlaywright()`, `closePageViaPlaywright()`
- `listPagesViaPlaywright()`, `focusPageByTargetIdViaPlaywright()`
- `getPageForTargetId()`, `ensurePageState()`

**Storage/state**:
- `cookiesGetViaPlaywright()`, `cookiesSetViaPlaywright()`, `cookiesClearViaPlaywright()`
- `storageGetViaPlaywright()`, `storageSetViaPlaywright()`, `storageClearViaPlaywright()`

**Advanced**:
- `evaluateViaPlaywright()` -- Execute JavaScript in page context
- `pdfViaPlaywright()` -- Generate PDF
- `downloadViaPlaywright()`, `waitForDownloadViaPlaywright()`
- `traceStartViaPlaywright()`, `traceStopViaPlaywright()` -- Playwright tracing
- `armDialogViaPlaywright()`, `armFileUploadViaPlaywright()`

#### Browser Bridge (`bridge-server.ts`)

A sandboxed browser server accessible over HTTP with security measures:

- **Express-based HTTP server** bound to loopback only (`127.0.0.1`)
- **Authentication**: Requires either `authToken` or `authPassword`
- **CSRF protection**: Via `browserMutationGuardMiddleware()` from `csrf.ts`
- **Request abort handling**: Creates per-request `AbortController`, propagates abort signals to route handlers
- **Profile management**: Supports multiple browser profiles with hot-reload

```typescript
type BrowserBridge = {
  server: Server;
  port: number;
  baseUrl: string;
  state: BrowserServerState;
};
```

#### Routes (`routes/`)

Organized route handlers:
- `agent.ts` -- Agent-oriented endpoints (snapshot, act, debug, storage)
- `basic.ts` -- Basic browser operations
- `tabs.ts` -- Tab management
- `dispatcher.ts` -- Request dispatching with abort support

#### Additional Components

- **`chrome.ts` / `chrome.executables.ts`**: Chrome/Chromium executable detection and management
- **`profiles.ts` / `profiles-service.ts`**: Browser profile creation and management with decorations
- **`cdp.ts` / `cdp.helpers.ts`**: Chrome DevTools Protocol integration
- **`pw-session.ts`**: Playwright session management, page-by-target-id resolution, browserless support
- **`control-service.ts` / `control-auth.ts`**: Browser control service with auth token management
- **`extension-relay.ts`**: Browser extension communication relay
- **`screenshot.ts`**: Screenshot capture with element labeling
- **`config.ts`**: Browser configuration resolution
- **`csrf.ts`**: CSRF token generation and validation
- **`http-auth.ts`**: HTTP authentication for browser requests

---

## Markdown Processing (`src/markdown/`)

Utilities for parsing and manipulating markdown content:

- Markdown-to-plain-text conversion for channels that do not support formatting
- Channel-specific formatting (e.g., strip formatting for SMS/plain-text channels)
- Used by the TTS system to strip markdown before speech synthesis (`stripMarkdown()`)

---

## Media Pipeline (`src/media/`)

Low-level media handling utilities used by the media understanding subsystem:

- **MIME detection** (`mime.ts`): Detects MIME types from file buffers and extensions. Classifies into image/audio/video/document categories. Provides `isAudioFileName()`, `kindFromMime()`, `getFileExtension()`.
- **Remote media fetching** (`fetch.ts`): `fetchRemoteMedia()` downloads media from URLs with `maxBytes` enforcement. Returns buffer, content type, and filename.
- **Audio utilities** (`audio.ts`): `isVoiceCompatibleAudio()` checks if an audio file is suitable for voice-note delivery.
- **Input file extraction** (`input-files.ts`): `extractFileContentFromSource()` handles PDF text extraction, text file reading, and structured data parsing. Supports configurable limits for pages, pixels, text length, and MIME allowlists.
- **Size caps enforcement**: Configurable per content type with defaults (10 MB images, 20 MB audio, 50 MB video).
- **Temporary file lifecycle**: `MediaAttachmentCache` manages temp files with `cleanup()` for automatic removal.

---

## Canvas Host (`src/canvas-host/`)

Provides canvas-based A2UI (Agent-to-User Interface) rendering:

- Hosts live interactive UI components generated by the agent
- Renders within the gateway's web interface
- The `canvas-host-url.ts` module in `src/infra/` resolves the URL for the canvas host
- Enables rich visual output beyond text-only responses
