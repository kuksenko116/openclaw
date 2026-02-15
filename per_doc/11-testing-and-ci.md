# Testing and CI/CD

## Test Framework: Vitest v4.0.18

- **Coverage engine:** V8 with enforced thresholds
  - Lines: 70%
  - Functions: 70%
  - Branches: 55%
  - Statements: 70%
- **Scale:** 1,111 test files across the codebase

### Test Configuration Files

- **vitest.config.ts** -- Base configuration shared by all test profiles. Sets a 120-second default timeout, uses the `process` pool with `forks` mode, and scales max workers depending on whether the run is inside CI.
- **vitest.unit.config.ts** -- Unit test profile. Includes `src/**` and `extensions/**` source paths. Excludes gateway-specific and extension-specific sources from the core unit suite so they run in their own lanes.
- **vitest.e2e.config.ts** -- End-to-end test profile. Uses the `vmForks` pool. Worker counts scale dynamically: 2--4 workers in CI, 4--16 workers on local machines.
- **vitest.live.config.ts** -- Live API test profile. Tests in this profile hit real external APIs and only execute when the `OPENCLAW_LIVE_TEST` environment variable is set.
- **vitest.gateway.config.ts** -- Gateway-specific test profile. Isolates gateway tests from the core unit suite.
- **vitest.extensions.config.ts** -- Extension test profile. Runs extension tests independently from the core codebase.

### Test Organization

| Convention | Scope | Example |
|---|---|---|
| `src/**/*.test.ts`, `extensions/**/*.test.ts` | Unit tests | `src/routing/resolve-route.test.ts` |
| `**/*.e2e.test.ts` | End-to-end tests | `src/gateway/gateway.e2e.test.ts` |
| `**/*.live.test.ts` | Live API tests (requires `OPENCLAW_LIVE_TEST=1`) | `src/providers/openai.live.test.ts` |
| `*.node.test.ts` | Node-only tests (skip browser environments) | `src/fs/watcher.node.test.ts` |
| `*.browser.test.ts` | Browser-only tests | `src/ui/render.browser.test.ts` |

### Parallel Test Orchestration (scripts/test-parallel.mjs)

The parallel orchestrator splits the full test suite into separate lanes that can run concurrently in CI:

- **unit-fast** -- Uses the `vmForks` pool for maximum throughput on pure-logic unit tests.
- **unit-isolated** -- Uses the `forks` pool for unit tests that need full process isolation (filesystem side effects, global state, etc.).
- **extensions** -- Uses the `vmForks` pool for all extension tests.
- **gateway** -- Uses the `vmForks` pool for gateway tests.

Additional behavior:

- Handles a Node 24 regression with the VM runtime by detecting the Node version and adjusting pool settings.
- Produces JSON reports that are uploaded as CI artifacts for post-run analysis.
- Slowest-test analysis is available via `vitest-slowest.mjs`, which parses JSON reports and surfaces the tests with the highest wall-clock times.

### Test Setup (test/setup.ts)

The shared test setup module runs before every test file and performs:

- Sets `process.setMaxListeners(128)` to avoid spurious warnings in highly concurrent test scenarios.
- Creates isolated temporary home directories per test so that file-system side effects do not leak between tests.
- Installs a warning filter that suppresses known noisy deprecation warnings from third-party dependencies.
- Stubs outbound adapters (HTTP clients, channel APIs, etc.) so that unit tests never make real network calls.

---

## CI/CD Pipeline (GitHub Actions)

### Main CI (ci.yml)

- **Concurrency:** Each PR gets its own concurrency group; new pushes cancel in-progress runs for the same PR.
- **Smart job skipping:** A `docs-scope` job detects docs-only changes and short-circuits the rest of the pipeline when only documentation files are modified.
- **Changed scope detection:** A `changed-scope` job determines which areas of the codebase were touched and enables or disables downstream test lanes accordingly.

Jobs in the pipeline:

| Job | Purpose |
|---|---|
| `docs-scope` | Detect docs-only changes; skip remaining jobs if true |
| `changed-scope` | Identify changed areas (core, gateway, extensions, etc.) |
| `build-artifacts` | Build distributable artifacts |
| `check` | Typecheck (`tsc`), lint (`oxlint`), format (`oxfmt`) |
| `checks` | Node test suite + Bun test suite + protocol check |
| `checks-windows` | Windows-specific test suite |
| `check-docs` | Documentation build and link validation |
| `secrets` | Run `detect-secrets` to prevent credential leaks |
| `macos` | TypeScript tests + Swift lint + Swift build + Swift tests |
| `android` | Gradle build and test |
| `release-check` | `npm pack` dry run to verify publishable package |

### Docker Release (docker-release.yml)

- **Trigger:** Pushes to `main` branch or tags matching `v*`.
- **Platforms:** Builds for both `amd64` and `arm64` architectures.
- **Registry:** Images are pushed to `ghcr.io`.
- **Multi-platform manifest:** After building platform-specific images, a combined manifest is created so that `docker pull` automatically selects the correct architecture.

### Other Workflows

| Workflow | Purpose |
|---|---|
| `install-smoke.yml` | Docker installation smoke tests -- verifies that the published Docker image starts correctly |
| `formal-conformance.yml` | Runs formal model checks sourced from an external repository |
| `workflow-sanity.yml` | Validates that workflow YAML files do not contain tabs (which can cause silent parsing issues) |
| `labeler.yml` | Automatically labels PRs based on changed file paths |
| `auto-response.yml` | Posts template responses on issues/PRs matching certain criteria |
| `stale.yml` | Closes stale issues after 7 days of inactivity and stale PRs after 5 days |

### Custom Actions (.github/actions/)

- **setup-node-env** -- Sets up Node 22, pnpm 10.23.0, optionally installs Bun, and checks out submodules.
- **setup-pnpm-store-cache** -- Caches the pnpm content-addressable store to speed up installs.
- **detect-docs-changes** -- Compares the current commit range against documentation paths and outputs a boolean flag indicating whether only docs changed.

---

## Deployment

### Docker

- **Dockerfile** -- Based on `node:22-bookworm`. Runs `pnpm install`, switches to a non-root user (`node`, UID 1000), and sets the default command to `gateway --allow-unconfigured`.
- **Dockerfile.sandbox** -- Based on `debian:bookworm-slim`. Creates a dedicated `sandbox` user and installs a minimal tool set: `bash`, `curl`, `git`, `jq`, `python3`, and `ripgrep`. Used for running untrusted agent workloads in isolation.
- **docker-compose.yml** -- Defines two services:
  - `openclaw-gateway`: Exposes ports 18789 (HTTP/WS) and 18790 (auxiliary).
  - `openclaw-cli`: Interactive CLI container linked to the gateway.

### Platforms

| Platform | Mechanism |
|---|---|
| Fly.io | `fly.toml` configuration |
| Render.com | `render.yaml` blueprint |
| macOS | `launchd` service + menu bar app |
| Linux | `systemd` daemon + CLI |
| npm | Published as the `openclaw` package |

### Dependency Management

Dependabot is configured for weekly updates across all ecosystems:

| Ecosystem | Group Strategy | Open PR Limit |
|---|---|---|
| npm | production, development | 10 |
| GitHub Actions | platform-specific | 5 |
| Swift | platform-specific | 5 |
| Gradle | platform-specific | 5 |

Dependencies are grouped into `production`, `development`, and `platform-specific` categories so that related version bumps land in a single PR.
