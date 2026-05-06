import http from "node:http";
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const port = Number(process.env.PORT ?? "9092");
const mode = process.env.STAFF_ENGINEER_RUNNER_MODE ?? "stub";
const runnerId = process.env.RUNNER_ID ?? "staff-engineer-runner-ts";
const nodeId = process.env.NODE_ID ?? process.env.HOSTNAME ?? "local-node";
const registryUrl = process.env.RUNNER_REGISTRY_URL ?? process.env.COAT_RUNNER_REGISTRY ?? "";
const runnerEndpoint = process.env.RUNNER_ENDPOINT ?? `http://localhost:${port}`;
const leaseTtlSeconds = numberEnv("RUNNER_LEASE_TTL_SECONDS", 300);
const heartbeatIntervalMs = numberEnv("RUNNER_HEARTBEAT_INTERVAL_MS", 30_000);
const maxConcurrency = numberEnv("RUNNER_MAX_CONCURRENCY", 1);
let runningTasks = 0;

type WorkerKind =
  | "planner"
  | "codex"
  | "staff_engineer_claude"
  | "research"
  | "reviewer"
  | "tester"
  | "validator"
  | "patch_merger"
  | "rust_tool";

type RunnerCapability =
  | "code"
  | "research"
  | "test"
  | "review"
  | "mcp_tools"
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
  | "network_open";

type ModelProviderKind =
  | "codex"
  | "open_ai"
  | "open_ai_compatible"
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
  | "local_weights";

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
    prompt: string;
    execution?: {
      model?: {
        candidates?: unknown[];
      };
      mcp?: unknown;
      results?: unknown;
    };
  };
};

type AgentRunResult = {
  task_id: string;
  status: "done" | "partial" | "blocked" | "failed";
  summary: string;
  review: ReviewOutput | null;
  research: ResearchOutput | null;
  runner_id: string | null;
  model_used: unknown | null;
  mcp_context_used: unknown | null;
  artifacts: Array<{ kind: string; uri: string; description: string; sha256?: string | null }>;
  git_result: unknown | null;
  object_artifacts: unknown[];
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
    evidence: string[];
    suggested_action: string | null;
  }>;
  retry_recommended: boolean;
  unification_summary: string | null;
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

