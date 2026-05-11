/**
 * Codex runner sidecar.
 *
 * Purpose: adapt COAT's `AgentRunRequest` / `AgentRunResult` contract to Codex
 * execution surfaces. Stub mode is the local smoke path; live modes are gated
 * by environment and should run only in isolated workspaces.
 *
 * Architecture references:
 * - docs/exec-plans/active/030-codex-worker.md
 * - docs/design-docs/010-distributed-runners-mcp.md
 * - docs/design-docs/060-result-channels-git-object-storage.md
 */
import http from "node:http";
import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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
  | "claude_code"
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

type ModelFeature =
  | "tool_use"
  | "json_schema"
  | "streaming"
  | "vision"
  | "long_context"
  | "reasoning"
  | "embeddings"
  | "local_weights"
  | "web_search";

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

type SecretRef = {
  provider?: string;
  name?: string;
  key?: string | null;
  namespace?: string | null;
  audience?: string | null;
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

type RunnerHeartbeat = {
  runner_id: string;
  node_id: string;
  running_tasks: number;
  capacity_remaining: number;
};

type AgentRunRequest = {
  goal_id: string;
  task: {
    id: string;
    role: string;
    purpose?: unknown;
    review_doctrine?: unknown;
    prompt: string;
    execution?: {
      model?: {
        candidates?: unknown[];
      };
      mcp?: unknown;
      subagents?: unknown;
      results?: unknown;
    };
    sandbox?: {
      filesystem?: string;
      network?: string;
      approval_policy?: string;
      isolated_runner?: boolean;
      [key: string]: unknown;
    };
    done_criteria?: {
      tests_pass?: boolean;
      artifact_exists?: boolean;
      validator_score_min?: number | null;
      [key: string]: unknown;
    };
  };
  timeout_seconds?: number | null;
};

type AgentRunResult = {
  task_id: string;
  status: "done" | "partial" | "blocked" | "failed" | "timed_out";
  summary: string;
  review: ReviewOutput | null;
  research: ResearchOutput | null;
  branch_vote: BranchVoteOutput | null;
  runner_id: string | null;
  model_used: unknown | null;
  mcp_context_used: unknown | null;
  sandbox_attestation: unknown | null;
  artifacts: Array<{ kind: string; uri: string; description: string; sha256?: string | null }>;
  git_result: unknown | null;
  object_artifacts: unknown[];
  checkpoints: unknown[];
  test_evidence: TestCommandEvidence[];
  child_requests: unknown[];
  confidence: number;
  next_actions: string[];
  diagnostics: string[];
  notification_reports: unknown[];
};

type ReviewOutput = {
  decision: "accept" | "changes_requested" | "blocked" | "inconclusive";
  reward: number;
  findings: Array<{
    severity: "info" | "low" | "medium" | "high" | "critical";
    title: string;
    body: string;
    file: string | null;
    start_line: number | null;
    end_line: number | null;
    priority: number | null;
    evidence: string[];
    suggested_action: string | null;
  }>;
  objective_results: ReviewObjectiveResult[];
  gate_results: ValidationGateResult[];
  retry_recommended: boolean;
  unification_summary: string | null;
};

type ReviewObjectiveResult = {
  objective_id: string;
  decision: "pass" | "fail" | "not_applicable" | "not_checked";
  score: number;
  evidence: string[];
  notes: string[];
};

type ValidationGateResult = {
  gate_id: string;
  passed: boolean;
  score: number;
  evidence: string[];
  notes: string[];
};

type TestCommandEvidence = {
  command: string;
  exit_code: number | null;
  passed: boolean;
  duration_ms: number | null;
  stdout_uri: string | null;
  stderr_uri: string | null;
  artifact_uri: string | null;
  notes: string[];
};

type ResearchOutput = {
  question: string;
  answer: string;
  sources: Array<{
    title: string;
    uri: string;
    quality: "primary" | "official_docs" | "peer_reviewed" | "reputable_secondary" | "community" | "unknown";
    captured_at: string | null;
    quote: string | null;
    summary: string;
    confidence: number;
  }>;
  confidence: number;
  use_plan: {
    facts_to_use: string[];
    facts_to_avoid: string[];
    proposed_task_updates: unknown[];
    validation_checks: string[];
  };
  open_questions: string[];
};

type BranchVoteOutput = {
  group_id: string;
  selected_task_id: string;
  ranked_task_ids: string[];
  confidence: number;
  rationale: string;
};

type MemoryContextResponse = {
  goal_id: string;
  task_id: string | null;
  query: string;
  hits: Array<{
    key: string;
    scope: string;
    score: number;
    summary: string;
    source: unknown;
    tags: string[];
  }>;
  use_plan: {
    facts_to_use: string[];
    facts_to_avoid: string[];
    proposed_task_updates: unknown[];
    validation_checks: string[];
  };
  adapter_reports: Array<{
    store_kind: string;
    operation: string;
    attempted: boolean;
    success: boolean;
    external_ref: string | null;
    error: string | null;
  }>;
};

type RunnerMode = "stub" | "live" | "replay" | "mcp-replay" | "mcp-healthcheck";

type JsonRpcMessage = Record<string, unknown>;

type AppServerRunTrace = {
  source: "live" | "replay";
  app_server_url: string | null;
  thread: Record<string, unknown>;
  turn: Record<string, unknown>;
  events: JsonRpcMessage[];
  server_requests: JsonRpcMessage[];
  approval_reports: Record<string, unknown>[];
  final_response: string;
  items: Record<string, unknown>[];
  usage: unknown | null;
  diagnostics: string[];
};

type McpFallbackRunTrace = {
  source: "replay";
  server: Record<string, unknown>;
  tool_calls: Record<string, unknown>[];
  final_response: string;
  usage: unknown | null;
  diagnostics: string[];
};

type ProviderVerificationLane = {
  lane_id: string;
  provider: string;
  surface: string;
  model: string | null;
  endpoint: string | null;
  source: string;
};

type ProviderVerificationProfile = ProviderVerificationLane & {
  status: "verified" | "skipped" | "failed";
  attempted: boolean;
  available: boolean | null;
  configured: boolean;
  skipped_reason: string | null;
  error: string | null;
  auth: Record<string, unknown>;
  checks: Array<Record<string, unknown>>;
  secret_values_exposed: false;
};

type AgentResultPatch = Partial<
  Pick<AgentRunResult, "status" | "summary" | "review" | "research" | "branch_vote" | "test_evidence" | "child_requests" | "confidence" | "next_actions">
>;

const port = Number(process.env.PORT ?? "9091");
const runnerId = process.env.RUNNER_ID ?? "codex-runner-ts";
const nodeId = process.env.NODE_ID ?? process.env.HOSTNAME ?? "local-node";
const registryUrl = process.env.RUNNER_REGISTRY_URL ?? process.env.COAT_RUNNER_REGISTRY_URL ?? "";
const runnerEndpoint = process.env.RUNNER_ENDPOINT ?? `http://localhost:${port}`;
const memoryGatewayUrl = process.env.COAT_MEMORY_GATEWAY_URL ?? process.env.MEMORY_GATEWAY_URL ?? "";
const memoryGatewayToken = process.env.COAT_MEMORY_GATEWAY_TOKEN ?? process.env.MEMORY_GATEWAY_TOKEN ?? "";
const memoryContextLimit = numberEnv("MEMORY_CONTEXT_LIMIT", 8);
const leaseTtlSeconds = numberEnv("RUNNER_LEASE_TTL_SECONDS", 300);
const heartbeatIntervalMs = numberEnv("RUNNER_HEARTBEAT_INTERVAL_MS", 30_000);
const maxConcurrency = numberEnv("RUNNER_MAX_CONCURRENCY", 1);
let runningTasks = 0;

const durableSubagentContext = [
  "Treat any request to use a subagent as a request to create a COAT durable child task.",
  "Do not spawn native Codex, SDK, MCP-client, or framework-local subagents from inside this runner.",
  "Return proposed child work only through AgentRunResult.child_requests.",
  "The coordinator validates budgets, approval policy, routing, memory context, and sandbox policy before dispatch.",
];

function runnerMode(): RunnerMode {
  const raw = (process.env.CODEX_RUNNER_MODE ?? "stub").trim().toLowerCase();
  if (raw === "live" || raw === "app-server" || raw === "app_server") return "live";
  if (raw === "replay" || raw === "fixture") return "replay";
  if (raw === "mcp-replay" || raw === "mcp_replay" || raw === "mcp-fallback-replay" || raw === "mcp_fallback_replay") {
    return "mcp-replay";
  }
  if (raw === "mcp-healthcheck") return "mcp-healthcheck";
  return "stub";
}

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET" && req.url === "/healthz") {
      return json(res, 200, { status: "ok", mode: runnerMode(), runner_id: runnerId });
    }
    if (req.method === "GET" && req.url === "/registration") {
      return json(res, 200, buildRegistration());
    }
    if (req.method === "GET" && req.url === "/capabilities") {
      return json(res, 200, buildCapabilities());
    }
    if (req.method === "GET" && req.url === "/verify") {
      return json(res, 200, await verifyCodexIntegration());
    }
    if (req.method === "POST" && req.url === "/run-task") {
      const body = (await readJson(req)) as AgentRunRequest;
      return json(res, 200, await runTask(body));
    }
    json(res, 404, { error: "not_found" });
  } catch (error) {
    json(res, 500, { error: error instanceof Error ? error.message : String(error) });
  }
});

if (isMainModule()) {
  server.listen(port, () => {
    console.log(`codex-runner-ts listening on :${port} (${runnerMode()})`);
    void registerAndHeartbeat();
  });
}

export async function runTask(request: AgentRunRequest): Promise<AgentRunResult> {
  runningTasks += 1;
  try {
    const mode = runnerMode();
    if (mode === "live") return await runLiveAppServerTask(request);
    if (mode === "replay") return runReplayTask(request);
    if (mode === "mcp-replay") return runMcpReplayTask(request);
    if (mode === "mcp-healthcheck") return await runMcpHealthcheckTask(request);

    let memoryContextError: string | null = null;
    const memoryContext = await fetchMemoryContext(request).catch((error) => {
      memoryContextError = error instanceof Error ? error.message : String(error);
      return null;
    });

    const gitResult = buildGitResult(request);
    const objectArtifacts = buildObjectArtifacts(request);
    const checkpoints = buildCheckpoints(request, gitResult, objectArtifacts);

    return {
      task_id: request.task.id,
      status: "done",
      summary: taskSummary(request),
      review: reviewOutput(request),
      research: researchOutput(request, memoryContext),
      branch_vote: branchVoteOutput(request),
      runner_id: runnerId,
      model_used: request.task.execution?.model?.candidates?.[0] ?? null,
      mcp_context_used: request.task.execution?.mcp ?? null,
      sandbox_attestation: sandboxAttestation(request, false, false, "stub"),
      artifacts: [
        {
          kind: "report",
          uri: `memory://codex-runner/${request.task.id}`,
          description: `${taskPurposeKind(request.task.purpose)} artifact from Codex runner`,
          sha256: null,
        },
        ...resultChannelArtifacts(request, gitResult, objectArtifacts),
        ...memoryContextArtifacts(request, memoryContext),
      ],
      git_result: gitResult,
      object_artifacts: objectArtifacts,
      checkpoints,
      test_evidence: testEvidence(request),
      child_requests: [],
      confidence: 0.9,
      next_actions: [],
      diagnostics: [
        `mode=${mode}`,
        `runner_id=${runnerId}`,
        `node_id=${nodeId}`,
        `task_purpose=${JSON.stringify(request.task.purpose ?? { kind: "work" })}`,
        ...subagentDiagnostics(request.task.execution?.subagents),
        ...mcpDiagnostics(request.task.execution?.mcp),
        ...memoryContextDiagnostics(memoryContext, memoryContextError),
      ],
      notification_reports: [],
    };
  } finally {
    runningTasks -= 1;
  }
}

async function runLiveAppServerTask(request: AgentRunRequest): Promise<AgentRunResult> {
  const gateFailures = appServerLiveGateFailures(request);
  if (gateFailures.length > 0) {
    return blockedResult(request, "Codex App Server live execution is not enabled for this task", [
      "mode=live",
      "app_server_live_gate=blocked",
      ...gateFailures.map((failure) => `app_server_live_gate_failure=${failure}`),
    ]);
  }

  let memoryContextError: string | null = null;
  const memoryContext = await fetchMemoryContext(request).catch((error) => {
    memoryContextError = error instanceof Error ? error.message : String(error);
    return null;
  });

  try {
    const trace = await runCodexAppServerTurn(request, memoryContext);
    return buildCodexAppServerResult(request, trace, memoryContext, memoryContextError);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return failedResult(request, "Codex App Server live execution failed before producing a valid result", [
      "mode=live",
      "app_server_live_gate=passed",
      `app_server_error=${message}`,
      ...memoryContextDiagnostics(memoryContext, memoryContextError),
    ]);
  }
}

function runReplayTask(request: AgentRunRequest): AgentRunResult {
  const trace = traceFromReplayFixture(loadReplayFixture());
  return buildCodexAppServerResult(request, trace, null, null);
}

function runMcpReplayTask(request: AgentRunRequest): AgentRunResult {
  const trace = traceFromMcpReplayFixture(loadMcpReplayFixture());
  return buildCodexMcpFallbackResult(request, trace);
}

