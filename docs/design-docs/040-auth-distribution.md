# Design Doc: Auth Distribution And Device Sessions

## Intent

Distributed runners need access to Codex, Claude Code, MCP servers, model providers, memories, trackers, and notification targets without copying raw tokens through durable workflow state.

The coordinator stores auth intent, references, leases, labels, and approval state. Runners resolve usable credentials through local policy, mounted secrets, workload identity, or a broker.

## Contract

`McpContextRef` now carries `auth_distribution`:

- `access_mode`: `single_user` by default, or `multi_user_oidc` when the extension is explicitly enabled.
- `user`: stable `UserPrincipalRef` for OIDC-authenticated user delegation.
- `oidc_delegation`: issuer, client, audience, scopes, broker, TTL, consent, and runner-label policy.
- `mode`: how auth may be made available for this task.
- `allowed_materials`: API tokens, MCP bearer tokens, OAuth tokens, device sessions, local CLI sessions, workload identity tokens, or service accounts.
- `required_runner_labels`: runner labels required before dispatch, for example `auth.codex.device=true`.
- `lease_ttl_seconds`: requested max lifetime for coordinator-issued or brokered credentials.
- `renewal`: whether refresh is disabled, manual, brokered, or runner-local.
- `allow_node_local_device_session`: whether a runner may use a device/browser login already present on that node.
- `allow_secret_sync`: whether any secret backend replication is allowed. Default is false.
- `require_human_approval_for_brokered_user_auth`: whether user-delegated auth needs an approval gate. Default is true.

`McpAuthRef` supports:

- `secret`: static or rotating material behind `SecretRef`.
- `workload_identity`: cloud or cluster-native identity.
- `oidc_delegation`: token broker or on-behalf-of exchange for a logged-in OIDC user.
- `oauth_delegation`: token exchange through a referenced exchange secret.
- `device_auth_session`: node-local CLI/app auth such as Codex or Claude Code login state.
- `brokered_user_session`: a broker-mediated human/device OAuth flow that returns a short-lived lease.

## Distribution Modes

`runner_local_only` means the task must run on a runner that already has the auth session. Do not copy the local login file. Use `required_runner_labels` and, when needed, `RunnerLocality::SameNode` or `LocalOnly`.

`runner_resolves_refs` means the runner reads `SecretRef` entries from its own environment, Kubernetes Secret, Vault, cloud secret manager, 1Password, Bitwarden, Doppler, SOPS material, local file, or external broker.

`coordinator_issues_lease` means the coordinator obtains a short-lived scoped token and passes only a lease reference through durable state. This is appropriate for service tokens, MCP context tokens, and explicit auth-broker leases.

`workload_identity` means the runner uses its node, pod, service account, or cloud identity. Prefer this for cloud deployments.

`oauth_device_broker` means a human completes a browser or device-code flow once, the broker returns a short-lived task lease, and the coordinator records approval plus lease metadata.

`external_broker` covers gateways such as enterprise secret brokers, LLM gateways, or internal auth services.

`multi_user_oidc` is an extension, not the default. It requires `oidc_user_delegation` runner capability, `auth.oidc.user_delegation=true`, tenant labels, a `UserPrincipalRef`, and short-lived brokered OIDC access tokens or leases. Use `single_user` for local personal operation.

## Codex

Codex App Server is the preferred integration surface for rich Codex auth state. The official app-server protocol exposes account state, API-key login, ChatGPT browser login, ChatGPT device-code login, logout, and rate-limit reads. A distributed runner should either:

- use API-key or OpenAI-compatible local model auth through `SecretRef`;
- use an App Server session that is node-local and advertised with labels such as `auth.codex.device=true`;
- use a brokered device-code flow that sends the verification URL and user code through the notification system, then stores only a lease reference.

Do not sync a local Codex login store between nodes unless an operator explicitly enables `allow_secret_sync` and the target storage is encrypted, scoped, and audited.

## Claude Code

Claude Code supports interactive login, API keys, bearer auth, credential helper scripts, long-lived OAuth tokens for CI, and cloud-provider auth. Its official docs describe platform-specific credential storage and credential precedence.

Use these paths:

- local developer runner: node-local Claude Code login, labelled `auth.claude.device=true`;
- non-interactive staff-engineer runner: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, or an `apiKeyHelper` backed by Vault or another broker;
- cloud runner: Bedrock, Vertex, Foundry, or workload identity where available.

Brokered Claude user auth must go through `ApprovalGatePolicy.require_for_brokered_user_auth`.

## Approval

Secret-bearing MCP contexts trigger `secret_access`.

Brokered user auth or non-local device-session use triggers `brokered_user_auth`, which is critical risk by default. Approval should notify a human thread, include the provider and requested scopes, and avoid exposing token values.

## Sidecar Requirements

Sidecars must:

- expose supported propagation modes through `/capabilities`;
- report `mcp_auth_distribution`, allowed material kinds, required runner labels, and secret-ref resolution status;
- never return secret values in diagnostics or artifacts;
- fail closed if a required `SecretRef` cannot be resolved in live mode;
- route user-auth flows through notifications and approval state, not stdin prompts hidden inside a worker.

## MCP Context

MCP auth context is distributable because it is a graph of references:

- server refs identify tool surfaces;
- `SecretRef` identifies secret material;
- auth distribution constrains which runners may receive the task;
- multi-user OIDC adds principal and tenant refs without token values;
- approval records which human allowed user-delegated auth;
- notification threads handle device-code, browser-login, or human-feedback prompts.

Forked tasks inherit the same references, not copied token values. Join/unifier tasks should record which auth-sensitive sources were used, but not credentials.

## Examples

- `examples/auth-distribution-codex-device.json`: node-local Codex App Server device session.
- `examples/auth-distribution-claude-brokered.json`: brokered Claude Code user session.
- `examples/mcp-context-multi-user-oidc.json`: opt-in multi-user OIDC user delegation for MCP.
