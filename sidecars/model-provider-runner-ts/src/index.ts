/**
 * Generic model-provider runner sidecar.
 *
 * Purpose: adapt COAT's durable runner contract to hosted and local model
 * providers that are not Codex or Claude Code harnesses. This sidecar is the
 * common wrapper for Bedrock, OpenAI-compatible APIs, vLLM, Ollama, llama.cpp,
 * Hugging Face endpoints, and local process providers.
 *
 * It intentionally starts in stub mode. Live provider calls are enabled only by
 * explicit mode/config because durable task state, MCP auth refs, sandboxing,
 * and human approvals must stay coordinator-owned.
 *
 * Architecture reference:
 * - docs/design-docs/010-distributed-runners-mcp.md
 * - docs/operations/model-runner-clusters.md
 * - docs/operations/ephemeral-kubernetes-runners.md
 */
import http from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const port = numberEnv("PORT", 9093);
const mode = process.env.MODEL_PROVIDER_RUNNER_MODE ?? "stub";
const provider = (process.env.MODEL_PROVIDER_KIND ?? "open_ai_compatible") as ModelProviderKind;
const runnerId = process.env.RUNNER_ID ?? "model-provider-runner-ts";
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

type ModelLatencyClass = "fast" | "balanced" | "deep" | "batch";
type ModelReasoningEffort = "minimal" | "low" | "medium" | "high" | "xhigh";

type ModelRuntimeParameters = {
  latency_class?: ModelLatencyClass;
  speed_tier?: string;
  temperature_milli?: number;
  top_p_milli?: number;
  max_output_tokens?: number;
  reasoning_effort?: ModelReasoningEffort;
  timeout_seconds?: number;
  extra?: Record<string, string>;
};

type ModelCandidate = {
  provider: ModelProviderKind;
  model: string;
  endpoint: string | null;
  priority: number;
  weight: number;
  context_window: number | null;
  features: ModelFeature[];
  params?: ModelRuntimeParameters;
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
  "Do not spawn provider-native, SDK, MCP-client, or framework-local subagents from inside this runner.",
  "Return proposed child work only through AgentRunResult.child_requests.",
  "The coordinator validates budgets, approval policy, routing, memory context, and sandbox policy before dispatch.",
];

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET" && req.url === "/healthz") return json(res, 200, { status: "ok", mode, provider, runner_id: runnerId });
    if (req.method === "GET" && req.url === "/registration") return json(res, 200, buildRegistration());
    if (req.method === "GET" && req.url === "/capabilities") return json(res, 200, buildCapabilities());
    if (req.method === "GET" && req.url === "/verify") return json(res, 200, await verifyProvider());
    if (req.method === "POST" && req.url === "/run-task") return json(res, 200, await runTask((await readJson(req)) as AgentRunRequest));
    return json(res, 404, { error: "not_found" });
  } catch (error) {
    return json(res, 500, { error: error instanceof Error ? error.message : String(error) });
  }
});

server.listen(port, () => {
  console.log(`model-provider-runner-ts listening on :${port} (${mode}, ${provider})`);
  void registerAndHeartbeat();
});