async function runMcpHealthcheckTask(request: AgentRunRequest): Promise<AgentRunResult> {
  try {
    await ensureCodexMcpStarts();
    return {
      task_id: request.task.id,
      status: "done",
      summary: `Codex MCP healthcheck completed for task ${request.task.id}`,
      review: null,
      research: null,
      branch_vote: null,
      runner_id: runnerId,
      model_used: request.task.execution?.model?.candidates?.[0] ?? null,
      mcp_context_used: request.task.execution?.mcp ?? null,
      sandbox_attestation: sandboxAttestation(request, false, false, "codex_mcp_healthcheck"),
      artifacts: [
        {
          kind: "report",
          uri: `codex-mcp://healthcheck/${request.task.id}`,
          description: "Codex MCP server startup healthcheck result",
          sha256: null,
        },
      ],
      git_result: null,
      object_artifacts: [],
      checkpoints: buildCheckpoints(request, null, [], {
        source: "live",
        app_server_url: null,
        thread: {},
        turn: { id: `mcp-healthcheck-${request.task.id}`, status: "completed" },
        events: [],
        server_requests: [],
        approval_reports: [],
        final_response: "",
        items: [],
        usage: null,
        diagnostics: ["codex_mcp_server=available"],
      }),
      test_evidence: [
        {
          command: "codex mcp-server",
          exit_code: 0,
          passed: true,
          duration_ms: 1000,
          stdout_uri: null,
          stderr_uri: null,
          artifact_uri: `codex-mcp://healthcheck/${request.task.id}`,
          notes: ["The Codex MCP server process started and stayed alive until the healthcheck timeout."],
        },
      ],
      child_requests: [],
      confidence: 0.8,
      next_actions: [],
      diagnostics: ["mode=mcp-healthcheck", "codex_mcp_server=available"],
      notification_reports: [],
    };
  } catch (error) {
    return failedResult(request, "Codex MCP healthcheck failed", [
      "mode=mcp-healthcheck",
      `codex_mcp_error=${error instanceof Error ? error.message : String(error)}`,
    ]);
  }
}

function appServerLiveGateFailures(request: AgentRunRequest): string[] {
  const failures: string[] = [];
  const authMode = (process.env.CODEX_AUTH_MODE ?? "env_api_key").trim().toLowerCase();
  const appServerUrl = process.env.CODEX_APP_SERVER_URL?.trim() ?? "";
  const cwd = liveWorkingDirectory(request);

  if (authMode !== "app_server") {
    failures.push("CODEX_AUTH_MODE must be app_server for Codex App Server live mode");
  }
  if (!appServerUrl) {
    failures.push("CODEX_APP_SERVER_URL is required");
  } else if (!normalizeAppServerWebSocketUrl(appServerUrl)) {
    failures.push("CODEX_APP_SERVER_URL must use ws://, wss://, http://, or https:// for this runner slice");
  }
  if (request.task.sandbox?.isolated_runner !== true && !truthyEnv("CODEX_ALLOW_NON_ISOLATED_LIVE")) {
    failures.push("task.sandbox.isolated_runner must be true for live Codex execution");
  }
  if (!cwd) {
    failures.push("CODEX_APP_SERVER_CWD, CODEX_WORKSPACE_DIR, or an existing task git worktree is required");
  } else if (!existsSync(cwd)) {
    failures.push(`live workspace does not exist: ${cwd}`);
  }
  return failures;
}

function blockedResult(request: AgentRunRequest, summary: string, diagnostics: string[]): AgentRunResult {
  return baseNonStubResult(request, "blocked", summary, diagnostics, 0.05);
}

function failedResult(request: AgentRunRequest, summary: string, diagnostics: string[]): AgentRunResult {
  return baseNonStubResult(request, "failed", summary, diagnostics, 0.05);
}

function baseNonStubResult(
  request: AgentRunRequest,
  status: AgentRunResult["status"],
  summary: string,
  diagnostics: string[],
  confidence: number,
): AgentRunResult {
  return {
    task_id: request.task.id,
    status,
    summary,
    review: null,
    research: null,
    branch_vote: null,
    runner_id: runnerId,
    model_used: request.task.execution?.model?.candidates?.[0] ?? null,
    mcp_context_used: request.task.execution?.mcp ?? null,
    sandbox_attestation: sandboxAttestation(request, false, false, "declared"),
    artifacts: [
      {
        kind: "report",
        uri: `codex-runner://${request.goal_id}/${request.task.id}/gate`,
        description: "Codex runner gate report",
        sha256: null,
      },
    ],
    git_result: null,
    object_artifacts: [],
    checkpoints: [],
    test_evidence: [],
    child_requests: [],
    confidence,
    next_actions: diagnostics
      .filter((line) => line.includes("_failure="))
      .map((line) => line.slice(line.indexOf("=") + 1)),
    diagnostics,
    notification_reports: [],
  };
}

function testEvidence(request: AgentRunRequest): TestCommandEvidence[] {
  const purpose = taskPurposeKind(request.task.purpose);
  if (purpose === "research" || purpose === "review" || purpose === "unification" || purpose === "branch_vote" || purpose === "branch_unification") {
    return [];
  }
  return [
    {
      command: "stub codex test evidence",
      exit_code: 0,
      passed: true,
      duration_ms: 0,
      stdout_uri: `memory://codex-runner/test-evidence/${request.task.id}/stdout`,
      stderr_uri: null,
      artifact_uri: `memory://codex-runner/test-evidence/${request.task.id}`,
      notes: ["Stub mode records the test evidence contract shape; live mode should include real command output."],
    },
  ];
}

function reviewOutput(request: AgentRunRequest): ReviewOutput | null {
  const purpose = taskPurposeKind(request.task.purpose);
  const objectiveResults = reviewObjectiveResults(request);
  const gateResults = validationGateResults(request);
  if (purpose === "review") {
    return {
      decision: "accept",
      reward: 0.9,
      findings: [],
      objective_results: objectiveResults,
      gate_results: gateResults,
      retry_recommended: false,
      unification_summary: null,
    };
  }
  if (purpose === "unification") {
    return {
      decision: "accept",
      reward: 0.9,
      findings: [],
      objective_results: objectiveResults,
      gate_results: gateResults,
      retry_recommended: false,
      unification_summary: "stub unifier accepted critic evidence",
    };
  }
  if (purpose === "branch_vote") {
    return {
      decision: "accept",
      reward: 0.8,
      findings: [],
      objective_results: objectiveResults,
      gate_results: gateResults,
      retry_recommended: false,
      unification_summary: "stub branch vote selected a candidate",
    };
  }
  if (purpose === "branch_unification") {
    return {
      decision: "accept",
      reward: 0.9,
      findings: [],
      objective_results: objectiveResults,
      gate_results: gateResults,
      retry_recommended: false,
      unification_summary: "stub branch unifier joined candidate votes",
    };
  }
  return null;
}

const PRESET_OBJECTIVES: Record<string, string[]> = {
  core_engineering: ["correctness.behavior", "quality.maintainability", "abstraction.future_time"],
  testing: ["testing.regression", "testing.hypothesis"],
  formal_methods: ["formal.type_soundness", "formal.verification"],
  functional_domain_driven_design: ["ddd.ubiquitous_language", "functional.core", "semantics.denotational"],
  laziness_lost: ["simplicity.negative_code"],
  security: ["security.boundaries"],
  performance: ["performance.evidence"],
};

const PRESET_GATES: Record<string, string[]> = {
  core_engineering: ["gate.compile"],
  testing: ["gate.tests"],
  formal_methods: ["gate.type_soundness"],
  functional_domain_driven_design: ["gate.domain_model"],
  laziness_lost: ["gate.simplicity"],
  security: ["gate.security_boundaries"],
  performance: [],
};

function reviewObjectiveResults(request: AgentRunRequest): ReviewObjectiveResult[] {
  return requiredDoctrineIds(request.task.review_doctrine, "custom_objectives", PRESET_OBJECTIVES, "objective").map((id) => ({
    objective_id: id,
    decision: "pass",
    score: 0.9,
    evidence: [`memory://review-objective/${request.task.id}/${id}`],
    notes: [`stub Codex review covered ${id}`],
  }));
}

function validationGateResults(request: AgentRunRequest): ValidationGateResult[] {
  return requiredDoctrineIds(request.task.review_doctrine, "custom_validation_gates", PRESET_GATES, "validation_gate").map((id) => ({
    gate_id: id,
    passed: true,
    score: 0.9,
    evidence: [`memory://validation-gate/${request.task.id}/${id}`],
    notes: [`stub Codex gate covered ${id}`],
  }));
}

function requiredDoctrineIds(
  doctrine: unknown,
  customKey: string,
  presets: Record<string, string[]>,
  overrideTarget: string,
): string[] {
  if (!isRecord(doctrine) || doctrine.enabled !== true) return [];
  const ids = new Set<string>();
  const presetNames = Array.isArray(doctrine.presets)
    ? doctrine.presets.filter((preset): preset is string => typeof preset === "string")
    : ["core_engineering"];
  for (const preset of presetNames) {
    for (const id of presets[preset] ?? []) ids.add(id);
  }
  const custom = doctrine[customKey];
  if (Array.isArray(custom)) {
    for (const item of custom) {
      if (isRecord(item) && typeof item.id === "string" && item.required !== false) ids.add(item.id);
    }
  }
  const overrides = Array.isArray(doctrine.overrides) ? doctrine.overrides : [];
  for (const override of overrides) {
    if (!isRecord(override) || override.target !== overrideTarget || typeof override.id !== "string") continue;
    if (override.action === "disable" || override.action === "make_optional") ids.delete(override.id);
    if (override.action === "require") ids.add(override.id);
  }
  return [...ids].sort();
}

function branchVoteOutput(request: AgentRunRequest): BranchVoteOutput | null {
  const purpose = request.task.purpose;
  if (!isRecord(purpose)) return null;
  if (purpose.kind !== "branch_vote" && purpose.kind !== "branch_unification") return null;
  const candidateTaskIds = Array.isArray(purpose.candidate_task_ids)
    ? purpose.candidate_task_ids.filter((candidate): candidate is string => typeof candidate === "string")
    : [];
  const groupId = typeof purpose.group_id === "string" ? purpose.group_id : null;
  const selectedTaskId = candidateTaskIds[0] ?? null;
  if (!groupId || !selectedTaskId) return null;
  return {
    group_id: groupId,
    selected_task_id: selectedTaskId,
    ranked_task_ids: candidateTaskIds,
    confidence: purpose.kind === "branch_unification" ? 0.82 : 0.75,
    rationale:
      purpose.kind === "branch_unification"
        ? "stub branch unifier selected the first validated candidate"
        : "stub branch voter selected the first candidate",
  };
}

function researchOutput(request: AgentRunRequest, memoryContext: MemoryContextResponse | null): ResearchOutput | null {
  const purpose = request.task.purpose;
  if (!isRecord(purpose) || purpose.kind !== "research") return null;
  const question = typeof purpose.question === "string" ? purpose.question : request.task.prompt;
  const contextFacts = memoryContext?.use_plan.facts_to_use.slice(0, 3) ?? [];
  return {
    question,
    answer: `stub Codex research answer for: ${question}`,
    sources: [
      {
        title: "Codex runner stub source",
        uri: `memory://codex-runner/research/${request.task.id}`,
        quality: "unknown",
        captured_at: null,
        quote: null,
        summary: "Structured placeholder source; enable a live research runner for web/doc search.",
        confidence: 0.75,
      },
    ],
    confidence: 0.75,
    use_plan: {
      facts_to_use: ["Use this only as a contract smoke result.", ...contextFacts],
      facts_to_avoid: [
        "Do not treat stub research as externally verified evidence.",
        ...(memoryContext?.use_plan.facts_to_avoid ?? []),
      ],
      proposed_task_updates: [],
      validation_checks: [
        "Replace stub source with primary or official sources.",
        ...(memoryContext?.use_plan.validation_checks ?? []),
      ],
    },
    open_questions: [],
  };
}

async function runCodexAppServerTurn(
  request: AgentRunRequest,
  memoryContext: MemoryContextResponse | null,
): Promise<AppServerRunTrace> {
  const appServerUrl = process.env.CODEX_APP_SERVER_URL?.trim() ?? "";
  const websocketUrl = normalizeAppServerWebSocketUrl(appServerUrl);
  if (!websocketUrl) throw new Error("unsupported CODEX_APP_SERVER_URL");

  const timeoutMs = Math.max(1, request.timeout_seconds ?? numberEnv("CODEX_APP_SERVER_TIMEOUT_SECONDS", 600)) * 1000;
  const client = new AppServerJsonRpcClient(websocketUrl, timeoutMs, appServerApprovalResult);
  await client.connect();
  try {
    const initialize = await client.request("initialize", {
      clientInfo: {
        name: "coat_codex_runner",
        title: "COAT Codex Runner",
        version: "0.1.0",
      },
      capabilities: {
        experimentalApi: false,
        optOutNotificationMethods: ["item/agentMessage/delta"],
      },
    });
    client.notify("initialized", {});

    const existingThreadId = requestedCodexThreadId(request);
    const threadParams = appServerThreadParams(request);
    const threadResponse = existingThreadId
      ? await client.request("thread/resume", { ...threadParams, threadId: existingThreadId, excludeTurns: true })
      : await client.request("thread/start", threadParams);
    const thread = responseRecord(threadResponse, "thread");
    const threadId = stringValue(thread.id) ?? existingThreadId;
    if (!threadId) throw new Error("Codex App Server did not return a thread id");

    const turnResponse = await client.request("turn/start", {
      threadId,
      input: [{ type: "text", text: codexAppServerPrompt(request, memoryContext) }],
      ...appServerTurnParams(request),
      outputSchema: agentRunResultOutputSchema(),
    });
    const startedTurn = responseRecord(turnResponse, "turn");
    const turnId = stringValue(startedTurn.id);
    if (!turnId) throw new Error("Codex App Server did not return a turn id");

    const completed = await client.waitFor((message) => {
      if (message.method !== "turn/completed") return false;
      const params = isRecord(message.params) ? message.params : {};
      const completedTurn = isRecord(params.turn) ? params.turn : params;
      return stringValue(completedTurn.id) === turnId || stringValue(params.turnId) === turnId;
    });
    const completedParams = isRecord(completed.params) ? completed.params : {};
    const completedTurn = isRecord(completedParams.turn) ? completedParams.turn : startedTurn;
    const items = completedItems(client.messages);
    const finalResponse = finalAgentMessage(items);

    return {
      source: "live",
      app_server_url: redactUrl(websocketUrl),
      thread,
      turn: { ...startedTurn, ...completedTurn },
      events: client.messages,
      server_requests: client.serverRequests,
      approval_reports: client.approvalReports,
      final_response: finalResponse,
      items,
      usage: completedParams.usage ?? null,
      diagnostics: [
        `app_server_initialize=${isRecord(initialize) ? "ok" : "unknown"}`,
        `app_server_thread_id=${threadId}`,
        `app_server_turn_id=${turnId}`,
        `app_server_items=${items.length}`,
        `app_server_server_requests=${client.serverRequests.length}`,
        `app_server_approvals=${client.approvalReports.length}`,
      ],
    };
  } finally {
    client.close();
  }
}

