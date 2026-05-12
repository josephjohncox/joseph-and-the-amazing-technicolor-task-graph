/**
 * Claude Code runner sidecar.
 *
 * Purpose: provide a generic Claude Code execution boundary separate from the
 * staff-engineer lifecycle bundle. Staff-engineer owns issue-to-PR ceremony;
 * this runner owns bounded task execution behind COAT's durable
 * `AgentRunRequest -> AgentRunResult` contract.
 *
 * Architecture reference:
 * - docs/design-docs/010-distributed-runners-mcp.md
 * - docs/operations/runner-context-initialization.md
 * - docs/operations/ephemeral-kubernetes-runners.md
 */
import http from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const port = numberEnv("PORT", 9094);
const mode = process.env.CLAUDE_CODE_RUNNER_MODE ?? "stub";
const runnerId = process.env.RUNNER_ID ?? "claude-code-runner-ts";
const nodeId = process.env.NODE_ID ?? process.env.HOSTNAME ?? "local-node";
const registryUrl = process.env.RUNNER_REGISTRY_URL ?? process.env.COAT_RUNNER_REGISTRY_URL ?? "";
const runnerEndpoint = process.env.RUNNER_ENDPOINT ?? `http://localhost:${port}`;
const leaseTtlSeconds = numberEnv("RUNNER_LEASE_TTL_SECONDS", 300);
const heartbeatIntervalMs = numberEnv("RUNNER_HEARTBEAT_INTERVAL_MS", 30_000);
const maxConcurrency = numberEnv("RUNNER_MAX_CONCURRENCY", 1);
let runningTasks = 0;

type WorkerKind =
  | "planner"
  | "codex"
  | "claude_code"
  | "staff_engineer_claude"
  | "model_provider"
  | "research"
  | "reviewer"
  | "tester"
  | "formal_methods"
  | "validator"
  | "patch_merger"
  | "rust_tool";

type RunnerCapability =
  | "code"
  | "research"
  | "web_search"
  | "test"
  | "review"
  | "mcp_tools"
  | "durable_child_tasks"
  | "oidc_user_delegation"
  | "local_commands"
  | "git_cli"
  | "docker_cli"
  | "helm_cli"
  | "kubernetes_cli"
  | "build_tooling"
  | "package_manager_cli"
  | "workspace_sandbox"
  | "git_worktree"
  | "git"
  | "object_storage"
  | "s3_compatible"
  | "browser"
  | "notifications"
  | "local_models"
  | "vllm"
  | "open_ai_compatible"
  | "gpu"
  | "network_open"
  | "formal_verification";

type ModelProviderKind =
  | "codex"
  | "open_ai"
  | "open_ai_compatible"
  | "bedrock"
  | "vllm"
  | "ollama"
  | "llama_cpp"
  | "anthropic"
  | "hugging_face"
  | "local_process"
  | "other";

type ModelFeature = "tool_use" | "json_schema" | "streaming" | "vision" | "long_context" | "reasoning" | "embeddings" | "local_weights" | "web_search";

type ModelCandidate = {
  provider: ModelProviderKind;
  model: string;
  endpoint: string | null;
  priority: number;
  weight: number;
  context_window: number | null;
  features: ModelFeature[];
  labels: Record<string, string>;
};

type McpServerRef = {
  name: string;
  transport: "stdio" | "http" | "sse";
  uri: string;
  allowed_tools: string[];
  auth: unknown;
};

type RunnerRegistration = {
  runner_id: string;
  node_id: string;
  endpoint: string;
  roles: WorkerKind[];
  capabilities: RunnerCapability[];
  models: ModelCandidate[];
  labels: Record<string, string>;
  mcp_servers: McpServerRef[];
  max_concurrency: number;
  lease_ttl_seconds: number;
};

type AgentRunRequest = {
  goal_id: string;
  task: {
    id: string;
    role: string;
    purpose?: unknown;
    prompt: string;
    execution?: {
      model?: { candidates?: unknown[] };
      mcp?: unknown;
      subagents?: unknown;
      results?: unknown;
    };
  };
};

