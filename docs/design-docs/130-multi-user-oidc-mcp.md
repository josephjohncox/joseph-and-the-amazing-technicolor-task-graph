# Multi-User OIDC MCP Delegation

COAT defaults to single-user mode. Local Compose, local runners, and personal Restate Cloud deployments assume one operator unless a goal or deployment explicitly opts into multi-user OIDC.

Multi-user OIDC is an extension layer for teams and hosted deployments where users log in through an identity provider and MCP calls must execute with that user's delegated authority.

## Default Mode

Default MCP context:

- `access_mode = single_user`;
- no `user` principal in task state;
- no OIDC token exchange;
- runners resolve service tokens, workload identity, local device sessions, or brokered leases exactly as before.

This is the right mode for local personal usage, single-user labs, and dev clusters.

## Opt-In Mode

Multi-user OIDC requires all of the following:

- `McpContextRef.access_mode = multi_user_oidc`;
- `McpContextRef.user` with stable user, issuer, subject, tenant, optional group, and consent references;
- `McpContextRef.oidc_delegation` with issuer, client ID, requested audiences/scopes, token broker, TTL, and runner labels;
- each user-authenticated MCP server uses `McpAuthRef.kind = oidc_delegation` or a policy-compatible brokered auth ref;
- runner advertises `oidc_user_delegation`;
- runner labels satisfy `auth.oidc.user_delegation=true` and any tenant or policy labels;
- approval or consent is present when `require_user_consent = true`.

Tasks store references and claims needed for routing. They do not store raw ID tokens, access tokens, refresh tokens, cookies, or browser sessions.

## OIDC Flow

1. User logs in to the control gateway or another trusted frontend through OIDC.
2. The frontend or API creates a durable goal with `UserPrincipalRef` and OIDC delegation policy.
3. The coordinator treats delegated user auth as brokered user auth and applies approval policy.
4. Runner dispatch requires `oidc_user_delegation` capability plus required labels.
5. Runner calls the configured token broker with task, user, MCP server, audience, scopes, and approval or consent ref.
6. Broker performs token exchange or on-behalf-of flow and returns a short-lived lease reference or in-memory bearer token scoped to one MCP server.
7. Runner calls the MCP server with that short-lived credential.
8. Results record which principal and MCP server were used, but never token values.

The broker should verify issuer, audience, tenant, scopes, consent, task ID, runner identity, and TTL before minting anything.

## MCP Server Rules

An MCP server that acts as a user must:

- verify OIDC issuer and audience;
- authorize by subject, tenant, groups, and scopes;
- accept only short-lived access tokens or broker leases, not durable user refresh tokens;
- log user subject, tenant, goal ID, task ID, runner ID, and tool call;
- reject tokens without the MCP server audience;
- support revocation or short TTLs.

Do not pass ID tokens to MCP servers unless a server explicitly expects ID tokens and `pass_id_token_to_mcp = true`. Prefer access tokens minted for the MCP server audience.

## Isolation

Single-user and multi-user work should not share credential caches. Use separate:

- runner pools;
- token broker namespaces;
- memory namespaces;
- object storage prefixes;
- audit projections;
- notification channels.

Tenant labels such as `tenant=tenant-a` should constrain runner dispatch. A runner without the tenant label or `oidc_user_delegation` capability is rejected before work starts.

## Example

See `examples/mcp-context-multi-user-oidc.json`.