function buildCodexAppServerResult(
  request: AgentRunRequest,
  trace: AppServerRunTrace,
  memoryContext: MemoryContextResponse | null,
  memoryContextError: string | null,
): AgentRunResult {
  const resultPatch = parseAgentResultPatch(trace.final_response);
  const gitResult = buildGitResultFromTrace(request, trace);
  const objectArtifacts = buildObjectArtifactsFromTrace(request, trace);
  const checkpoints = buildCheckpoints(request, gitResult, objectArtifacts, trace);
  const patchStatus = normalizeResultStatus(resultPatch.patch.status);
  if (!resultPatch.parsed || !patchStatus) {
    return invalidCodexAppServerStructuredResult(
      request,
      trace,
      memoryContext,
      memoryContextError,
      gitResult,
      objectArtifacts,
      checkpoints,
      resultPatch.parsed,
    );
  }
  const status = patchStatus;
  const summary =
    typeof resultPatch.patch.summary === "string" && resultPatch.patch.summary.trim()
      ? resultPatch.patch.summary.trim()
      : `Codex App Server ${trace.source} completed task ${request.task.id}`;

  return {
    task_id: request.task.id,
    status,
    summary,
    review: resultPatch.patch.review ?? reviewOutput(request),
    research: resultPatch.patch.research ?? researchOutput(request, memoryContext),
    branch_vote: resultPatch.patch.branch_vote ?? branchVoteOutput(request),
    runner_id: runnerId,
    model_used: request.task.execution?.model?.candidates?.[0] ?? null,
    mcp_context_used: request.task.execution?.mcp ?? null,
    sandbox_attestation: sandboxAttestation(request, false, false, "codex_app_server"),
    artifacts: [
      {
        kind: "report",
        uri: codexThreadUri(trace),
        description: "Codex App Server thread and turn transcript reference",
        sha256: null,
      },
      ...resultChannelArtifacts(request, gitResult, objectArtifacts),
      ...memoryContextArtifacts(request, memoryContext),
    ],
    git_result: gitResult,
    object_artifacts: objectArtifacts,
    checkpoints,
    test_evidence: [...commandEvidenceFromTrace(request, trace), ...(resultPatch.patch.test_evidence ?? [])],
    child_requests: Array.isArray(resultPatch.patch.child_requests) ? resultPatch.patch.child_requests : [],
    confidence: typeof resultPatch.patch.confidence === "number" ? resultPatch.patch.confidence : trace.source === "live" ? 0.75 : 0.85,
    next_actions: Array.isArray(resultPatch.patch.next_actions) ? resultPatch.patch.next_actions.filter(isString) : [],
    diagnostics: [
      `mode=${trace.source === "live" ? "live" : "replay"}`,
      `codex_app_server_source=${trace.source}`,
      `codex_app_server_thread_id=${stringValue(trace.thread.id) ?? "unknown"}`,
      `codex_app_server_session_id=${stringValue(trace.thread.sessionId) ?? stringValue(trace.thread.session_id) ?? "unknown"}`,
      `codex_app_server_turn_id=${stringValue(trace.turn.id) ?? "unknown"}`,
      `codex_app_server_events=${trace.events.length}`,
      `codex_app_server_items=${trace.items.length}`,
      `codex_app_server_final_json=${resultPatch.parsed}`,
      ...trace.diagnostics,
      ...memoryContextDiagnostics(memoryContext, memoryContextError),
      ...subagentDiagnostics(request.task.execution?.subagents),
      ...mcpDiagnostics(request.task.execution?.mcp),
    ],
    notification_reports: [],
  };
}

function buildCodexMcpFallbackResult(request: AgentRunRequest, trace: McpFallbackRunTrace): AgentRunResult {
  const resultPatch = parseAgentResultPatch(trace.final_response);
  const gitResult = buildGitResult(request);
  const objectArtifacts = buildObjectArtifactsFromMcpTrace(request);
  const checkpoints = buildCheckpoints(request, gitResult, objectArtifacts, trace);
  const patchStatus = normalizeResultStatus(resultPatch.patch.status);
  if (!resultPatch.parsed || !patchStatus) {
    return invalidCodexMcpStructuredResult(request, trace, gitResult, objectArtifacts, checkpoints, resultPatch.parsed);
  }
  const status = patchStatus;
  const summary =
    typeof resultPatch.patch.summary === "string" && resultPatch.patch.summary.trim()
      ? resultPatch.patch.summary.trim()
      : `Codex MCP fallback replay completed task ${request.task.id}`;

  return {
    task_id: request.task.id,
    status,
    summary,
    review: resultPatch.patch.review ?? reviewOutput(request),
    research: resultPatch.patch.research ?? null,
    branch_vote: resultPatch.patch.branch_vote ?? branchVoteOutput(request),
    runner_id: runnerId,
    model_used: request.task.execution?.model?.candidates?.[0] ?? null,
    mcp_context_used: request.task.execution?.mcp ?? null,
    sandbox_attestation: sandboxAttestation(request, false, false, "codex_mcp_fallback_replay"),
    artifacts: [
      {
        kind: "report",
        uri: codexMcpFallbackUri(request),
        description: "Codex MCP fallback replay transcript reference",
        sha256: null,
      },
      ...resultChannelArtifacts(request, gitResult, objectArtifacts),
    ],
    git_result: gitResult,
    object_artifacts: objectArtifacts,
    checkpoints,
    test_evidence: [...commandEvidenceFromMcpTrace(request, trace), ...(resultPatch.patch.test_evidence ?? [])],
    child_requests: Array.isArray(resultPatch.patch.child_requests) ? resultPatch.patch.child_requests : [],
    confidence: typeof resultPatch.patch.confidence === "number" ? resultPatch.patch.confidence : 0.82,
    next_actions: Array.isArray(resultPatch.patch.next_actions) ? resultPatch.patch.next_actions.filter(isString) : [],
    diagnostics: [
      "mode=mcp-replay",
      `codex_mcp_source=${trace.source}`,
      `codex_mcp_server_command=${stringValue(trace.server.command) ?? "codex mcp-server"}`,
      `codex_mcp_transport=${stringValue(trace.server.transport) ?? "stdio"}`,
      `codex_mcp_tool_calls=${trace.tool_calls.length}`,
      `codex_mcp_final_json=${resultPatch.parsed}`,
      ...trace.diagnostics,
      ...subagentDiagnostics(request.task.execution?.subagents),
      ...mcpDiagnostics(request.task.execution?.mcp),
    ],
    notification_reports: [],
  };
}

function invalidCodexAppServerStructuredResult(
  request: AgentRunRequest,
  trace: AppServerRunTrace,
  memoryContext: MemoryContextResponse | null,
  memoryContextError: string | null,
  gitResult: Record<string, unknown> | null,
  objectArtifacts: Record<string, unknown>[],
  checkpoints: unknown[],
  parsed: boolean,
): AgentRunResult {
  return {
    task_id: request.task.id,
    status: "failed",
    summary: `Codex App Server ${trace.source} did not return a valid structured AgentRunResult payload.`,
    review: null,
    research: null,
    branch_vote: null,
    runner_id: runnerId,
    model_used: request.task.execution?.model?.candidates?.[0] ?? null,
    mcp_context_used: request.task.execution?.mcp ?? null,
    sandbox_attestation: sandboxAttestation(request, false, false, "codex_app_server"),
    artifacts: [
      {
        kind: "report",
        uri: codexThreadUri(trace),
        description: "Codex App Server thread and turn transcript reference for invalid structured output",
        sha256: null,
      },
      ...resultChannelArtifacts(request, gitResult, objectArtifacts),
      ...memoryContextArtifacts(request, memoryContext),
    ],
    git_result: gitResult,
    object_artifacts: objectArtifacts,
    checkpoints,
    test_evidence: commandEvidenceFromTrace(request, trace),
    child_requests: [],
    confidence: 0,
    next_actions: ["Retry with a runner prompt that MUST emit a valid structured AgentRunResult JSON payload."],
    diagnostics: [
      `mode=${trace.source === "live" ? "live" : "replay"}`,
      `codex_app_server_source=${trace.source}`,
      `codex_app_server_thread_id=${stringValue(trace.thread.id) ?? "unknown"}`,
      `codex_app_server_turn_id=${stringValue(trace.turn.id) ?? "unknown"}`,
      `codex_app_server_final_json=${parsed}`,
      "codex_app_server_final_status_valid=false",
      ...trace.diagnostics,
      ...memoryContextDiagnostics(memoryContext, memoryContextError),
      ...subagentDiagnostics(request.task.execution?.subagents),
      ...mcpDiagnostics(request.task.execution?.mcp),
    ],
    notification_reports: [],
  };
}

function invalidCodexMcpStructuredResult(
  request: AgentRunRequest,
  trace: McpFallbackRunTrace,
  gitResult: Record<string, unknown> | null,
  objectArtifacts: Record<string, unknown>[],
  checkpoints: unknown[],
  parsed: boolean,
): AgentRunResult {
  return {
    task_id: request.task.id,
    status: "failed",
    summary: "Codex MCP fallback replay did not return a valid structured AgentRunResult payload.",
    review: null,
    research: null,
    branch_vote: null,
    runner_id: runnerId,
    model_used: request.task.execution?.model?.candidates?.[0] ?? null,
    mcp_context_used: request.task.execution?.mcp ?? null,
    sandbox_attestation: sandboxAttestation(request, false, false, "codex_mcp_fallback_replay"),
    artifacts: [
      {
        kind: "report",
        uri: codexMcpFallbackUri(request),
        description: "Codex MCP fallback replay transcript reference for invalid structured output",
        sha256: null,
      },
      ...resultChannelArtifacts(request, gitResult, objectArtifacts),
    ],
    git_result: gitResult,
    object_artifacts: objectArtifacts,
    checkpoints,
    test_evidence: commandEvidenceFromMcpTrace(request, trace),
    child_requests: [],
    confidence: 0,
    next_actions: ["Retry through a runner path that MUST emit a valid structured AgentRunResult JSON payload."],
    diagnostics: [
      "mode=mcp-replay",
      `codex_mcp_source=${trace.source}`,
      `codex_mcp_tool_calls=${trace.tool_calls.length}`,
      `codex_mcp_final_json=${parsed}`,
      "codex_mcp_final_status_valid=false",
      ...trace.diagnostics,
      ...subagentDiagnostics(request.task.execution?.subagents),
      ...mcpDiagnostics(request.task.execution?.mcp),
    ],
    notification_reports: [],
  };
}

export function loadReplayFixture(path = process.env.CODEX_REPLAY_FIXTURE ?? "examples/codex-app-server-replay.json"): unknown {
  return JSON.parse(readFileSync(path, "utf8")) as unknown;
}

export function traceFromReplayFixture(fixture: unknown): AppServerRunTrace {
  if (!isRecord(fixture)) throw new Error("replay fixture must be an object");
  const appServer = isRecord(fixture.app_server) ? fixture.app_server : fixture;
  const events = Array.isArray(appServer.events) ? appServer.events.filter(isRecord) : [];
  const items = completedItems(events);
  const thread =
    (isRecord(appServer.thread) ? appServer.thread : null) ??
    firstNotificationRecord(events, "thread/started", "thread") ??
    {};
  const turnSnapshot = isRecord(appServer.turn) ? appServer.turn : null;
  const completedTurn = firstNotificationRecord(events, "turn/completed", "turn");
  const startedTurn = firstNotificationRecord(events, "turn/started", "turn");
  const turn = { ...(turnSnapshot ?? startedTurn ?? {}), ...(completedTurn ?? {}) };
  return {
    source: "replay",
    app_server_url: typeof appServer.url === "string" ? appServer.url : null,
    thread,
    turn,
    events,
    server_requests: Array.isArray(appServer.server_requests) ? appServer.server_requests.filter(isRecord) : [],
    approval_reports: Array.isArray(appServer.approval_reports) ? appServer.approval_reports.filter(isRecord) : [],
    final_response: typeof appServer.final_response === "string" ? appServer.final_response : finalAgentMessage(items),
    items,
    usage: isRecord(appServer.usage) ? appServer.usage : firstNotificationUsage(events),
    diagnostics: ["app_server_replay_fixture=loaded"],
  };
}

export function loadMcpReplayFixture(
  path = process.env.CODEX_MCP_REPLAY_FIXTURE ?? "examples/codex-mcp-fallback-replay.json",
): unknown {
  return JSON.parse(readFileSync(path, "utf8")) as unknown;
}

