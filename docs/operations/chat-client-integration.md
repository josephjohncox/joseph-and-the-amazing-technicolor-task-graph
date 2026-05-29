# Chat Client Integration

Use this runbook to connect Codex, Claude Code, or another primary chat
interface to a remote COAT control gateway.

The integration has two parts:

- an HTTP MCP server exposed by `coat-control-web` at `/mcp`;
- a portable `coat-control-plane` skill that tells chat agents how to use COAT
  without bypassing the durable coordinator.

The source copy of the skill lives at `skills/coat-control-plane/SKILL.md`.
`coat setup chat-client --install-codex-skill` and
`coat setup chat-client --install-claude-skill` install that Markdown file; do
not maintain a second skill body in CLI code or docs.

## Install

Generate local provider configuration first when the control gateway should use
live hosted or local models. The no-flag command opens an interactive wizard;
the `--write-env` form is the non-interactive path:

```sh
coat setup local-auth
coat setup local-auth --write-env --output infra/compose/local-providers.env
coat setup login --codex --claude --preflight
```

Start the gateway locally, or point the commands below at a remote gateway URL:

```sh
coat deploy local preflight --env-file infra/compose/local-providers.env
coat deploy local up --env-file infra/compose/local-providers.env
```

The local-auth wizard supports tokenless local auth paths as first-class modes.
Choose runner-local Codex or Claude Code device/browser auth when the runner
node has already completed `coat setup login --codex --claude`; choose brokered auth
when a separate approval/lease service will satisfy the task. The env file
records only mode and label metadata such as `CODEX_AUTH_MODE=runner_local_device`;
raw browser sessions, cookies, refresh tokens, and user tokens stay out of COAT
config and task state.

Install MCP and the skill interactively:

```sh
export COAT_CONTROL_MCP_TOKEN=<redacted-token>
coat setup chat-client
```

The wizard asks for the control gateway MCP URL, server name, token mode,
Claude Code scope, whether to write a project `.mcp.json`, and which personal
skills or client registrations to install. It uses Dialoguer prompts for
confirmations, text inputs, single-selects, and multi-selects while preserving
the explicit flag path below for automation.

Non-interactive install examples:

```sh
export COAT_CONTROL_MCP_TOKEN=<redacted-token>

coat setup chat-client \
  --mcp-url https://coat.example.com/mcp \
  --server-name coat-control \
  --token-env COAT_CONTROL_MCP_TOKEN

coat setup chat-client \
  --mcp-url https://coat.example.com/mcp \
  --server-name coat-control \
  --token-env COAT_CONTROL_MCP_TOKEN \
  --install-codex-mcp \
  --install-codex-skill

coat setup chat-client \
  --mcp-url https://coat.example.com/mcp \
  --server-name coat-control \
  --token-env COAT_CONTROL_MCP_TOKEN \
  --install-claude-mcp \
  --install-claude-skill
```

For a project-scoped Claude Code config instead of mutating user config:

```sh
coat setup chat-client \
  --mcp-url https://coat.example.com/mcp \
  --server-name coat-control \
  --token-env COAT_CONTROL_MCP_TOKEN \
  --write-claude-project-config \
  --claude-project-config .mcp.json
```

Codex registration uses remote HTTP MCP with a bearer-token environment
variable. Claude Code registration uses remote HTTP MCP and, when a token is
configured, writes a JSON config that expands `${COAT_CONTROL_MCP_TOKEN}` in the
Authorization header.

With no action flags, `coat setup chat-client` starts the interactive wizard.
With explicit action flags, it stays non-interactive. It mutates Codex or
Claude Code configuration only when the selected wizard actions or corresponding
flags request `--install-codex-mcp`, `--install-claude-mcp`,
`--write-claude-project-config`, `--install-codex-skill`, or
`--install-claude-skill`.

## Verify

```sh
codex mcp get coat-control
claude mcp get coat-control
```

In Claude Code, run `/mcp` to inspect connection status.

From any MCP-capable client, call `tools/list` and confirm that COAT exposes
planning, goal, runner, approval, memory, checkpoint, and steering tools.
The operator tools should include the compact surface:
`coat_operator_workspace`, `coat_operator_goal`, `coat_operator_actions`,
`coat_operator_action_resolve`, `coat_operator_agent_context`,
`coat_operator_goal_submit`, and `coat_operator_goal_steer`.
Old overview, snapshot, activity, approval-list, runner-list, and helper tool
names are intentionally removed once the corresponding operator tool exists.

## Chat-Agent Rules

The installed skill enforces these rules as XML-like instruction blocks:

```text
<coat_control_contract>
  <authority>
    <rule>The coordinator MUST own truth.</rule>
    <rule>Restate MUST own durable time and replay.</rule>
    <rule>The chat client MUST NOT mutate projections as if they are source-of-truth state.</rule>
  </authority>
  <subagent_policy>
    <rule>In COAT, "subagent" MUST mean a coordinator-owned durable child task.</rule>
    <rule>The chat client MUST NOT spawn native subagents for COAT work.</rule>
  </subagent_policy>
  <security>
    <rule>Raw tokens MUST NOT enter goal JSON, task state, memory, artifacts, diagnostics, or chat output.</rule>
  </security>
</coat_control_contract>
```

Use the MCP tools in this order for normal work:

1. `coat_subagent_policy`
2. `coat_operator_workspace`
3. `coat_plan_list`, `coat_plan_get`, or `coat_plan_continuity`
4. `coat_chat_assist`
5. `coat_plan_draft` or `coat_plan_revise`
6. `coat_plan_compile`
7. `coat_operator_goal_submit` only after explicit user confirmation
8. `coat_operator_goal`, `coat_operator_workspace`, and `coat_checkpoint_history`
9. `coat_operator_actions` and `coat_operator_action_resolve` only after explicit user confirmation
10. `coat_operator_goal_steer` for user-approved steering

Do not use removed helper names as compatibility shims. When a chat client
needs state, read the compact operator projection; when it needs to mutate
state, submit the typed operator action and show the returned action result.

## Non-Local Runners

Primary chat clients should register non-local runners through the control MCP
surface only when the runner endpoint is reachable by the coordinator or runner
registry:

```json
{
  "runner_id": "remote-codex-01",
  "node_id": "mac-mini-lab-01",
  "endpoint": "https://runner.example.com/run-task",
  "roles": ["codex", "reviewer", "tester"],
  "capabilities": ["code", "review", "test", "mcp_tools", "durable_child_tasks"],
  "models": [
    {
      "provider": "codex",
      "model": "codex-default",
      "endpoint": null,
      "priority": 100,
      "weight": 1,
      "context_window": null,
      "features": ["tool_use", "json_schema", "streaming"],
      "labels": {}
    }
  ],
  "labels": {
    "pool": "remote",
    "runtime": "codex",
    "auth.codex.device": "runner_local_only"
  },
  "mcp_servers": [],
  "max_concurrency": 1,
  "lease_ttl_seconds": 300
}
```

Use `coat_runner_register` with that payload from MCP, or the CLI equivalent:

```sh
coat runner register --file examples/runner-remote-codex.json
```

The runner itself still needs to heartbeat and return structured
`AgentRunResult` values. MCP registration does not make a chat client a worker;
it only connects the chat interface to the durable control surface.

## References

- Codex local CLI help: `codex mcp add --help`
- Claude Code MCP: https://code.claude.com/docs/en/mcp
- Claude Code skills: https://code.claude.com/docs/en/skills