type AgentRunResult = {
  task_id: string;
  status: "done" | "partial" | "blocked" | "failed" | "timed_out";
  summary: string;
  review: unknown | null;
  research: unknown | null;
  branch_vote: unknown | null;
  runner_id: string | null;
  model_used: unknown | null;
  mcp_context_used: unknown | null;
  artifacts: Array<{ kind: string; uri: string; description: string; sha256?: string | null }>;
  git_result: unknown | null;
  object_artifacts: unknown[];
  checkpoints: unknown[];
  test_evidence: unknown[];
  child_requests: unknown[];
  confidence: number;
  next_actions: string[];
  diagnostics: string[];
  notification_reports: unknown[];
};

const durableSubagentContext = [
  "Treat any request to use a subagent as a request to create a COAT durable child task.",
  "Do not spawn native Claude Code, Codex, SDK, MCP-client, or framework-local subagents from inside this runner.",
  "Return proposed child work only through AgentRunResult.child_requests.",
  "The coordinator validates budgets, approval policy, routing, memory context, and sandbox policy before dispatch.",
];

const server = http.createServer(async (req, res) => {
  try {
    log("debug", "http_request", { method: req.method, url: req.url });
    if (req.method === "GET" && req.url === "/healthz") return json(res, 200, { status: "ok", mode, runner_id: runnerId });
    if (req.method === "GET" && req.url === "/registration") return json(res, 200, buildRegistration());
    if (req.method === "GET" && req.url === "/capabilities") return json(res, 200, buildCapabilities());
    if (req.method === "GET" && req.url === "/verify") return json(res, 200, await verifyClaudeCode());
    if (req.method === "POST" && req.url === "/run-task") return json(res, 200, await runTask((await readJson(req)) as AgentRunRequest));
    return json(res, 404, { error: "not_found" });
  } catch (error) {
    log("error", "http_request_failed", { method: req.method, url: req.url, error: errorMessage(error) });
    return json(res, 500, { error: error instanceof Error ? error.message : String(error) });
  }
});

server.listen(port, () => {
  log("info", "listening", { port, mode, runner_id: runnerId, node_id: nodeId });
  void registerAndHeartbeat();
});

async function runTask(request: AgentRunRequest): Promise<AgentRunResult> {
  const startedAt = Date.now();
  runningTasks += 1;
  log("info", "task_started", {
    goal_id: request.goal_id,
    task_id: request.task.id,
    role: request.task.role,
    mode,
    running_tasks: runningTasks,
  });
  try {
    const selected = selectedModel(request);
    return {
      task_id: request.task.id,
      status: mode === "stub" || mode === "cli-healthcheck" ? "done" : "blocked",
      summary:
        mode === "stub"
          ? `stub Claude Code runner accepted ${taskPurposeKind(request.task.purpose)} task ${request.task.id}`
          : `Claude Code live execution is gated; mode=${mode}`,
      review: reviewOutput(request),
      research: researchOutput(request),
      branch_vote: branchVoteOutput(request),
      runner_id: runnerId,
      model_used: selected,
      mcp_context_used: request.task.execution?.mcp ?? null,
      artifacts: [
        {
          kind: "report",
          uri: `memory://claude-code-runner/${request.task.id}`,
          description: "Claude Code runner contract artifact",
          sha256: null,
        },
      ],
      git_result: null,
      object_artifacts: [],
      checkpoints: checkpointRefs(request),
      test_evidence: testEvidence(request),
      child_requests: [],
      confidence: mode === "stub" ? 0.75 : 0.4,
      next_actions: mode === "stub" ? [] : ["enable a reviewed live Claude Code adapter in an isolated workspace"],
      diagnostics: [
        `mode=${mode}`,
        `runner_id=${runnerId}`,
        `node_id=${nodeId}`,
        `selected_model=${JSON.stringify(selected)}`,
        ...subagentDiagnostics(request.task.execution?.subagents),
        ...mcpDiagnostics(request.task.execution?.mcp),
      ],
      notification_reports: [],
    };
  } finally {
    runningTasks -= 1;
    log("info", "task_finished", {
      task_id: request.task.id,
      duration_ms: Date.now() - startedAt,
      running_tasks: runningTasks,
    });
  }
}