export function traceFromMcpReplayFixture(fixture: unknown): McpFallbackRunTrace {
  if (!isRecord(fixture)) throw new Error("Codex MCP replay fixture must be an object");
  const mcp = isRecord(fixture.codex_mcp) ? fixture.codex_mcp : fixture;
  const toolCalls = Array.isArray(mcp.tool_calls) ? mcp.tool_calls.filter(isRecord) : [];
  const diagnostics = Array.isArray(mcp.diagnostics) ? mcp.diagnostics.filter(isString) : [];
  return {
    source: "replay",
    server: isRecord(mcp.server) ? mcp.server : {},
    tool_calls: toolCalls,
    final_response:
      typeof mcp.final_response === "string"
        ? mcp.final_response
        : isRecord(mcp.final_result)
          ? JSON.stringify(mcp.final_result)
          : finalMcpToolResult(toolCalls),
    usage: isRecord(mcp.usage) ? mcp.usage : null,
    diagnostics: ["codex_mcp_replay_fixture=loaded", ...diagnostics],
  };
}

function normalizeAppServerWebSocketUrl(raw: string): string | null {
  try {
    const url = new URL(raw);
    if (url.protocol === "ws:" || url.protocol === "wss:") return url.toString();
    if (url.protocol === "http:") {
      url.protocol = "ws:";
      return url.toString();
    }
    if (url.protocol === "https:") {
      url.protocol = "wss:";
      return url.toString();
    }
    return null;
  } catch {
    return null;
  }
}

function appServerThreadParams(request: AgentRunRequest): Record<string, unknown> {
  return {
    model: selectedModelName(request),
    cwd: liveWorkingDirectory(request),
    approvalPolicy: appServerApprovalPolicy(request),
    sandbox: appServerSandbox(request),
    personality: "pragmatic",
    serviceName: "coat_codex_runner",
    sessionStartSource: "startup",
  };
}

function appServerTurnParams(request: AgentRunRequest): Record<string, unknown> {
  return {
    cwd: liveWorkingDirectory(request),
    approvalPolicy: appServerApprovalPolicy(request),
    sandboxPolicy: appServerSandboxPolicy(request),
    model: selectedModelName(request),
    effort: selectedReasoningEffort(request),
    summary: "concise",
    personality: "pragmatic",
  };
}

function liveWorkingDirectory(request: AgentRunRequest): string | null {
  const configured = process.env.CODEX_APP_SERVER_CWD ?? process.env.CODEX_WORKSPACE_DIR ?? "";
  if (configured.trim()) return configured.trim();
  const plannedGit = buildGitResult(request);
  if (isRecord(plannedGit) && typeof plannedGit.worktree_path === "string" && existsSync(plannedGit.worktree_path)) {
    return plannedGit.worktree_path;
  }
  if (truthyEnv("CODEX_ALLOW_REPO_CWD_LIVE")) return process.cwd();
  return null;
}

function requestedCodexThreadId(request: AgentRunRequest): string | null {
  const fromEnv = process.env.CODEX_THREAD_ID?.trim();
  if (fromEnv) return fromEnv;
  const execution = request.task.execution;
  if (!isRecord(execution)) return null;
  const executionRecord = execution as Record<string, unknown>;
  for (const key of ["codex_app_server", "codex", "thread", "runner_context"]) {
    const section = executionRecord[key];
    if (isRecord(section)) {
      const id = stringValue(section.thread_id) ?? stringValue(section.threadId) ?? stringValue(section.id);
      if (id) return id;
    }
  }
  return null;
}

function selectedModelName(request: AgentRunRequest): string | null {
  const candidate = request.task.execution?.model?.candidates?.[0];
  if (isRecord(candidate) && typeof candidate.model === "string") return candidate.model;
  return process.env.CODEX_MODEL ?? null;
}

function selectedReasoningEffort(request: AgentRunRequest): string | null {
  const candidate = request.task.execution?.model?.candidates?.[0];
  if (isRecord(candidate) && isRecord(candidate.params) && typeof candidate.params.reasoning_effort === "string") {
    return candidate.params.reasoning_effort;
  }
  return process.env.CODEX_REASONING_EFFORT ?? null;
}

function appServerApprovalPolicy(request: AgentRunRequest): string {
  const policy = request.task.sandbox?.approval_policy;
  if (policy === "never") return "never";
  if (policy === "on_failure") return "onFailure";
  if (policy === "on_request") return "unlessTrusted";
  return process.env.CODEX_APP_SERVER_APPROVAL_POLICY ?? "unlessTrusted";
}

function appServerSandbox(request: AgentRunRequest): string {
  const filesystem = request.task.sandbox?.filesystem;
  if (filesystem === "read_only") return "readOnly";
  if (filesystem === "full_access") return "dangerFullAccess";
  return "workspaceWrite";
}

function appServerSandboxPolicy(request: AgentRunRequest): Record<string, unknown> {
  const filesystem = request.task.sandbox?.filesystem;
  const network = request.task.sandbox?.network;
  if (filesystem === "read_only") return { type: "readOnly" };
  if (filesystem === "full_access") return { type: "dangerFullAccess" };
  return {
    type: "workspaceWrite",
    writableRoots: [liveWorkingDirectory(request)].filter(isString),
    networkAccess: network === "open" || network === "enabled",
  };
}

function appServerApprovalResult(request: JsonRpcMessage): Record<string, unknown> {
  const decision = process.env.CODEX_APP_SERVER_APPROVAL_DECISION ?? "decline";
  const method = typeof request.method === "string" ? request.method : "unknown";
  if (method === "mcpServer/elicitation/request") return { action: "decline", content: null };
  if (method === "item/permissions/requestApproval") return { scope: "turn", permissions: {} };
  return { decision };
}

function codexAppServerPrompt(request: AgentRunRequest, memoryContext: MemoryContextResponse | null): string {
  return [
    "You are the Codex worker for a COAT durable task.",
    "Do the bounded task in the current workspace and return a structured worker result.",
    ...durableSubagentContext,
    "Final response requirements:",
    "- Return only JSON matching the provided output schema.",
    "- Put proposed durable child work in child_requests only.",
    "- Include test_evidence only for commands actually run.",
    "- Do not claim artifact uploads, git pushes, sandbox enforcement, or approvals that did not happen.",
    "",
    "Task contract:",
    JSON.stringify(
      {
        goal_id: request.goal_id,
        task_id: request.task.id,
        role: request.task.role,
        purpose: request.task.purpose ?? { kind: "work" },
        prompt: request.task.prompt,
        sandbox: request.task.sandbox ?? null,
        done_criteria: request.task.done_criteria ?? null,
        mcp: request.task.execution?.mcp ?? null,
        memory_context: memoryContext
          ? {
              hits: memoryContext.hits.map((hit) => ({ key: hit.key, scope: hit.scope, score: hit.score, summary: hit.summary })),
              use_plan: memoryContext.use_plan,
            }
          : null,
      },
      null,
      2,
    ),
  ].join("\n");
}

function agentRunResultOutputSchema(): Record<string, unknown> {
  return {
    type: "object",
    properties: {
      status: { type: "string", enum: ["done", "partial", "blocked", "failed", "timed_out"] },
      summary: { type: "string" },
      confidence: { type: "number", minimum: 0, maximum: 1 },
      next_actions: { type: "array", items: { type: "string" } },
      child_requests: { type: "array", items: { type: "object" } },
      test_evidence: { type: "array", items: { type: "object" } },
      review: { type: ["object", "null"] },
      research: { type: ["object", "null"] },
      branch_vote: { type: ["object", "null"] },
    },
    required: ["status", "summary", "confidence", "next_actions", "child_requests"],
    additionalProperties: true,
  };
}

async function fetchMemoryContext(request: AgentRunRequest): Promise<MemoryContextResponse | null> {
  if (!memoryGatewayUrl) return null;
  const response = await fetch(`${memoryGatewayUrl.replace(/\/$/, "")}/memory/context`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(memoryGatewayToken ? { authorization: `Bearer ${memoryGatewayToken}` } : {}),
    },
    body: JSON.stringify({
      goal_id: request.goal_id,
      task_id: request.task.id,
      objective: contextObjective(request),
      scopes: ["goal", "task", "repo", "persona"],
      limit: memoryContextLimit,
      store: null,
    }),
  });
  if (!response.ok) {
    throw new Error(`memory context returned ${response.status}: ${await response.text()}`);
  }
  return (await response.json()) as MemoryContextResponse;
}

function contextObjective(request: AgentRunRequest): string {
  const purpose = request.task.purpose;
  if (isRecord(purpose) && typeof purpose.question === "string") return purpose.question;
  return request.task.prompt;
}

function memoryContextArtifacts(
  request: AgentRunRequest,
  memoryContext: MemoryContextResponse | null,
): AgentRunResult["artifacts"] {
  if (!memoryContext) return [];
  return [
    {
      kind: "report",
      uri: `memory-context://${request.goal_id}/${request.task.id}`,
      description: `memory_context returned ${memoryContext.hits.length} hits and ${memoryContext.adapter_reports.length} adapter reports`,
      sha256: null,
    },
  ];
}

function memoryContextDiagnostics(memoryContext: MemoryContextResponse | null, error: string | null): string[] {
  if (!memoryGatewayUrl) return ["memory_context=disabled"];
  if (!memoryContext) return ["memory_context=unavailable", `memory_context_error=${error ?? "unknown"}`];
  const failedAdapters = memoryContext.adapter_reports.filter((report) => report.attempted && !report.success);
  return [
    "memory_context=enabled",
    `memory_context_hits=${memoryContext.hits.length}`,
    `memory_context_adapter_reports=${memoryContext.adapter_reports.length}`,
    `memory_context_failed_adapters=${failedAdapters.length}`,
  ];
}

function buildGitResult(request: AgentRunRequest): Record<string, unknown> | null {
  const git = resultPolicy(request, "git");
  if (!isRecord(git) || git.enabled !== true) return null;
  const branchPrefix = typeof git.branch_prefix === "string" ? git.branch_prefix.replace(/\/+$/, "") : "jattg/task";
  const worktreeRoot = typeof git.worktree_root === "string" ? git.worktree_root.replace(/\/+$/, "") : null;
  return {
    repo: null,
    remote: typeof git.remote === "string" ? git.remote : "origin",
    base_ref: typeof git.base_ref === "string" ? git.base_ref : "HEAD",
    branch: `${branchPrefix}/${request.goal_id}/${request.task.id}`,
    worktree_path: worktreeRoot ? `${worktreeRoot}/${request.goal_id}/${request.task.id}` : null,
    commit: null,
    pushed: false,
    pull_request_url: null,
    diff_uri: null,
  };
}

function buildObjectArtifacts(request: AgentRunRequest): Record<string, unknown>[] {
  const objectStorage = resultPolicy(request, "object_storage");
  if (!isRecord(objectStorage) || objectStorage.enabled !== true || !isRecord(objectStorage.store)) return [];
  const store = objectStorage.store;
  const bucket = typeof store.bucket === "string" ? store.bucket : "jattg-artifacts";
  const prefixTemplate =
    typeof objectStorage.key_prefix_template === "string"
      ? objectStorage.key_prefix_template
      : "goals/{goal_id}/tasks/{task_id}";
  const keyPrefix = prefixTemplate
    .replaceAll("{goal_id}", request.goal_id)
    .replaceAll("{task_id}", request.task.id)
    .replace(/^\/+|\/+$/g, "");
  const key = `${keyPrefix}/artifact-manifest.json`;
  return [
    {
      store,
      key,
      uri: `s3://${bucket}/${key}`,
      content_type: "application/json",
      size_bytes: null,
      sha256: null,
      description: "object storage artifact manifest location",
    },
  ];
}

function buildCheckpoints(
  request: AgentRunRequest,
  gitResult: Record<string, unknown> | null,
  objectArtifacts: Record<string, unknown>[],
  trace?: AppServerRunTrace | McpFallbackRunTrace,
): Record<string, unknown>[] {
  const checkpointPolicy = resultPolicy(request, "checkpoints");
  if (isRecord(checkpointPolicy) && checkpointPolicy.enabled === false) return [];
  const checkpoints: Record<string, unknown>[] = [];
  if (gitResult) {
    checkpoints.push({
      id: deterministicUuid(`${request.goal_id}:${request.task.id}:git`),
      goal_id: request.goal_id,
      task_id: request.task.id,
      parent_checkpoint_id: null,
      kind: gitResult.commit ? "git_commit" : "git_branch",
      label: "task-git-result",
      summary: "Git checkpoint for the task result branch.",
      artifact: {
        kind: "checkpoint",
        uri: `git+checkpoint://${String(gitResult.branch)}`,
        description: "git task checkpoint",
        sha256: null,
      },
      git_result: gitResult,
      object_artifact: null,
      sequence: 1,
      created_at: null,
      payload_json: checkpointPayload(trace),
    });
  }
  objectArtifacts.forEach((artifact, index) => {
    checkpoints.push({
      id: deterministicUuid(`${request.goal_id}:${request.task.id}:object:${index}`),
      goal_id: request.goal_id,
      task_id: request.task.id,
      parent_checkpoint_id: null,
      kind: "object_storage_archive",
      label: `object-artifact-${index + 1}`,
      summary: "Object storage checkpoint for large task artifacts.",
      artifact: {
        kind: "checkpoint",
        uri: typeof artifact.uri === "string" ? artifact.uri : `s3://unknown/${request.task.id}`,
        description: "object storage checkpoint",
        sha256: typeof artifact.sha256 === "string" ? artifact.sha256 : null,
      },
      git_result: null,
      object_artifact: artifact,
      sequence: index + 2,
      created_at: null,
      payload_json: checkpointPayload(trace),
    });
  });
  if (checkpoints.length === 0) {
    checkpoints.push({
      id: deterministicUuid(`${request.goal_id}:${request.task.id}:metadata`),
      goal_id: request.goal_id,
      task_id: request.task.id,
      parent_checkpoint_id: null,
      kind: "metadata",
      label: "runner-result",
      summary: "Metadata checkpoint for a task result without git or object artifacts.",
      artifact: {
        kind: "checkpoint",
        uri: `checkpoint://goal/${request.goal_id}/task/${request.task.id}/runner-result`,
        description: "metadata checkpoint",
        sha256: null,
      },
      git_result: null,
      object_artifact: null,
      sequence: 1,
      created_at: null,
      payload_json: checkpointPayload(trace),
    });
  }
  return checkpoints;
}

