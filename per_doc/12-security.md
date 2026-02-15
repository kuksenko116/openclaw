# Security

## Authentication & Authorization

### Gateway Auth Modes

The gateway supports four authentication modes, configured at startup:

| Mode | Description |
|---|---|
| `none` | No authentication required. Suitable only for local development or fully trusted networks. |
| `token` | Shared secret token. Clients present the token in the connection handshake; the server validates it with a constant-time comparison. |
| `password` | Password-based authentication. Similar to token mode but intended for human-memorable credentials. |
| `trusted-proxy` | Trust an upstream reverse proxy. The gateway reads identity from `X-Forwarded-*` headers and skips its own authentication. |

### Device Identity Authentication

Mobile and desktop clients authenticate using a device identity model:

1. **Device ID derivation** -- The device ID is derived from the client's public key, providing a stable, cryptographically bound identifier.
2. **Signature freshness check** -- Every authentication request includes a timestamp. The server rejects requests where the timestamp deviates more than +/-10 minutes from server time.
3. **Nonce validation** -- For remote (non-local) connections, the server issues a nonce in the `connect.challenge` frame. The client must sign this nonce to prove liveness and prevent replay attacks.
4. **Signature verification versions:**
   - **v1 (legacy):** Simple signature over a fixed payload. Maintained for backward compatibility.
   - **v2 (nonce):** Signature covers the server-issued nonce, providing replay protection.
5. **Device pairing status check** -- After signature verification, the server confirms that the device is in the "paired" state. Unpaired devices are rejected even if their signature is valid.

### Rate Limiting (auth-rate-limit.ts)

- **Window:** 60 seconds.
- **Threshold:** 20 failed authentication attempts per window.
- **Token comparison:** Constant-time (`crypto.timingSafeEqual`) to prevent timing side-channel attacks.
- **Query parameter tokens:** Rejected outright with a `400` error. Tokens must be sent in headers or the WebSocket handshake body, never in the URL, to avoid leaking credentials in server logs and referrer headers.

### Authorization Model

After authentication, clients are assigned a **role** and a set of **scopes**:

**Roles:**

| Role | Description |
|---|---|
| `node` | Mobile devices and embedded hardware. Limited to device-specific operations (camera, location, voice, etc.). |
| `operator` | Humans and applications (CLI, Web UI, API consumers). Full access governed by scopes. |

**Operator Scopes:**

| Scope | Grants |
|---|---|
| `admin` | Full administrative control (configuration, shutdown, user management) |
| `read` | Read access to sessions, history, agent state |
| `write` | Ability to send messages, modify sessions, update agent configuration |
| `approvals` | Ability to approve or deny execution approval requests |
| `pairing` | Ability to pair new devices |

**Method-level access control:** Each gateway method declares its required role and scope. The request router checks these before dispatching to the handler.

---

## DM & Channel Security

### DM Policy

The `dmPolicy` setting controls how the system handles direct messages from unknown contacts:

- **`pairing`** (default) -- New contacts must exchange an approval code before conversations begin. This prevents unsolicited messages from reaching the agent.
- **`allow`** -- Accept DMs from anyone.
- **`block`** -- Reject all DMs from unknown contacts.

### Per-Channel Allowlist/Blocklist

Each channel can define an allowlist and/or blocklist to control which senders can interact with the agent. Entries are matched by multiple source types:

| AllowlistMatch Source Type | Example |
|---|---|
| `wildcard` | `*` (match all senders) |
| `id` | Platform-specific user ID |
| `name` | Display name |
| `tag` | Platform tag (e.g., Discord `#1234`) |
| `username` | Username handle |
| `prefixed-id` | Platform-prefixed ID (e.g., `telegram:123456`) |
| `slug` | Slug-form identifier |
| `localpart` | E.164 phone number local part |

### Group Mention Gating

For group chats, the `mentionRequired` flag can be enabled to require that the bot be explicitly @mentioned before it responds. This prevents the agent from reacting to every message in a busy group.

### Command Authorization

Commands exposed by the agent (e.g., `/reset`, `/model`, `/think`) are organized into access groups. Each access group has a mode:

- **`allow`** -- Anyone who passes the allowlist may use commands in this group.
- **`deny`** -- Commands in this group are disabled entirely.
- **`configured`** -- Only explicitly configured users/roles may use commands in this group.

---

## Execution Approvals (exec-approvals)

The execution approval system provides a human-in-the-loop checkpoint for potentially dangerous tool invocations.

### Workflow

1. The agent encounters a tool call that requires approval (bash commands, file writes, etc.).
2. The agent sends an `exec.approval.request` to the gateway.
3. The gateway broadcasts an `exec.approval.requested` event to all connected operator clients.
4. An operator reviews the request and calls `exec.approval.resolve` with an approve or deny decision.
5. The gateway broadcasts an `exec.approval.resolved` event.
6. The agent, which has been waiting via `exec.approval.waitDecision`, receives the decision and proceeds or aborts.

### Components

- **ExecApprovalManager** -- Tracks pending approval requests in memory. Provides methods to create, list, and resolve requests.
- **Gateway methods:**
  - `exec.approval.request` -- Submit a new approval request.
  - `exec.approval.waitDecision` -- Block until a decision is made (with timeout).
  - `exec.approval.resolve` -- Approve or deny a pending request.
- **Events:**
  - `exec.approval.requested` -- Broadcast when a new request is created.
  - `exec.approval.resolved` -- Broadcast when a request is approved or denied.

---

## Network Security

- **Default bind address:** `127.0.0.1` (loopback only). The gateway does not listen on all interfaces unless explicitly configured, preventing accidental exposure on public networks.
- **TLS support:** The gateway can be configured with TLS certificates (`cert` and `key` paths) for encrypted connections. Self-signed certificates are supported for development; production deployments should use CA-signed certificates.
- **Tailscale integration:** The gateway can bind to a Tailscale interface, placing it inside a secure mesh network. This provides mutual authentication and encrypted transport without managing certificates manually.
- **SSH tunneling support:** For environments where direct connections are not possible, the gateway supports SSH tunnel configurations to securely bridge networks.

---

## Container Security

### Docker (Dockerfile)

- Runs as the non-root `node` user (UID 1000) after the build stage completes. The application never runs as root in the final image.
- Uses a minimal set of installed packages to reduce the attack surface.

### Sandbox (Dockerfile.sandbox)

- Designed for running untrusted agent workloads in fully isolated Docker containers.
- Creates a dedicated `sandbox` user with no elevated privileges.
- Installs only the tools required for agent execution (`bash`, `curl`, `git`, `jq`, `python3`, `ripgrep`).
- Separate tool allowlists are defined for sandboxed execution, restricting which tools the agent can invoke inside the container.

---

## Secret Management

- **detect-secrets in CI** -- The `secrets` job in the CI pipeline runs `detect-secrets` against the codebase to catch accidentally committed credentials.
- **.secrets.baseline** -- A baseline file that records known false positives so that `detect-secrets` does not flag them on every run.
- **Credential storage** -- Credentials are stored in `~/.openclaw/credentials/`, outside the project directory, to prevent accidental commits.
- **Environment variable fallbacks** -- API keys and tokens can be provided via environment variables (e.g., `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`) as an alternative to file-based credential storage.

---

## Pre-commit Security

- **git-hooks/pre-commit** -- A pre-commit hook that runs lint (`oxlint`) and format (`oxfmt`) checks on staged files before allowing a commit. This catches common issues before they reach CI.
- **zizmor.yml** -- A GitHub Actions security scanning configuration. Zizmor analyzes workflow files for security issues such as script injection, excessive permissions, and unsafe artifact handling.