function buildRegistration(): RunnerRegistration {
  return {
    runner_id: runnerId,
    node_id: nodeId,
    endpoint: runnerEndpoint,
    roles: parseJsonEnv("RUNNER_ROLES_JSON", [
      "claude_code",
      "planner",
      "research",
      "reviewer",
      "tester",
      "validator",
      "patch_merger",
      "formal_methods",
    ] satisfies WorkerKind[]),
    capabilities: parseJsonEnv("RUNNER_CAPABILITIES_JSON", [
      "code",
      "research",
      ...(webSearchEnabled() ? ["web_search" as RunnerCapability, "network_open" as RunnerCapability] : []),
      "test",
      "review",
      "mcp_tools",
      "durable_child_tasks",
      "workspace_sandbox",
      "git_worktree",
      "git",
      "object_storage",
      "s3_compatible",
      "notifications",
      "formal_verification",
    ] satisfies RunnerCapability[]),
    models: parseJsonEnv("RUNNER_MODELS_JSON", [
      {
        provider: "anthropic",
        model: process.env.CLAUDE_CODE_MODEL ?? "claude-code-default",
        endpoint: null,
        priority: 100,
        weight: 1,
        context_window: null,
        features: ["tool_use", "json_schema", "streaming", ...(webSearchEnabled() ? ["web_search" as ModelFeature] : [])],
        labels: {},
      },
    ] satisfies ModelCandidate[]),
    labels: parseJsonEnv("RUNNER_LABELS_JSON", { pool: "default", runtime: "claude-code", "auth.claude.device": "false" }),
    mcp_servers: parseJsonEnv("RUNNER_MCP_SERVERS_JSON", [] satisfies McpServerRef[]),
    max_concurrency: maxConcurrency,
    lease_ttl_seconds: leaseTtlSeconds,
  };
}

function webSearchEnabled(): boolean {
  return ["CLAUDE_CODE_NATIVE_WEB_SEARCH", "COAT_WEB_SEARCH_ENABLED"].some((key) => truthyEnv(key));
}

function truthyEnv(key: string): boolean {
  const value = process.env[key]?.trim().toLowerCase();
  return value === "1" || value === "true" || value === "yes" || value === "on";
}

function buildCapabilities(): Record<string, unknown> {
  const registration = buildRegistration();
  return {
    runner_id: runnerId,
    node_id: nodeId,
    mode,
    endpoints: ["/healthz", "/registration", "/capabilities", "/verify", "/run-task"],
    registration,
    package_verification: {
      endpoint: "/verify",
      binary: process.env.CLAUDE_CODE_BINARY ?? "claude",
      live_execution_requires: ["Claude Code CLI", "Anthropic or Claude Code auth", "isolated workspace"],
      secret_values_exposed: false,
    },
    model_routing: {
      providers: [...new Set(registration.models.map((model) => model.provider))],
      models: registration.models,
    },
    mcp: {
      advertised_servers: registration.mcp_servers.map(redactedMcpServer),
      default_memory_mcp_url: process.env.COAT_MEMORY_MCP_URL ?? null,
      propagation_supported: ["coordinator_issued", "runner_resolves_refs", "runner_local_only", "oauth_device_broker", "external_broker"],
      access_modes_supported: ["single_user", ...(registration.capabilities.includes("oidc_user_delegation") ? ["multi_user_oidc"] : [])],
      secret_values_exposed: false,
    },
    subagents: {
      durable_task_queue_required: true,
      native_subagent_spawn_disabled: true,
      child_request_channel: "AgentRunResult.child_requests",
      instructions: durableSubagentContext,
    },
    result_channels: {
      git_worktrees: registration.capabilities.includes("git_worktree"),
      object_storage: registration.capabilities.includes("object_storage"),
      s3_compatible: registration.capabilities.includes("s3_compatible"),
      secret_values_exposed: false,
    },
  };
}

async function verifyClaudeCode(): Promise<Record<string, unknown>> {
  const binary = process.env.CLAUDE_CODE_BINARY ?? "claude";
  const authMode = process.env.CLAUDE_CODE_AUTH_MODE ?? "env_api_key";
  const probeCli = process.env.CLAUDE_CODE_VERIFY_CLI === "1";
  if (!probeCli) {
    return {
      available: Boolean(process.env.ANTHROPIC_API_KEY || process.env.ANTHROPIC_AUTH_TOKEN || process.env.CLAUDE_CODE_OAUTH_TOKEN || authMode === "runner_local_device" || authMode === "oauth_device_broker" || authMode === "external_broker"),
      binary,
      auth_mode: authMode,
      cli_probe_attempted: false,
      has_api_key: Boolean(process.env.ANTHROPIC_API_KEY),
      has_auth_token: Boolean(process.env.ANTHROPIC_AUTH_TOKEN),
      has_oauth_token: Boolean(process.env.CLAUDE_CODE_OAUTH_TOKEN),
      runner_local_device_allowed: authMode === "runner_local_device",
      brokered_auth_allowed: authMode === "oauth_device_broker" || authMode === "external_broker",
      auth_state_path_configured: Boolean(process.env.CLAUDE_CODE_AUTH_STATE_PATH),
      secret_values_exposed: false,
    };
  }
  try {
    const { stdout } = await execFileAsync(binary, ["--version"], { timeout: 5000 });
    return { available: true, binary, cli_probe_attempted: true, version: stdout.trim(), secret_values_exposed: false };
  } catch (error) {
    return { available: false, binary, cli_probe_attempted: true, error: error instanceof Error ? error.message : String(error), secret_values_exposed: false };
  }
}