function checkpointPayload(trace?: AppServerRunTrace | McpFallbackRunTrace): Record<string, unknown> {
  if (!trace) return {};
  if (isMcpFallbackTrace(trace)) {
    return {
      codex_mcp: {
        source: trace.source,
        server_command: stringValue(trace.server.command) ?? "codex mcp-server",
        transport: stringValue(trace.server.transport) ?? "stdio",
        tool_call_count: trace.tool_calls.length,
        tool_calls: trace.tool_calls.map((call) => ({
          id: stringValue(call.id) ?? null,
          tool: mcpToolName(call),
          status: stringValue(call.status) ?? null,
        })),
        usage: trace.usage,
      },
    };
  }
  return {
    codex_app_server: {
      source: trace.source,
      app_server_url: trace.app_server_url,
      thread_id: stringValue(trace.thread.id) ?? null,
      session_id: stringValue(trace.thread.sessionId) ?? stringValue(trace.thread.session_id) ?? null,
      turn_id: stringValue(trace.turn.id) ?? null,
      turn_status: stringValue(trace.turn.status) ?? null,
      event_count: trace.events.length,
      item_ids: trace.items.map((item) => stringValue(item.id)).filter(isString),
      server_request_count: trace.server_requests.length,
      approval_count: trace.approval_reports.length,
      usage: trace.usage,
    },
  };
}

function isMcpFallbackTrace(trace: AppServerRunTrace | McpFallbackRunTrace): trace is McpFallbackRunTrace {
  return "tool_calls" in trace;
}

function buildGitResultFromTrace(request: AgentRunRequest, trace: AppServerRunTrace): Record<string, unknown> | null {
  const planned = buildGitResult(request);
  const gitInfo = isRecord(trace.thread.gitInfo)
    ? trace.thread.gitInfo
    : isRecord(trace.thread.git_info)
      ? trace.thread.git_info
      : null;
  if (!gitInfo) return planned;
  const branch = stringValue(gitInfo.branch) ?? (isRecord(planned) ? stringValue(planned.branch) : null);
  if (!branch && !planned) return null;
  return {
    repo: stringValue(gitInfo.originUrl) ?? stringValue(gitInfo.origin_url) ?? (isRecord(planned) ? planned.repo : null),
    remote: isRecord(planned) && typeof planned.remote === "string" ? planned.remote : "origin",
    base_ref: isRecord(planned) && typeof planned.base_ref === "string" ? planned.base_ref : "HEAD",
    branch: branch ?? `jattg/task/${request.goal_id}/${request.task.id}`,
    worktree_path: liveWorkingDirectory(request) ?? (isRecord(planned) ? planned.worktree_path : null),
    commit: stringValue(gitInfo.sha) ?? stringValue(gitInfo.commit) ?? (isRecord(planned) ? planned.commit : null),
    pushed: isRecord(planned) && typeof planned.pushed === "boolean" ? planned.pushed : false,
    pull_request_url: isRecord(planned) ? planned.pull_request_url : null,
    diff_uri: `${codexThreadUri(trace)}/git-diff`,
  };
}

function buildObjectArtifactsFromTrace(request: AgentRunRequest, trace: AppServerRunTrace): Record<string, unknown>[] {
  const objectArtifacts = buildObjectArtifacts(request);
  const objectStorage = resultPolicy(request, "object_storage");
  if (!isRecord(objectStorage) || objectStorage.enabled !== true || !isRecord(objectStorage.store)) return objectArtifacts;
  const keyPrefix = objectStorageKeyPrefix(request, objectStorage);
  const store = objectStorage.store;
  const bucket = typeof store.bucket === "string" ? store.bucket : "jattg-artifacts";
  const replayKey = `${keyPrefix}/codex-app-server-replay.json`;
  objectArtifacts.push({
    store,
    key: replayKey,
    uri: `s3://${bucket}/${replayKey}`,
    content_type: "application/json",
    size_bytes: null,
    sha256: null,
    description: `${trace.source} Codex App Server replay trace with thread, turn, item, approval, git, and diagnostic refs`,
  });
  return objectArtifacts;
}

function buildObjectArtifactsFromMcpTrace(request: AgentRunRequest): Record<string, unknown>[] {
  const objectArtifacts = buildObjectArtifacts(request);
  const objectStorage = resultPolicy(request, "object_storage");
  if (!isRecord(objectStorage) || objectStorage.enabled !== true || !isRecord(objectStorage.store)) return objectArtifacts;
  const keyPrefix = objectStorageKeyPrefix(request, objectStorage);
  const store = objectStorage.store;
  const bucket = typeof store.bucket === "string" ? store.bucket : "jattg-artifacts";
  const replayKey = `${keyPrefix}/codex-mcp-fallback-replay.json`;
  objectArtifacts.push({
    store,
    key: replayKey,
    uri: `s3://${bucket}/${replayKey}`,
    content_type: "application/json",
    size_bytes: null,
    sha256: null,
    description: "replay Codex MCP fallback trace with tool-call and diagnostic refs",
  });
  return objectArtifacts;
}

function objectStorageKeyPrefix(request: AgentRunRequest, objectStorage: Record<string, unknown>): string {
  const prefixTemplate =
    typeof objectStorage.key_prefix_template === "string"
      ? objectStorage.key_prefix_template
      : "goals/{goal_id}/tasks/{task_id}";
  return prefixTemplate
    .replaceAll("{goal_id}", request.goal_id)
    .replaceAll("{task_id}", request.task.id)
    .replace(/^\/+|\/+$/g, "");
}

function codexThreadUri(trace: AppServerRunTrace): string {
  const threadId = stringValue(trace.thread.id) ?? "unknown-thread";
  const turnId = stringValue(trace.turn.id) ?? "unknown-turn";
  return `codex-app-server://thread/${encodeURIComponent(threadId)}/turn/${encodeURIComponent(turnId)}`;
}

function codexMcpFallbackUri(request: AgentRunRequest): string {
  return `codex-mcp://fallback-replay/${encodeURIComponent(request.task.id)}`;
}

function parseAgentResultPatch(text: string): { parsed: boolean; patch: AgentResultPatch } {
  for (const candidate of jsonCandidates(text)) {
    try {
      const parsed = JSON.parse(candidate) as unknown;
      if (!isRecord(parsed)) continue;
      return { parsed: true, patch: parsed as AgentResultPatch };
    } catch {
      // Try the next candidate.
    }
  }
  return { parsed: false, patch: {} };
}

function jsonCandidates(text: string): string[] {
  const trimmed = text.trim();
  const candidates: string[] = [];
  if (trimmed) candidates.push(trimmed);
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/i);
  if (fenced?.[1]) candidates.push(fenced[1].trim());
  const first = trimmed.indexOf("{");
  const last = trimmed.lastIndexOf("}");
  if (first >= 0 && last > first) candidates.push(trimmed.slice(first, last + 1));
  return [...new Set(candidates)];
}

function normalizeResultStatus(status: unknown): AgentRunResult["status"] | null {
  if (typeof status !== "string") return null;
  const normalized = status.trim().toLowerCase();
  if (normalized === "completed" || normalized === "complete") return "done";
  if (["done", "partial", "blocked", "failed", "timed_out"].includes(normalized)) {
    return normalized as AgentRunResult["status"];
  }
  if (normalized === "timedout" || normalized === "timeout") return "timed_out";
  return null;
}

function commandEvidenceFromTrace(request: AgentRunRequest, trace: AppServerRunTrace): TestCommandEvidence[] {
  return trace.items.filter(isCommandExecutionItem).map((item) => {
    const id = stringValue(item.id) ?? deterministicUuid(`${request.task.id}:command`);
    const exitCode = numberValue(item.exitCode) ?? numberValue(item.exit_code);
    const status = stringValue(item.status);
    return {
      command: commandText(item.command),
      exit_code: exitCode,
      passed: status === "completed" && exitCode === 0,
      duration_ms: numberValue(item.durationMs) ?? numberValue(item.duration_ms),
      stdout_uri: `${codexThreadUri(trace)}/items/${encodeURIComponent(id)}/output`,
      stderr_uri: null,
      artifact_uri: `${codexThreadUri(trace)}/items/${encodeURIComponent(id)}`,
      notes: [
        `Codex App Server commandExecution item ${id} finished with status=${status ?? "unknown"} exit_code=${
          exitCode ?? "unknown"
        }`,
      ],
    };
  });
}

function commandEvidenceFromMcpTrace(request: AgentRunRequest, trace: McpFallbackRunTrace): TestCommandEvidence[] {
  return trace.tool_calls.map((call) => {
    const id = stringValue(call.id) ?? deterministicUuid(`${request.task.id}:${mcpToolName(call)}`);
    const passed = mcpToolCallPassed(call);
    return {
      command: `codex mcp-server replay: ${mcpToolName(call)}`,
      exit_code: passed ? 0 : 1,
      passed,
      duration_ms: numberValue(call.durationMs) ?? numberValue(call.duration_ms),
      stdout_uri: null,
      stderr_uri: null,
      artifact_uri: `${codexMcpFallbackUri(request)}/tool-calls/${encodeURIComponent(id)}`,
      notes: [
        `Codex MCP fallback tool call ${id} ran ${mcpToolName(call)} with status=${
          stringValue(call.status) ?? (passed ? "completed" : "failed")
        }`,
      ],
    };
  });
}

function isCommandExecutionItem(item: Record<string, unknown>): boolean {
  return item.type === "commandExecution" || item.type === "command_execution";
}

function commandText(command: unknown): string {
  if (Array.isArray(command)) return command.map(String).join(" ");
  if (typeof command === "string") return command;
  return "codex app-server command";
}

function completedItems(messages: JsonRpcMessage[]): Record<string, unknown>[] {
  const items = new Map<string, Record<string, unknown>>();
  for (const message of messages) {
    if (message.method !== "item/completed" && message.method !== "item/started" && message.method !== "item/updated") continue;
    const params = isRecord(message.params) ? message.params : {};
    const item = isRecord(params.item) ? params.item : isRecord(message.item) ? message.item : null;
    if (!item) continue;
    const id = stringValue(item.id) ?? deterministicUuid(JSON.stringify(item));
    items.set(id, item);
  }
  return [...items.values()];
}

function finalAgentMessage(items: Record<string, unknown>[]): string {
  for (const item of [...items].reverse()) {
    if (item.type !== "agentMessage" && item.type !== "agent_message") continue;
    const text = stringValue(item.text) ?? stringValue(item.message);
    if (text) return text;
  }
  return "";
}

function finalMcpToolResult(toolCalls: Record<string, unknown>[]): string {
  for (const call of [...toolCalls].reverse()) {
    if (typeof call.result === "string") return call.result;
    if (isRecord(call.result)) return JSON.stringify(call.result);
  }
  return "";
}

function mcpToolName(call: Record<string, unknown>): string {
  return stringValue(call.tool) ?? stringValue(call.name) ?? "codex.mcp.tool";
}

function mcpToolCallPassed(call: Record<string, unknown>): boolean {
  if (typeof call.success === "boolean") return call.success;
  if (isRecord(call.error)) return false;
  const status = stringValue(call.status)?.toLowerCase();
  return status === "completed" || status === "done" || status === "success" || status === "ok";
}

function firstNotificationRecord(events: JsonRpcMessage[], method: string, key: string): Record<string, unknown> | null {
  for (const event of events) {
    if (event.method !== method || !isRecord(event.params)) continue;
    const value = event.params[key];
    if (isRecord(value)) return value;
  }
  return null;
}

function firstNotificationUsage(events: JsonRpcMessage[]): unknown | null {
  for (const event of events) {
    if (event.method !== "turn/completed" || !isRecord(event.params)) continue;
    return event.params.usage ?? null;
  }
  return null;
}

function responseRecord(response: unknown, key: string): Record<string, unknown> {
  if (isRecord(response) && isRecord(response[key])) return response[key];
  return {};
}

function sandboxAttestation(
  request: AgentRunRequest,
  enforceable: boolean,
  strongIsolation: boolean,
  source: string,
): Record<string, unknown> {
  const filesystem = request.task.sandbox?.filesystem ?? "workspace_write";
  const network = request.task.sandbox?.network ?? "restricted";
  return {
    backend: "local_workspace",
    runtime_class: null,
    enforceable,
    strong_isolation: strongIsolation,
    isolation_summary: `${source} sandbox declaration with filesystem=${filesystem}, network=${network}; no strong sandbox attestation claimed`,
    warnings: [
      strongIsolation
        ? "strong sandbox was requested and should include executor evidence"
        : "local workspace sandboxing is not a strong isolation boundary",
    ],
    evidence: [
      {
        kind: "report",
        uri: `codex-runner://${request.goal_id}/${request.task.id}/sandbox`,
        description: "declared sandbox profile carried through the worker result",
        sha256: null,
      },
    ],
  };
}

function deterministicUuid(input: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < input.length; i += 1) {
    h1 ^= input.charCodeAt(i);
    h1 = Math.imul(h1, 0x01000193);
    h2 ^= input.charCodeAt(input.length - i - 1);
    h2 = Math.imul(h2, 0x811c9dc5);
  }
  const hex = `${(h1 >>> 0).toString(16).padStart(8, "0")}${(h2 >>> 0).toString(16).padStart(8, "0")}0000000000000000`;
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-8${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
}