async function runTask(request: AgentRunRequest): Promise<AgentRunResult> {
  runningTasks += 1;
  try {
    const selected = selectedModel(request);
    return {
      task_id: request.task.id,
      status: mode === "stub" ? "done" : "blocked",
      summary:
        mode === "stub"
          ? `stub ${provider} runner accepted ${taskPurposeKind(request.task.purpose)} task ${request.task.id}`
          : `live ${provider} execution is not enabled by this safety stub`,
      review: reviewOutput(request),
      research: researchOutput(request),
      branch_vote: branchVoteOutput(request),
      runner_id: runnerId,
      model_used: selected,
      mcp_context_used: request.task.execution?.mcp ?? null,
      artifacts: [
        {
          kind: "report",
          uri: `memory://model-provider-runner/${provider}/${request.task.id}`,
          description: `${provider} runner contract artifact`,
          sha256: null,
        },
      ],
      git_result: null,
      object_artifacts: [],
      checkpoints: checkpointRefs(request),
      test_evidence: testEvidence(request),
      child_requests: [],
      confidence: mode === "stub" ? 0.72 : 0.35,
      next_actions: mode === "stub" ? [] : ["enable a provider-specific live adapter after sandbox and approval review"],
      diagnostics: [
        `mode=${mode}`,
        `provider=${provider}`,
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
  }
}

function buildRegistration(): RunnerRegistration {
  const local = provider === "vllm" || provider === "ollama" || provider === "llama_cpp" || provider === "local_process";
  return {
    runner_id: runnerId,
    node_id: nodeId,
    endpoint: runnerEndpoint,
    roles: parseJsonEnv("RUNNER_ROLES_JSON", [
      "model_provider",
      "planner",
      "research",
      "reviewer",
      "tester",
      "validator",
      "patch_merger",
      "formal_methods",
    ] satisfies WorkerKind[]),
    capabilities: parseJsonEnv("RUNNER_CAPABILITIES_JSON", [
      "research",
      ...(webSearchEnabled() ? ["web_search" as RunnerCapability, "network_open" as RunnerCapability] : []),
      "review",
      "test",
      "mcp_tools",
      "durable_child_tasks",
      "open_ai_compatible",
      ...(local ? ["local_models" as RunnerCapability] : []),
      ...(provider === "vllm" ? ["vllm" as RunnerCapability, "gpu" as RunnerCapability] : []),
    ] satisfies RunnerCapability[]),
    models: parseJsonEnv("RUNNER_MODELS_JSON", [defaultModelCandidate()]),
    labels: parseJsonEnv("RUNNER_LABELS_JSON", {
      pool: "default",
      runtime: "model-provider",
      provider,
      locality: local ? "local" : "hosted",
    }),
    mcp_servers: parseJsonEnv("RUNNER_MCP_SERVERS_JSON", [] satisfies McpServerRef[]),
    max_concurrency: maxConcurrency,
    lease_ttl_seconds: leaseTtlSeconds,
  };
}

function defaultModelCandidate(): ModelCandidate {
  const configuredModel = configuredModelForLane();
  return {
    provider,
    model: configuredModel,
    endpoint: configuredEndpointForProvider(provider),
    priority: numberEnv("MODEL_PROVIDER_PRIORITY", 100),
    weight: numberEnv("MODEL_PROVIDER_WEIGHT", 1),
    context_window: optionalNumberEnv("MODEL_PROVIDER_CONTEXT_WINDOW"),
    features: parseJsonEnv("MODEL_PROVIDER_FEATURES_JSON", defaultFeaturesFor(provider)),
    params: modelRuntimeParameters(),
    labels: {
      ...modelParameterLabels(),
      model_configured: configuredModel ? "true" : "false",
      llm_gateway_configured: nonEmptyEnv("COAT_LLM_GATEWAY_URL") ? "true" : "false",
      llm_gateway_provider: nonEmptyEnv("COAT_LLM_GATEWAY_PROVIDER") ?? "",
      ...parseJsonEnv("MODEL_PROVIDER_MODEL_LABELS_JSON", {}),
    },
  };
}

function configuredModelForLane(): string {
  const model = nonEmptyEnv("MODEL_PROVIDER_MODEL");
  if (model) return model;
  if (runnerLane() === "research") {
    const researchModel = nonEmptyEnv("COAT_LLM_GATEWAY_RESEARCH_MODEL");
    if (researchModel) return researchModel;
  }
  return nonEmptyEnv("COAT_LLM_GATEWAY_WORK_MODEL")
    ?? nonEmptyEnv("COAT_LLM_GATEWAY_DEFAULT_MODEL")
    ?? "";
}

function configuredEndpointForProvider(kind: ModelProviderKind): string | null {
  return nonEmptyEnv("MODEL_PROVIDER_ENDPOINT")
    ?? ((kind === "open_ai_compatible" || kind === "open_ai") ? nonEmptyEnv("COAT_LLM_GATEWAY_URL") : undefined)
    ?? defaultEndpointFor(kind);
}

function runnerLane(): "research" | "work" {
  const raw = `${runnerId} ${nodeId} ${process.env.RUNNER_LABELS_JSON ?? ""}`.toLowerCase();
  return raw.includes("research") ? "research" : "work";
}

function modelRuntimeParameters(): ModelRuntimeParameters {
  const params: ModelRuntimeParameters = {};
  const latencyClass = enumEnv<ModelLatencyClass>("MODEL_PROVIDER_LATENCY_CLASS", ["fast", "balanced", "deep", "batch"]);
  if (latencyClass) params.latency_class = latencyClass;
  const speedTier = process.env.MODEL_PROVIDER_SPEED_TIER?.trim();
  if (speedTier) params.speed_tier = speedTier;
  const temperature = optionalMilliEnv("MODEL_PROVIDER_TEMPERATURE");
  if (temperature !== null) params.temperature_milli = temperature;
  const topP = optionalMilliEnv("MODEL_PROVIDER_TOP_P");
  if (topP !== null) params.top_p_milli = topP;
  const maxOutputTokens = optionalIntegerEnv("MODEL_PROVIDER_MAX_OUTPUT_TOKENS");
  if (maxOutputTokens !== null) params.max_output_tokens = maxOutputTokens;
  const reasoningEffort = enumEnv<ModelReasoningEffort>("MODEL_PROVIDER_REASONING_EFFORT", ["minimal", "low", "medium", "high", "xhigh"]);
  if (reasoningEffort) params.reasoning_effort = reasoningEffort;
  const timeoutSeconds = optionalIntegerEnv("MODEL_PROVIDER_TIMEOUT_SECONDS");
  if (timeoutSeconds !== null) params.timeout_seconds = timeoutSeconds;
  const extra = parseJsonEnv("MODEL_PROVIDER_PARAMS_EXTRA_JSON", {} as Record<string, string>);
  if (Object.keys(extra).length > 0) params.extra = extra;
  return params;
}

function modelParameterLabels(): Record<string, string> {
  const labels: Record<string, string> = {};
  for (const [envName, labelName] of [
    ["MODEL_PROVIDER_LATENCY_CLASS", "latency_class"],
    ["MODEL_PROVIDER_SPEED_TIER", "speed_tier"],
    ["MODEL_PROVIDER_TEMPERATURE", "temperature"],
    ["MODEL_PROVIDER_TOP_P", "top_p"],
    ["MODEL_PROVIDER_MAX_OUTPUT_TOKENS", "max_output_tokens"],
    ["MODEL_PROVIDER_REASONING_EFFORT", "reasoning_effort"],
    ["MODEL_PROVIDER_TIMEOUT_SECONDS", "timeout_seconds"],
  ] as const) {
    const value = process.env[envName];
    if (value) labels[labelName] = value;
  }
  return labels;
}

function buildCapabilities(): Record<string, unknown> {
  const registration = buildRegistration();
  return {
    runner_id: runnerId,
    node_id: nodeId,
    mode,
    provider,
    endpoints: ["/healthz", "/registration", "/capabilities", "/verify", "/run-task"],
    registration,
    provider_verification: {
      endpoint: "/verify",
      live_execution_requires: providerRequirements(provider),
      secret_values_exposed: false,
    },
    model_routing: {
      providers: [...new Set(registration.models.map((model) => model.provider))],
      models: registration.models,
      default_params: modelRuntimeParameters(),
      supports_multiple_models: true,
      selection: "coordinator-routed candidate first, runner default fallback",
    },
    mcp: {
      advertised_servers: registration.mcp_servers.map(redactedMcpServer),
      default_memory_mcp_url: process.env.COAT_MEMORY_MCP_URL ?? null,
      propagation_supported: ["coordinator_issued", "runner_resolves_refs", "workload_identity", "external_broker"],
      access_modes_supported: ["single_user", ...(registration.capabilities.includes("oidc_user_delegation") ? ["multi_user_oidc"] : [])],
      secret_values_exposed: false,
    },
    subagents: {
      durable_task_queue_required: true,
      native_subagent_spawn_disabled: true,
      child_request_channel: "AgentRunResult.child_requests",
      instructions: durableSubagentContext,
    },
  };
}

async function verifyProvider(): Promise<Record<string, unknown>> {
  const endpoint = configuredEndpointForProvider(provider);
  const verifyEndpoint = process.env.MODEL_PROVIDER_VERIFY_ENDPOINT === "1";
  const authMode = process.env.MODEL_PROVIDER_AUTH_MODE ?? (["ollama", "vllm", "llama_cpp", "local_process"].includes(provider) ? "none" : "api_key_or_none");
  const auth = {
    mode: authMode,
    has_openai_api_key: Boolean(process.env.OPENAI_API_KEY),
    has_model_provider_api_key: Boolean(process.env.MODEL_PROVIDER_API_KEY),
    has_llm_gateway_api_key: Boolean(process.env.COAT_LLM_GATEWAY_API_KEY),
    has_anthropic_api_key: Boolean(process.env.ANTHROPIC_API_KEY),
    brokered_auth_allowed: authMode === "external_broker" || authMode === "oauth_device_broker",
    workload_identity_allowed: authMode === "workload_identity",
    unauthenticated_allowed: authMode === "none" || authMode === "api_key_or_none",
    secret_values_exposed: false,
  };
  const model_params = modelRuntimeParameters();
  if (provider === "bedrock") {
    return {
      provider,
      mode,
      available: Boolean(process.env.AWS_REGION || process.env.AWS_DEFAULT_REGION),
      auth,
      model_params,
      checks: {
        aws_region: process.env.AWS_REGION ?? process.env.AWS_DEFAULT_REGION ?? null,
        has_static_key: Boolean(process.env.AWS_ACCESS_KEY_ID),
        has_web_identity: Boolean(process.env.AWS_WEB_IDENTITY_TOKEN_FILE),
      },
      secret_values_exposed: false,
    };
  }
  if (provider === "local_process") {
    const command = process.env.MODEL_PROVIDER_COMMAND ?? "";
    return { provider, mode, auth, model_params, available: Boolean(command), command_configured: Boolean(command), secret_values_exposed: false };
  }
  if (!endpoint || !verifyEndpoint) {
    return { provider, mode, auth, model_params, available: Boolean(endpoint), endpoint, endpoint_probe_attempted: false, secret_values_exposed: false };
  }
  try {
    const apiKey = providerApiKey();
    const response = await fetch(modelsUrlFromEndpoint(endpoint), {
      method: "GET",
      headers: apiKey ? { authorization: `Bearer ${apiKey}` } : {},
    });
    return { provider, mode, auth, model_params, available: response.ok, endpoint, endpoint_probe_attempted: true, status: response.status, secret_values_exposed: false };
  } catch (error) {
    return { provider, mode, auth, model_params, available: false, endpoint, endpoint_probe_attempted: true, error: error instanceof Error ? error.message : String(error), secret_values_exposed: false };
  }
}

async function registerAndHeartbeat(): Promise<void> {
  if (!registryUrl) return;
  try {
    await registerRunner();
    await sendHeartbeat();
    setInterval(() => void sendHeartbeat().catch((error) => console.error("runner heartbeat failed", error)), heartbeatIntervalMs);
  } catch (error) {
    console.error("runner registration failed", error);
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
    const match = candidates.find((candidate) => isRecord(candidate) && candidate.provider === provider);
    if (match) return match;
    if (candidates[0]) return candidates[0];
  }
  return defaultModelCandidate();
}

function reviewOutput(request: AgentRunRequest): unknown | null {
  const purpose = taskPurposeKind(request.task.purpose);
  if (purpose !== "review" && purpose !== "unification" && purpose !== "branch_vote" && purpose !== "branch_unification") return null;
  return {
    decision: mode === "stub" ? "inconclusive" : "blocked",
    reward: 0.65,
    findings: [],
    objective_results: [],
    gate_results: [],
    retry_recommended: mode !== "stub",
    unification_summary: purpose.includes("unification") ? `${provider} runner returned a ${mode} unification placeholder` : null,
  };
}

function researchOutput(request: AgentRunRequest): unknown | null {
  const purpose = request.task.purpose;
  if (!isRecord(purpose) || purpose.kind !== "research") return null;
  const question = typeof purpose.question === "string" ? purpose.question : request.task.prompt;
  return {
    question,
    answer: `${mode} ${provider} placeholder answer for: ${question}`,
    sources: [
      {
        title: `${provider} runner placeholder source`,
        uri: `memory://model-provider-runner/${provider}/research/${request.task.id}`,
        quality: "unknown",
        captured_at: null,
        quote: null,
        summary: "Replace with sourced provider/web/MCP research before production validation.",
        confidence: 0.6,
      },
    ],
    confidence: 0.6,
    use_plan: {
      facts_to_use: ["Use only as contract smoke output."],
      facts_to_avoid: ["Do not treat model-provider stub output as sourced evidence."],
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
  return { group_id: purpose.group_id, selected_task_id: candidates[0], ranked_task_ids: candidates, confidence: 0.6, rationale: `${provider} placeholder vote` };
}

function testEvidence(request: AgentRunRequest): unknown[] {
  if (["research", "review", "unification", "branch_vote", "branch_unification"].includes(taskPurposeKind(request.task.purpose))) return [];
  return [
    {
      command: `stub ${provider} execution evidence`,
      exit_code: 0,
      passed: mode === "stub",
      duration_ms: 0,
      stdout_uri: `memory://model-provider-runner/${provider}/${request.task.id}/stdout`,
      stderr_uri: null,
      artifact_uri: `memory://model-provider-runner/${provider}/${request.task.id}`,
      notes: ["Stub mode records evidence shape; live provider adapters must record real execution outputs."],
    },
  ];
}

function checkpointRefs(request: AgentRunRequest): unknown[] {
  if (!isRecord(request.task.execution?.results) || !isRecord(request.task.execution?.results.checkpoints)) return [];
  if (request.task.execution.results.checkpoints.enabled !== true) return [];
  return [
    {
      kind: "metadata",
      uri: `memory://model-provider-runner/${provider}/checkpoint/${request.task.id}`,
      description: `${provider} runner checkpoint placeholder`,
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

function defaultEndpointFor(kind: ModelProviderKind): string | null {
  switch (kind) {
    case "open_ai":
      return "https://api.openai.com/v1";
    case "open_ai_compatible":
      return nonEmptyEnv("OPENAI_COMPATIBLE_BASE_URL") ?? null;
    case "vllm":
      return "http://vllm:8000/v1";
    case "ollama":
      return "http://ollama:11434/v1";
    case "llama_cpp":
      return "http://llama-cpp:8080/v1";
    default:
      return null;
  }
}

function modelsUrlFromEndpoint(endpoint: string): string {
  const base = endpoint.trim().replace(/\/+$/, "");
  if (base.endsWith("/models")) return base;
  if (base.endsWith("/v1")) return `${base}/models`;
  if (base.endsWith("/v1/chat/completions")) return `${base.slice(0, -"/chat/completions".length)}/models`;
  if (base.endsWith("/chat/completions")) return `${base.slice(0, -"/chat/completions".length)}/models`;
  return `${base}/v1/models`;
}

function defaultFeaturesFor(kind: ModelProviderKind): ModelFeature[] {
  const base: ModelFeature[] = ["json_schema", "streaming"];
  if (kind === "vllm" || kind === "ollama" || kind === "llama_cpp" || kind === "local_process") base.push("local_weights");
  if (kind !== "local_process") base.push("tool_use");
  if (webSearchEnabled()) base.push("web_search");
  return base;
}

function webSearchEnabled(): boolean {
  return [
    "MODEL_PROVIDER_WEB_SEARCH_ENABLED",
    "MODEL_PROVIDER_RESEARCH_WEB_SEARCH_ENABLED",
    "COAT_WEB_SEARCH_ENABLED",
  ].some((key) => truthyEnv(key));
}

function truthyEnv(key: string): boolean {
  const value = process.env[key]?.trim().toLowerCase();
  return value === "1" || value === "true" || value === "yes" || value === "on";
}

function providerRequirements(kind: ModelProviderKind): string[] {
  switch (kind) {
    case "bedrock":
      return ["AWS region", "IAM role or AWS credentials", "model access grant"];
    case "vllm":
      return ["vLLM OpenAI-compatible endpoint", "served model name"];
    case "ollama":
      return ["Ollama endpoint", "pulled model"];
    case "llama_cpp":
      return ["llama.cpp server endpoint", "loaded model"];
    case "local_process":
      return ["MODEL_PROVIDER_COMMAND", "sandboxed process execution"];
    default:
      return ["provider endpoint", "provider credentials when required"];
  }
}

function providerApiKey(): string | null {
  const laneKey = nonEmptyEnv("MODEL_PROVIDER_API_KEY");
  if (laneKey) return laneKey;
  const gatewayKey = nonEmptyEnv("COAT_LLM_GATEWAY_API_KEY");
  if (gatewayKey) return gatewayKey;
  switch (provider) {
    case "open_ai":
      return nonEmptyEnv("OPENAI_API_KEY") ?? null;
    case "anthropic":
      return nonEmptyEnv("ANTHROPIC_API_KEY") ?? nonEmptyEnv("ANTHROPIC_AUTH_TOKEN") ?? null;
    case "hugging_face":
      return nonEmptyEnv("HF_TOKEN") ?? nonEmptyEnv("HUGGINGFACE_TOKEN") ?? null;
    default:
      return null;
  }
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

function parseJsonEnv<T>(name: string, fallback: T): T {
  const raw = process.env[name];
  if (!raw) return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch (error) {
    console.error(`invalid ${name}; using fallback`, error);
    return fallback;
  }
}

function nonEmptyEnv(name: string): string | undefined {
  const value = process.env[name]?.trim();
  return value ? value : undefined;
}

function numberEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function optionalNumberEnv(name: string): number | null {
  const raw = process.env[name];
  if (!raw) return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

function optionalIntegerEnv(name: string): number | null {
  const parsed = optionalNumberEnv(name);
  return parsed === null ? null : Math.max(0, Math.trunc(parsed));
}

function optionalMilliEnv(name: string): number | null {
  const parsed = optionalNumberEnv(name);
  return parsed === null ? null : Math.max(0, Math.round(parsed * 1000));
}

function enumEnv<T extends string>(name: string, allowed: readonly T[]): T | null {
  const raw = process.env[name];
  if (!raw) return null;
  return allowed.includes(raw as T) ? (raw as T) : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