const memoryGatewayUrl = process.env.COAT_MEMORY_GATEWAY_URL ?? process.env.MEMORY_GATEWAY_URL ?? "";
const memoryGatewayToken = process.env.COAT_MEMORY_GATEWAY_TOKEN ?? process.env.MEMORY_GATEWAY_TOKEN ?? "";
const memoryContextLimit = numberEnv("MEMORY_CONTEXT_LIMIT", 8);

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET" && req.url === "/healthz") {
      return json(res, 200, { status: "ok", mode, runner_id: runnerId });
    }
    if (req.method === "GET" && req.url === "/registration") {
      return json(res, 200, buildRegistration());
    }
    if (req.method === "GET" && req.url === "/capabilities") {
      return json(res, 200, buildCapabilities());
    }
    if (req.method === "GET" && req.url === "/verify") {
      return json(res, 200, await verifyCtxrPackage());
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

server.listen(port, () => {
  console.log(`staff-engineer-runner-ts listening on :${port} (${mode})`);
  void registerAndHeartbeat();
});

async function verifyCtxrPackage(): Promise<Record<string, unknown>> {
  try {
    const { stdout } = await execFileAsync("npm", [
      "view",
      "@ctxr/agent-staff-engineer",
      "version",
      "--json",
    ]);
    return { available: true, version: JSON.parse(stdout), package: "@ctxr/agent-staff-engineer" };
  } catch (error) {
    return {
      available: false,
      package: "@ctxr/agent-staff-engineer",
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

async function runTask(request: AgentRunRequest): Promise<AgentRunResult> {
  runningTasks += 1;
  try {
    let memoryContextError: string | null = null;
    const memoryContext = await fetchMemoryContext(request).catch((error) => {
      memoryContextError = error instanceof Error ? error.message : String(error);
      return null;
    });
    const gitResult = buildGitResult(request);
    const objectArtifacts = buildObjectArtifacts(request);
    return {
      task_id: request.task.id,
      status: taskStatus(request),
      summary: taskSummary(request),
      review: reviewOutput(request),
      research: researchOutput(request, memoryContext),
      runner_id: runnerId,
      model_used: request.task.execution?.model?.candidates?.[0] ?? null,
      mcp_context_used: request.task.execution?.mcp ?? null,
      artifacts: [
        {
          kind: "report",
          uri: `memory://staff-engineer-runner/${request.task.id}`,
          description: `${taskPurposeKind(request.task.purpose)} setup blockers from staff-engineer runner`,
          sha256: null,
        },
        ...resultChannelArtifacts(request, gitResult, objectArtifacts),
        ...memoryContextArtifacts(request, memoryContext),
      ],
      git_result: gitResult,
      object_artifacts: objectArtifacts,
      child_requests: [],
      confidence: taskPurposeKind(request.task.purpose) === "work" ? 0.2 : 0.7,
      next_actions: [
        "verify @ctxr/kit and @ctxr/agent-staff-engineer in the target environment",
        "configure tracker and Claude Code credentials before live runs",
      ],
      diagnostics: [
        `mode=${mode}`,
        `runner_id=${runnerId}`,
        `node_id=${nodeId}`,
        `task_purpose=${JSON.stringify(request.task.purpose ?? { kind: "work" })}`,
        ...mcpDiagnostics(request.task.execution?.mcp),
        ...memoryContextDiagnostics(memoryContext, memoryContextError),
      ],
      notification_reports: [],
    };
  } finally {
    runningTasks -= 1;
  }
}

function taskStatus(request: AgentRunRequest): "done" | "blocked" {
  const purpose = taskPurposeKind(request.task.purpose);
  return purpose === "review" || purpose === "unification" || purpose === "research" ? "done" : "blocked";
}

function reviewOutput(request: AgentRunRequest): ReviewOutput | null {
  const purpose = taskPurposeKind(request.task.purpose);
  if (purpose === "review") {
    return {
      decision: "inconclusive",
      reward: 0.7,
      findings: [
        {
          severity: "medium",
          title: "Live staff-engineer review is disabled",
          body: "The stub runner verified the review contract shape but did not run Claude Code or tracker-aware review.",
          evidence: [`mode=${mode}`],
          suggested_action: "Enable live staff-engineer mode before treating this as a production review.",
        },
      ],
      retry_recommended: true,
      unification_summary: null,
    };
  }
  if (purpose === "unification") {
    return {
      decision: "inconclusive",
      reward: 0.7,
      findings: [
        {
          severity: "medium",
          title: "Live review unification is disabled",
          body: "The stub runner accepted the unification contract but did not reconcile real review threads.",
          evidence: [`mode=${mode}`],
          suggested_action: "Enable live staff-engineer mode before treating this as a production unification.",
        },
      ],
      retry_recommended: true,
      unification_summary: "stub unification is inconclusive",
    };
  }
  return null;
}

function researchOutput(request: AgentRunRequest, memoryContext: MemoryContextResponse | null): ResearchOutput | null {
  const purpose = request.task.purpose;
  if (!isRecord(purpose) || purpose.kind !== "research") return null;
  const question = typeof purpose.question === "string" ? purpose.question : request.task.prompt;
  const contextFacts = memoryContext?.use_plan.facts_to_use.slice(0, 3) ?? [];
  return {
    question,
    answer: `staff-engineer stub research answer for: ${question}`,
    sources: [
      {
        title: "staff-engineer runner stub source",
        uri: `memory://staff-engineer-runner/research/${request.task.id}`,
        quality: "unknown",
        captured_at: null,
        quote: null,
        summary: "Structured placeholder source; enable live tracker/repo/web research for real evidence.",
        confidence: 0.7,
      },
    ],
    confidence: 0.7,
    use_plan: {
      facts_to_use: ["Use this only to validate the research contract shape.", ...contextFacts],
      facts_to_avoid: [
        "Do not use stub research as reviewed evidence.",
        ...(memoryContext?.use_plan.facts_to_avoid ?? []),
      ],
      proposed_task_updates: [],
      validation_checks: [
        "Replace stub source with tracker, repo, web, or docs evidence.",
        ...(memoryContext?.use_plan.validation_checks ?? []),
      ],
    },
    open_questions: [],
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
  const branchPrefix = typeof git.branch_prefix === "string" ? git.branch_prefix.replace(/\/+$/, "") : "coat/task";
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
  const bucket = typeof store.bucket === "string" ? store.bucket : "coat-artifacts";
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
      description: "stub object storage artifact manifest location",
    },
  ];
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
  if (mode !== "stub") {
    return `staff-engineer runner mode ${mode} accepted ${purpose} ${request.task.id}`;
  }
  if (purpose === "review") {
    return `staff-engineer critic contract accepted ${request.task.id}; live Claude Code review is not enabled`;
  }
  if (purpose === "unification") {
    return `staff-engineer unification contract accepted ${request.task.id}; live Claude Code review merge is not enabled`;
  }
  if (purpose === "research") {
    return `staff-engineer research contract accepted ${request.task.id}; live source gathering is not enabled`;
  }
  return `staff-engineer worker contract accepted ${request.task.id}; live Claude Code execution is not enabled`;
}

function taskPurposeKind(purpose: unknown): string {
  return isRecord(purpose) && typeof purpose.kind === "string" ? purpose.kind : "work";
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
      ["staff_engineer_claude", "reviewer", "patch_merger"] satisfies WorkerKind[],
    ),
    capabilities: parseJsonEnv("RUNNER_CAPABILITIES_JSON", [
      "code",
      "review",
      "workspace_sandbox",
      "git_worktree",
      "git",
      "object_storage",
      "s3_compatible",
      "notifications",
    ] satisfies RunnerCapability[]),
    models: parseJsonEnv("RUNNER_MODELS_JSON", [
      {
        provider: "local_process",
        model: process.env.STAFF_ENGINEER_MODEL ?? "claude-code-staff-engineer",
        endpoint: null,
        priority: 100,
        weight: 1,
        context_window: null,
        features: ["tool_use", "json_schema"],
        labels: {},
      },
    ] satisfies ModelCandidate[]),
    labels: parseJsonEnv("RUNNER_LABELS_JSON", { pool: "default", runtime: "staff-engineer" }),
    mcp_servers: parseJsonEnv("RUNNER_MCP_SERVERS_JSON", [] satisfies McpServerRef[]),
    max_concurrency: maxConcurrency,
    lease_ttl_seconds: leaseTtlSeconds,
  };
}

function buildCapabilities(): Record<string, unknown> {
  const registration = buildRegistration();
  return {
    runner_id: runnerId,
    node_id: nodeId,
    mode,
    running_tasks: runningTasks,
    capacity_remaining: Math.max(maxConcurrency - runningTasks, 0),
    endpoints: ["/healthz", "/registration", "/capabilities", "/verify", "/run-task"],
    registration,
    package_verification: {
      endpoint: "/verify",
      package: "@ctxr/agent-staff-engineer",
      live_execution_requires: ["@ctxr/kit", "Claude Code credentials", "tracker credentials"],
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
      secret_values_exposed: false,
    },
    review_contract: {
      supports_review_output: true,
      supports_unification: registration.roles.includes("patch_merger"),
      supports_issue_to_pr_lifecycle: true,
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

function mcpDiagnostics(mcp: unknown): string[] {
  if (!isRecord(mcp)) return ["mcp_context=none"];
  const servers = Array.isArray(mcp.servers) ? mcp.servers : [];
  const secretRefs = collectSecretRefs(mcp);
  const authDistribution = isRecord(mcp.auth_distribution) ? mcp.auth_distribution : undefined;
  const requiredLabels = isRecord(authDistribution?.required_runner_labels)
    ? Object.entries(authDistribution.required_runner_labels).map(([key, value]) => `${key}=${String(value)}`)
    : [];
  const materials = Array.isArray(authDistribution?.allowed_materials)
    ? authDistribution.allowed_materials.map(String)
    : [];
  return [
    `mcp_servers=${servers.length}`,
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
  if (Array.isArray(mcp.servers)) {
    for (const server of mcp.servers.filter(isRecord)) {
      const auth = server.auth;
      if (!isRecord(auth)) continue;
      if (auth.kind === "secret" && isSecretRef(auth.secret)) refs.push(auth.secret);
      if (auth.kind === "oauth_delegation" && isSecretRef(auth.token_exchange_secret)) {
        refs.push(auth.token_exchange_secret);
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
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