function resultChannelArtifacts(
  request: AgentRunRequest,
  gitResult: Record<string, unknown> | null,
  objectArtifacts: Record<string, unknown>[],
): AgentRunResult["artifacts"] {
  return [
    ...(gitResult
      ? [
          {
            kind: "git_branch",
            uri: `git+branch://${String(gitResult.branch)}`,
            description: "git result branch for task worktree output",
            sha256: null,
          },
        ]
      : []),
    ...objectArtifacts.map((artifact) => ({
      kind: "artifact_manifest",
      uri: typeof artifact.uri === "string" ? artifact.uri : `s3://unknown/${request.task.id}`,
      description: "object storage artifact manifest",
      sha256: typeof artifact.sha256 === "string" ? artifact.sha256 : null,
    })),
  ];
}

function resultPolicy(request: AgentRunRequest, key: string): unknown {
  const results = request.task.execution?.results;
  return isRecord(results) ? results[key] : undefined;
}

function taskSummary(request: AgentRunRequest): string {
  const purpose = taskPurposeKind(request.task.purpose);
  if (purpose === "review") {
    return `stub Codex critic reviewed actor task ${request.task.id}`;
  }
  if (purpose === "unification") {
    return `stub Codex unifier joined critic branches for task ${request.task.id}`;
  }
  if (purpose === "branch_vote") {
    return `stub Codex voter compared branch candidates for task ${request.task.id}`;
  }
  if (purpose === "branch_unification") {
    return `stub Codex branch unifier selected a candidate for task ${request.task.id}`;
  }
  if (purpose === "candidate_branch") {
    return `stub Codex branch candidate implemented alternate path ${request.task.id}`;
  }
  if (purpose === "research") {
    return `stub Codex researcher answered ${request.task.id} with placeholder source capture`;
  }
  return `stub Codex actor accepted ${request.task.role} task ${request.task.id}`;
}

function taskPurposeKind(purpose: unknown): string {
  return isRecord(purpose) && typeof purpose.kind === "string" ? purpose.kind : "work";
}

async function ensureCodexMcpStarts(): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn("codex", ["mcp-server"], { stdio: ["ignore", "ignore", "pipe"] });
    const timer = setTimeout(() => {
      child.kill();
      resolve();
    }, 1000);
    child.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      if (code === 0 || code === null) resolve();
      else reject(new Error(`codex mcp-server exited with ${code}`));
    });
  });
}

export async function verifyCodexIntegration(): Promise<Record<string, unknown>> {
  const cli = await checkCommand("codex", ["--version"], 1000);
  const mcp = process.env.CODEX_VERIFY_MCP === "1" ? await checkCodexMcp() : { attempted: false };
  const appServerUrl = process.env.CODEX_APP_SERVER_URL ?? "";
  const authMode = process.env.CODEX_AUTH_MODE ?? "env_api_key";
  const appServer =
    appServerUrl && process.env.CODEX_VERIFY_APP_SERVER === "1"
      ? await checkAppServer(appServerUrl, 1000)
      : { attempted: false, configured: Boolean(appServerUrl) };
  return {
    runner_id: runnerId,
    mode: runnerMode(),
    package: "@openai/codex-sdk",
    codex_sdk_dependency_declared: true,
    codex_cli: cli,
    codex_mcp_server: mcp,
    codex_app_server: appServer,
    auth: {
      mode: authMode,
      has_openai_api_key: Boolean(process.env.OPENAI_API_KEY),
      has_codex_api_key: Boolean(process.env.CODEX_API_KEY),
      app_server_configured: Boolean(appServerUrl),
      runner_local_device_allowed: authMode === "runner_local_device",
      brokered_auth_allowed: authMode === "oauth_device_broker" || authMode === "external_broker",
      auth_state_path_configured: Boolean(process.env.CODEX_AUTH_STATE_PATH),
      secret_values_exposed: false,
    },
    live_execution_requires: [
      "CODEX_RUNNER_MODE=live",
      "CODEX_AUTH_MODE=app_server",
      "CODEX_APP_SERVER_URL pointing at a reachable Codex App Server websocket endpoint",
      "task.sandbox.isolated_runner=true and an existing live workspace directory",
    ],
    provider_profiles: await buildProviderVerificationProfiles({ cli, mcp, appServer }),
  };
}

async function buildProviderVerificationProfiles(probes: {
  cli: Record<string, unknown>;
  mcp: Record<string, unknown>;
  appServer: Record<string, unknown>;
}): Promise<ProviderVerificationProfile[]> {
  return Promise.all(configuredProviderVerificationLanes().map((lane) => providerVerificationProfile(lane, probes)));
}

function configuredProviderVerificationLanes(): ProviderVerificationLane[] {
  const registration = buildRegistration();
  const lanes = new Map<string, ProviderVerificationLane>();

  const addLane = (lane: ProviderVerificationLane) => {
    lanes.set(lane.lane_id, lane);
  };

  for (const model of registration.models) {
    const provider = String(model.provider);
    if (provider === "codex") {
      addLane({
        lane_id: "codex:app_server",
        provider: "codex",
        surface: "app_server",
        model: model.model,
        endpoint: process.env.CODEX_APP_SERVER_URL ?? model.endpoint ?? null,
        source: "runner_model",
      });
      addLane({
        lane_id: "codex:mcp",
        provider: "codex",
        surface: "mcp_fallback",
        model: model.model,
        endpoint: null,
        source: "runner_model",
      });
      continue;
    }
    addLane({
      lane_id: providerLaneId(provider, model.model, model.endpoint),
      provider,
      surface: providerSurface(provider),
      model: model.model,
      endpoint: model.endpoint ?? providerEndpointFromEnv(provider),
      source: "runner_model",
    });
  }

  for (const configured of parseJsonEnv<unknown[]>("CODEX_PROVIDER_VERIFY_PROFILES_JSON", [])) {
    if (!isRecord(configured)) continue;
    const provider = stringValue(configured.provider);
    if (!provider) continue;
    const model = stringValue(configured.model);
    const endpoint = stringValue(configured.endpoint) ?? providerEndpointFromEnv(provider);
    const surface = stringValue(configured.surface) ?? providerSurface(provider);
    addLane({
      lane_id: stringValue(configured.lane_id) ?? providerLaneId(provider, model, endpoint),
      provider,
      surface,
      model,
      endpoint,
      source: "env_profile",
    });
  }

  return [...lanes.values()].sort((left, right) => left.lane_id.localeCompare(right.lane_id));
}

async function providerVerificationProfile(
  lane: ProviderVerificationLane,
  probes: {
    cli: Record<string, unknown>;
    mcp: Record<string, unknown>;
    appServer: Record<string, unknown>;
  },
): Promise<ProviderVerificationProfile> {
  if (lane.provider === "codex" && lane.surface === "app_server") {
    return codexAppServerProviderProfile(lane, probes.appServer);
  }
  if (lane.provider === "codex" && lane.surface === "mcp_fallback") {
    return codexMcpProviderProfile(lane, probes.cli, probes.mcp);
  }

  const auth = providerAuthProfile(lane.provider);
  const missing = providerMissingSetup(lane, auth);
  if (missing.length > 0) {
    return providerProfile(lane, {
      status: "skipped",
      attempted: false,
      available: null,
      configured: false,
      skipped_reason: missing.join("; "),
      error: null,
      auth,
      checks: [
        {
          name: "configuration",
          attempted: true,
          passed: false,
          evidence: missing,
        },
      ],
    });
  }

  if (!truthyEnv("CODEX_VERIFY_PROVIDER_NETWORK")) {
    return providerProfile(lane, {
      status: "skipped",
      attempted: false,
      available: null,
      configured: true,
      skipped_reason: "CODEX_VERIFY_PROVIDER_NETWORK=1 not set; emitted profile without a live provider call",
      error: null,
      auth,
      checks: [
        {
          name: "configuration",
          attempted: true,
          passed: true,
          evidence: ["required endpoint/auth setup is present or not required"],
        },
        {
          name: "live_provider_probe",
          attempted: false,
          passed: null,
          evidence: ["network probe disabled"],
        },
      ],
    });
  }

  if (!lane.endpoint) {
    return providerProfile(lane, {
      status: "skipped",
      attempted: false,
      available: null,
      configured: true,
      skipped_reason: `${lane.provider} does not expose a configured HTTP endpoint for this verifier`,
      error: null,
      auth,
      checks: [
        {
          name: "configuration",
          attempted: true,
          passed: true,
          evidence: ["required auth setup is present or not required"],
        },
        {
          name: "live_provider_probe",
          attempted: false,
          passed: null,
          evidence: ["no HTTP endpoint configured for automated probe"],
        },
      ],
    });
  }

  const probe = await checkProviderHttp(lane, 1000);
  const available = probe.available === true;
  const reachableButNotVerified = probe.reachable === true && probe.available !== true && probe.server_error !== true;
  if (reachableButNotVerified) {
    return providerProfile(lane, {
      status: "skipped",
      attempted: true,
      available: null,
      configured: true,
      skipped_reason: String(probe.classification ?? "provider endpoint reachable but verifier probe was not authoritative"),
      error: null,
      auth,
      checks: [
        {
          name: "configuration",
          attempted: true,
          passed: true,
          evidence: ["required endpoint/auth setup is present or not required"],
        },
        {
          name: "live_provider_probe",
          attempted: true,
          passed: null,
          evidence: [probe],
        },
      ],
    });
  }
  return providerProfile(lane, {
    status: available ? "verified" : "failed",
    attempted: true,
    available,
    configured: true,
    skipped_reason: null,
    error: typeof probe.error === "string" ? probe.error : null,
    auth,
    checks: [
      {
        name: "configuration",
        attempted: true,
        passed: true,
        evidence: ["required endpoint/auth setup is present or not required"],
      },
      {
        name: "live_provider_probe",
        attempted: true,
        passed: available,
        evidence: [probe],
      },
    ],
  });
}

function codexAppServerProviderProfile(lane: ProviderVerificationLane, appServer: Record<string, unknown>): ProviderVerificationProfile {
  const auth = providerAuthProfile("codex");
  const configured = Boolean(process.env.CODEX_APP_SERVER_URL);
  if (appServer.attempted === true) {
    return providerProfile(lane, {
      status: appServer.available === true ? "verified" : "failed",
      attempted: true,
      available: appServer.available === true,
      configured,
      skipped_reason: null,
      error: typeof appServer.error === "string" ? appServer.error : null,
      auth,
      checks: [
        {
          name: "codex_app_server_initialize",
          attempted: true,
          passed: appServer.available === true,
          evidence: [appServer],
        },
      ],
    });
  }
  return providerProfile(lane, {
    status: "skipped",
    attempted: false,
    available: null,
    configured,
    skipped_reason: configured
      ? "CODEX_VERIFY_APP_SERVER=1 not set; App Server live verification skipped"
      : "CODEX_APP_SERVER_URL not set; App Server live verification skipped",
    error: null,
    auth,
    checks: [
      {
        name: "codex_app_server_config",
        attempted: true,
        passed: configured,
        evidence: [configured ? "CODEX_APP_SERVER_URL configured" : "CODEX_APP_SERVER_URL missing"],
      },
      {
        name: "codex_app_server_initialize",
        attempted: false,
        passed: null,
        evidence: ["live App Server probe disabled"],
      },
    ],
  });
}

function codexMcpProviderProfile(
  lane: ProviderVerificationLane,
  cli: Record<string, unknown>,
  mcp: Record<string, unknown>,
): ProviderVerificationProfile {
  const auth = providerAuthProfile("codex");
  if (mcp.attempted === true) {
    return providerProfile(lane, {
      status: mcp.available === true ? "verified" : "failed",
      attempted: true,
      available: mcp.available === true,
      configured: cli.available === true,
      skipped_reason: null,
      error: typeof mcp.error === "string" ? mcp.error : null,
      auth,
      checks: [
        {
          name: "codex_cli",
          attempted: true,
          passed: cli.available === true,
          evidence: [redactProbe(cli)],
        },
        {
          name: "codex_mcp_server",
          attempted: true,
          passed: mcp.available === true,
          evidence: [mcp],
        },
      ],
    });
  }
  return providerProfile(lane, {
    status: "skipped",
    attempted: false,
    available: null,
    configured: cli.available === true,
    skipped_reason:
      cli.available === true
        ? "CODEX_VERIFY_MCP=1 not set; MCP fallback live verification skipped"
        : "codex CLI unavailable; MCP fallback live verification skipped",
    error: null,
    auth,
    checks: [
      {
        name: "codex_cli",
        attempted: true,
        passed: cli.available === true,
        evidence: [redactProbe(cli)],
      },
      {
        name: "codex_mcp_server",
        attempted: false,
        passed: null,
        evidence: ["live MCP server startup probe disabled"],
      },
    ],
  });
}

function providerProfile(
  lane: ProviderVerificationLane,
  data: Omit<ProviderVerificationProfile, keyof ProviderVerificationLane | "secret_values_exposed">,
): ProviderVerificationProfile {
  return {
    ...lane,
    ...data,
    secret_values_exposed: false,
  };
}

function providerLaneId(provider: string, model: string | null, endpoint: string | null): string {
  const endpointLabel = endpoint ? `:${redactUrl(endpoint).replace(/[^a-zA-Z0-9_.-]+/g, "_")}` : "";
  return `${provider}:${model ?? "default"}${endpointLabel}`;
}

function providerSurface(provider: string): string {
  if (provider === "claude_code") return "cli_runner";
  if (provider === "open_ai_compatible") return "openai_compatible_http";
  if (provider === "vllm" || provider === "ollama" || provider === "llama_cpp") return "local_http";
  if (provider === "bedrock") return "aws_bedrock";
  if (provider === "hugging_face") return "hugging_face";
  return "model_provider";
}