async function registerAndHeartbeat(): Promise<void> {
  if (!registryUrl) {
    log("warn", "runner_registry_unset");
    return;
  }
  try {
    await registerRunner();
    await sendHeartbeat();
    const timer = setInterval(() => void sendHeartbeat().catch((error) => log("warn", "runner_heartbeat_failed", { error: errorMessage(error) })), heartbeatIntervalMs);
    timer.unref();
    log("info", "runner_registered", { registry_url: registryUrl });
  } catch (error) {
    log("warn", "runner_registration_failed", { error: errorMessage(error) });
  }
}

async function registerRunner(): Promise<void> {
  const response = await fetch(`${registryUrl.replace(/\/$/, "")}/runners`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(buildRegistration()),
  });
  if (!response.ok) throw new Error(`registry returned ${response.status}: ${await response.text()}`);
}

async function sendHeartbeat(): Promise<void> {
  const response = await fetch(`${registryUrl.replace(/\/$/, "")}/runners/heartbeat`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      runner_id: runnerId,
      node_id: nodeId,
      running_tasks: runningTasks,
      capacity_remaining: Math.max(maxConcurrency - runningTasks, 0),
    }),
  });
  if (!response.ok) throw new Error(`registry heartbeat returned ${response.status}: ${await response.text()}`);
}

function selectedModel(request: AgentRunRequest): unknown {
  const candidates = request.task.execution?.model?.candidates;
  if (Array.isArray(candidates)) {
    const match = candidates.find((candidate) => isRecord(candidate) && (candidate.provider === "anthropic" || candidate.provider === "bedrock"));
    if (match) return match;
    if (candidates[0]) return candidates[0];
  }
  return buildRegistration().models[0] ?? null;
}

function reviewOutput(request: AgentRunRequest): unknown | null {
  const purpose = taskPurposeKind(request.task.purpose);
  if (purpose !== "review" && purpose !== "unification" && purpose !== "branch_vote" && purpose !== "branch_unification") return null;
  return {
    decision: mode === "stub" ? "inconclusive" : "blocked",
    reward: 0.7,
    findings: [],
    objective_results: [],
    gate_results: [],
    retry_recommended: mode !== "stub",
    unification_summary: purpose.includes("unification") ? "Claude Code placeholder unification" : null,
  };
}

function researchOutput(request: AgentRunRequest): unknown | null {
  const purpose = request.task.purpose;
  if (!isRecord(purpose) || purpose.kind !== "research") return null;
  const question = typeof purpose.question === "string" ? purpose.question : request.task.prompt;
  return {
    question,
    answer: `Claude Code stub answer for: ${question}`,
    sources: [
      {
        title: "Claude Code runner placeholder source",
        uri: `memory://claude-code-runner/research/${request.task.id}`,
        quality: "unknown",
        captured_at: null,
        quote: null,
        summary: "Replace with sourced research before production validation.",
        confidence: 0.6,
      },
    ],
    confidence: 0.6,
    use_plan: {
      facts_to_use: ["Use only as contract smoke output."],
      facts_to_avoid: ["Do not treat Claude Code stub output as sourced evidence."],
      proposed_task_updates: [],
      validation_checks: ["Run a sourced research or review task before satisfaction."],
    },
    open_questions: [],
  };
}

function branchVoteOutput(request: AgentRunRequest): unknown | null {
  const purpose = request.task.purpose;
  if (!isRecord(purpose) || (purpose.kind !== "branch_vote" && purpose.kind !== "branch_unification")) return null;
  const candidates = Array.isArray(purpose.candidate_task_ids) ? purpose.candidate_task_ids.filter((id): id is string => typeof id === "string") : [];
  if (!candidates.length || typeof purpose.group_id !== "string") return null;
  return { group_id: purpose.group_id, selected_task_id: candidates[0], ranked_task_ids: candidates, confidence: 0.62, rationale: "Claude Code placeholder vote" };
}

