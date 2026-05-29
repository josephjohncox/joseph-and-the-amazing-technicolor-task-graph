---
name: coat-control-plane
description: Use when operating Joseph and the Amazing Technicolor Task Graph from chat: author durable goals and plans, inspect task and runner progress, steer or approve work, and route subagent requests through COAT MCP tools.
---

# COAT Control Plane

<coat_control_contract>
  <identity>
    You are operating Joseph and the Amazing Technicolor Task Graph through the `coat-control` MCP server.
  </identity>

  <authority>
    <rule>The coordinator MUST own truth.</rule>
    <rule>Restate MUST own durable time and replay.</rule>
    <rule>The chat client MUST NOT mutate projections as if they are source-of-truth state.</rule>
    <rule>The chat client MUST NOT claim durable state changed unless a COAT backend tool returned success.</rule>
  </authority>

  <subagent_policy>
    <rule>In COAT, "subagent" MUST mean a coordinator-owned durable child task.</rule>
    <rule>You MUST call `coat_subagent_policy` before advising agent fan-out, child tasks, fork/join, actor/critic, or reviewer swarms.</rule>
    <rule>You MUST NOT spawn native Codex, Claude Code, SDK, framework, or MCP-client subagents for COAT work.</rule>
    <rule>You MUST propose child work through durable plans, steering directives, or structured `ChildTaskRequest` payloads.</rule>
  </subagent_policy>

  <security>
    <rule>You MUST NOT put raw provider, MCP, OAuth, device, cloud, or user tokens into goals, tasks, memory, artifacts, diagnostics, or chat output.</rule>
    <rule>You MUST use `SecretRef`, runner-local auth, workload identity, or an approved auth broker for credentials.</rule>
    <rule>You MUST ask the user before `coat_operator_goal_submit`, `coat_operator_action_resolve`, high-risk `coat_operator_goal_steer`, or remote runner registration.</rule>
  </security>
</coat_control_contract>

<mcp_workflow>
  <step order="1" tool="coat_subagent_policy">MUST load the durable-child-task policy for any task distribution or subagent discussion.</step>
  <step order="2" tool="coat_operator_workspace">MUST inspect service health, runners, human queues, events, and current activity before steering live work.</step>
  <step order="3" tool="coat_plan_list|coat_plan_get|coat_plan_continuity">MUST inspect existing durable plans before changing or compiling a plan.</step>
  <step order="4" tool="coat_chat_assist">Use for plain-language drafting. The output is a draft, not durable state.</step>
  <step order="5" tool="coat_plan_draft|coat_plan_revise">Use for durable planning-mode records.</step>
  <step order="6" tool="coat_plan_compile">Use to produce a `GoalSpec`; MUST ask before submitting.</step>
  <step order="7" tool="coat_operator_goal_submit">MUST only submit after explicit user confirmation.</step>
  <step order="8" tool="coat_operator_goal|coat_operator_workspace|coat_checkpoint_history">Use for progress, prompts, evidence, checkpoints, and runner status.</step>
  <step order="9" tool="coat_operator_actions|coat_operator_action_resolve">MUST ask before approving, rejecting, resuming, replanning, retrying, or cancelling on the user's behalf.</step>
  <step order="10" tool="coat_operator_goal_steer">MUST use for user-approved pause, resume, research, retry, branch, review, or constraint directives.</step>
  <step order="11" tool="coat_memory_context|coat_memory_search">Use scoped retrieval before substantial work. MUST preserve provenance.</step>
</mcp_workflow>

<non_local_runner_registration>
  <rule>You MUST register remote runner endpoints only when the endpoint is reachable by the coordinator or runner registry.</rule>
  <rule>A registration MUST include stable `runner_id`, `node_id`, `endpoint`, roles, capabilities, labels, model candidates, MCP server refs, concurrency, and lease TTL.</rule>
  <rule>A runner MUST advertise only capabilities it can enforce or attest.</rule>
  <rule>A runner MUST NOT claim sandboxing, device auth, OIDC delegation, local model access, or network policy enforcement without evidence.</rule>
  <rule>Codex and Claude Code device/browser auth MUST default to `runner_local_only` unless the task explicitly uses approved brokered auth.</rule>
</non_local_runner_registration>

<goal_authoring_policy>
  <rule>If the request is ambiguous, you MUST create or revise a durable plan before creating a goal.</rule>
  <rule>A goal MUST have objective, acceptance evidence, constraints, budget, done criteria, execution profile, memory policy, research policy, approval policy, and stop conditions.</rule>
  <rule>Non-trivial code goals SHOULD require reviewer or critic evidence before satisfaction.</rule>
  <rule>Research tasks MUST return sourced facts plus an information-use plan.</rule>
</goal_authoring_policy>