function providerEndpointFromEnv(provider: string): string | null {
  if (provider === "open_ai") return process.env.OPENAI_BASE_URL ?? "https://api.openai.com/v1";
  if (provider === "open_ai_compatible") {
    return process.env.COAT_LLM_GATEWAY_URL ?? process.env.OPENAI_COMPATIBLE_BASE_URL ?? process.env.OPENAI_BASE_URL ?? null;
  }
  if (provider === "vllm") return process.env.VLLM_BASE_URL ?? process.env.VLLM_ENDPOINT ?? null;
  if (provider === "ollama") return process.env.OLLAMA_BASE_URL ?? "http://localhost:11434";
  if (provider === "llama_cpp") return process.env.LLAMA_CPP_BASE_URL ?? process.env.LLAMA_CPP_ENDPOINT ?? null;
  if (provider === "hugging_face") return process.env.HUGGING_FACE_INFERENCE_ENDPOINT ?? process.env.HF_INFERENCE_ENDPOINT ?? null;
  return null;
}

function providerAuthProfile(provider: string): Record<string, unknown> {
  if (provider === "codex") {
    const authMode = process.env.CODEX_AUTH_MODE ?? "env_api_key";
    return {
      mode: authMode,
      has_openai_api_key: Boolean(process.env.OPENAI_API_KEY),
      has_codex_api_key: Boolean(process.env.CODEX_API_KEY),
      app_server_configured: Boolean(process.env.CODEX_APP_SERVER_URL),
      auth_state_path_configured: Boolean(process.env.CODEX_AUTH_STATE_PATH),
      secret_values_exposed: false,
    };
  }
  if (provider === "open_ai") {
    return {
      has_openai_api_key: Boolean(process.env.OPENAI_API_KEY),
      base_url_configured: Boolean(process.env.OPENAI_BASE_URL),
      secret_values_exposed: false,
    };
  }
  if (provider === "open_ai_compatible") {
    return {
      has_gateway_api_key: Boolean(process.env.COAT_LLM_GATEWAY_API_KEY || process.env.OPENAI_COMPATIBLE_API_KEY || process.env.OPENAI_API_KEY),
      gateway_url_configured: Boolean(process.env.COAT_LLM_GATEWAY_URL || process.env.OPENAI_COMPATIBLE_BASE_URL || process.env.OPENAI_BASE_URL),
      secret_values_exposed: false,
    };
  }
  if (provider === "bedrock") {
    return {
      aws_profile_configured: Boolean(process.env.AWS_PROFILE),
      aws_access_key_configured: Boolean(process.env.AWS_ACCESS_KEY_ID),
      aws_web_identity_configured: Boolean(process.env.AWS_WEB_IDENTITY_TOKEN_FILE),
      aws_region_configured: Boolean(process.env.AWS_REGION || process.env.AWS_DEFAULT_REGION),
      secret_values_exposed: false,
    };
  }
  if (provider === "hugging_face") {
    return {
      has_hf_token: Boolean(process.env.HF_TOKEN || process.env.HUGGING_FACE_HUB_TOKEN),
      inference_endpoint_configured: Boolean(process.env.HUGGING_FACE_INFERENCE_ENDPOINT || process.env.HF_INFERENCE_ENDPOINT),
      secret_values_exposed: false,
    };
  }
  if (provider === "claude_code" || provider === "anthropic") {
    return {
      has_anthropic_api_key: Boolean(process.env.ANTHROPIC_API_KEY),
      claude_auth_state_path_configured: Boolean(process.env.CLAUDE_CODE_AUTH_STATE_PATH),
      secret_values_exposed: false,
    };
  }
  return {
    auth_not_required_by_default: provider === "vllm" || provider === "ollama" || provider === "llama_cpp" || provider === "local_process",
    secret_values_exposed: false,
  };
}

function providerMissingSetup(lane: ProviderVerificationLane, auth: Record<string, unknown>): string[] {
  const missing: string[] = [];
  if (!lane.endpoint && ["open_ai_compatible", "vllm", "ollama", "llama_cpp", "hugging_face"].includes(lane.provider)) {
    missing.push(`${lane.provider} endpoint is not configured`);
  }
  if (lane.provider === "open_ai" && auth.has_openai_api_key !== true) missing.push("OPENAI_API_KEY is not set");
  if (lane.provider === "open_ai_compatible" && auth.has_gateway_api_key !== true) {
    missing.push("OpenAI-compatible gateway API key is not set");
  }
  if (
    lane.provider === "bedrock" &&
    auth.aws_profile_configured !== true &&
    auth.aws_access_key_configured !== true &&
    auth.aws_web_identity_configured !== true
  ) {
    missing.push("AWS Bedrock credentials are not configured");
  }
  if (lane.provider === "hugging_face" && auth.has_hf_token !== true) missing.push("Hugging Face token is not configured");
  if (
    (lane.provider === "claude_code" || lane.provider === "anthropic") &&
    auth.has_anthropic_api_key !== true &&
    auth.claude_auth_state_path_configured !== true
  ) {
    missing.push("Claude Code or Anthropic auth is not configured");
  }
  return missing;
}

function redactProbe(probe: Record<string, unknown>): Record<string, unknown> {
  const redacted: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(probe)) {
    redacted[key] = typeof value === "string" && key.toLowerCase().includes("url") ? redactUrl(value) : value;
  }
  return redacted;
}

async function checkCodexMcp(): Promise<Record<string, unknown>> {
  try {
    await ensureCodexMcpStarts();
    return { attempted: true, available: true };
  } catch (error) {
    return {
      attempted: true,
      available: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

async function checkCommand(command: string, args: string[], timeoutMs: number): Promise<Record<string, unknown>> {
  return new Promise((resolve) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    const timer = setTimeout(() => {
      child.kill();
      resolve({ attempted: true, available: false, timed_out: true });
    }, timeoutMs);
    child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
    child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
    child.once("error", (error) => {
      clearTimeout(timer);
      resolve({ attempted: true, available: false, error: error.message });
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      resolve({
        attempted: true,
        available: code === 0,
        exit_code: code,
        stdout: Buffer.concat(stdout).toString("utf8").trim(),
        stderr: Buffer.concat(stderr).toString("utf8").trim(),
      });
    });
  });
}

async function checkHttp(url: string, timeoutMs: number): Promise<Record<string, unknown>> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { signal: controller.signal });
    return {
      attempted: true,
      configured: true,
      available: response.ok,
      status: response.status,
    };
  } catch (error) {
    return {
      attempted: true,
      configured: true,
      available: false,
      error: error instanceof Error ? error.message : String(error),
    };
  } finally {
    clearTimeout(timer);
  }
}

async function checkProviderHttp(lane: ProviderVerificationLane, timeoutMs: number): Promise<Record<string, unknown>> {
  const endpoint = lane.endpoint;
  if (!endpoint) return { attempted: false, configured: false, available: null, error: "missing endpoint" };
  const url = providerProbeUrl(lane.provider, endpoint);
  const headers = providerProbeHeaders(lane.provider);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { signal: controller.signal, headers });
    const classification = classifyProviderProbeStatus(response.status);
    return {
      attempted: true,
      configured: true,
      available: response.ok,
      reachable: true,
      status: response.status,
      server_error: response.status >= 500,
      probe_url: redactUrl(url),
      classification,
    };
  } catch (error) {
    return {
      attempted: true,
      configured: true,
      available: false,
      reachable: false,
      probe_url: redactUrl(url),
      error: error instanceof Error ? error.message : String(error),
    };
  } finally {
    clearTimeout(timer);
  }
}

function providerProbeUrl(provider: string, endpoint: string): string {
  try {
    const url = new URL(endpoint);
    if (provider === "ollama") {
      url.pathname = `${url.pathname.replace(/\/+$/g, "")}/api/tags`.replace(/\/api\/tags\/api\/tags$/, "/api/tags");
      return url.toString();
    }
    if (provider === "open_ai" || provider === "open_ai_compatible" || provider === "vllm") {
      const path = url.pathname.replace(/\/+$/g, "");
      if (path.endsWith("/models")) return url.toString();
      url.pathname = path.endsWith("/v1") ? `${path}/models` : `${path}/v1/models`;
      return url.toString();
    }
    return url.toString();
  } catch {
    return endpoint;
  }
}

function providerProbeHeaders(provider: string): HeadersInit {
  const token =
    provider === "hugging_face"
      ? process.env.HF_TOKEN || process.env.HUGGING_FACE_HUB_TOKEN
      : provider === "open_ai_compatible"
        ? process.env.COAT_LLM_GATEWAY_API_KEY || process.env.OPENAI_COMPATIBLE_API_KEY || process.env.OPENAI_API_KEY
        : provider === "open_ai"
          ? process.env.OPENAI_API_KEY
          : null;
  return token ? { authorization: `Bearer ${token}` } : {};
}

function classifyProviderProbeStatus(status: number): string {
  if (status >= 200 && status < 300) return "verified";
  if (status === 401 || status === 403) return "provider endpoint reachable but authentication was rejected";
  if (status === 404 || status === 405) return "provider endpoint reachable but probe route was not supported";
  if (status >= 500) return "provider endpoint returned a server error";
  return `provider endpoint returned HTTP ${status}`;
}

async function checkAppServer(url: string, timeoutMs: number): Promise<Record<string, unknown>> {
  const websocketUrl = normalizeAppServerWebSocketUrl(url);
  if (!websocketUrl) {
    return { attempted: true, configured: true, available: false, error: "unsupported_app_server_url" };
  }
  const client = new AppServerJsonRpcClient(websocketUrl, timeoutMs, appServerApprovalResult);
  try {
    await client.connect();
    const initialize = await client.request("initialize", {
      clientInfo: {
        name: "coat_codex_runner_verify",
        title: "COAT Codex Runner Verify",
        version: "0.1.0",
      },
    });
    client.notify("initialized", {});
    return {
      attempted: true,
      configured: true,
      available: true,
      transport: "websocket",
      url: redactUrl(websocketUrl),
      initialized: isRecord(initialize),
    };
  } catch (error) {
    return {
      attempted: true,
      configured: true,
      available: false,
      transport: "websocket",
      url: redactUrl(websocketUrl),
      error: error instanceof Error ? error.message : String(error),
    };
  } finally {
    client.close();
  }
}

class AppServerJsonRpcClient {
  readonly messages: JsonRpcMessage[] = [];
  readonly serverRequests: JsonRpcMessage[] = [];
  readonly approvalReports: Record<string, unknown>[] = [];

  private ws: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<
    number,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  private waiters: Array<{
    predicate: (message: JsonRpcMessage) => boolean;
    resolve: (message: JsonRpcMessage) => void;
    reject: (error: Error) => void;
    timer: ReturnType<typeof setTimeout>;
  }> = [];

  constructor(
    private readonly url: string,
    private readonly timeoutMs: number,
    private readonly serverRequestHandler: (request: JsonRpcMessage) => Record<string, unknown>,
  ) {}