function testEvidence(request: AgentRunRequest): unknown[] {
  if (["research", "review", "unification", "branch_vote", "branch_unification"].includes(taskPurposeKind(request.task.purpose))) return [];
  return [
    {
      command: "stub claude-code execution evidence",
      exit_code: 0,
      passed: mode === "stub" || mode === "cli-healthcheck",
      duration_ms: 0,
      stdout_uri: `memory://claude-code-runner/${request.task.id}/stdout`,
      stderr_uri: null,
      artifact_uri: `memory://claude-code-runner/${request.task.id}`,
      notes: ["Stub mode records evidence shape; live Claude Code adapters must record real execution outputs."],
    },
  ];
}

function checkpointRefs(request: AgentRunRequest): unknown[] {
  if (!isRecord(request.task.execution?.results) || !isRecord(request.task.execution?.results.checkpoints)) return [];
  if (request.task.execution.results.checkpoints.enabled !== true) return [];
  return [
    {
      kind: "metadata",
      uri: `memory://claude-code-runner/checkpoint/${request.task.id}`,
      description: "Claude Code runner checkpoint placeholder",
      created_at: new Date().toISOString(),
      task_id: request.task.id,
    },
  ];
}

function taskPurposeKind(purpose: unknown): string {
  return isRecord(purpose) && typeof purpose.kind === "string" ? purpose.kind : "work";
}

function subagentDiagnostics(subagents: unknown): string[] {
  const mode = isRecord(subagents) && typeof subagents.mode === "string" ? subagents.mode : "coordinator_durable_tasks";
  const nativeSpawn = isRecord(subagents) && typeof subagents.native_spawn === "string" ? subagents.native_spawn : "disabled";
  return [`subagents.mode=${mode}`, `subagents.native_spawn=${nativeSpawn}`, "subagents.child_requests_only=true"];
}

function mcpDiagnostics(mcp: unknown): string[] {
  if (!isRecord(mcp)) return ["mcp_context=absent"];
  const servers = Array.isArray(mcp.servers) ? mcp.servers.length : 0;
  const propagation = typeof mcp.propagation === "string" ? mcp.propagation : "unknown";
  return [`mcp_servers=${servers}`, `mcp_propagation=${propagation}`, "mcp_secret_values_exposed=false"];
}

function redactedMcpServer(server: McpServerRef): Record<string, unknown> {
  return {
    name: server.name,
    transport: server.transport,
    uri: server.uri,
    allowed_tools: server.allowed_tools,
    auth_kind: isRecord(server.auth) && typeof server.auth.kind === "string" ? server.auth.kind : "unknown",
  };
}

async function readJson(req: http.IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  const raw = Buffer.concat(chunks).toString("utf8");
  return raw ? JSON.parse(raw) : {};
}

function json(res: http.ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body, null, 2));
}

type LogLevel = "debug" | "info" | "warn" | "error";

function log(level: LogLevel, message: string, fields: Record<string, unknown> = {}): void {
  if (!logEnabled(level)) return;
  const entry = {
    ts: new Date().toISOString(),
    level,
    service: "claude-code-runner-ts",
    message,
    runner_id: runnerId,
    node_id: nodeId,
    ...fields,
  };
  if ((process.env.COAT_LOG_FORMAT ?? "compact").toLowerCase() === "json") {
    console.error(JSON.stringify(entry));
    return;
  }
  console.error(`${entry.ts} ${level.toUpperCase()} ${entry.service} ${message} ${JSON.stringify(fields)}`);
}

function logEnabled(level: LogLevel): boolean {
  const order: Record<LogLevel, number> = { debug: 10, info: 20, warn: 30, error: 40 };
  const configured = (process.env.COAT_NODE_LOG_LEVEL ?? process.env.COAT_LOG_LEVEL ?? "info").toLowerCase() as LogLevel;
  return order[level] >= (order[configured] ?? order.info);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function parseJsonEnv<T>(name: string, fallback: T): T {
  const raw = process.env[name];
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch (error) {
    log("warn", "invalid_json_env_using_fallback", { name, error: errorMessage(error) });
    return fallback;
  }
}

function numberEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
