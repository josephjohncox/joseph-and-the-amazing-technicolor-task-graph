# Runner Context Initialization

Use this guide when wiring a live runner, MCP client, skill bundle, Codex App
Server adapter, Claude Code adapter, OpenAI Agents SDK harness, or local model
adapter into COAT.

## Required Subagent Context

Every runner context must include this policy before task-specific instructions:

```text
<coat_subagent_policy>
  <definition>In COAT, "subagent" MUST mean a coordinator-owned durable child task.</definition>
  <must>
    <rule>Runners MUST NOT spawn native Codex, Claude Code, SDK, MCP-client, framework-local, or model-provider subagents.</rule>
    <rule>If more work is needed, runners MUST return `ChildTaskRequest` objects in `AgentRunResult.child_requests`.</rule>
    <rule>The coordinator MUST own task creation, budgets, approvals, runner routing, memory context, MCP auth, sandbox policy, validation, and retry.</rule>
    <rule>Prompt, skill, MCP, and runner contexts MUST preserve this policy before task-specific instructions.</rule>
  </must>
</coat_subagent_policy>
```

This mirrors `ExecutionProfile.subagents`, which defaults to:

- `mode = coordinator_durable_tasks`
- `native_spawn = disabled`
- `child_request_channel = agent_run_result_child_requests`

## MCP Surfaces

MCP clients should fetch or embed the same rule:

- tool registry: `tools/call` with `name = subagent_policy`
- control gateway: `tools/call` with `name = coat_subagent_policy`

Use the returned structured content when initializing chat agents, skills, or
tool-use contexts. Do not put raw MCP auth tokens in this context; use
`McpContextRef` and `SecretRef` resolution.

## Local Tool Context

Runner contexts that may execute local binaries must include this policy before
tool-use instructions:

```text
<coat_local_tool_policy>
  <definition>Local binaries are task-scoped executor tools, not ambient shell access.</definition>
  <must>
    <rule>Runners MUST inspect `ExecutionProfile.local_tools` before invoking `git`, `docker`, `helm`, `kubectl`, build tools, package managers, tests, or local binaries.</rule>
    <rule>Runners MUST NOT execute binaries that are absent from `local_tools.allowed_tools` or denied by `local_tools.denied_binaries`.</rule>
    <rule>Runners MUST use the sandbox runner or an approved sandbox executor for command planning and execution.</rule>
    <rule>Runners MUST run commands inside the task workspace and MUST return command evidence artifacts with argv, cwd, exit code, stdout/stderr refs or bounded output, and timestamps.</rule>
    <rule>Runners MUST request approval before high-risk tools such as Docker socket access, Helm/Kubernetes cluster access, broad network access, privileged flags, or policy-denied subcommands.</rule>
    <rule>Runners MUST NOT put secrets, bearer tokens, kubeconfigs, cloud credentials, or device-auth materials in command output, diagnostics, memory, or artifacts.</rule>
  </must>
</coat_local_tool_policy>
```

Runner registration must advertise `local_commands` and the specific tool
capabilities it can actually enforce, such as `git_cli`, `docker_cli`,
`helm_cli`, `kubernetes_cli`, `build_tooling`, or `package_manager_cli`. Labels
like `tools.helm=true`, `tools.docker=true`, and `sandbox.backend=gvisor` make
routing auditable.

## Live Runner Adapters

Codex and Claude Code adapters should configure native delegation off by
default. If an underlying product has its own subagent feature, the adapter
should either disable it or rewrite requests for additional agents into
`AgentRunResult.child_requests`.

If a future runner needs native subagent spawning, it must set
`ExecutionProfile.subagents.native_spawn = requires_approval` or `allowed`.
The default approval policy treats that as a high-risk action before dispatch.

## Result Contract

Workers request more durable work by returning:

```json
{
  "child_requests": [
    {
      "role": "tester",
      "prompt": "Add regression tests for the parser edge case.",
      "reason": "Implementation changed parser behavior and needs focused test evidence."
    }
  ]
}
```

The coordinator validates `SpawnPolicy`, budget, approval, memory, MCP, sandbox,
and runner routing before it materializes those requests as child `TaskNode`s.