  async connect(): Promise<void> {
    if (typeof WebSocket !== "function") {
      throw new Error("global WebSocket is unavailable; Node 22+ is required for Codex App Server live mode");
    }
    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(this.url);
      const timer = setTimeout(() => {
        ws.close();
        reject(new Error(`timed out connecting to ${redactUrl(this.url)}`));
      }, this.timeoutMs);
      ws.addEventListener("open", () => {
        clearTimeout(timer);
        this.ws = ws;
        resolve();
      });
      ws.addEventListener("message", (event) => this.handleMessage(event.data));
      ws.addEventListener("error", () => {
        clearTimeout(timer);
        reject(new Error(`failed to connect to ${redactUrl(this.url)}`));
      });
      ws.addEventListener("close", () => {
        this.rejectAll(new Error(`Codex App Server connection closed: ${redactUrl(this.url)}`));
      });
    });
  }

  request(method: string, params?: Record<string, unknown>): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Codex App Server request timed out: ${method}`));
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try {
        this.send({ id, method, ...(params === undefined ? {} : { params }) });
      } catch (error) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  notify(method: string, params?: Record<string, unknown>): void {
    this.send({ method, ...(params === undefined ? {} : { params }) });
  }

  waitFor(predicate: (message: JsonRpcMessage) => boolean): Promise<JsonRpcMessage> {
    for (const message of this.messages) {
      if (predicate(message)) return Promise.resolve(message);
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((waiter) => waiter.resolve !== resolve);
        reject(new Error("timed out waiting for Codex App Server notification"));
      }, this.timeoutMs);
      this.waiters.push({ predicate, resolve, reject, timer });
    });
  }

  close(): void {
    this.ws?.close();
    this.ws = null;
  }

  private send(message: JsonRpcMessage): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      throw new Error("Codex App Server websocket is not open");
    }
    this.ws.send(JSON.stringify(message));
  }

  private handleMessage(data: unknown): void {
    const text = typeof data === "string" ? data : Buffer.from(data as ArrayBuffer).toString("utf8");
    const message = JSON.parse(text) as JsonRpcMessage;
    this.messages.push(message);

    const id = numberValue(message.id);
    if (id !== null && !("method" in message)) {
      const pending = this.pending.get(id);
      if (!pending) return;
      clearTimeout(pending.timer);
      this.pending.delete(id);
      if (isRecord(message.error)) {
        pending.reject(new Error(stringValue(message.error.message) ?? JSON.stringify(message.error)));
      } else {
        pending.resolve(message.result ?? {});
      }
      return;
    }

    if (id !== null && typeof message.method === "string") {
      this.serverRequests.push(message);
      const result = this.serverRequestHandler(message);
      this.approvalReports.push({
        method: message.method,
        request_id: id,
        result,
      });
      this.send({ id, result });
      return;
    }

    for (const waiter of [...this.waiters]) {
      if (!waiter.predicate(message)) continue;
      clearTimeout(waiter.timer);
      this.waiters = this.waiters.filter((candidate) => candidate !== waiter);
      waiter.resolve(message);
    }
  }

  private rejectAll(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    for (const waiter of this.waiters) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
    this.waiters = [];
  }
}

async function registerAndHeartbeat(): Promise<void> {
  if (!registryUrl) {
    console.log("RUNNER_REGISTRY_URL is not set; sidecar will not auto-register");
    return;
  }

  await registerRunnerWithRetry();
  const timer = setInterval(() => {
    void sendHeartbeat().catch((error) => {
      console.warn(`runner heartbeat failed: ${error instanceof Error ? error.message : String(error)}`);
    });
  }, heartbeatIntervalMs);
  timer.unref();
}

async function registerRunnerWithRetry(): Promise<void> {
  const attempts = numberEnv("RUNNER_REGISTRATION_ATTEMPTS", 20);
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      await registerRunner();
      await sendHeartbeat();
      console.log(`registered runner ${runnerId} with ${registryUrl}`);
      return;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.warn(`runner registration attempt ${attempt}/${attempts} failed: ${message}`);
      await sleep(Math.min(500 * attempt, 5_000));
    }
  }
}

async function registerRunner(): Promise<void> {
  const response = await fetch(`${registryUrl.replace(/\/$/, "")}/runners`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(buildRegistration()),
  });
  if (!response.ok) {
    throw new Error(`registry returned ${response.status}: ${await response.text()}`);
  }
}

async function sendHeartbeat(): Promise<void> {
  const heartbeat: RunnerHeartbeat = {
    runner_id: runnerId,
    node_id: nodeId,
    running_tasks: runningTasks,
    capacity_remaining: Math.max(maxConcurrency - runningTasks, 0),
  };
  const response = await fetch(`${registryUrl.replace(/\/$/, "")}/runners/heartbeat`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(heartbeat),
  });
  if (!response.ok) {
    throw new Error(`registry heartbeat returned ${response.status}: ${await response.text()}`);
  }
}

function buildRegistration(): RunnerRegistration {
  return {
    runner_id: runnerId,
    node_id: nodeId,
    endpoint: runnerEndpoint,
    roles: parseJsonEnv(
      "RUNNER_ROLES_JSON",
      [
        "planner",
        "codex",
        "tester",
        "reviewer",
        "validator",
        "patch_merger",
        "research",
        "formal_methods",
      ] satisfies WorkerKind[],
    ),
    capabilities: parseJsonEnv("RUNNER_CAPABILITIES_JSON", [
      "code",
      "test",
      "review",
      ...(webSearchEnabled() ? ["web_search" as RunnerCapability, "network_open" as RunnerCapability] : []),
      "mcp_tools",
      "durable_child_tasks",
      "workspace_sandbox",
      "git_worktree",
      "git",
      "object_storage",
      "s3_compatible",
      "formal_verification",
    ] satisfies RunnerCapability[]),
    models: parseJsonEnv("RUNNER_MODELS_JSON", [
      {
        provider: "codex",
        model: process.env.CODEX_MODEL ?? "codex-default",
        endpoint: null,
        priority: 100,
        weight: 1,
        context_window: null,
        features: ["tool_use", "json_schema", "streaming", ...(webSearchEnabled() ? ["web_search" as ModelFeature] : [])],
        labels: {},
      },
    ] satisfies ModelCandidate[]),
    labels: buildRunnerLabels(),
    mcp_servers: parseJsonEnv("RUNNER_MCP_SERVERS_JSON", [] satisfies McpServerRef[]),
    max_concurrency: maxConcurrency,
    lease_ttl_seconds: leaseTtlSeconds,
  };
}

function buildRunnerLabels(): Record<string, string> {
  const configured = parseJsonEnv<Record<string, string>>("RUNNER_LABELS_JSON", {});
  const mode = runnerMode();
  const authMode = process.env.CODEX_AUTH_MODE ?? "env_api_key";
  return {
    pool: "default",
    runtime: "codex",
    ...configured,
    mode,
    "runner.mode": mode,
    "auth.codex.mode": authMode,
    "auth.codex.app_server": String(authMode === "app_server" && Boolean(process.env.CODEX_APP_SERVER_URL)),
  };
}

function webSearchEnabled(): boolean {
  return ["CODEX_NATIVE_WEB_SEARCH", "COAT_WEB_SEARCH_ENABLED"].some((key) => truthyEnv(key));
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
    mode: runnerMode(),
    running_tasks: runningTasks,
    capacity_remaining: Math.max(maxConcurrency - runningTasks, 0),
    endpoints: ["/healthz", "/registration", "/capabilities", "/verify", "/run-task"],
    registration,
    execution_modes: {
      active: runnerMode(),
      stub: {
        explicit: runnerMode() === "stub",
        local_smoke_only: true,
      },
      replay: {
        enabled: runnerMode() === "replay",
        fixture: process.env.CODEX_REPLAY_FIXTURE ?? "examples/codex-app-server-replay.json",
      },
      mcp_replay: {
        enabled: runnerMode() === "mcp-replay",
        fixture: process.env.CODEX_MCP_REPLAY_FIXTURE ?? "examples/codex-mcp-fallback-replay.json",
      },
      live_app_server: {
        enabled: runnerMode() === "live",
        auth_mode: process.env.CODEX_AUTH_MODE ?? "env_api_key",
        configured: Boolean(process.env.CODEX_APP_SERVER_URL),
        url: process.env.CODEX_APP_SERVER_URL ? redactUrl(process.env.CODEX_APP_SERVER_URL) : null,
        supported_transports: ["ws", "wss"],
      },
    },
    package_verification: {
      endpoint: "/verify",
      package: "@openai/codex-sdk",
      provider_profiles: configuredProviderVerificationLanes(),
      live_execution_requires: [
        "CODEX_RUNNER_MODE=live",
        "CODEX_AUTH_MODE=app_server",
        "CODEX_APP_SERVER_URL",
        "isolated task workspace",
      ],
    },
    model_routing: {
      providers: [...new Set(registration.models.map((model) => model.provider))],
      models: registration.models.map((model) => ({
        provider: model.provider,
        model: model.model,
        endpoint: model.endpoint,
        priority: model.priority,
        weight: model.weight,
        context_window: model.context_window,
        features: model.features,
        labels: model.labels,
      })),
    },
    mcp: {
      advertised_servers: registration.mcp_servers.map((server) => ({
        name: server.name,
        transport: server.transport,
        uri: server.uri,
        allowed_tools: server.allowed_tools,
        auth_kind: isRecord(server.auth) && typeof server.auth.kind === "string" ? server.auth.kind : "unknown",
      })),
      default_memory_mcp_url: process.env.COAT_MEMORY_MCP_URL ?? null,
      propagation_supported: [
        "coordinator_issued",
        "runner_resolves_refs",
        "workload_identity",
        "runner_local_only",
        "oauth_device_broker",
        "external_broker",
      ],
      access_modes_supported: ["single_user", ...(registration.capabilities.includes("oidc_user_delegation") ? ["multi_user_oidc"] : [])],
      secret_values_exposed: false,
    },
    subagents: {
      durable_task_queue_required: true,
      native_subagent_spawn_disabled: true,
      child_request_channel: "AgentRunResult.child_requests",
      runner_context_injected: true,
      instructions: durableSubagentContext,
    },
    review_contract: {
      supports_review_output: true,
      supports_unification: registration.roles.includes("patch_merger"),
      decisions: ["accept", "changes_requested", "blocked", "inconclusive"],
      reward_range: [0, 1],
    },
    result_channels: {
      git_worktrees: registration.capabilities.includes("git_worktree"),
      object_storage: registration.capabilities.includes("object_storage"),
      s3_compatible: registration.capabilities.includes("s3_compatible"),
      object_store_endpoint_configured: Boolean(process.env.COAT_OBJECT_STORE_ENDPOINT),
      object_store_bucket: process.env.COAT_OBJECT_STORE_BUCKET ?? null,
      secret_values_exposed: false,
    },
  };
}

function subagentDiagnostics(subagents: unknown): string[] {
  const mode = isRecord(subagents) && typeof subagents.mode === "string" ? subagents.mode : "coordinator_durable_tasks";
  const nativeSpawn = isRecord(subagents) && typeof subagents.native_spawn === "string" ? subagents.native_spawn : "disabled";
  const childChannel =
    isRecord(subagents) && typeof subagents.child_request_channel === "string"
      ? subagents.child_request_channel
      : "agent_run_result_child_requests";
  return [
    `subagent_mode=${mode}`,
    `native_subagent_spawn=${nativeSpawn}`,
    `child_request_channel=${childChannel}`,
    "subagent_runner_context=durable_child_tasks_only",
  ];
}

function mcpDiagnostics(mcp: unknown): string[] {
  if (!isRecord(mcp)) return ["mcp_context=none"];
  const servers = Array.isArray(mcp.servers) ? mcp.servers : [];
  const secretRefs = collectSecretRefs(mcp);
  const authDistribution = isRecord(mcp.auth_distribution) ? mcp.auth_distribution : undefined;
  const oidcDelegation = isRecord(mcp.oidc_delegation) ? mcp.oidc_delegation : undefined;
  const user = isRecord(mcp.user) ? mcp.user : undefined;
  const requiredLabels = isRecord(authDistribution?.required_runner_labels)
    ? Object.entries(authDistribution.required_runner_labels).map(([key, value]) => `${key}=${String(value)}`)
    : [];
  const materials = Array.isArray(authDistribution?.allowed_materials)
    ? authDistribution.allowed_materials.map(String)
    : [];
  return [
    `mcp_servers=${servers.length}`,
    `mcp_access_mode=${typeof mcp.access_mode === "string" ? mcp.access_mode : "single_user"}`,
    `mcp_oidc_delegation=${Boolean(oidcDelegation)}`,
    `mcp_user_ref=${typeof user?.user_id === "string" ? user.user_id : "none"}`,
    `mcp_secret_refs=${secretRefs.length}`,
    `mcp_auth_distribution=${typeof authDistribution?.mode === "string" ? authDistribution.mode : "default"}`,
    `mcp_auth_materials=${materials.length ? materials.join(",") : "default"}`,
    `mcp_auth_required_runner_labels=${requiredLabels.length ? requiredLabels.join(",") : "none"}`,
    `mcp_auth_secret_sync_allowed=${authDistribution?.allow_secret_sync === true}`,
    `mcp_auth_node_local_device_allowed=${authDistribution?.allow_node_local_device_session !== false}`,
    ...secretRefs.map(describeSecretRef),
  ];
}

function collectSecretRefs(mcp: Record<string, unknown>): SecretRef[] {
  const refs: SecretRef[] = [];
  if (Array.isArray(mcp.secret_refs)) {
    refs.push(...mcp.secret_refs.filter(isSecretRef));
  }
  if (isRecord(mcp.oidc_delegation)) {
    if (isSecretRef(mcp.oidc_delegation.token_broker)) refs.push(mcp.oidc_delegation.token_broker);
    if (isSecretRef(mcp.oidc_delegation.token_cache_ref)) refs.push(mcp.oidc_delegation.token_cache_ref);
  }
  if (Array.isArray(mcp.servers)) {
    for (const server of mcp.servers.filter(isRecord)) {
      const auth = server.auth;
      if (!isRecord(auth)) continue;
      if (auth.kind === "secret" && isSecretRef(auth.secret)) refs.push(auth.secret);
      if (auth.kind === "oauth_delegation" && isSecretRef(auth.token_exchange_secret)) {
        refs.push(auth.token_exchange_secret);
      }
      if (auth.kind === "oidc_delegation" && isSecretRef(auth.broker)) {
        refs.push(auth.broker);
      }
      if (auth.kind === "device_auth_session") {
        if (isSecretRef(auth.session_ref)) refs.push(auth.session_ref);
        if (isSecretRef(auth.refresh_ref)) refs.push(auth.refresh_ref);
      }
      if (auth.kind === "brokered_user_session" && isSecretRef(auth.broker)) {
        refs.push(auth.broker);
      }
    }
  }
  return refs;
}

function describeSecretRef(secret: SecretRef): string {
  const provider = secret.provider ?? "unknown";
  const name = secret.name ?? "unnamed";
  const key = secret.key ?? "";
  if (provider === "env") {
    const envName = key || name;
    return `secret_ref env:${envName} resolved=${Boolean(process.env[envName])}`;
  }
  if (provider === "local_file") {
    return `secret_ref local_file:${name} resolved=${existsSync(name)}`;
  }
  if (
    [
      "kubernetes_secret",
      "vault",
      "aws_secrets_manager",
      "gcp_secret_manager",
      "azure_key_vault",
      "one_password",
      "bitwarden",
      "doppler",
      "sops",
      "external_broker",
    ].includes(provider)
  ) {
    return `secret_ref ${provider}:${name}${key ? `/${key}` : ""} resolved=delegated`;
  }
  return `secret_ref ${provider}:${name}${key ? `/${key}` : ""} resolved=delegated`;
}

function isSecretRef(value: unknown): value is SecretRef {
  return isRecord(value) && typeof value.name === "string";
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function redactUrl(raw: string): string {
  try {
    const url = new URL(raw);
    if (url.username) url.username = "redacted";
    if (url.password) url.password = "redacted";
    for (const key of [...url.searchParams.keys()]) {
      if (key.toLowerCase().includes("token") || key.toLowerCase().includes("secret")) {
        url.searchParams.set(key, "redacted");
      }
    }
    return url.toString();
  } catch {
    return raw;
  }
}

function isMainModule(): boolean {
  const entrypoint = process.argv[1];
  return Boolean(entrypoint) && fileURLToPath(import.meta.url) === entrypoint;
}

function parseJsonEnv<T>(name: string, fallback: T): T {
  const raw = process.env[name];
  if (!raw) return fallback;
  return JSON.parse(raw) as T;
}

function numberEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const value = Number(raw);
  return Number.isFinite(value) ? value : fallback;
}

async function sleep(ms: number): Promise<void> {
  await new Promise<void>((resolve) => setTimeout(resolve, ms));
}

function json(res: http.ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body, null, 2));
}

async function readJson(req: http.IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  const raw = Buffer.concat(chunks).toString("utf8");
  return raw ? JSON.parse(raw) : {};
}
