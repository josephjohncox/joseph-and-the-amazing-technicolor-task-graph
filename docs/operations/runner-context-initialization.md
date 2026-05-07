# Runner Context Initialization

Use this guide when wiring a live runner, MCP client, skill bundle, Codex App
Server adapter, Claude Code adapter, OpenAI Agents SDK harness, or local model
adapter into COAT.

## Required Subagent Context

Every runner context must include this policy before task-specific instructions:

```text
Subagent policy:
- In COAT, "subagent" means a coordinator-owned durable child task.
- Do not spawn native Codex, Claude Code, SDK, MCP-client, or framework-local subagents.
- If more work is needed, return ChildTaskRequest objects in AgentRunResult.child_requests.
- The coordinator owns task creation, budgets, approvals, runner routing, memory context, MCP auth, sandbox policy, validation, and retry.
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
