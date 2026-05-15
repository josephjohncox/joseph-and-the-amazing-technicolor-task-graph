/**
 * Optional COAT control gateway and SPA host.
 *
 * Purpose: expose operator visibility and steering APIs for goals, tasks,
 * plans, approvals, runners, events, memory, and MCP dashboard tools. This is a
 * remote-control surface over backend services; it is not a durable engine and
 * must not mutate projections as if they were source-of-truth state.
 *
 * Architecture references:
 * - docs/design-docs/110-control-gateway-spa.md
 * - docs/design-docs/120-durable-planning-mode.md
 */
import http from "node:http";
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname } from "node:path";

type JsonMap = Record<string, unknown>;

type ServiceRef = {
  name: string;
  baseUrl: string;
  healthPath: string;
};

type ProxyResult = {
  ok: boolean;
  status: number;
  url: string;
  data: unknown;
};

type ActiveStateStatus = "fresh" | "stale" | "unavailable";

type ActiveStateRead = {
  snapshot: JsonMap | null;
  attempts: number;
  status: ActiveStateStatus;
  unavailableReads: string[];
  error?: string;
};

type SteeringDirective = {
  id: string;
  goal_id: string;
  task_id: string | null;
  operator: string | null;
  message: string;
  kind: JsonMap;
};

type ChatMessage = {
  role: "system" | "user" | "assistant";
  content: string;
};

type ChatSessionEntry = {
  session_id: string;
  goal_id: string | null;
  mode: string;
  role: "user" | "assistant";
  content: string;
  created_at: string;
  provider?: string;
  model?: string | null;
  payload_json?: JsonMap;
};

type ChatRunTrace = {
  run_id: string;
  session_id: string;
  goal_id: string | null;
  mode: string;
  status: "running" | "done" | "error";
  stage: string;
  started_at: string;
  updated_at: string;
  finished_at?: string;
  elapsed_ms?: number;
  backend?: JsonMap;
  model_params?: JsonMap;
  chat_log?: JsonMap;
  error?: string;
  steps: Array<{ stage: string; at: string; detail?: JsonMap }>;
};

type ChatBackend = {
  provider: string;
  source: "env" | "runner_registry";
  resolutionPurpose: "operator_chat";
  durableTaskDispatch: false;
  userRequest: true;
  completionsUrl: string;
  model: string;
  apiKey: string;
  runnerId?: string;
  requestedModel?: string | null;
  modelParams?: JsonMap;
  runnerLabels?: JsonMap;
  modelLabels?: JsonMap;
};

type StoredDraft = {
  draft_id: string;
  kind: string;
  session_id: string;
  run_id: string;
  created_at: string;
  expires_at: string;
  payload: JsonMap;
};

const port = Number(process.env.PORT ?? "9090");
const host = process.env.HOST ?? "0.0.0.0";
const gatewayToken = process.env.COAT_CONTROL_GATEWAY_TOKEN ?? "";

const restateIngress = trimSlash(process.env.COAT_RESTATE_INGRESS ?? "http://localhost:8080");
const restateAdminUrl = trimSlash(process.env.COAT_RESTATE_ADMIN_URL ?? "http://localhost:9070");
const goalStoreUrl = trimSlash(process.env.COAT_GOAL_STORE_URL ?? "http://localhost:9088");
const eventGatewayUrl = trimSlash(process.env.COAT_EVENT_GATEWAY_URL ?? "http://localhost:9089");
const eventGatewayToken = process.env.COAT_EVENT_GATEWAY_TOKEN ?? "";
const notifierUrl = trimSlash(process.env.COAT_NOTIFIER_URL ?? "http://localhost:9086");
const runnerRegistryUrl = trimSlash(
  process.env.COAT_RUNNER_REGISTRY_URL ?? "http://localhost:9085",
);
const memoryGatewayUrl = trimSlash(process.env.COAT_MEMORY_GATEWAY_URL ?? "http://localhost:9087");
const memoryGatewayToken = process.env.COAT_MEMORY_GATEWAY_TOKEN ?? process.env.MEMORY_GATEWAY_TOKEN ?? "";
const controlMcpToken = process.env.COAT_CONTROL_MCP_TOKEN ?? gatewayToken;
const llmGatewayProvider = nonEmptyEnv("COAT_LLM_GATEWAY_PROVIDER") ?? "openai_compatible_gateway";
const llmGatewayUrl = nonEmptyEnv("COAT_LLM_GATEWAY_URL") ?? "";
const controlChatProvider = nonEmptyEnv("COAT_CONTROL_CHAT_PROVIDER") ?? "";
const chatBackendMode = (nonEmptyEnv("COAT_CONTROL_CHAT_BACKEND") ?? "configured").toLowerCase();
const chatModel =
  nonEmptyEnv("COAT_CONTROL_CHAT_MODEL")
  ?? nonEmptyEnv("COAT_LLM_GATEWAY_CHAT_MODEL")
  ?? nonEmptyEnv("COAT_LLM_GATEWAY_DEFAULT_MODEL")
  ?? "";
const chatApiKey =
  nonEmptyEnv("COAT_CONTROL_CHAT_API_KEY")
  ?? nonEmptyEnv("COAT_LLM_GATEWAY_API_KEY")
  ?? nonEmptyEnv("OPENAI_API_KEY")
  ?? "";
const llmGatewayChatCompletionsUrl =
  nonEmptyEnv("COAT_LLM_GATEWAY_CHAT_COMPLETIONS_URL")
  ?? (llmGatewayUrl ? chatCompletionsUrlFromEndpoint(llmGatewayUrl) : undefined);
const chatCompletionsUrl =
  nonEmptyEnv("COAT_CONTROL_CHAT_COMPLETIONS_URL")
  ?? llmGatewayChatCompletionsUrl
  ?? (directOpenAiChatConfigured()
    ? "https://api.openai.com/v1/chat/completions"
    : "");
const chatLatencyClass = nonEmptyEnv("COAT_CONTROL_CHAT_LATENCY_CLASS") ?? "";
const chatSpeedTier = nonEmptyEnv("COAT_CONTROL_CHAT_SPEED_TIER") ?? "";
const chatTemperature = optionalNumberEnv("COAT_CONTROL_CHAT_TEMPERATURE") ?? 0.2;
const chatTopP = optionalNumberEnv("COAT_CONTROL_CHAT_TOP_P");
const chatMaxOutputTokens = optionalNumberEnv("COAT_CONTROL_CHAT_MAX_OUTPUT_TOKENS");
const chatReasoningEffort = nonEmptyEnv("COAT_CONTROL_CHAT_REASONING_EFFORT") ?? "";
const chatTimeoutSeconds = optionalNumberEnv("COAT_CONTROL_CHAT_TIMEOUT_SECONDS");
const chatStoreBackend = nonEmptyEnv("COAT_CONTROL_CHAT_STORE_BACKEND") ?? "goal_store";
const chatJournalPath = nonEmptyEnv("COAT_CONTROL_CHAT_JOURNAL_PATH") ?? "";
const workflowHandlers = new Set([
  "status",
  "progress",
  "tasks",
  "steer",
  "vote",
  "restart",
  "branch",
  "select_branch",
  "create_thunk",
  "resume_thunk",
  "mechanism_start",
  "mechanism_ballot",
  "compute_graph",
  "cancel",
  "approve",
  "inject_feedback",
]);
const workflowReadHandlers = new Set(["status", "progress", "tasks", "compute_graph"]);
const chatRuns = new Map<string, ChatRunTrace>();
const chatRunTtlMs = 30 * 60 * 1000;
const chatDrafts = new Map<string, StoredDraft>();
const chatDraftTtlMs = 4 * 60 * 60 * 1000;

const services: ServiceRef[] = [
  { name: "restate", baseUrl: restateAdminUrl, healthPath: "/health" },
  { name: "goal-store", baseUrl: goalStoreUrl, healthPath: "/healthz" },
  { name: "event-gateway", baseUrl: eventGatewayUrl, healthPath: "/healthz" },
  { name: "notifier", baseUrl: notifierUrl, healthPath: "/healthz" },
  { name: "runner-registry", baseUrl: runnerRegistryUrl, healthPath: "/healthz" },
  { name: "memory-gateway", baseUrl: memoryGatewayUrl, healthPath: "/healthz" },
];

const staticRootUrl = new URL("./public/", import.meta.url);

function trimSlash(value: string): string {
  return value.replace(/\/+$/, "");
}

function nonEmptyEnv(name: string): string | undefined {
  const value = process.env[name]?.trim();
  return value ? value : undefined;
}

function optionalNumberEnv(name: string): number | null {
  const raw = nonEmptyEnv(name);
  if (!raw) return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

function directOpenAiChatConfigured(): boolean {
  const provider = controlChatProvider.toLowerCase();
  if (provider !== "openai") {
    return false;
  }
  return Boolean(chatModel && (nonEmptyEnv("OPENAI_API_KEY") || nonEmptyEnv("COAT_CONTROL_CHAT_API_KEY")));
}

function jsonHeaders(extra: Record<string, string> = {}): HeadersInit {
  return {
    "content-type": "application/json",
    ...extra,
  };
}

function bearer(token: string): Record<string, string> {
  return token ? { authorization: `Bearer ${token}` } : {};
}

function sendJson(res: any, status: number, body: unknown): void {
  res.writeHead(status, jsonHeaders({ "cache-control": "no-store" }));
  res.end(JSON.stringify(body, null, 2));
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

type LogLevel = "debug" | "info" | "warn" | "error";

function log(level: LogLevel, message: string, fields: Record<string, unknown> = {}): void {
  if (!logEnabled(level)) return;
  const entry = {
    ts: new Date().toISOString(),
    level,
    service: "coat-control-plane-web",
    message,
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

function sendBytes(res: any, status: number, body: Uint8Array, contentType: string, immutable = false): void {
  res.writeHead(status, {
    "content-type": contentType,
    "cache-control": immutable ? "public, max-age=31536000, immutable" : "no-cache",
  });
  res.end(body);
}

async function serveSpa(req: any, res: any, url: URL): Promise<void> {
  if (req.method !== "GET" && req.method !== "HEAD") {
    sendJson(res, 405, { error: "method not allowed" });
    return;
  }
  const assetPath = url.pathname === "/" ? "/index.html" : url.pathname;
  const fileUrl = staticFileUrl(assetPath);
  if (fileUrl) {
    try {
      const body = await readFile(fileUrl);
      sendBytes(res, 200, body, contentTypeFor(assetPath), assetPath.startsWith("/assets/"));
      return;
    } catch {
      // Fall through to the SPA shell so client-side routes work.
    }
  }
  try {
    const shell = await readFile(new URL("index.html", staticRootUrl));
    sendBytes(res, 200, shell, "text/html; charset=utf-8");
  } catch (error) {
    sendJson(res, 500, {
      error: "control SPA assets are missing; run `npm run --prefix ui/control-plane-web build`",
      detail: error instanceof Error ? error.message : String(error),
    });
  }
}

function staticFileUrl(pathname: string): URL | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (decoded.includes("\0") || decoded.split("/").includes("..")) {
    return null;
  }
  const relative = decoded.replace(/^\/+/, "");
  if (!relative || relative.endsWith("/")) {
    return null;
  }
  return new URL(relative, staticRootUrl);
}

function contentTypeFor(pathname: string): string {
  if (pathname.endsWith(".html")) return "text/html; charset=utf-8";
  if (pathname.endsWith(".js") || pathname.endsWith(".mjs")) return "text/javascript; charset=utf-8";
  if (pathname.endsWith(".css")) return "text/css; charset=utf-8";
  if (pathname.endsWith(".svg")) return "image/svg+xml";
  if (pathname.endsWith(".png")) return "image/png";
  if (pathname.endsWith(".jpg") || pathname.endsWith(".jpeg")) return "image/jpeg";
  if (pathname.endsWith(".ico")) return "image/x-icon";
  if (pathname.endsWith(".json") || pathname.endsWith(".webmanifest")) return "application/json; charset=utf-8";
  return "application/octet-stream";
}

function isAuthorized(req: any, mcp = false): boolean {
  const expected = mcp ? controlMcpToken : gatewayToken;
  if (!expected) {
    return true;
  }
  const auth = String(req.headers.authorization ?? "");
  return auth === `Bearer ${expected}`;
}

async function readBody(req: any): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Uint8Array[] = [];
    req.on("data", (chunk: Uint8Array) => chunks.push(chunk));
    req.on("end", () => resolve(new TextDecoder().decode(concat(chunks))));
    req.on("error", reject);
  });
}

function concat(chunks: Uint8Array[]): Uint8Array {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

async function readJson(req: any): Promise<unknown> {
  const body = await readBody(req);
  if (!body.trim()) {
    return {};
  }
  return JSON.parse(body);
}

async function proxyJson(
  baseUrl: string,
  path: string,
  init: RequestInit = {},
  extraHeaders: Record<string, string> = {},
): Promise<ProxyResult> {
  const url = `${baseUrl}${path}`;
  try {
    const response = await fetch(url, {
      ...init,
      headers: {
        ...(init.headers ?? {}),
        ...extraHeaders,
      },
    });
    return {
      ok: response.ok,
      status: response.status,
      url,
      data: await parseResponse(response),
    };
  } catch (error) {
    return {
      ok: false,
      status: 0,
      url,
      data: { error: error instanceof Error ? error.message : String(error) },
    };
  }
}

async function parseResponse(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text.trim()) {
    return null;
  }
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

async function healthCheck(service: ServiceRef): Promise<ProxyResult & { name: string }> {
  const result = await proxyJson(service.baseUrl, service.healthPath, { method: "GET" });
  return { name: service.name, ...result };
}

async function backendProjection(): Promise<JsonMap> {
  const [health, runnerStatus, threads, eventSources, events, triggers] = await Promise.all([
    Promise.all(services.map(healthCheck)),
    proxyJson(runnerRegistryUrl, "/runners/status", { method: "GET" }),
    proxyJson(notifierUrl, "/threads", { method: "GET" }),
    proxyJson(eventGatewayUrl, "/event-sources", { method: "GET" }, bearer(eventGatewayToken)),
    proxyJson(eventGatewayUrl, "/events", { method: "GET" }, bearer(eventGatewayToken)),
    proxyJson(eventGatewayUrl, "/triggers", { method: "GET" }, bearer(eventGatewayToken)),
  ]);
  const [goals, agents] = await Promise.all([
    proxyJson(goalStoreUrl, "/goal-store/goals?limit=25", { method: "GET" }),
    proxyJson(goalStoreUrl, "/goal-store/tasks?limit=100", { method: "GET" }),
  ]);
  const [plans, approvals] = await Promise.all([
    proxyJson(goalStoreUrl, "/goal-store/plans?limit=25", { method: "GET" }),
    proxyJson(goalStoreUrl, "/goal-store/approvals?limit=50", { method: "GET" }),
  ]);

  return {
    generated_at: new Date().toISOString(),
    services: health,
    runner_status: normalizeRunnerStatusResult(runnerStatus),
    human_threads: threads,
    event_sources: eventSources,
    recent_events: events,
    triggers,
    goals,
    agents,
    plans,
    approvals,
  };
}

async function runnerStatus(): Promise<ProxyResult> {
  return normalizeRunnerStatusResult(await proxyJson(runnerRegistryUrl, "/runners/status", { method: "GET" }));
}

function operatorGatewayConfig(): JsonMap {
  return {
    gateway_token_required: Boolean(gatewayToken),
    endpoints: {
      restate_ingress: restateIngress,
      goal_store: goalStoreUrl,
      event_gateway: eventGatewayUrl,
      notifier: notifierUrl,
      runner_registry: runnerRegistryUrl,
      memory_gateway: memoryGatewayUrl,
      restate_admin: restateAdminUrl,
    },
    chat_backend: {
      mode: chatBackendMode,
      provider: controlChatProvider || null,
      model_configured: Boolean(chatModel),
      completions_url_configured: Boolean(chatCompletionsUrl),
      runner_registry_discovery: chatRunnerDiscoveryEnabled(),
    },
  };
}

function normalizeRunnerStatusResult(result: ProxyResult): ProxyResult {
  return {
    ...result,
    data: normalizeRunnerStatusData(result.data),
  };
}

function normalizeRunnerStatusData(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(normalizeRunnerRow);
  }
  const record = asRecord(value);
  if (Array.isArray(record.data)) {
    return {
      ...record,
      data: record.data.map(normalizeRunnerRow),
    };
  }
  return value;
}

function normalizeRunnerRow(value: unknown): JsonMap {
  const row = asRecord(value);
  const registration = Object.keys(asRecord(row.registration)).length
    ? asRecord(row.registration)
    : row;
  const labels = {
    ...asRecord(registration.labels),
    ...asRecord(row.labels),
  };
  const runnerId = stringField(row, "runner_id") || stringField(registration, "runner_id");
  const nodeId = stringField(row, "node_id") || stringField(registration, "node_id");
  const endpoint = stringField(row, "endpoint") || stringField(registration, "endpoint");
  const runtime = stringField(labels, "runtime");
  const pool = stringField(labels, "pool");
  const displayName = stringField(row, "display_name")
    || stringField(labels, "display_name")
    || stringField(labels, "name")
    || [runtime, pool].filter(Boolean).join(" / ")
    || runnerId;
  return {
    ...row,
    registration,
    runner_id: runnerId,
    node_id: nodeId,
    endpoint,
    display_name: displayName,
    runtime: runtime || null,
    pool: pool || null,
    labels,
    roles: arrayField(registration, "roles"),
    capabilities: arrayField(registration, "capabilities"),
    models: arrayField(registration, "models"),
    max_concurrency: row.max_concurrency ?? registration.max_concurrency ?? null,
    lease_ttl_seconds: row.lease_ttl_seconds ?? registration.lease_ttl_seconds ?? null,
    status: runnerState(row),
  };
}

function runnerState(row: JsonMap): string {
  if (row.stale === true) return "stale";
  if (row.full === true) return "full";
  if (row.dispatchable === false) return "unavailable";
  return "active";
}

function stringField(record: JsonMap, key: string): string {
  const value = record[key];
  return typeof value === "string" ? value.trim() : "";
}

async function composedGoalSnapshot(goalId: string): Promise<JsonMap> {
  const encodedGoalId = encodeURIComponent(goalId);
  const [goal, tasks, events, artifacts, checkpoints, approvals, workflowStatus, progress, computeGraph, humanThreads, goalChatSession] = await Promise.all([
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/tasks`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/events`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/artifacts`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/checkpoints`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/approvals`, { method: "GET" }),
    workflowReadPost(goalId, "status", {}),
    workflowReadPost(goalId, "progress", {}),
    workflowReadPost(goalId, "compute_graph", {}),
    proxyJson(notifierUrl, "/threads", { method: "GET" }),
    chatSession(`goal:${goalId}`),
  ]);
  const agentActivity = buildAgentActivity(tasks.data, progress.data, events.data, artifacts.data);
  const agentContext = buildAgentContext(goalId, agentActivity, goalChatSession, humanThreads.data);
  return {
    generated_at: new Date().toISOString(),
    goal_id: goalId,
    workflow_status: workflowStatus,
    workflow_progress: progress,
    workflow_compute_graph: computeGraph,
    goal_store_goal: goal,
    tasks,
    events,
    artifacts,
    checkpoints,
    approvals,
    human_threads: humanThreads,
    chat_session: goalChatSession,
    agent_activity: agentActivity,
    agent_context: agentContext,
  };
}

async function operatorWorkspace(
  goalId?: string | null,
  eventFilter: { eventType?: string | null; since?: string | null } = {},
): Promise<JsonMap> {
  const operatorEventParams = new URLSearchParams({ limit: "100" });
  if (goalId) operatorEventParams.set("goal_id", goalId);
  if (eventFilter.eventType) operatorEventParams.set("event_type", eventFilter.eventType);
  if (eventFilter.since) operatorEventParams.set("since", eventFilter.since);
  const operatorEventPath = `/goal-store/operator-events?${operatorEventParams.toString()}`;
  const [backendProjectionResult, goalsResult, approvalsResult, tasksResult, operatorEventsResult, runnersResult, selectedGoal] = await Promise.all([
    backendProjection(),
    proxyJson(goalStoreUrl, "/goal-store/goals?limit=100", { method: "GET" }),
    proxyJson(goalStoreUrl, "/goal-store/approvals?limit=100", { method: "GET" }),
    proxyJson(goalStoreUrl, "/goal-store/tasks?limit=200", { method: "GET" }),
    proxyJson(goalStoreUrl, operatorEventPath, { method: "GET" }),
    runnerStatus(),
    goalId ? composedGoalSnapshot(goalId) : Promise.resolve(null),
  ]);
  const goalRows = rowsFromProxyResult(goalsResult, "goals");
  const taskRows = rowsFromProxyResult(tasksResult, "tasks");
  const approvalRows = rowsFromProxyResult(approvalsResult, "approvals");
  const selectedGoalId = goalId ?? firstGoalId(goalRows);
  const selectedSnapshot = selectedGoal ?? (selectedGoalId ? await composedGoalSnapshot(selectedGoalId) : null);
  const actions = operatorActionsFromRows(approvalRows, taskRows, selectedSnapshot);
  return {
    generated_at: new Date().toISOString(),
    selected_goal_id: selectedGoalId,
    goals: goalRows.map(operatorGoalSummary),
    selected_goal: selectedSnapshot ? operatorGoalDetail(selectedSnapshot) : null,
    actions,
    events: [
      ...operatorEventsFromDurableEvents(operatorEventsResult.data),
      ...operatorEventsFromBackendProjection(backendProjectionResult),
    ],
    event_sources: backendProjectionResult.event_sources ?? null,
    human_threads: backendProjectionResult.human_threads ?? null,
    worker_runs: operatorWorkerRuns(taskRows),
    evidence: selectedSnapshot ? operatorEvidenceFromSnapshot(selectedSnapshot) : [],
    services: backendProjectionResult.services ?? [],
    runners: runnersResult,
    config: operatorGatewayConfig(),
    source: {
      restate: "durable_orchestration",
      postgres_goal_store: "operator_event_log_and_projection",
      gateway: "api_and_sse_projection",
    },
  };
}

async function operatorGoalList(search: string): Promise<JsonMap> {
  const result = await proxyJson(goalStoreUrl, `/goal-store/goals${search || "?limit=100"}`, { method: "GET" });
  return {
    generated_at: new Date().toISOString(),
    goals: rowsFromProxyResult(result, "goals").map((goal) => operatorGoalSummary(goal)),
    source: result,
  };
}

async function operatorGoalGraph(goalId: string): Promise<JsonMap> {
  const snapshot = await composedGoalSnapshot(goalId);
  const graph = asRecord(asRecord(snapshot.workflow_compute_graph).data ?? snapshot.workflow_compute_graph);
  return {
    generated_at: new Date().toISOString(),
    goal_id: goalId,
    graph,
    tasks: arrayField(asRecord(asRecord(snapshot.tasks).data ?? snapshot.tasks), "tasks"),
    actions: operatorActionsFromRows([], [], snapshot),
  };
}

async function submitOperatorGoalSpec(input: unknown, causationId = "operator_goal_submit"): Promise<JsonMap> {
  const { goalId, spec } = goalSpecWithId(input);
  const specRecord = asRecord(spec);
  const result = await workflowMutationEnvelope(goalId, "run", spec);
  const operatorEvent = await appendOperatorEvent({
    transition: "submit_goal",
    actor: goalActor(goalId),
    payload: {
      goal_id: goalId,
      title: specRecord.title ?? "Untitled goal",
      objective: specRecord.objective ?? "",
      accepted: Boolean((result as JsonMap).ok),
      result,
    },
    idempotencyKey: `operator:goal:${goalId}:submit`,
    causationId,
    correlationId: goalId,
  });
  return {
    ...result,
    goal_id: goalId,
    result,
    operator_event: operatorEvent,
    active_state: activeStateFromActionEnvelope(result as ProxyResult),
  };
}

async function operatorGoalActionEnvelope(
  goalId: string,
  action: string,
  body: unknown,
  causationId = `operator_goal_${action}`,
): Promise<JsonMap> {
  const handler = action === "select-branch" ? "select_branch" : action;
  if (!workflowHandlers.has(handler)) {
    return { ok: false, status: 400, error: `unsupported operator goal action: ${action}`, goal_id: goalId, action };
  }
  const result = await workflowMutationEnvelope(goalId, handler, body);
  const operatorEvent = await appendOperatorEvent({
    transition: transitionForGoalAction(action),
    actor: goalActor(goalId),
    payload: {
      goal_id: goalId,
      action,
      request: asRecord(body),
      accepted: Boolean((result as JsonMap).ok),
      result,
    },
    idempotencyKey: `operator:goal:${goalId}:${action}:${String(asRecord(body).request_id ?? Date.now())}`,
    causationId,
    correlationId: goalId,
  });
  return {
    ...result,
    goal_id: goalId,
    action,
    result,
    operator_event: operatorEvent,
    active_state: activeStateFromActionEnvelope(result as ProxyResult),
  };
}

type OperatorActorRef = {
  kind: "goal" | "task" | "thunk" | "worker_run" | "review" | "approval" | "event";
  id: string;
  goal_id: string | null;
  task_id: string | null;
};

function operatorEventTypeForTransition(transition: string): string {
  const eventTypes: Record<string, string> = {
    submit_goal: "goal.updated",
    draft_accepted: "goal.updated",
    task_dispatched: "task.updated",
    worker_result_received: "worker.completed",
    task_blocked: "task.updated",
    thunk_created: "thunk.created",
    thunk_resumed: "task.updated",
    approval_requested: "approval.requested",
    approval_resolved: "task.updated",
    review_completed: "review.completed",
    branch_selected: "task.updated",
    goal_steered: "goal.updated",
    goal_cancelled: "goal.cancelled",
    goal_satisfied: "goal.satisfied",
  };
  return eventTypes[transition] ?? "goal.updated";
}

function goalActor(goalId: string): OperatorActorRef {
  return {
    kind: "goal",
    id: goalId,
    goal_id: goalId,
    task_id: null,
  };
}

async function appendOperatorEvent(input: {
  transition: string;
  actor: OperatorActorRef;
  payload: JsonMap;
  idempotencyKey: string;
  causationId?: string | null;
  correlationId?: string | null;
}): Promise<JsonMap> {
  const event = {
    event_id: crypto.randomUUID(),
    event_type: operatorEventTypeForTransition(input.transition),
    actor: input.actor,
    transition: input.transition,
    idempotency_key: input.idempotencyKey,
    causation_id: input.causationId ?? null,
    correlation_id: input.correlationId ?? input.actor.goal_id ?? null,
    restate_invocation_id: null,
    created_at: new Date().toISOString(),
    payload_json: input.payload,
  };
  const result = await proxyJson(goalStoreUrl, "/goal-store/operator-events", {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify({ event }),
  });
  if (!result.ok) {
    console.warn("append operator event failed", result.status, result.data);
  }
  return {
    ok: result.ok,
    status: result.status,
    event,
    data: result.data,
  };
}

function transitionForResolvedAction(kind: string, resolution: string): string {
  if (kind === "approval" || resolution === "approve" || resolution === "reject") return "approval_resolved";
  if (kind === "thunk" || ["continue", "answer", "add_context"].includes(resolution)) return "thunk_resumed";
  if (resolution === "cancel_goal" || kind === "cancel") return "goal_cancelled";
  return "goal_steered";
}

function transitionForGoalAction(action: string): string {
  if (action === "cancel") return "goal_cancelled";
  if (action === "select-branch" || action === "select_branch") return "branch_selected";
  if (action === "approve") return "approval_resolved";
  if (action === "resume_thunk") return "thunk_resumed";
  if (action === "run") return "submit_goal";
  return "goal_steered";
}

function actorForResolvedAction(parsed: { kind: string; goalId: string; targetId: string }, record: JsonMap, goalId: string): OperatorActorRef {
  const taskId = record.task_id ? String(record.task_id) : null;
  if (parsed.kind === "approval" || record.approval_id) {
    const approvalId = String(record.approval_id ?? parsed.targetId);
    return { kind: "approval", id: approvalId, goal_id: goalId, task_id: taskId };
  }
  if (parsed.kind === "thunk" || record.thunk_id) {
    const thunkId = String(record.thunk_id ?? parsed.targetId);
    return { kind: "thunk", id: thunkId, goal_id: goalId, task_id: taskId };
  }
  if (taskId || parsed.kind === "task") {
    const id = taskId ?? parsed.targetId;
    return { kind: "task", id, goal_id: goalId, task_id: id };
  }
  return goalActor(goalId);
}

async function operatorActionList(goalId?: string | null): Promise<JsonMap> {
  const [approvalsResult, tasksResult, selectedGoal] = await Promise.all([
    proxyJson(goalStoreUrl, `/goal-store/approvals?limit=100${goalId ? `&goal_id=${encodeURIComponent(goalId)}` : ""}`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/tasks?limit=200${goalId ? `&goal_id=${encodeURIComponent(goalId)}` : ""}`, { method: "GET" }),
    goalId ? composedGoalSnapshot(goalId) : Promise.resolve(null),
  ]);
  return {
    generated_at: new Date().toISOString(),
    actions: operatorActionsFromRows(
      rowsFromProxyResult(approvalsResult, "approvals"),
      rowsFromProxyResult(tasksResult, "tasks"),
      selectedGoal,
    ),
  };
}

function rowsFromProxyResult(result: ProxyResult, key: string): JsonMap[] {
  const data = result.data;
  if (Array.isArray(data)) {
    return data.map(asRecord);
  }
  const record = asRecord(data);
  const direct = arrayField(record, key);
  if (direct.length) {
    return direct.map(asRecord);
  }
  const nestedData = record.data;
  if (Array.isArray(nestedData)) {
    return nestedData.map(asRecord);
  }
  const nestedRows = arrayField(asRecord(nestedData), key);
  return nestedRows.map(asRecord);
}

async function resolveOperatorAction(actionId: string, body: unknown): Promise<JsonMap> {
  const record = asRecord(body);
  const parsed = parseOperatorActionId(actionId);
  const goalId = String(record.goal_id ?? parsed.goalId ?? "");
  if (!goalId) {
    return { ok: false, error: "goal_id is required to resolve an operator action", action_id: actionId };
  }
  const resolution = String(record.resolution ?? record.intent ?? parsed.kind ?? "continue").trim().toLowerCase();
  const responseSummary = String(record.response_summary ?? record.answer ?? record.context ?? "Operator resolved the action.").trim();
  let result: JsonMap;
  if (parsed.kind === "approval" || resolution === "approve" || resolution === "reject") {
    const approvalId = String(record.approval_id ?? parsed.targetId ?? "");
    if (!approvalId) {
      return { ok: false, error: "approval_id is required for approval resolution", action_id: actionId, goal_id: goalId };
    }
    result = await workflowMutationEnvelope(goalId, "approve", {
      approval_id: approvalId,
      approved: resolution !== "reject",
      note: responseSummary,
    });
  } else if (parsed.kind === "thunk" || resolution === "continue" || resolution === "answer" || resolution === "add_context") {
    const thunkId = String(record.thunk_id ?? parsed.targetId ?? "");
    if (!thunkId) {
      return { ok: false, error: "thunk_id is required for continuation resume", action_id: actionId, goal_id: goalId };
    }
    result = await workflowMutationEnvelope(goalId, "resume_thunk", {
      thunk_id: thunkId,
      responder: String(record.operator ?? "operator"),
      response_summary: responseSummary,
      artifact_refs: Array.isArray(record.artifact_refs) ? record.artifact_refs : [],
    });
  } else if (resolution === "cancel_goal" || parsed.kind === "cancel") {
    result = await workflowMutationEnvelope(goalId, "cancel", responseSummary || "Operator cancelled the goal.");
  } else if (resolution === "replan") {
    result = await workflowMutationEnvelope(goalId, "steer", {
      id: crypto.randomUUID(),
      goal_id: goalId,
      task_id: record.task_id ?? parsed.targetId ?? null,
      operator: record.operator ?? "operator",
      message: responseSummary || "Operator requested replan.",
      kind: {
        request_replan: {
          reason: responseSummary || "Operator requested replan.",
        },
      },
    });
  } else {
    result = await workflowMutationEnvelope(goalId, "restart", {
      goal_id: goalId,
      scope: record.task_id || parsed.targetId ? "task" : "goal",
      reason: "operator_requested",
      message: responseSummary || "Operator requested restart.",
      task_id: record.task_id ?? parsed.targetId ?? null,
      reset_attempts: true,
      preserve_artifacts: true,
      operator: record.operator ?? "operator",
    });
  }
  const operatorEvent = await appendOperatorEvent({
    transition: transitionForResolvedAction(parsed.kind, resolution),
    actor: actorForResolvedAction(parsed, record, goalId),
    payload: {
      action_id: actionId,
      goal_id: goalId,
      resolution,
      request: record,
      result,
    },
    idempotencyKey: `operator:action:${actionId}:${resolution}:${String(record.request_id ?? record.idempotency_key ?? Date.now())}`,
    causationId: actionId,
    correlationId: goalId,
  });
  return {
    action_id: actionId,
    goal_id: goalId,
    resolution,
    result,
    operator_event: operatorEvent,
    active_state: activeStateFromActionEnvelope(result),
  };
}

function parseOperatorActionId(actionId: string): { kind: string; goalId: string; targetId: string } {
  const [kind = "", goalId = "", targetId = ""] = actionId.split(":");
  return { kind, goalId, targetId };
}

function firstGoalId(rows: JsonMap[]): string {
  return String(rows[0]?.goal_id ?? rows[0]?.id ?? "").trim();
}

function operatorGoalSummary(row: JsonMap): JsonMap {
  const goalId = String(row.goal_id ?? row.id ?? "");
  return {
    goal_id: goalId,
    id: goalId,
    title: row.title ?? "Untitled goal",
    objective: row.objective ?? "",
    status: row.status ?? "unknown",
    percent_done: row.percent_done ?? 0,
    open_tasks: row.open_tasks ?? 0,
    blocked_tasks: row.blocked_tasks ?? 0,
    failed_tasks: row.failed_tasks ?? 0,
    satisfied: row.satisfied === true,
    updated_at: row.updated_at ?? row.updated_at_text ?? null,
  };
}

function operatorGoalDetail(snapshot: JsonMap): JsonMap {
  const goal = asRecord(asRecord(snapshot.goal_store_goal).data ?? snapshot.goal_store_goal);
  const goalRecord = asRecord(goal.goal);
  const summary = operatorGoalSummary(goalRecord);
  return {
    summary,
    progress: asRecord(asRecord(snapshot.workflow_progress).data ?? snapshot.workflow_progress),
    graph: asRecord(asRecord(snapshot.workflow_compute_graph).data ?? snapshot.workflow_compute_graph),
    tasks: arrayField(asRecord(asRecord(snapshot.tasks).data ?? snapshot.tasks), "tasks"),
    actions: operatorActionsFromRows([], [], snapshot),
    evidence: operatorEvidenceFromSnapshot(snapshot),
    snapshot,
  };
}

function operatorActionsFromRows(approvals: JsonMap[], tasks: JsonMap[], snapshot: JsonMap | null): JsonMap[] {
  const actions: JsonMap[] = [];
  for (const approval of approvals) {
    const status = String(approval.status ?? "");
    if (status && status !== "pending") continue;
    const goalId = String(approval.goal_id ?? "");
    const approvalId = String(approval.approval_id ?? approval.id ?? "");
    if (!goalId || !approvalId) continue;
    actions.push({
      action_id: `approval:${goalId}:${approvalId}`,
      kind: "resolve_approval",
      goal_id: goalId,
      task_id: approval.task_id ?? null,
      title: "Approval required",
      question: approval.requested_action ?? approval.reason ?? "Approve or reject this request.",
      status: "pending",
      allowed_resolutions: ["approve", "reject", "add_context", "cancel_goal"],
      approval,
      payload_json: approval,
    });
  }

  const statusPayload = asRecord(asRecord(snapshot ?? {}).workflow_status);
  const progressPayload = asRecord(asRecord(snapshot ?? {}).workflow_progress);
  const statusData = asRecord(statusPayload.data ?? statusPayload);
  const progressData = asRecord(progressPayload.data ?? progressPayload);
  const thunks = arrayField(statusData, "delayed_compute_thunks")
    .concat(arrayField(progressData, "delayed_compute_thunks"));
  const seenThunkIds = new Set<string>();
  for (const thunkValue of thunks) {
    const thunk = asRecord(thunkValue);
    const status = String(thunk.status ?? "");
    const thunkId = String(thunk.id ?? thunk.thunk_id ?? "");
    const goalId = String(thunk.goal_id ?? asRecord(snapshot ?? {}).goal_id ?? "");
    if (!thunkId || !goalId || seenThunkIds.has(thunkId) || (status && status !== "pending")) continue;
    seenThunkIds.add(thunkId);
    actions.push({
      action_id: `thunk:${goalId}:${thunkId}`,
      kind: "resume_thunk",
      goal_id: goalId,
      task_id: thunk.task_id ?? null,
      title: "Input needed",
      question: thunk.requested_input ?? thunk.reason ?? "Continue this delayed work.",
      status: status || "pending",
      allowed_resolutions: ["continue", "answer", "add_context", "replan", "cancel_goal"],
      thunk,
      payload_json: thunk,
    });
  }

  const tasksPayload = asRecord(asRecord(snapshot ?? {}).tasks);
  const taskRows = tasks.length ? tasks : arrayField(asRecord(tasksPayload.data ?? tasksPayload), "tasks");
  for (const taskValue of taskRows) {
    const task = asRecord(taskValue);
    const status = String(task.status ?? "");
    if (!["blocked", "failed", "waiting_input", "waiting-input"].includes(status)) continue;
    const goalId = String(task.goal_id ?? asRecord(snapshot ?? {}).goal_id ?? "");
    const taskId = String(task.task_id ?? task.id ?? "");
    if (!goalId || !taskId) continue;
    actions.push({
      action_id: `task:${goalId}:${taskId}`,
      kind: "restart_task",
      goal_id: goalId,
      task_id: taskId,
      title: status.includes("waiting") ? "Task waiting" : "Recover task",
      question: task.title ?? task.current_prompt ?? "Restart, replan, or cancel this work.",
      status,
      allowed_resolutions: ["retry", "replan", "add_context", "cancel_goal"],
      payload_json: task,
    });
  }
  return actions;
}

function operatorEventsFromBackendProjection(value: JsonMap): JsonMap[] {
  return arrayField(asRecord(asRecord(value.recent_events).data ?? value.recent_events), "events")
    .map(asRecord)
    .map((event) => ({
      event_id: String(event.event_id ?? event.id ?? event.sequence ?? crypto.randomUUID()),
      event_type: String(event.kind ?? event.event_type ?? "event"),
      goal_id: event.goal_id ?? null,
      task_id: event.task_id ?? null,
      title: String(event.message ?? event.kind ?? "Event"),
      detail: String(event.actor ?? event.source ?? ""),
      created_at: event.created_at ?? null,
      payload_json: event,
    }));
}

function operatorEventsFromDurableEvents(value: unknown): JsonMap[] {
  return arrayField(asRecord(value), "events")
    .map(asRecord)
    .map((event) => {
      const actor = asRecord(event.actor);
      return {
        event_id: String(event.event_id ?? crypto.randomUUID()),
        event_type: String(event.event_type ?? "event"),
        goal_id: actor.goal_id ?? null,
        task_id: actor.task_id ?? null,
        title: titleForOperatorEvent(String(event.event_type ?? "event"), String(event.transition ?? "")),
        detail: detailForOperatorEvent(event),
        created_at: event.created_at ?? null,
        payload_json: event,
      };
    });
}

function titleForOperatorEvent(eventType: string, transition: string): string {
  if (transition === "submit_goal") return "Goal submitted";
  if (transition === "draft_accepted") return "Draft accepted";
  if (transition === "thunk_resumed") return "Input provided";
  if (transition === "approval_resolved") return "Approval resolved";
  if (transition === "goal_cancelled") return "Goal cancelled";
  if (transition === "goal_satisfied") return "Goal satisfied";
  if (transition === "review_completed") return "Review completed";
  if (eventType === "worker.completed") return "Worker completed";
  return transition.replaceAll("_", " ") || eventType;
}

function detailForOperatorEvent(event: JsonMap): string {
  const payload = asRecord(event.payload_json);
  const request = asRecord(payload.request);
  return String(payload.summary ?? payload.message ?? request.response_summary ?? request.answer ?? "");
}

function operatorEvidenceFromSnapshot(snapshot: JsonMap): JsonMap[] {
  return arrayField(asRecord(asRecord(snapshot.artifacts).data ?? snapshot.artifacts), "artifacts")
    .map(asRecord)
    .map((artifact) => {
      const ref = asRecord(artifact.artifact);
      const checkpoint = asRecord(artifact.checkpoint);
      return {
        evidence_id: String(ref.uri ?? checkpoint.id ?? crypto.randomUUID()),
        goal_id: artifact.goal_id ?? snapshot.goal_id ?? null,
        task_id: artifact.task_id ?? null,
        title: String(ref.description ?? checkpoint.label ?? "Evidence"),
        uri: ref.uri ?? null,
        checkpoint: Object.keys(checkpoint).length ? checkpoint : null,
        created_at: artifact.created_at ?? null,
        payload_json: artifact,
      };
    });
}

function operatorWorkerRuns(tasks: JsonMap[]): JsonMap[] {
  return tasks.map((task) => ({
    run_id: String(task.run_id ?? task.task_id ?? task.id ?? crypto.randomUUID()),
    goal_id: task.goal_id ?? null,
    task_id: task.task_id ?? task.id ?? null,
    worker: task.role ?? "planner",
    status: task.status ?? "unknown",
    summary: task.title ?? task.current_prompt ?? "",
    started_at: task.started_at ?? null,
    finished_at: task.finished_at ?? null,
    payload_json: task,
  }));
}

function activeStateFromActionEnvelope(result: unknown): JsonMap | null {
  const record = asRecord(result);
  const data = asRecord(record.data);
  const active = asRecord(record.active_state ?? data.active_state);
  return Object.keys(active).length ? active : null;
}

async function streamOperatorState(req: any, res: any, url: URL): Promise<void> {
  res.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-store",
    connection: "keep-alive",
    "x-accel-buffering": "no",
  });
  let closed = false;
  req.on("close", () => {
    closed = true;
  });
  const goalId = url.searchParams.get("goal_id");
  const eventTypeFilter = url.searchParams.get("event_type");
  const eventSince = url.searchParams.get("event_since");
  const started = Date.now();
  let sequence = Number(req.headers["last-event-id"] ?? url.searchParams.get("since") ?? 0) || 0;
  let lastPayload = "";
  while (!closed && Date.now() - started < 5 * 60 * 1000) {
    try {
      const workspace = await operatorWorkspace(goalId, {
        eventType: eventTypeFilter,
        since: eventSince,
      });
      const payload = JSON.stringify(workspace);
      if (payload !== lastPayload) {
        const eventName = operatorStreamEventName(workspace, eventTypeFilter);
        res.write(`event: ${eventName}\n`);
        res.write(`id: ${sequence}\n`);
        res.write(`retry: 1500\n`);
        res.write(`data: ${payload}\n\n`);
        lastPayload = payload;
      } else {
        res.write(`event: stream.heartbeat\n`);
        res.write(`id: ${sequence}\n`);
        res.write(`data: ${JSON.stringify({ goal_id: goalId, generated_at: new Date().toISOString() })}\n\n`);
      }
    } catch (error) {
      res.write(`event: stream.error\n`);
      res.write(`id: ${sequence}\n`);
      res.write(`data: ${JSON.stringify({ error: errorMessage(error), goal_id: goalId })}\n\n`);
    }
    sequence += 1;
    await sleep(1_500);
  }
  if (!closed) {
    res.write(`event: stream.done\n`);
    res.write(`id: ${sequence}\n`);
    res.write(`data: ${JSON.stringify({ reason: "stream_ttl_elapsed", goal_id: goalId })}\n\n`);
    res.end();
  }
}

function operatorStreamEventName(workspace: JsonMap, explicitEventType?: string | null): string {
  if (explicitEventType) {
    return explicitEventType;
  }
  const actions = arrayField(workspace, "actions").map(asRecord);
  if (actions.some((action) => String(action.kind ?? "") === "resume_thunk")) {
    return "action.required";
  }
  if (actions.some((action) => String(action.kind ?? "") === "resolve_approval")) {
    return "approval.requested";
  }
  if (actions.length > 0) {
    return "action.required";
  }

  const selectedGoal = asRecord(workspace.selected_goal);
  const summary = asRecord(selectedGoal.summary);
  const status = String(summary.status ?? "");
  if (summary.satisfied === true || status === "done" || status === "satisfied") {
    return "goal.satisfied";
  }
  if (status === "cancelled") {
    return "goal.cancelled";
  }

  const workerRuns = arrayField(workspace, "worker_runs").map(asRecord);
  if (workerRuns.some((run) => ["running", "runnable", "waiting_input", "waiting-approval"].includes(String(run.status ?? "")))) {
    return "task.updated";
  }
  if (workerRuns.some((run) => ["done", "failed", "blocked"].includes(String(run.status ?? "")))) {
    return "worker.completed";
  }
  return goalIdFromWorkspace(workspace) ? "goal.updated" : "workspace.updated";
}

function goalIdFromWorkspace(workspace: JsonMap): string {
  return String(workspace.selected_goal_id ?? asRecord(asRecord(workspace.selected_goal).summary).goal_id ?? "").trim();
}

async function goalAgentContext(goalId: string, taskId: string | null): Promise<JsonMap> {
  const snapshot = await composedGoalSnapshot(goalId);
  const context = asRecord(snapshot.agent_context);
  if (!taskId) {
    return context;
  }
  const tasks = arrayField(context, "tasks")
    .map(asRecord)
    .filter((task) => String(task.task_id ?? "") === taskId);
  return {
    ...context,
    task_id: taskId,
    tasks,
    found: tasks.length > 0,
  };
}

async function planContinuity(planId: string): Promise<JsonMap> {
  const planResponse = await proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}`, { method: "GET" });
  return buildPlanContinuity(planId, planResponse);
}

function buildPlanContinuity(planId: string, planResponse: ProxyResult): JsonMap {
  const data = asRecord(planResponse.data);
  const plan = asRecord(data.plan);
  if (!plan || Object.keys(plan).length === 0) {
    return {
      generated_at: new Date().toISOString(),
      plan_id: planId,
      found: false,
      source: planResponse,
    };
  }
  const current = asRecord(plan.current);
  const authoring = asRecord(current.authoring);
  const goalPlan = asRecord(current.plan);
  const questions = arrayField(current, "questions").map(asRecord);
  const decisions = arrayField(current, "decisions").map(asRecord);
  const subgoals = arrayField(goalPlan, "subgoals").map(asRecord);
  const initialTasks = arrayField(current, "initial_tasks").map(asRecord);
  const revisions = arrayField(plan, "revisions").map(asRecord);
  const openQuestions = questions.filter((question) => String(question.status ?? "") === "open");
  const requiredOpenQuestions = openQuestions.filter((question) => question.required === true);
  const answeredQuestions = questions.filter((question) => String(question.status ?? "") === "answered");
  const deferredQuestions = questions.filter((question) => String(question.status ?? "") === "deferred");
  const closedQuestionText = new Set(
    questions
      .filter((question) => ["answered", "deferred"].includes(String(question.status ?? "")))
      .map((question) => String(question.question ?? "")),
  );
  const authoringOpenQuestions = arrayField(authoring, "open_questions")
    .filter((question) => !closedQuestionText.has(String(question)));
  const distributionNotes = arrayField(goalPlan, "distribution_notes");
  const compiledQuality = asRecord(plan.compiled_quality);
  const qualityNextActions = arrayField(compiledQuality, "suggested_next_actions");
  const status = String(plan.status ?? "");

  return {
    generated_at: new Date().toISOString(),
    plan_id: String(plan.id ?? planId),
    title: plan.title ?? "",
    objective: plan.objective ?? "",
    repo: plan.repo ?? null,
    status,
    mode: plan.mode ?? "",
    version: plan.version ?? current.version ?? null,
    updated_at: plan.updated_at ?? null,
    compiled_goal_id: plan.compiled_goal_id ?? null,
    continuity: {
      intake_summary: authoring.intake_summary ?? "",
      plan_summary: goalPlan.summary ?? "",
      acceptance_evidence: arrayField(authoring, "acceptance_evidence"),
      constraints: arrayField(authoring, "constraints"),
      assumptions: arrayField(authoring, "assumptions"),
      out_of_scope: arrayField(authoring, "out_of_scope"),
      authoring_open_questions: authoringOpenQuestions,
      open_questions: openQuestions,
      required_open_questions: requiredOpenQuestions,
      answered_questions: answeredQuestions,
      deferred_questions: deferredQuestions,
      decisions,
      subgoals,
      distribution_notes: distributionNotes,
      initial_tasks: initialTasks,
      revisions: revisions.map((revision) => summarizePlanRevision(revision)),
      next_actions: planNextActions({
        status,
        requiredOpenQuestionCount: requiredOpenQuestions.length,
        openQuestionCount: openQuestions.length,
        authoringOpenQuestionCount: authoringOpenQuestions.length,
        decisionCount: decisions.length,
        subgoalCount: subgoals.length,
        initialTaskCount: initialTasks.length,
        acceptanceEvidenceCount: arrayField(authoring, "acceptance_evidence").length,
        compiledGoalId: plan.compiled_goal_id,
        qualityNextActions,
      }),
      compiled_quality: plan.compiled_quality ?? null,
    },
    counts: {
      open_questions: openQuestions.length,
      required_open_questions: requiredOpenQuestions.length,
      answered_questions: answeredQuestions.length,
      deferred_questions: deferredQuestions.length,
      authoring_open_questions: authoringOpenQuestions.length,
      decisions: decisions.length,
      subgoals: subgoals.length,
      initial_tasks: initialTasks.length,
      revisions: revisions.length,
    },
    source_status: {
      ok: planResponse.ok,
      status: planResponse.status,
      url: planResponse.url,
    },
  };
}

function summarizePlanRevision(revision: JsonMap): JsonMap {
  const plan = asRecord(revision.plan);
  const questions = arrayField(revision, "questions").map(asRecord);
  return {
    id: revision.id ?? null,
    version: revision.version ?? null,
    author: revision.author ?? "",
    summary: revision.summary ?? "",
    operator_message: revision.operator_message ?? null,
    created_at: revision.created_at ?? null,
    subgoal_count: arrayField(plan, "subgoals").length,
    initial_task_count: arrayField(revision, "initial_tasks").length,
    open_question_count: questions.filter((question) => String(question.status ?? "") === "open").length,
    decision_count: arrayField(revision, "decisions").length,
  };
}

function planNextActions(input: {
  status: string;
  requiredOpenQuestionCount: number;
  openQuestionCount: number;
  authoringOpenQuestionCount: number;
  decisionCount: number;
  subgoalCount: number;
  initialTaskCount: number;
  acceptanceEvidenceCount: number;
  compiledGoalId: unknown;
  qualityNextActions: unknown[];
}): string[] {
  const actions: string[] = [];
  if (input.requiredOpenQuestionCount > 0) {
    actions.push("answer required open planning questions before compiling");
  } else if (input.openQuestionCount > 0 || input.authoringOpenQuestionCount > 0) {
    actions.push("answer, defer, or explicitly accept remaining planning questions");
  }
  if (input.acceptanceEvidenceCount === 0) {
    actions.push("add acceptance evidence before execution");
  }
  if (input.subgoalCount === 0) {
    actions.push("define stable subgoals with IDs before distributing work");
  }
  if (input.initialTaskCount === 0 && input.subgoalCount > 0) {
    actions.push("seed the first coordinator-owned initial tasks from stable subgoals");
  }
  if (input.decisionCount === 0) {
    actions.push("record important planning decisions and rationale");
  }
  if (input.status === "compiled" && input.compiledGoalId) {
    actions.push("lint or submit the compiled GoalSpec when the operator is ready");
  } else if (["ready_for_review", "approved"].includes(input.status) && input.requiredOpenQuestionCount === 0) {
    actions.push("compile the durable plan into a GoalSpec without submitting it");
  }
  for (const action of input.qualityNextActions) {
    if (typeof action === "string" && action && !actions.includes(action)) {
      actions.push(action);
    }
  }
  if (actions.length === 0) {
    actions.push("review the plan or compile it into a GoalSpec");
  }
  return actions;
}

function buildAgentActivity(
  tasksResponse: unknown,
  progressResponse: unknown,
  eventsResponse: unknown,
  artifactsResponse: unknown,
): unknown[] {
  const tasks = extractArray(tasksResponse, ["tasks"]);
  const nextTasks = extractArray(progressResponse, ["next_tasks"]);
  const progressById = new Map(nextTasks.map((item) => [String((item as JsonMap).task_id ?? ""), item as JsonMap]));
  const events = extractArray(eventsResponse, ["events"]);
  const artifacts = extractArray(artifactsResponse, ["artifacts"]);

  return tasks.map((item) => {
    const task = item as JsonMap;
    const payload = (task.payload_json && typeof task.payload_json === "object" ? task.payload_json : {}) as JsonMap;
    const taskId = String(task.task_id ?? payload.id ?? "");
    const progress = progressById.get(taskId);
    return {
      goal_id: task.goal_id ?? payload.goal_id ?? null,
      task_id: taskId,
      parent_task_id: task.parent_task_id ?? payload.parent_id ?? null,
      subgoal_id: task.subgoal_id ?? payload.subgoal_id ?? null,
      title: task.title ?? payload.title ?? "",
      color: task.color ?? payload.color ?? null,
      role: task.role ?? payload.role ?? "",
      purpose: task.purpose_kind ?? task.purpose ?? payload.purpose ?? null,
      status: task.status ?? payload.status ?? "",
      depth: task.depth ?? payload.depth ?? 0,
      priority: task.priority ?? payload.priority ?? null,
      attempts: task.attempts ?? payload.attempts ?? 0,
      runnable: task.runnable ?? progress?.runnable ?? false,
      prompt: payload.prompt ?? null,
      current_prompt: payload.prompt ?? null,
      persona: projectedPersona(payload),
      model: projectedModel(payload),
      runner: projectedRunner(payload),
      execution: payload.execution ?? null,
      budget: payload.budget ?? null,
      sandbox: payload.sandbox ?? null,
      done_criteria: payload.done_criteria ?? null,
      dependencies: payload.dependencies ?? null,
      children: payload.children ?? null,
      result: payload.result ?? task.result_uri ?? null,
      progress,
      recent_events: events.filter((event) => String((event as JsonMap).task_id ?? "") === taskId).slice(-8),
      artifacts: artifacts.filter((artifact) => String((artifact as JsonMap).task_id ?? "") === taskId),
      raw_task: task,
    };
  });
}

function buildAgentContext(
  goalId: string,
  agentActivity: unknown[],
  chatSessionResponse: JsonMap,
  humanThreadsResponse: unknown,
): JsonMap {
  const notificationThreads = relevantNotificationThreads(goalId, humanThreadsResponse);
  const entries = arrayField(chatSessionResponse, "entries").map(asRecord);
  const turns = entries.length ? entries : arrayField(chatSessionResponse, "messages").map(asRecord);
  return {
    generated_at: new Date().toISOString(),
    goal_id: goalId,
    source: {
      tasks: "goal-store task projection payload_json",
      chat_session: `goal:${goalId}`,
      notification_threads: "notifier thread projection",
      durable_state: "not owned by control gateway",
    },
    chat_session: {
      session_id: String(chatSessionResponse.session_id ?? `goal:${goalId}`),
      durable: Boolean(chatSessionResponse.durable),
      chat_log: asRecord(chatSessionResponse.chat_log),
      message_count: arrayField(chatSessionResponse, "messages").length,
      latest_turns: turns.slice(-8).map((turn) => ({
        role: turn.role ?? "",
        content: turn.content ?? "",
        created_at: turn.created_at ?? null,
        provider: turn.provider ?? null,
        model: turn.model ?? null,
      })),
    },
    notification_threads: notificationThreads,
    tasks: agentActivity.map((item) => agentContextTask(goalId, asRecord(item), notificationThreads)),
  };
}

function agentContextTask(goalId: string, activity: JsonMap, notificationThreads: JsonMap[]): JsonMap {
  const taskId = String(activity.task_id ?? "");
  const execution = asRecord(activity.execution);
  const rawTask = asRecord(activity.raw_task);
  const payload = asRecord(rawTask.payload_json);
  const result = asRecord(activity.result);
  const taskThreads = notificationThreads.filter((thread) => {
    const threadTaskId = String(thread.task_id ?? "");
    const threadKey = String(thread.thread_key ?? "");
    return threadTaskId === taskId || Boolean(taskId && threadKey.includes(taskId));
  });
  const goalThreads = notificationThreads.filter((thread) => !thread.task_id || String(thread.task_id) === "");

  return {
    goal_id: activity.goal_id ?? goalId,
    task_id: taskId,
    parent_task_id: activity.parent_task_id ?? null,
    subgoal_id: activity.subgoal_id ?? null,
    title: activity.title ?? "",
    role: activity.role ?? "",
    purpose: activity.purpose ?? null,
    status: activity.status ?? "",
    current_prompt: activity.current_prompt ?? activity.prompt ?? null,
    prompt: activity.prompt ?? null,
    persona: activity.persona ?? projectedPersona(payload),
    model: activity.model ?? projectedModel(payload),
    runner: activity.runner ?? projectedRunner(payload),
    runner_id: result.runner_id ?? rawTask.runner_id ?? null,
    execution_profile: execution,
    task_purpose: payload.purpose ?? activity.purpose ?? null,
    session_refs: sessionRefs(goalId, taskId, payload, result),
    thread_refs: threadRefs(taskId, payload, result, taskThreads, goalThreads),
    chat_session_ref: `goal:${goalId}`,
    notification_threads: taskThreads.length ? taskThreads : goalThreads,
    artifacts: activity.artifacts ?? [],
    recent_events: activity.recent_events ?? [],
    source: {
      task_projection: "goal-store",
      notifications: "notifier",
      chat: "goal-store chat session or configured gateway fallback",
    },
  };
}

function projectedPersona(payload: JsonMap): unknown {
  const execution = asRecord(payload.execution);
  return execution.persona ?? payload.persona ?? null;
}

function projectedModel(payload: JsonMap): unknown {
  const execution = asRecord(payload.execution);
  return execution.model ?? execution.model_route ?? payload.model ?? null;
}

function projectedRunner(payload: JsonMap): unknown {
  const execution = asRecord(payload.execution);
  return execution.runner ?? execution.runner_ref ?? execution.runner_id ?? payload.runner ?? null;
}

function sessionRefs(goalId: string, taskId: string, payload: JsonMap, result: JsonMap): JsonMap {
  return compactRecord({
    goal_chat_session: `goal:${goalId}`,
    task_session_id: payload.session_id ?? result.session_id ?? null,
    runner_session_id: result.runner_session_id ?? null,
    thread_id: payload.thread_id ?? result.thread_id ?? null,
    task_id: taskId || null,
  });
}

function threadRefs(taskId: string, payload: JsonMap, result: JsonMap, taskThreads: JsonMap[], goalThreads: JsonMap[]): JsonMap {
  return compactRecord({
    task_id: taskId || null,
    payload_thread_id: payload.thread_id ?? null,
    result_thread_id: result.thread_id ?? null,
    notification_thread_keys: (taskThreads.length ? taskThreads : goalThreads).map((thread) => thread.thread_key).filter(Boolean),
  });
}

function relevantNotificationThreads(goalId: string, response: unknown): JsonMap[] {
  return extractFlexibleArray(response, ["threads", "data"]).map(asRecord).filter((thread) => {
    const threadGoalId = String(thread.goal_id ?? "");
    const threadKey = String(thread.thread_key ?? "");
    return threadGoalId === goalId || threadKey.includes(goalId) || !threadGoalId;
  });
}

function extractFlexibleArray(value: unknown, keys: string[]): unknown[] {
  if (Array.isArray(value)) {
    return value;
  }
  const record = asRecord(value);
  for (const key of keys) {
    const items = arrayField(record, key);
    if (items.length) {
      return items;
    }
  }
  return [];
}

function compactRecord(record: JsonMap): JsonMap {
  return Object.fromEntries(
    Object.entries(record).filter(([, value]) => {
      if (value === null || value === undefined || value === "") {
        return false;
      }
      return !(Array.isArray(value) && value.length === 0);
    }),
  );
}

function extractArray(value: unknown, path: string[]): unknown[] {
  let current = value;
  for (const key of path) {
    if (!current || typeof current !== "object") {
      return [];
    }
    current = (current as JsonMap)[key];
  }
  return Array.isArray(current) ? current : [];
}

function asRecord(value: unknown): JsonMap {
  return value && typeof value === "object" && !Array.isArray(value) ? value as JsonMap : {};
}

function arrayField(record: JsonMap, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
}

function rowsFromData(value: unknown): JsonMap[] {
  if (Array.isArray(value)) {
    return value.map(asRecord).filter((item) => Object.keys(item).length > 0);
  }
  const record = asRecord(value);
  for (const key of ["plans", "items", "records", "rows", "data"]) {
    const candidate = record[key];
    if (candidate !== value) {
      const rows = rowsFromData(candidate);
      if (rows.length) {
        return rows;
      }
    }
  }
  return [];
}

async function workflowPost(goalId: string, handler: string, body: unknown): Promise<ProxyResult> {
  const path = `/GoalWorkflow/${encodeURIComponent(goalId)}/${handler}`;
  return proxyJson(
    restateIngress,
    path,
    {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(body ?? {}),
    },
  );
}

async function workflowMutationEnvelope(goalId: string, handler: string, body: unknown): Promise<JsonMap> {
  const startedAt = Date.now();
  const result = await workflowPost(goalId, handler, body);
  const activeState = await readActiveGoalState(goalId, result.ok ? 4 : 1);
  return {
    ok: result.ok,
    status: result.status,
    url: result.url,
    data: result.data,
    action: {
      goal_id: goalId,
      handler,
      accepted: result.ok,
      submitted_at: new Date(startedAt).toISOString(),
    },
    active_state: activeState.snapshot,
    active_state_status: activeState.status,
    active_state_available: activeState.status === "fresh",
    active_state_unavailable_reads: activeState.unavailableReads,
    observability: {
      active_state_attempts: activeState.attempts,
      active_state_after_ms: Date.now() - startedAt,
      active_state_status: activeState.status,
      active_state_available: activeState.status === "fresh",
      active_state_unavailable_reads: activeState.unavailableReads,
      active_state_error: activeState.error ?? null,
      stream_url: `/api/operator/stream?goal_id=${encodeURIComponent(goalId)}`,
    },
  };
}

async function readActiveGoalState(goalId: string, maxAttempts: number): Promise<ActiveStateRead> {
  let lastError = "";
  let lastSnapshot: JsonMap | null = null;
  let lastStatus: ActiveStateStatus = "unavailable";
  let lastUnavailableReads: string[] = [];
  const attempts = Math.max(1, maxAttempts);
  for (let index = 0; index < attempts; index += 1) {
    try {
      const snapshot = await composedGoalSnapshot(goalId);
      const activeState = activeStateHealth(snapshot);
      if (activeState.status === "fresh") {
        return {
          snapshot,
          attempts: index + 1,
          status: "fresh",
          unavailableReads: [],
        };
      }
      lastSnapshot = snapshot;
      lastStatus = activeState.status;
      lastUnavailableReads = activeState.unavailableReads;
      lastError = activeState.error ?? "";
    } catch (error) {
      lastError = errorMessage(error);
    }
    if (index < attempts - 1) {
      await sleep(200);
    }
  }
  return {
    snapshot: lastSnapshot,
    attempts,
    status: lastSnapshot ? lastStatus : "unavailable",
    unavailableReads: lastUnavailableReads,
    error: lastError || "active state unavailable",
  };
}

function activeStateHealth(snapshot: JsonMap): { status: ActiveStateStatus; unavailableReads: string[]; error?: string } {
  const unavailableReads: string[] = [];
  const readFields: Array<[string, string]> = [
    ["workflow_status", "status"],
    ["workflow_progress", "progress"],
    ["workflow_compute_graph", "compute_graph"],
  ];
  for (const [field, handler] of readFields) {
    const read = asRecord(snapshot[field]);
    const data = read.data;
    const dataRecord = asRecord(data);
    if (
      data === null
      || data === undefined
      || dataRecord.unavailable === true
      || dataRecord.stale === true
      || (read.ok === false && read.status !== 404)
      || read.status === 0
    ) {
      unavailableReads.push(handler);
    }
  }
  if (!unavailableReads.length) {
    return { status: "fresh", unavailableReads };
  }
  return {
    status: "stale",
    unavailableReads,
    error: `Restate active-state reads are unavailable or stale: ${unavailableReads.join(", ")}`,
  };
}

async function workflowReadPost(goalId: string, handler: string, body: unknown): Promise<ProxyResult> {
  if (handler === "tasks") {
    return normalizeWorkflowReadResult(await workflowPost(goalId, handler, body), handler);
  }
  const path = `/GoalWorkflow/${encodeURIComponent(goalId)}/${handler}`;
  return normalizeWorkflowReadResult(await proxyJson(restateIngress, path, { method: "POST" }), handler);
}

function workflowMutationHttpStatus(result: ProxyResult): number {
  if (result.status === 0) {
    return 503;
  }
  if (result.status >= 400 && result.status <= 599) {
    return result.status;
  }
  return 200;
}

async function applyResearchOutput(goalId: string, payload: unknown): Promise<JsonMap> {
  if (!goalId) {
    throw new Error("goal_id is required");
  }
  const body = asRecord(payload);
  const operator = String(body.operator ?? "control-gateway");
  const researchOutput = asRecord(body.research_output ?? body.research ?? body);
  const usePlan = asRecord(researchOutput.use_plan ?? body.use_plan ?? body);
  const directives = researchUsePlanDirectives(goalId, operator, usePlan);
  if (!directives.length) {
    throw new Error("research output did not contain facts_to_use, proposed_goal_updates, or proposed_task_updates");
  }
  const results: unknown[] = [];
  for (const directive of directives) {
    results.push(await workflowPost(goalId, "steer", directive));
  }
  return {
    goal_id: goalId,
    applied_directives: directives,
    responses: results,
  };
}

function researchUsePlanDirectives(goalId: string, operator: string, usePlan: JsonMap): SteeringDirective[] {
  const directives: SteeringDirective[] = [];
  const factsToUse = arrayField(usePlan, "facts_to_use").map((value) => String(value)).filter(Boolean);
  if (factsToUse.length) {
    directives.push(steeringDirective(goalId, operator, "Apply sourced research facts to future work.", {
      kind: "add_constraint",
      constraint: `Use these sourced research facts unless superseded by newer evidence: ${factsToUse.join(" ")}`,
    }));
  }
  for (const update of arrayField(usePlan, "proposed_goal_updates")) {
    const record = asRecord(update);
    const recommendation = String(record.recommendation ?? "");
    if (!recommendation) {
      continue;
    }
    const target = String(record.target ?? "goal");
    const path = String(record.path ?? "");
    const reason = String(record.reason ?? "research recommendation");
    directives.push(steeringDirective(goalId, operator, `Apply research recommendation for ${target}.`, {
      kind: "add_constraint",
      constraint: `Research recommendation${path ? ` at ${path}` : ""}: ${recommendation} Reason: ${reason}`,
    }));
  }
  for (const task of arrayField(usePlan, "proposed_task_updates")) {
    const record = asRecord(task);
    const prompt = String(record.prompt ?? "");
    if (!prompt) {
      continue;
    }
    directives.push(steeringDirective(goalId, operator, String(record.title ?? "Inject research follow-up task."), {
      kind: "inject_task",
      role: String(record.role ?? "research"),
      prompt,
      reason: String(record.reason ?? "research use plan proposed a follow-up task"),
    }));
  }
  return directives;
}

function steeringDirective(goalId: string, operator: string, message: string, kind: JsonMap): SteeringDirective {
  return {
    id: crypto.randomUUID(),
    goal_id: goalId,
    task_id: null,
    operator,
    message,
    kind,
  };
}

function normalizeWorkflowReadResult(result: ProxyResult, handler: string): ProxyResult {
  if (result.status === 200 && (result.data === null || result.data === undefined)) {
    return {
      ...result,
      ok: false,
      data: {
        unavailable: true,
        stale: true,
        handler,
        reason:
          "Restate returned HTTP 200 with a null workflow read body. Treating this active-state read as unavailable/stale until a non-null coordinator projection is available.",
        restate_response: result.data,
      },
    };
  }
  if (result.status !== 404) {
    return result;
  }
  return {
    ...result,
    ok: false,
    data: {
      unavailable: true,
      stale: true,
      handler,
      reason:
        "Restate returned 404 for this workflow read. The workflow may not be started yet, or the coordinator deployment may still be registering.",
      restate_response: result.data,
    },
  };
}

async function controlChat(payload: unknown): Promise<JsonMap> {
  const request = asRecord(payload);
  const mode = String(request.mode ?? "general");
  const goalId = String(request.goal_id ?? "");
  const sessionId = String(request.session_id ?? (goalId ? `goal:${goalId}` : "operator:default"));
  const runId = String(request.run_id ?? crypto.randomUUID());
  const prompt = String(request.prompt ?? "").trim();
  const messages = await chatMessagesForRequest(sessionId, request, prompt);
  if (!messages.length) {
    throw new Error("chat request requires a prompt or at least one message");
  }
  beginChatRun(runId, sessionId, goalId || null, mode);
  try {
    updateChatRun(runId, "loading_goal_context", goalId ? { goal_id: goalId } : { scope: "operator" });
    const context = await chatContext(goalId);
    updateChatRun(runId, "resolving_backend");
    const backend = await resolveChatBackend();
    const latestUserMessage = [...messages].reverse().find((message) => message.role === "user") ?? null;
    let response: JsonMap;
    if (backend) {
      updateChatRun(runId, "calling_model", { backend: publicChatBackend(backend) });
      response = await callChatModel(mode, messages, context, backend);
    } else {
      updateChatRun(runId, "using_stub", { reason: stubChatReason() });
      response = stubChat(mode, messages, context);
    }
    response = compactChatDraftResponse(response, sessionId, runId);
    updateChatRun(runId, "journaling_turns", {
      provider: response.provider ?? null,
      model: response.model ?? null,
    });
    const chatLog = await appendChatTurn(sessionId, goalId || null, mode, latestUserMessage, response);
    const result = {
      ...response,
      session_id: sessionId,
      run_id: runId,
      chat_log: chatLog,
    };
    const trace = finishChatRun(runId, result, null);
    return {
      ...result,
      chat_run: compactChatRunTrace(trace),
    };
  } catch (error) {
    finishChatRun(runId, {}, error);
    throw error;
  }
}

async function chatMessagesForRequest(sessionId: string, request: JsonMap, prompt: string): Promise<ChatMessage[]> {
  if (prompt) {
    const { entries } = await readChatSessionEntries(sessionId);
    const history = entries
      .slice(-8)
      .map((entry) => ({ role: entry.role, content: entry.content }))
      .filter((entry) => entry.content);
    return [...history, { role: "user", content: prompt }];
  }
  return chatMessagesFrom(request.messages);
}

function beginChatRun(runId: string, sessionId: string, goalId: string | null, mode: string): ChatRunTrace {
  cleanupChatRuns();
  const now = new Date().toISOString();
  const trace: ChatRunTrace = {
    run_id: runId,
    session_id: sessionId,
    goal_id: goalId,
    mode,
    status: "running",
    stage: "received",
    started_at: now,
    updated_at: now,
    steps: [{ stage: "received", at: now }],
  };
  chatRuns.set(runId, trace);
  return trace;
}

function updateChatRun(runId: string, stage: string, detail?: JsonMap): ChatRunTrace | null {
  const trace = chatRuns.get(runId);
  if (!trace) {
    return null;
  }
  const now = new Date().toISOString();
  trace.stage = stage;
  trace.updated_at = now;
  trace.steps.push({ stage, at: now, ...(detail ? { detail } : {}) });
  if (detail?.backend && typeof detail.backend === "object" && !Array.isArray(detail.backend)) {
    trace.backend = detail.backend as JsonMap;
  }
  return trace;
}

function finishChatRun(runId: string, response: JsonMap, error: unknown): ChatRunTrace | null {
  const trace = chatRuns.get(runId);
  if (!trace) {
    return null;
  }
  const now = new Date().toISOString();
  trace.status = error ? "error" : "done";
  trace.stage = error ? "error" : "done";
  trace.updated_at = now;
  trace.finished_at = now;
  trace.elapsed_ms = Date.parse(now) - Date.parse(trace.started_at);
  if (error) {
    trace.error = error instanceof Error ? error.message : String(error);
  }
  trace.backend = asRecord(response.chat_backend) || trace.backend;
  trace.model_params = asRecord(response.model_params);
  trace.chat_log = asRecord(response.chat_log);
  trace.steps.push({
    stage: trace.stage,
    at: now,
    detail: error ? { error: trace.error ?? "unknown error" } : { provider: response.provider ?? null, model: response.model ?? null },
  });
  return trace;
}

function chatRunSnapshot(runId: string): JsonMap {
  cleanupChatRuns();
  const trace = chatRuns.get(runId);
  if (!trace) {
    return {
      run_id: runId,
      found: false,
      status: "missing",
      reason: "chat run trace was not found or has expired",
    };
  }
  return {
    found: true,
    ...trace,
  };
}

function compactChatRunTrace(trace: ChatRunTrace | null): JsonMap | null {
  if (!trace) {
    return null;
  }
  return {
    run_id: trace.run_id,
    status: trace.status,
    stage: trace.stage,
    started_at: trace.started_at,
    updated_at: trace.updated_at,
    finished_at: trace.finished_at ?? null,
    elapsed_ms: trace.elapsed_ms ?? null,
  };
}

function cleanupChatRuns(): void {
  const now = Date.now();
  for (const [runId, trace] of chatRuns) {
    if (now - Date.parse(trace.updated_at) > chatRunTtlMs) {
      chatRuns.delete(runId);
    }
  }
}

function compactChatDraftResponse(response: JsonMap, sessionId: string, runId: string): JsonMap {
  cleanupChatDrafts();
  const drafts = asRecord(response.drafts);
  const compactDrafts: JsonMap = {};
  const draftRefs: JsonMap = {};
  const draftSummary: JsonMap = {};
  for (const [kind, value] of Object.entries(drafts)) {
    const draft = asRecord(value);
    if (kind === "goal_spec" && Object.keys(draft).length > 0) {
      const stored = storeChatDraft(kind, sessionId, runId, draft);
      const compact = compactGoalSpecDraft(draft, stored.draft_id);
      compactDrafts[kind] = compact;
      draftRefs[kind] = {
        draft_id: stored.draft_id,
        kind,
        expires_at: stored.expires_at,
      };
      draftSummary[kind] = goalDraftSummary(compact);
      continue;
    }
    compactDrafts[kind] = compactGenericDraft(value, kind);
    draftSummary[kind] = genericDraftSummary(value, kind);
  }
  return compactRecord({
    provider: response.provider ?? null,
    model: response.model ?? null,
    mode: response.mode ?? null,
    assistant: String(response.assistant ?? ""),
    drafts: compactDrafts,
    draft_refs: draftRefs,
    draft_summary: draftSummary,
    chat_backend: asRecord(response.chat_backend),
    model_params: asRecord(response.model_params),
  });
}

function storeChatDraft(kind: string, sessionId: string, runId: string, payload: JsonMap): StoredDraft {
  const now = Date.now();
  const stored: StoredDraft = {
    draft_id: crypto.randomUUID(),
    kind,
    session_id: sessionId,
    run_id: runId,
    created_at: new Date(now).toISOString(),
    expires_at: new Date(now + chatDraftTtlMs).toISOString(),
    payload: JSON.parse(JSON.stringify(payload)) as JsonMap,
  };
  chatDrafts.set(stored.draft_id, stored);
  return stored;
}

function cleanupChatDrafts(): void {
  const now = Date.now();
  for (const [draftId, draft] of chatDrafts) {
    if (Date.parse(draft.expires_at) <= now) {
      chatDrafts.delete(draftId);
    }
  }
}

function compactGoalSpecDraft(draft: JsonMap, draftId: string): JsonMap {
  const authoring = asRecord(draft.authoring);
  const plan = asRecord(draft.plan);
  const subgoals = rowsFromData(plan.subgoals).map((subgoal, index) => compactRecord({
    id: String(subgoal.id ?? subgoal.subgoal_id ?? `subgoal-${index + 1}`),
    title: String(subgoal.title ?? `Subgoal ${index + 1}`),
    objective: String(subgoal.objective ?? subgoal.summary ?? subgoal.title ?? ""),
    owner_role: subgoal.owner_role ?? subgoal.owner ?? subgoal.role ?? null,
    tags: arrayField(subgoal, "tags"),
    acceptance_evidence: arrayField(subgoal, "acceptance_evidence"),
    color: subgoal.color ?? null,
  }));
  const initialTasks = rowsFromData(draft.initial_tasks).slice(0, 8).map((task, index) => compactRecord({
    role: task.role ?? "planner",
    title: String(task.title ?? `Initial task ${index + 1}`),
    subgoal_id: task.subgoal_id ?? null,
    reason: task.reason ?? null,
    prompt: truncateText(String(task.prompt ?? ""), 500),
    tags: arrayField(task, "tags"),
    color: task.color ?? null,
  }));
  return compactRecord({
    draft_id: draftId,
    kind: "goal_spec",
    compact: true,
    title: String(draft.title ?? "Untitled goal draft"),
    objective: String(draft.objective ?? ""),
    repo: draft.repo ?? null,
    authoring: compactRecord({
      intake_summary: authoring.intake_summary ?? draft.objective ?? null,
      acceptance_evidence: arrayField(authoring, "acceptance_evidence"),
      constraints: arrayField(authoring, "constraints"),
      out_of_scope: arrayField(authoring, "out_of_scope"),
      assumptions: arrayField(authoring, "assumptions"),
      open_questions: arrayField(authoring, "open_questions"),
    }),
    plan: compactRecord({
      summary: plan.summary ?? null,
      subgoals,
      distribution_notes: arrayField(plan, "distribution_notes"),
    }),
    initial_tasks: initialTasks,
    done_criteria: asRecord(draft.done_criteria),
  });
}

function compactGenericDraft(value: unknown, kind: string): JsonMap {
  const record = asRecord(value);
  if (!Object.keys(record).length) {
    return compactRecord({
      kind,
      summary: truncateText(String(value ?? ""), 220),
    });
  }
  return compactRecord({
    kind,
    title: record.title ?? record.name ?? null,
    summary: truncateText(String(record.summary ?? record.objective ?? record.prompt ?? record.query ?? record.message ?? ""), 300),
    query: record.query ?? null,
    goal_id: record.goal_id ?? null,
    action: record.action ?? null,
    draft_id: record.draft_id ?? null,
  });
}

function goalDraftSummary(draft: JsonMap): JsonMap {
  const plan = asRecord(draft.plan);
  return compactRecord({
    draft_id: draft.draft_id ?? null,
    title: draft.title ?? null,
    objective_preview: truncateText(String(draft.objective ?? ""), 220),
    subgoal_count: rowsFromData(plan.subgoals).length,
    initial_task_count: rowsFromData(draft.initial_tasks).length,
  });
}

function genericDraftSummary(value: unknown, kind: string): JsonMap {
  const record = asRecord(value);
  return compactRecord({
    kind,
    title: record.title ?? record.name ?? null,
    preview: truncateText(String(record.summary ?? record.objective ?? record.prompt ?? record.query ?? record.message ?? value ?? ""), 220),
  });
}

function truncateText(value: string, maxLength: number): string {
  const cleaned = value.replace(/\s+/g, " ").trim();
  return cleaned.length <= maxLength ? cleaned : `${cleaned.slice(0, Math.max(0, maxLength - 3))}...`;
}

function chatMessagesFrom(value: unknown): ChatMessage[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .map((item) => asRecord(item))
    .map((item) => ({
      role: chatRole(String(item.role ?? "user")),
      content: String(item.content ?? "").trim(),
    }))
    .filter((item) => item.content);
}

function chatRole(value: string): ChatMessage["role"] {
  return value === "assistant" || value === "system" ? value : "user";
}

async function chatSession(sessionId: string, includeEntries = false): Promise<JsonMap> {
  const { entries, chatLog } = await readChatSessionEntries(sessionId);
  return compactRecord({
    session_id: sessionId,
    durable: Boolean(chatLog.durable),
    chat_log: chatLog,
    messages: entries.map((entry) => ({
      role: entry.role,
      content: entry.content,
      created_at: entry.created_at,
    })),
    entries: includeEntries ? entries : undefined,
  });
}

async function appendChatTurn(
  sessionId: string,
  goalId: string | null,
  mode: string,
  userMessage: ChatMessage | null,
  response: JsonMap,
): Promise<JsonMap> {
  const createdAt = new Date().toISOString();
  const entries: ChatSessionEntry[] = [];
  if (userMessage?.content) {
    entries.push({
      session_id: sessionId,
      goal_id: goalId,
      mode,
      role: "user",
      content: userMessage.content,
      created_at: createdAt,
      payload_json: { source: "control_gateway" },
    });
  }
  const assistant = String(response.assistant ?? "").trim();
  if (assistant) {
    entries.push({
      session_id: sessionId,
      goal_id: goalId,
      mode,
      role: "assistant",
      content: assistant,
      created_at: new Date().toISOString(),
      provider: typeof response.provider === "string" ? response.provider : undefined,
      model: typeof response.model === "string" ? response.model : null,
      payload_json: chatAssistantTurnPayload(response),
    });
  }
  if (!entries.length) {
    return chatLogStatus("none", false);
  }

  if (chatStoreBackend !== "jsonl" && chatStoreBackend !== "disabled") {
    const goalStoreResult = await appendChatEntriesToGoalStore(entries);
    if (goalStoreResult.durable) {
      return goalStoreResult;
    }
    if (!chatJournalPath) {
      return goalStoreResult;
    }
    const jsonlResult = await appendChatEntriesToJsonl(entries);
    return {
      ...jsonlResult,
      fallback_from: "goal_store",
      goal_store_error: goalStoreResult.error ?? null,
    };
  }

  if (chatStoreBackend === "disabled" || !chatJournalPath) {
    return chatLogStatus("none", false);
  }
  return appendChatEntriesToJsonl(entries);
}

function chatAssistantTurnPayload(response: JsonMap): JsonMap {
  return {
    source: "control_gateway",
    draft_refs: asRecord(response.draft_refs),
    draft_summary: asRecord(response.draft_summary),
    chat_backend: asRecord(response.chat_backend),
    model_params: asRecord(response.model_params),
    mode: response.mode ?? null,
  };
}

async function appendChatEntriesToGoalStore(entries: ChatSessionEntry[]): Promise<JsonMap> {
  for (const entry of entries) {
    const response = await proxyJson(
      goalStoreUrl,
      "/goal-store/chat/turns",
      {
        method: "POST",
        headers: jsonHeaders(),
        body: JSON.stringify(goalStoreChatTurnBody(entry)),
      },
    );
    if (!response.ok) {
      return chatLogStatus("goal_store", false, `goal-store chat append failed with ${response.status}`);
    }
  }
  return chatLogStatus("goal_store", true);
}

function goalStoreChatTurnBody(entry: ChatSessionEntry): JsonMap {
  return {
    session_id: entry.session_id,
    goal_id: uuidOrUndefined(entry.goal_id),
    mode: entry.mode,
    role: entry.role,
    content: entry.content,
    created_at: entry.created_at,
    provider: entry.provider,
    model: entry.model,
    payload_json: entry.payload_json ?? {},
  };
}

function uuidOrUndefined(value: string | null): string | undefined {
  if (!value) return undefined;
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)
    ? value
    : undefined;
}

async function appendChatEntriesToJsonl(entries: ChatSessionEntry[]): Promise<JsonMap> {
  await mkdir(dirname(chatJournalPath), { recursive: true });
  await appendFile(chatJournalPath, entries.map((entry) => JSON.stringify(entry)).join("\n") + "\n");
  return {
    ...chatLogStatus("jsonl", true),
    journal_path: "configured",
  };
}

async function readChatSessionEntries(sessionId: string): Promise<{ entries: ChatSessionEntry[]; chatLog: JsonMap }> {
  if (chatStoreBackend !== "jsonl" && chatStoreBackend !== "disabled") {
    const goalStoreResult = await readChatSessionEntriesFromGoalStore(sessionId);
    if (goalStoreResult.entries.length || goalStoreResult.chatLog.durable || !chatJournalPath) {
      return goalStoreResult;
    }
    const jsonlResult = await readChatSessionEntriesFromJsonl(sessionId);
    return {
      entries: jsonlResult.entries,
      chatLog: {
        ...jsonlResult.chatLog,
        fallback_from: "goal_store",
        goal_store_error: goalStoreResult.chatLog.error ?? null,
      },
    };
  }
  if (chatStoreBackend === "disabled" || !chatJournalPath) {
    return { entries: [], chatLog: chatLogStatus("none", false) };
  }
  return readChatSessionEntriesFromJsonl(sessionId);
}

async function readChatSessionEntriesFromGoalStore(sessionId: string): Promise<{ entries: ChatSessionEntry[]; chatLog: JsonMap }> {
  const response = await proxyJson(
    goalStoreUrl,
    `/goal-store/chat/sessions/${encodeURIComponent(sessionId)}`,
    { method: "GET" },
  );
  if (!response.ok) {
    return {
      entries: [],
      chatLog: chatLogStatus("goal_store", false, `goal-store chat session read failed with ${response.status}`),
    };
  }
  const body = asRecord(response.data);
  return {
    entries: arrayField(body, "turns").map(chatEntryFromGoalStoreTurn).filter((entry) => entry.content),
    chatLog: chatLogStatus("goal_store", true),
  };
}

function chatEntryFromGoalStoreTurn(value: unknown): ChatSessionEntry {
  const record = asRecord(value);
  const role = record.role === "assistant" ? "assistant" : "user";
  return {
    session_id: String(record.session_id ?? ""),
    goal_id: typeof record.goal_id === "string" ? record.goal_id : null,
    mode: String(record.mode ?? "general"),
    role,
    content: String(record.content ?? "").trim(),
    created_at: String(record.created_at ?? ""),
    provider: typeof record.provider === "string" ? record.provider : undefined,
    model: typeof record.model === "string" ? record.model : null,
    payload_json: asRecord(record.payload_json),
  };
}

async function readChatSessionEntriesFromJsonl(sessionId: string): Promise<{ entries: ChatSessionEntry[]; chatLog: JsonMap }> {
  let text = "";
  try {
    text = await readFile(chatJournalPath, "utf8");
  } catch (error) {
    if (asRecord(error).code === "ENOENT") {
      return {
        entries: [],
        chatLog: {
          ...chatLogStatus("jsonl", true),
          journal_path: "configured",
        },
      };
    }
    throw error;
  }
  const entries: ChatSessionEntry[] = [];
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) {
      continue;
    }
    const parsed = safeJsonValue(line);
    const record = asRecord(parsed);
    if (record.session_id !== sessionId) {
      continue;
    }
    const role = record.role === "assistant" ? "assistant" : record.role === "user" ? "user" : null;
    const content = String(record.content ?? "").trim();
    if (!role || !content) {
      continue;
    }
    entries.push({
      session_id: String(record.session_id),
      goal_id: typeof record.goal_id === "string" ? record.goal_id : null,
      mode: String(record.mode ?? "general"),
      role,
      content,
      created_at: String(record.created_at ?? ""),
      provider: typeof record.provider === "string" ? record.provider : undefined,
      model: typeof record.model === "string" ? record.model : null,
      payload_json: asRecord(record.payload_json),
    });
  }
  return {
    entries,
    chatLog: {
      ...chatLogStatus("jsonl", true),
      journal_path: "configured",
    },
  };
}

function chatLogStatus(backend: string, durable: boolean, error?: string): JsonMap {
  return {
    durable,
    backend,
    store_backend: chatStoreBackend,
    error: error ?? null,
  };
}

async function chatContext(goalId: string): Promise<JsonMap> {
  const context: JsonMap = {
    goal_id: goalId || null,
    engine_boundary: "The chat assistant drafts and explains. Durable mutations still require explicit workflow, plan, memory, or approval API calls.",
    available_actions: [
      "draft_plan",
      "draft_goal",
      "draft_steering_directive",
      "explain_goal_state",
      "summarize_next_actions",
    ],
  };
  if (!goalId) {
    return context;
  }
  try {
    context.goal = compactGoalContext(await composedGoalSnapshot(goalId));
  } catch (error) {
    context.goal_error = error instanceof Error ? error.message : String(error);
  }
  return context;
}

function compactGoalContext(snapshot: JsonMap): JsonMap {
  const progress = asRecord(asRecord(snapshot.workflow_progress).data ?? snapshot.workflow_progress);
  const status = asRecord(asRecord(snapshot.workflow_status).data ?? snapshot.workflow_status);
  const computeGraph = asRecord(asRecord(snapshot.workflow_compute_graph).data ?? snapshot.workflow_compute_graph);
  const tasks = rowsFromData(snapshot.agent_activity).slice(0, 12).map((task) => compactRecord({
    task_id: task.task_id ?? task.id ?? null,
    title: task.title ?? null,
    status: task.status ?? null,
    role: task.role ?? null,
    purpose: task.purpose ?? task.purpose_kind ?? null,
    subgoal_id: task.subgoal_id ?? null,
  }));
  return compactRecord({
    goal_id: snapshot.goal_id ?? progress.goal_id ?? status.goal_id ?? null,
    status: status.status ?? progress.status ?? null,
    progress: compactRecord({
      total_tasks: progress.total_tasks ?? null,
      runnable_tasks: progress.runnable_tasks ?? null,
      completed_tasks: progress.completed_tasks ?? null,
      blocked_tasks: progress.blocked_tasks ?? null,
      failed_tasks: progress.failed_tasks ?? null,
      waiting_tasks: progress.waiting_tasks ?? null,
      open_thunks: computeGraph.open_thunks ?? null,
    }),
    next_tasks: tasks,
  });
}

async function resolveChatBackend(): Promise<ChatBackend | null> {
  if (chatBackendMode === "stub") {
    return null;
  }
  if (chatCompletionsUrl && chatModel) {
    return {
      provider: configuredChatProvider(chatCompletionsUrl),
      source: "env",
      resolutionPurpose: "operator_chat",
      durableTaskDispatch: false,
      userRequest: true,
      completionsUrl: chatCompletionsUrl,
      model: chatModel,
      apiKey: chatApiKey,
      requestedModel: chatModel,
    };
  }
  if (chatCompletionsUrl || chatApiKey || chatModel) {
    return null;
  }
  if (!chatRunnerDiscoveryEnabled()) {
    return null;
  }
  return discoverChatBackendFromRunners();
}

function configuredChatProvider(completionsUrl: string): string {
  if (controlChatProvider) {
    return controlChatProvider;
  }
  if (llmGatewayChatCompletionsUrl && completionsUrl === llmGatewayChatCompletionsUrl) {
    return `llm_gateway:${llmGatewayProvider}`;
  }
  return completionsUrl.includes("api.openai.com") ? "openai" : "openai_compatible";
}

function chatRunnerDiscoveryEnabled(): boolean {
  const override = nonEmptyEnv("COAT_CONTROL_CHAT_RUNNER_DISCOVERY");
  if (override) {
    if (labelTruthy(override.toLowerCase())) return true;
    if (labelFalsey(override.toLowerCase())) return false;
  }
  return chatBackendMode === "runner_registry" || chatBackendMode === "auto";
}

async function discoverChatBackendFromRunners(): Promise<ChatBackend | null> {
  const result = await proxyJson(runnerRegistryUrl, "/runners/status", { method: "GET" });
  const statuses = Array.isArray(result.data) ? result.data : [];
  const candidates: ChatBackend[] = [];
  for (const status of statuses) {
    const statusRecord = asRecord(status);
    if (statusRecord.dispatchable === false || statusRecord.stale === true) {
      continue;
    }
    const registration = asRecord(statusRecord.registration);
    const runnerId = String(registration.runner_id ?? "");
    const runnerLabels = {
      ...asRecord(statusRecord.labels),
      ...asRecord(registration.labels),
    };
    for (const modelRecord of arrayField(registration, "models").map(asRecord)) {
      const endpoint = String(modelRecord.endpoint ?? "").trim();
      const model = String(modelRecord.model ?? "").trim();
      const modelLabels = asRecord(modelRecord.labels);
      if (!endpoint || !model || !/^https?:\/\//i.test(endpoint)) {
        continue;
      }
      if (!modelAllowsOperatorChat(modelLabels, runnerLabels)) {
        continue;
      }
      candidates.push({
        provider: String(modelRecord.provider ?? "openai_compatible"),
        source: "runner_registry",
        resolutionPurpose: "operator_chat",
        durableTaskDispatch: false,
        userRequest: true,
        completionsUrl: chatCompletionsUrlFromEndpoint(endpoint),
        model,
        apiKey: "",
        runnerId,
        requestedModel: chatModel || null,
        modelParams: asRecord(modelRecord.params),
        runnerLabels,
        modelLabels,
      });
    }
  }
  if (!candidates.length) {
    return null;
  }
  const exact = chatModel
    ? candidates.find((candidate) => candidate.model === chatModel)
    : null;
  return exact ?? candidates[0];
}

function modelAllowsOperatorChat(modelLabels: JsonMap, runnerLabels: JsonMap): boolean {
  if (chatRoutingLabelRejects(runnerLabels) || chatRoutingLabelRejects(modelLabels)) {
    return false;
  }
  return chatRoutingLabelAllows(modelLabels) || chatRoutingLabelAllows(runnerLabels);
}

function chatRoutingLabelAllows(labels: JsonMap): boolean {
  if (labelTruthy(labelValue(labels, "control_chat"))) return true;
  if (labelTruthy(labelValue(labels, "chat.enabled"))) return true;
  const intent = labelValue(labels, "chat.intent");
  if (intent === "user_request" || intent === "operator_chat" || intent === "control_chat") return true;
  const scope = labelValue(labels, "routing_scope");
  if (scope === "operator_chat" || scope === "control_chat") return true;
  const purpose = labelValue(labels, "purpose");
  return purpose === "chat" || purpose === "operator_chat" || purpose === "control_chat";
}

function chatRoutingLabelRejects(labels: JsonMap): boolean {
  if (labelFalsey(labelValue(labels, "control_chat"))) return true;
  if (labelFalsey(labelValue(labels, "chat.enabled"))) return true;
  const intent = labelValue(labels, "chat.intent");
  if (intent === "durable_task" || intent === "task_dispatch" || intent === "agent_task") return true;
  const scope = labelValue(labels, "routing_scope");
  return scope === "durable_task" || scope === "task_dispatch" || scope === "agent_task" || scope === "work";
}

function labelValue(labels: JsonMap, key: string): string {
  const value = labels[key];
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value.trim().toLowerCase();
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") return String(value).trim().toLowerCase();
  return JSON.stringify(value).trim().toLowerCase();
}

function labelTruthy(value: string): boolean {
  return ["1", "true", "yes", "on", "enabled"].includes(value);
}

function labelFalsey(value: string): boolean {
  return ["0", "false", "no", "off", "disabled"].includes(value);
}

function chatCompletionsUrlFromEndpoint(endpoint: string): string {
  const base = trimSlash(endpoint.trim());
  if (base.endsWith("/chat/completions")) {
    return base;
  }
  if (base.endsWith("/v1")) {
    return `${base}/chat/completions`;
  }
  return `${base}/v1/chat/completions`;
}

function publicChatBackend(backend: ChatBackend): JsonMap {
  return {
    provider: backend.provider,
    source: backend.source,
    resolution_purpose: backend.resolutionPurpose,
    durable_task_dispatch: backend.durableTaskDispatch,
    user_request: backend.userRequest,
    completions_url: backend.completionsUrl,
    model: backend.model,
    runner_id: backend.runnerId ?? null,
    requested_model: backend.requestedModel ?? null,
    runner_labels: backend.runnerLabels ?? null,
    model_labels: backend.modelLabels ?? null,
  };
}

async function callChatModel(
  mode: string,
  messages: ChatMessage[],
  context: JsonMap,
  backend: ChatBackend,
): Promise<JsonMap> {
  const body: JsonMap = {
    model: backend.model,
    temperature: chatTemperature,
    messages: [
      {
        role: "system",
        content: controlChatSystemPrompt(mode, context),
      },
      ...messages,
    ],
  };
  if (chatTopP !== null) body.top_p = chatTopP;
  if (chatMaxOutputTokens !== null) body.max_tokens = chatMaxOutputTokens;
  if (supportsHostedChatTuning(backend)) {
    if (chatReasoningEffort) body.reasoning_effort = chatReasoningEffort;
    if (chatSpeedTier) body.service_tier = chatSpeedTier;
  }

  const controller = chatTimeoutSeconds && chatTimeoutSeconds > 0 ? new AbortController() : null;
  const timeoutHandle = controller
    ? setTimeout(() => controller.abort(), chatTimeoutSeconds! * 1000)
    : null;
  try {
    const response = await fetch(backend.completionsUrl, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(backend.apiKey ? { authorization: `Bearer ${backend.apiKey}` } : {}),
      },
      signal: controller?.signal,
      body: JSON.stringify(body),
    });
    const text = await response.text();
    const data = text ? safeJsonValue(text) : {};
    if (!response.ok) {
      throw new Error(`chat model request failed with ${response.status}: ${text}`);
    }
    const content = String(atRecord(data, ["choices", "0", "message", "content"]) ?? "");
    const parsed = parseChatAssistantPayload(content);
    return {
      provider: backend.provider,
      model: backend.model,
      chat_backend: {
        source: backend.source,
        backend_mode: chatBackendMode,
        resolution_purpose: backend.resolutionPurpose,
        durable_task_dispatch: backend.durableTaskDispatch,
        user_request: backend.userRequest,
        runner_id: backend.runnerId ?? null,
        requested_model: backend.requestedModel ?? null,
        completions_url: backend.completionsUrl,
        runner_labels: backend.runnerLabels ?? null,
        model_labels: backend.modelLabels ?? null,
      },
      model_params: {
        latency_class: chatLatencyClass || null,
        speed_tier: chatSpeedTier || null,
        temperature: chatTemperature,
        top_p: chatTopP,
        max_output_tokens: chatMaxOutputTokens,
        reasoning_effort: chatReasoningEffort || null,
        timeout_seconds: chatTimeoutSeconds,
        runner_params: backend.modelParams ?? null,
      },
      mode,
      assistant: parsed.assistant,
      drafts: parsed.drafts,
      raw_model_response: parsed.raw_model_response,
      context,
    };
  } finally {
    if (timeoutHandle) clearTimeout(timeoutHandle);
  }
}

function supportsHostedChatTuning(backend: ChatBackend): boolean {
  return backend.provider === "openai" || backend.completionsUrl.includes("api.openai.com");
}

function controlChatSystemPrompt(mode: string, context: JsonMap): string {
  return [
    "<coat_chat_assistant>",
    "  <role>You are the COAT control-plane chat assistant.</role>",
    "  <mission>Help the operator author goals, durable plans, steering directives, memory notes, and review requests.</mission>",
    "  <authority>",
    "    <rule>This request is operator chat assistance for a user request. You MUST NOT treat it as a durable task run or claim runner task dispatch.</rule>",
    "    <rule>You MUST NOT claim that durable state changed unless the caller provides a successful backend result.</rule>",
    "    <rule>You MUST treat all mutations as requiring explicit backend forms, API calls, or MCP tools.</rule>",
    "    <rule>You MUST treat any subagent request as a COAT durable child-task request, not native model delegation.</rule>",
    "  </authority>",
    "  <output_contract>",
    "    <rule>You MUST return one JSON object.</rule>",
    "    <rule>The JSON object MUST have keys: assistant string, drafts object.</rule>",
    "    <rule>Draft payloads MUST be valid JSON under drafts.</rule>",
    "    <rule>Assistant prose MUST be concise and operational.</rule>",
    "  </output_contract>",
    "  <drafting_rules>",
    "    <rule>Goals MUST include objective, evidence, constraints, budget, done criteria, execution, memory, research, approval, and stop conditions when known.</rule>",
    "    <rule>Steering drafts MUST be explicit about goal_id, task_id when known, operator intent, directive kind, and approval risk.</rule>",
    "    <rule>Memory drafts MUST preserve provenance and MUST NOT write unreviewed branch conclusions as durable facts.</rule>",
    "    <rule>Search drafts MUST return a structured search_request and, when live web/reference research is needed, a coordinator-owned research steering directive. You MUST NOT claim that memory, web, or reference search already ran unless a tool result is present.</rule>",
    "  </drafting_rules>",
    `  <requested_mode>${escapeXml(mode)}</requested_mode>`,
    `  <context_json>${escapeXml(JSON.stringify(context).slice(0, 12_000))}</context_json>`,
    "</coat_chat_assistant>",
  ].join("\n");
}

function escapeXml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function parseChatAssistantPayload(content: string): JsonMap {
  const tagged = extractTaggedAssistantPayload(content);
  if (tagged) {
    return {
      ...tagged,
      raw_model_response: content,
    };
  }
  const parsed = extractJsonObject(content);
  if (parsed && ("assistant" in parsed || "drafts" in parsed)) {
    return {
      assistant: String(parsed.assistant ?? content),
      drafts: asRecord(parsed.drafts),
      raw_model_response: content,
    };
  }
  return {
    assistant: content || "The model returned an empty response.",
    drafts: {},
    raw_model_response: content,
  };
}

function extractTaggedAssistantPayload(content: string): JsonMap | null {
  const assistantMatch = content.match(/<assistant>\s*([\s\S]*?)\s*<\/assistant>/i);
  const draftsMatch = content.match(/<drafts>\s*([\s\S]*?)\s*<\/drafts>/i);
  if (!assistantMatch && !draftsMatch) {
    return null;
  }
  const assistantRaw = assistantMatch?.[1]?.trim() ?? "";
  const assistantParsed = assistantRaw ? safeJsonValue(assistantRaw) : "";
  const draftsRaw = draftsMatch?.[1]?.trim() ?? "";
  const draftsParsed = draftsRaw ? extractJsonObject(draftsRaw) ?? safeJsonValue(draftsRaw) : {};
  return {
    assistant: typeof assistantParsed === "string" ? assistantParsed : JSON.stringify(assistantParsed),
    drafts: asRecord(draftsParsed),
  };
}

function extractJsonObject(content: string): JsonMap | null {
  const fenced = content.match(/```(?:json)?\s*([\s\S]*?)```/i);
  const candidate = fenced ? fenced[1] : content;
  const first = candidate.indexOf("{");
  const last = candidate.lastIndexOf("}");
  if (first >= 0 && last > first) {
    const parsed = safeJsonValue(candidate.slice(first, last + 1));
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as JsonMap;
    }
  }
  const objects = extractBalancedJsonObjects(candidate);
  return objects.find((object) => "assistant" in object || "drafts" in object) ?? objects.at(-1) ?? null;
}

function extractBalancedJsonObjects(content: string): JsonMap[] {
  const objects: JsonMap[] = [];
  let start = -1;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = 0; index < content.length; index += 1) {
    const char = content[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === "\"") {
        inString = false;
      }
      continue;
    }
    if (char === "\"") {
      inString = true;
      continue;
    }
    if (char === "{") {
      if (depth === 0) {
        start = index;
      }
      depth += 1;
      continue;
    }
    if (char !== "}" || depth === 0) {
      continue;
    }
    depth -= 1;
    if (depth === 0 && start >= 0) {
      const parsed = safeJsonValue(content.slice(start, index + 1));
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        objects.push(parsed as JsonMap);
      }
      start = -1;
    }
  }
  return objects;
}

function stubChat(mode: string, messages: ChatMessage[], context: JsonMap): JsonMap {
  const latest = messages[messages.length - 1]?.content ?? "";
  const drafts = stubDrafts(mode, latest, context);
  const assistant = stubAssistantText(mode);
  return {
    provider: "stub",
    model: null,
    mode,
    assistant,
    drafts,
    chat_backend: {
      source: "stub",
      backend_mode: chatBackendMode,
      resolution_purpose: "operator_chat",
      durable_task_dispatch: false,
      user_request: true,
      reason: stubChatReason(),
    },
    context,
  };
}

function stubChatReason(): string {
  if (chatBackendMode === "stub") {
    return "control chat backend is set to stub";
  }
  if (chatCompletionsUrl && !chatModel) {
    return "chat completions URL or LLM gateway configured without COAT_CONTROL_CHAT_MODEL, COAT_LLM_GATEWAY_CHAT_MODEL, or COAT_LLM_GATEWAY_DEFAULT_MODEL";
  }
  if (chatModel && !chatCompletionsUrl) {
    return "chat model is configured but no matching gateway, OpenAI-compatible chat URL, or direct OpenAI provider is configured";
  }
  if (!chatRunnerDiscoveryEnabled()) {
    return "no configured chat backend; runner-registry discovery is disabled by default for operator chat";
  }
  return "no live chat model configured";
}

function stubAssistantText(mode: string): string {
  if (mode === "draft_goal") {
    return "Goal draft ready. Review the fields, then accept or discard it.";
  }
  if (mode === "draft_steering") {
    return "Drafted a steering directive that can be reviewed before it changes durable workflow state.";
  }
  if (mode === "draft_search") {
    return "Drafted a backend-routed search request and a coordinator-owned research task proposal.";
  }
  if (mode === "explain_state") {
    return "Prepared a state-oriented response from the available backend projection.";
  }
  return "Drafted a durable plan payload with subgoal structure, acceptance evidence, and review gates.";
}

function stubDrafts(mode: string, prompt: string, context: JsonMap): JsonMap {
  if (mode === "draft_goal") {
    return { goal_spec: goalSpecDraft(prompt) };
  }
  if (mode === "draft_plan") {
    return { plan_draft: planDraft(prompt) };
  }
  if (mode === "draft_steering") {
    return { steering_directive: steeringDraft(prompt, String(context.goal_id ?? "")) };
  }
  if (mode === "draft_search") {
    return {
      search_request: searchRequestDraft(prompt, context),
      steering_directive: researchSteeringDraft(prompt, String(context.goal_id ?? "")),
    };
  }
  if (mode === "explain_state") {
    return {};
  }
  return {
    plan_draft: planDraft(prompt),
    steering_directive: steeringDraft(prompt, String(context.goal_id ?? "")),
  };
}

function goalSpecDraft(prompt: string): JsonMap {
  const objective = prompt || "Define the objective in concrete, testable terms.";
  return {
    title: shortTitle(objective),
    objective,
    repo: null,
    authoring: {
      intake_summary: objective,
      acceptance_evidence: ["operator reviewed the generated GoalSpec", "validator can determine completion"],
      constraints: [],
      out_of_scope: [],
      assumptions: [],
      open_questions: [],
    },
    plan: {
      summary: "Chat-authored goal draft; revise before submission.",
      subgoals: [
        {
          id: "plan-next-frontier",
          title: "Plan next frontier",
          objective: "Turn the operator objective into coordinator-owned durable task slices.",
          owner_role: "planner",
          color: {
            key: "planning",
            label: "Planning",
            hex: "#7c3aed",
            meaning: "goal decomposition and durable task planning",
          },
          acceptance_evidence: ["first durable task frontier is visible to the coordinator"],
        },
      ],
      distribution_notes: ["Coordinator owns task creation; workers may only request child tasks."],
    },
    root_budget: defaultBudget(),
    done_criteria: { tests_pass: true, artifact_exists: true, validator_score_min: 0.85 },
    initial_tasks: [
      {
        role: "planner",
        title: "Plan next frontier",
        subgoal_id: "plan-next-frontier",
        prompt: `Turn this objective into the next durable task frontier: ${objective}`,
        reason: "Seed coordinator-owned decomposition from the chat-authored goal.",
      },
    ],
  };
}

function planDraft(prompt: string): JsonMap {
  const objective = prompt || "Refine this rough request into a durable plan.";
  return {
    title: shortTitle(objective),
    objective,
    repo: null,
    prompt: objective,
    mode: "interactive",
    author: "operator",
    authoring: {
      intake_summary: objective,
      acceptance_evidence: ["plan can compile into a GoalSpec", "initial tasks are coordinator-owned"],
      constraints: [],
      out_of_scope: [],
      assumptions: [],
      open_questions: [],
    },
    plan: {
      summary: "Chat-authored durable plan draft.",
      subgoals: [],
      distribution_notes: ["Use the coordinator task tree for subagents and fork/join work."],
    },
    initial_tasks: [],
    questions: [],
    decisions: [],
  };
}

function steeringDraft(prompt: string, goalId: string): JsonMap {
  return {
    id: crypto.randomUUID(),
    goal_id: goalId || undefined,
    task_id: null,
    operator: "operator",
    message: prompt || "Steer the goal based on operator chat guidance.",
    kind: {
      kind: "add_constraint",
      constraint: prompt || "Clarify the desired steering constraint before sending.",
    },
  };
}

function searchRequestDraft(prompt: string, context: JsonMap): JsonMap {
  const query = prompt || "Clarify the search question before running search.";
  const goalId = String(context.goal_id ?? "");
  return {
    query,
    intent: "operator_search",
    scopes: [
      { kind: "memory", goal_id: goalId || null, tool: "coat_memory_search" },
      { kind: "reference", tool: "coat_web_search", requires_configured_gateway: true },
    ],
    limit: 10,
    require_sources: true,
    use: "Return sourced facts and an InformationUsePlan before changing durable state.",
  };
}

function researchSteeringDraft(prompt: string, goalId: string): JsonMap {
  const query = prompt || "Clarify the research question before spawning a research task.";
  return {
    id: crypto.randomUUID(),
    goal_id: goalId || undefined,
    task_id: null,
    operator: "operator",
    message: `Search and synthesize: ${query}`,
    kind: {
      kind: "inject_task",
      role: "research",
      reason: "operator requested search from control chat",
      prompt: `<task>
  <role>research</role>
  <instruction>MUST search approved memory, MCP, documentation, reference, and web sources allowed by the goal research policy. MUST cite sources. MUST return ResearchOutput plus InformationUsePlan. MUST NOT mutate durable state directly.</instruction>
  <question>${escapeXml(query)}</question>
  <expected_output>ResearchOutput with sourced facts, confidence, gaps, and recommended coordinator-owned next actions.</expected_output>
</task>`,
    },
  };
}

async function coatWebSearch(args: Record<string, unknown>): Promise<JsonMap> {
  const query = String(args.query ?? "").trim();
  if (!query) {
    throw new Error("query is required");
  }
  const goalId = typeof args.goal_id === "string" ? args.goal_id.trim() : "";
  const route = String(args.route ?? "coordinator_task");
  const context = Array.isArray(args.context)
    ? args.context.filter((item): item is string => typeof item === "string")
    : [];
  const searchRequest = {
    query,
    goal_id: goalId || null,
    limit: typeof args.limit === "number" ? args.limit : undefined,
    context,
    route,
    required_capabilities: ["research", "web_search"],
    required_model_features: ["tool_use", "json_schema"],
    result_contract: "ResearchOutput plus InformationUsePlan",
  };

  if (route === "coordinator_task" && goalId) {
    const directive = researchSteeringDraft(query, goalId);
    const steering = await workflowPost(goalId, "steer", directive);
    return {
      status: "planned",
      route,
      search_request: searchRequest,
      steering_directive: directive,
      steering,
      note: "web/reference search was submitted as a coordinator-owned durable research task",
    };
  }

  return {
    status: "planned",
    route,
    search_request: searchRequest,
    steering_directive: goalId ? researchSteeringDraft(query, goalId) : null,
    note: "submit the steering_directive for durable execution or call the Rust tool-registry coat_web_search with runner_registry routing",
  };
}

function defaultBudget(): JsonMap {
  return {
    max_tokens: 2_000_000,
    remaining_tokens: 2_000_000,
    max_runtime_seconds: 14_400,
    remaining_runtime_seconds: 14_400,
    max_tool_calls: 2_000,
    remaining_tool_calls: 2_000,
    max_child_tasks: 64,
    remaining_child_tasks: 64,
    max_patch_size: 500_000,
  };
}

function shortTitle(value: string): string {
  const cleaned = value.replace(/\s+/g, " ").trim();
  if (!cleaned) {
    return "Chat Authored Goal";
  }
  return cleaned.length <= 64 ? cleaned : `${cleaned.slice(0, 61)}...`;
}

function safeJsonValue(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function atRecord(value: unknown, path: string[]): unknown {
  let current = value;
  for (const key of path) {
    if (Array.isArray(current) && /^\d+$/.test(key)) {
      current = current[Number(key)];
      continue;
    }
    if (!current || typeof current !== "object") {
      return null;
    }
    current = (current as JsonMap)[key];
  }
  return current;
}

async function routeApi(req: any, res: any, url: URL): Promise<void> {
  if (!isAuthorized(req)) {
    sendJson(res, 401, { error: "missing or invalid COAT_CONTROL_GATEWAY_TOKEN bearer token" });
    return;
  }

  const segments = url.pathname.split("/").filter(Boolean);

  if (req.method === "GET" && url.pathname === "/api/operator/stream") {
    await streamOperatorState(req, res, url);
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/operator/workspace") {
    sendJson(res, 200, await operatorWorkspace(url.searchParams.get("goal_id")));
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/operator/goals") {
    sendJson(res, 200, await operatorGoalList(url.search));
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/operator/goals") {
    const result = await submitOperatorGoalSpec(await readJson(req));
    sendJson(res, workflowMutationHttpStatus(result as ProxyResult), result);
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/operator/actions") {
    sendJson(res, 200, await operatorActionList(url.searchParams.get("goal_id")));
    return;
  }

  if (segments[0] === "api" && segments[1] === "operator" && segments[2] === "actions" && segments[3] && segments[4] === "resolve") {
    if (req.method !== "POST") {
      sendJson(res, 405, { error: "operator action resolution requires POST" });
      return;
    }
    sendJson(res, 200, await resolveOperatorAction(decodeURIComponent(segments[3]), await readJson(req)));
    return;
  }

  if (segments[0] === "api" && segments[1] === "operator" && segments[2] === "goals" && segments[3]) {
    const goalId = decodeURIComponent(segments[3]);
    if (req.method === "GET" && segments.length === 4) {
      sendJson(res, 200, operatorGoalDetail(await composedGoalSnapshot(goalId)));
      return;
    }
    if (req.method === "GET" && segments[4] === "graph") {
      sendJson(res, 200, await operatorGoalGraph(goalId));
      return;
    }
    if (req.method === "GET" && segments[4] === "agent-context") {
      sendJson(res, 200, await goalAgentContext(goalId, url.searchParams.get("task_id")));
      return;
    }
    if (req.method === "POST" && segments[4]) {
      const action = segments[4];
      const result = await operatorGoalActionEnvelope(goalId, action, await readJson(req));
      sendJson(res, workflowMutationHttpStatus(result as ProxyResult), result);
      return;
    }
  }

  if (req.method === "POST" && url.pathname === "/api/chat") {
    sendJson(res, 200, await controlChat(await readJson(req)));
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/chat/session") {
    sendJson(res, 200, await chatSession(url.searchParams.get("session_id") || "operator:default", url.searchParams.get("debug") === "1"));
    return;
  }

  if (req.method === "GET" && segments[0] === "api" && segments[1] === "chat" && segments[2] === "runs" && segments[3]) {
    sendJson(res, 200, chatRunSnapshot(decodeURIComponent(segments[3])));
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/plans") {
    sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/plans${url.search}`, { method: "GET" }));
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/plans") {
    const body = await readJson(req);
    sendJson(res, 200, await proxyJson(goalStoreUrl, "/goal-store/plans", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(body),
    }));
    return;
  }

  if (segments[0] === "api" && segments[1] === "plans" && segments[2]) {
    const planId = decodeURIComponent(segments[2]);
    if (req.method === "GET" && segments[3] === "continuity") {
      sendJson(res, 200, await planContinuity(planId));
      return;
    }
    if (req.method === "GET" && segments.length === 3) {
      sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}`, { method: "GET" }));
      return;
    }
    if (req.method === "POST" && segments[3] === "revisions") {
      const body = await readJson(req);
      sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}/revisions`, {
        method: "POST",
        headers: jsonHeaders(),
        body: JSON.stringify(body),
      }));
      return;
    }
    if (req.method === "POST" && segments[3] === "compile") {
      const body = await readJson(req);
      sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}/compile`, {
        method: "POST",
        headers: jsonHeaders(),
        body: JSON.stringify(body),
      }));
      return;
    }
  }

  if (req.method === "POST" && segments[0] === "api" && segments[1] === "research" && segments[2] === "apply") {
    const body = await readJson(req);
    const record = asRecord(body);
    const goalId = String(record.goal_id ?? "");
    sendJson(res, 200, await applyResearchOutput(goalId, body));
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/human/notify") {
    const body = await readJson(req);
    sendJson(res, 200, await proxyJson(notifierUrl, "/notify", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(body),
    }));
    return;
  }

  if (segments[0] === "api" && segments[1] === "events") {
    await proxyEventRoute(req, res, url, segments);
    return;
  }

  if (segments[0] === "api" && segments[1] === "memory") {
    await proxyMemoryRoute(req, res, segments);
    return;
  }

  sendJson(res, 404, { error: "not found" });
}

async function proxyEventRoute(req: any, res: any, url: URL, segments: string[]): Promise<void> {
  const eventHeaders = bearer(eventGatewayToken);
  if (req.method === "GET" && segments.length === 2) {
    sendJson(res, 200, await proxyJson(eventGatewayUrl, `/events${url.search}`, { method: "GET" }, eventHeaders));
    return;
  }
  if (req.method === "GET" && segments[2] === "sources") {
    sendJson(res, 200, await proxyJson(eventGatewayUrl, "/event-sources", { method: "GET" }, eventHeaders));
    return;
  }
  if (req.method === "GET" && segments[2] === "triggers") {
    sendJson(res, 200, await proxyJson(eventGatewayUrl, "/triggers", { method: "GET" }, eventHeaders));
    return;
  }
  if (req.method === "POST" && segments[2] === "sources") {
    const body = await readJson(req);
    const approvalId = new URLSearchParams(url.search).get("approval_id");
    const path = approvalId ? `/event-sources?approval_id=${encodeURIComponent(approvalId)}` : "/event-sources";
    sendJson(res, 200, await proxyJson(eventGatewayUrl, path, {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(body),
    }, eventHeaders));
    return;
  }
  if (req.method === "POST" && segments[2] === "ingest") {
    const body = await readJson(req);
    const route = new URLSearchParams(url.search).get("route") === "true";
    sendJson(res, 200, await proxyJson(eventGatewayUrl, `/events?route=${route}`, {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(body),
    }, eventHeaders));
    return;
  }
  if (req.method === "POST" && segments[2] === "trigger") {
    const body = await readJson(req);
    sendJson(res, 200, await proxyJson(eventGatewayUrl, "/triggers", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(body),
    }, eventHeaders));
    return;
  }
  sendJson(res, 404, { error: "unknown event gateway route" });
}

async function proxyMemoryRoute(req: any, res: any, segments: string[]): Promise<void> {
  const memoryHeaders = bearer(memoryGatewayToken);
  const action = segments[2] ?? "";
  if (req.method === "GET" && action === "events" && segments[3]) {
    sendJson(res, 200, await proxyJson(memoryGatewayUrl, `/memory/events/${encodeURIComponent(decodeURIComponent(segments[3]))}`, { method: "GET" }, memoryHeaders));
    return;
  }
  if (req.method !== "POST") {
    sendJson(res, 405, { error: "memory routes require POST, except /api/memory/events/:goalId" });
    return;
  }
  const pathByAction: Record<string, string> = {
    write: "/memory/write",
    search: "/memory/search",
    context: "/memory/context",
    join: "/memory/join",
    retract: "/memory/retract",
    edit: "/memory/edit",
    "edit-preview": "/memory/edit/preview",
    repair: "/memory/repair",
  };
  const path = pathByAction[action];
  if (!path) {
    sendJson(res, 404, { error: `unknown memory action: ${action}` });
    return;
  }
  const body = await readJson(req);
  sendJson(res, 200, await proxyJson(memoryGatewayUrl, path, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify(body),
  }, memoryHeaders));
}

function goalIdFromSpec(body: unknown): string | null {
  if (!body || typeof body !== "object") {
    return null;
  }
  const record = body as Record<string, unknown>;
  const value = record.goal_id ?? record.id;
  return typeof value === "string" && value.length > 0 ? value : null;
}

function goalSpecWithId(body: unknown): { goalId: string; spec: unknown } {
  if (!body || typeof body !== "object" || Array.isArray(body)) {
    throw new Error("goal spec must be a JSON object");
  }
  const record = resolveGoalDraftForSubmit(body as JsonMap);
  const goalId = goalIdFromSpec(record) ?? crypto.randomUUID();
  return { goalId, spec: normalizeGoalSpecForSubmit(record, goalId) };
}

function resolveGoalDraftForSubmit(record: JsonMap): JsonMap {
  const draftId = String(record.draft_id ?? asRecord(record.draft_ref).draft_id ?? "").trim();
  const stored = draftId ? chatDrafts.get(draftId) : undefined;
  if (stored?.kind === "goal_spec") {
    return mergeGoalDraftEdits(stored.payload, record);
  }
  if (record.compact === true || (draftId && !record.root_budget)) {
    return expandCompactGoalDraft(record);
  }
  return record;
}

function mergeGoalDraftEdits(baseDraft: JsonMap, editedDraft: JsonMap): JsonMap {
  const base = JSON.parse(JSON.stringify(baseDraft)) as JsonMap;
  const authoring = {
    ...asRecord(base.authoring),
    ...asRecord(editedDraft.authoring),
  };
  const editedPlan = asRecord(editedDraft.plan);
  const plan = editedDraft.compact === true
    ? asRecord(base.plan)
    : {
        ...asRecord(base.plan),
        ...editedPlan,
      };
  return compactRecord({
    ...base,
    title: editedDraft.title ?? base.title,
    objective: editedDraft.objective ?? base.objective,
    repo: "repo" in editedDraft ? editedDraft.repo : base.repo,
    authoring,
    plan,
    done_criteria: Object.keys(asRecord(editedDraft.done_criteria)).length ? asRecord(editedDraft.done_criteria) : base.done_criteria,
  });
}

function expandCompactGoalDraft(draft: JsonMap): JsonMap {
  const objective = String(draft.objective ?? draft.title ?? "Define the objective in concrete, testable terms.");
  const authoring = asRecord(draft.authoring);
  const plan = asRecord(draft.plan);
  const subgoals = rowsFromData(plan.subgoals);
  return compactRecord({
    title: String(draft.title ?? shortTitle(objective)),
    objective,
    repo: draft.repo ?? null,
    authoring: {
      intake_summary: String(authoring.intake_summary ?? objective),
      acceptance_evidence: arrayField(authoring, "acceptance_evidence"),
      constraints: arrayField(authoring, "constraints"),
      out_of_scope: arrayField(authoring, "out_of_scope"),
      assumptions: arrayField(authoring, "assumptions"),
      open_questions: arrayField(authoring, "open_questions"),
    },
    plan: {
      summary: String(plan.summary ?? "Chat-authored compact goal draft."),
      subgoals,
      distribution_notes: arrayField(plan, "distribution_notes"),
    },
    root_budget: Object.keys(asRecord(draft.root_budget)).length ? asRecord(draft.root_budget) : defaultBudget(),
    done_criteria: Object.keys(asRecord(draft.done_criteria)).length
      ? asRecord(draft.done_criteria)
      : { tests_pass: true, artifact_exists: true, validator_score_min: 0.85 },
    initial_tasks: rowsFromData(draft.initial_tasks),
  });
}

function normalizeGoalSpecForSubmit(record: Record<string, unknown>, goalId: string): JsonMap {
  let initialTasks = normalizeInitialTaskRequests(arrayField(record as JsonMap, "initial_tasks"), record);
  const plan = asRecord(record.plan);
  let subgoals = normalizeSubgoalSpecs(rowsFromData(plan.subgoals), initialTasks);
  if (initialTasks.length === 0) {
    initialTasks = [synthesizeInitialTaskRequest(record, subgoals)];
    subgoals = normalizeSubgoalSpecs(subgoals, initialTasks);
  }
  return {
    ...record,
    id: goalId,
    plan: {
      ...plan,
      subgoals,
    },
    initial_tasks: initialTasks,
  };
}

function synthesizeInitialTaskRequest(source: Record<string, unknown>, subgoals: JsonMap[]): JsonMap {
  const firstSubgoal = subgoals[0];
  const objective = String(source.objective ?? source.title ?? firstSubgoal?.objective ?? firstSubgoal?.title ?? "the submitted goal");
  return {
    role: "planner",
    title: "Plan next frontier",
    subgoal_id: firstSubgoal?.id,
    prompt: `Plan the next durable task frontier for: ${objective}`,
    reason: "Seed coordinator-owned work because the submitted goal draft did not include an initial task frontier.",
    color: normalizeGraphColor(firstSubgoal?.color),
  };
}

function normalizeSubgoalSpecs(subgoals: JsonMap[], initialTasks: JsonMap[]): JsonMap[] {
  const roleBySubgoal = new Map<string, string>();
  for (const task of initialTasks) {
    const subgoalId = String(task.subgoal_id ?? "").trim();
    const role = normalizeWorkerKind(task.role);
    if (subgoalId && role) {
      roleBySubgoal.set(subgoalId, role);
    }
  }
  return subgoals.map((subgoal, index) => {
    const id = String(subgoal.id ?? subgoal.subgoal_id ?? slugFromText(String(subgoal.title ?? subgoal.objective ?? `subgoal-${index + 1}`))).trim();
    const ownerRole = normalizeWorkerKind(subgoal.owner_role ?? subgoal.owner ?? subgoal.role)
      ?? roleBySubgoal.get(id)
      ?? "planner";
    return {
      ...subgoal,
      id,
      title: String(subgoal.title ?? humanTitleFromSlug(id) ?? `Subgoal ${index + 1}`),
      objective: String(subgoal.objective ?? subgoal.summary ?? subgoal.title ?? `Complete ${humanTitleFromSlug(id)}`),
      owner_role: ownerRole,
      color: normalizeGraphColor(subgoal.color),
    };
  });
}

function normalizeInitialTaskRequests(tasks: unknown[], source: Record<string, unknown>): JsonMap[] {
  const subgoals = rowsFromData(asRecord(source.plan).subgoals);
  const onlySubgoalId = subgoals.length === 1 ? String(subgoals[0].id ?? subgoals[0].subgoal_id ?? "").trim() : "";
  return tasks.map((task, index) => {
    const record = asRecord(task);
    const role = normalizeWorkerKind(record.role ?? record.owner_role) ?? "planner";
    return {
      ...record,
      role,
      title: record.title ?? (index === 0 ? "Plan next frontier" : `Initial task ${index + 1}`),
      subgoal_id: record.subgoal_id ?? (onlySubgoalId || undefined),
      prompt: String(record.prompt ?? `Plan the next durable task frontier for: ${source.objective ?? source.title ?? "this goal"}`),
      reason: String(record.reason ?? "Seed coordinator-owned work from the submitted goal draft."),
      color: normalizeGraphColor(record.color),
    };
  });
}

function normalizeWorkerKind(value: unknown): string | null {
  const raw = String(value ?? "").trim();
  if (!raw) {
    return null;
  }
  const normalized = raw
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[\s-]+/g, "_")
    .toLowerCase();
  const aliases: Record<string, string> = {
    claude: "claude_code",
    claude_code_runner: "claude_code",
    staff_engineer: "staff_engineer_claude",
    formal: "formal_methods",
    model: "model_provider",
  };
  const canonical = aliases[normalized] ?? normalized;
  const allowed = new Set([
    "planner",
    "codex",
    "claude_code",
    "staff_engineer_claude",
    "model_provider",
    "research",
    "reviewer",
    "tester",
    "formal_methods",
    "validator",
    "patch_merger",
    "rust_tool",
  ]);
  return allowed.has(canonical) ? canonical : "planner";
}

function normalizeGraphColor(value: unknown): unknown {
  if (value == null) {
    return undefined;
  }
  const record = asRecord(value);
  if (Object.keys(record).length > 0) {
    const key = String(record.key ?? "").trim();
    if (!key) {
      return undefined;
    }
    const label = String(record.label ?? humanTitleFromSlug(key) ?? key).trim();
    const hex = /^#[0-9a-fA-F]{6}$/.test(String(record.hex ?? "")) ? String(record.hex) : colorHexForKey(key);
    const meaning = String(record.meaning ?? `visual graph color ${key}`).trim();
    return { ...record, key, label, hex, meaning };
  }
  const key = slugFromText(String(value));
  return key ? {
    key,
    label: humanTitleFromSlug(key),
    hex: colorHexForKey(key),
    meaning: `visual graph color ${key}`,
  } : undefined;
}

function slugFromText(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

function humanTitleFromSlug(value: string): string {
  return value
    .split(/[-_]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function colorHexForKey(key: string): string {
  const palette: Record<string, string> = {
    red: "#c2410c",
    orange: "#d97706",
    yellow: "#ca8a04",
    green: "#16a34a",
    cyan: "#0891b2",
    blue: "#2563eb",
    purple: "#7c3aed",
    pink: "#db2777",
    work: "#2563eb",
    review: "#d97706",
    research: "#0891b2",
    validation: "#16a34a",
  };
  return palette[key] ?? "#7d8b94";
}

async function routeMcp(req: any, res: any): Promise<void> {
  if (!isAuthorized(req, true)) {
    sendJson(res, 401, { error: "missing or invalid COAT_CONTROL_MCP_TOKEN bearer token" });
    return;
  }
  const body = (await readJson(req)) as Record<string, unknown>;
  const method = String(body.method ?? "");
  const id = body.id ?? null;
  try {
    if (method === "initialize") {
      sendJson(res, 200, {
        jsonrpc: "2.0",
        id,
        result: {
          protocolVersion: "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name: "coat-control-plane-web", version: "0.1.0" },
        },
      });
      return;
    }
    if (method === "tools/list") {
      sendJson(res, 200, { jsonrpc: "2.0", id, result: { tools: mcpTools() } });
      return;
    }
    if (method === "tools/call") {
      const params = (body.params ?? {}) as Record<string, unknown>;
      const name = String(params.name ?? "");
      const args = (params.arguments ?? {}) as Record<string, unknown>;
      const result = await callMcpTool(name, args);
      sendJson(res, 200, { jsonrpc: "2.0", id, result: { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] } });
      return;
    }
    sendJson(res, 200, { jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } });
  } catch (error) {
    sendJson(res, 200, {
      jsonrpc: "2.0",
      id,
      error: { code: -32000, message: error instanceof Error ? error.message : String(error) },
    });
  }
}

function mcpTools(): unknown[] {
  return [
    {
      name: "coat_operator_workspace",
      description: "Read the compact COAT operator workspace: goals, selected goal, actions, events, worker runs, evidence, service health, runners, event sources, and human threads.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        properties: {
          goal_id: { type: "string" },
          event_type: { type: "string" },
          since: { type: "string" },
        },
      },
    },
    {
      name: "coat_operator_goal",
      description: "Read the product-shaped operator goal detail for one goal.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id"],
        properties: { goal_id: { type: "string" } },
      },
    },
    {
      name: "coat_operator_actions",
      description: "List product-shaped operator actions across goals or for one selected goal.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        properties: { goal_id: { type: "string" } },
      },
    },
    {
      name: "coat_operator_action_resolve",
      description: "Resolve a product-shaped operator action such as approval, continuation, retry, replan, or cancel.",
      inputSchema: {
        type: "object",
        additionalProperties: true,
        required: ["action_id", "goal_id", "resolution"],
        properties: {
          action_id: { type: "string" },
          goal_id: { type: "string" },
          resolution: { type: "string" },
          response_summary: { type: "string" },
          approval_id: { type: "string" },
          thunk_id: { type: "string" },
          task_id: { type: "string" },
        },
      },
    },
    {
      name: "coat_operator_agent_context",
      description: "Read drill-down task context for a goal from existing task, chat-session, artifact, and notification-thread projections.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id"],
        properties: { goal_id: { type: "string" }, task_id: { type: "string" } },
      },
    },
    {
      name: "coat_plan_list",
      description: "List durable planning-mode drafts and compiled plans.",
      inputSchema: { type: "object", additionalProperties: false, properties: { limit: { type: "integer", minimum: 1 } } },
    },
    {
      name: "coat_plan_draft",
      description: "Create or store a durable planning-mode draft through the goal-store plan surface.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_plan_get",
      description: "Read one durable plan by plan_id.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["plan_id"],
        properties: { plan_id: { type: "string" } },
      },
    },
    {
      name: "coat_plan_compile",
      description: "Compile a durable plan into a GoalSpec without submitting it.",
      inputSchema: {
        type: "object",
        additionalProperties: true,
        required: ["plan_id"],
        properties: { plan_id: { type: "string" } },
      },
    },
    {
      name: "coat_plan_revise",
      description: "Append a revision to an existing durable planning-mode draft.",
      inputSchema: {
        type: "object",
        additionalProperties: true,
        required: ["plan_id"],
        properties: { plan_id: { type: "string" } },
      },
    },
    {
      name: "coat_plan_continuity",
      description: "Read a continuity summary for one durable plan: questions, decisions, subgoals, initial tasks, revisions, and next actions.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["plan_id"],
        properties: { plan_id: { type: "string" } },
      },
    },
    {
      name: "coat_operator_goal_steer",
      description: "Submit a SteeringDirective through the operator goal action surface.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id", "directive"],
        properties: { goal_id: { type: "string" }, directive: { type: "object" } },
      },
    },
    {
      name: "coat_operator_goal_submit",
      description: "Submit a GoalSpec through the operator goal surface. If id is omitted, the gateway assigns one before submission.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_chat_assist",
      description: "Ask the control-plane chat assistant to explain state or draft a GoalSpec, plan, or steering directive without mutating durable state.",
      inputSchema: {
        type: "object",
        additionalProperties: true,
        required: ["messages"],
        properties: {
          mode: { type: "string" },
          goal_id: { type: "string" },
          messages: { type: "array" },
        },
      },
    },
    {
      name: "coat_subagent_policy",
      description: "Return the COAT rule that subagents are durable coordinator-owned child tasks, not native runner subagents.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
    },
    {
      name: "coat_checkpoint_history",
      description: "Read projected checkpoint history for a goal, including git and snapshot refs.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id"],
        properties: { goal_id: { type: "string" } },
      },
    },
    {
      name: "coat_runner_register",
      description: "Register a non-local runner endpoint with the runner registry using the RunnerRegistration payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_search",
      description: "Search the memory gateway using the standard MemorySearchRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_web_search",
      description: "Route web/reference search as a coordinator-owned research task or typed WebSearchRequest; never claim live search ran unless a runner/tool result is returned.",
      inputSchema: {
        type: "object",
        additionalProperties: true,
        required: ["query"],
        properties: {
          query: { type: "string" },
          goal_id: { type: "string" },
          route: { type: "string", enum: ["plan_only", "coordinator_task", "runner_registry"] },
          context: { type: "array", items: { type: "string" } },
          limit: { type: "integer", minimum: 1 },
        },
      },
    },
    {
      name: "coat_memory_context",
      description: "Fetch a scoped memory context pack using the standard MemoryContextRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_write",
      description: "Write a reviewed memory event through the memory gateway using the standard MemoryWriteRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_join",
      description: "Promote or invalidate branch memories after unifier review using the standard MemoryJoinRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_retract",
      description: "Retract selected memory records after operator or unifier review using the standard MemoryRetractRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_edit",
      description: "Retract old memory keys and write a linked replacement using the standard MemoryEditRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_edit_preview",
      description: "Preview existing memory keys versus a replacement before committing the edit.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_repair",
      description: "Replay selected memory events into configured adapters after Graphiti, Qdrant, or embedding credentials recover.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_memory_events",
      description: "Read memory events projected for one goal.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id"],
        properties: { goal_id: { type: "string" } },
      },
    },
    {
      name: "coat_apply_research_output",
      description: "Convert a ResearchOutput or InformationUsePlan into coordinator-owned SteeringDirective calls.",
      inputSchema: {
        type: "object",
        additionalProperties: true,
        required: ["goal_id"],
        properties: { goal_id: { type: "string" }, research_output: { type: "object" }, use_plan: { type: "object" } },
      },
    },
    {
      name: "coat_event_sources",
      description: "List event sources visible to the event gateway.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
    },
  ];
}

async function callMcpTool(name: string, args: Record<string, unknown>): Promise<unknown> {
  if (name === "coat_operator_workspace") {
    return operatorWorkspace(
      typeof args.goal_id === "string" ? args.goal_id : null,
      {
        eventType: typeof args.event_type === "string" ? args.event_type : null,
        since: typeof args.since === "string" ? args.since : null,
      },
    );
  }
  if (name === "coat_operator_goal") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return operatorGoalDetail(await composedGoalSnapshot(goalId));
  }
  if (name === "coat_operator_actions") {
    return operatorActionList(typeof args.goal_id === "string" ? args.goal_id : null);
  }
  if (name === "coat_operator_action_resolve") {
    const actionId = String(args.action_id ?? "");
    if (!actionId) {
      throw new Error("action_id is required");
    }
    return resolveOperatorAction(actionId, args);
  }
  if (name === "coat_operator_agent_context") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    const taskId = typeof args.task_id === "string" ? args.task_id : null;
    return goalAgentContext(goalId, taskId);
  }
  if (name === "coat_plan_list") {
    const limit = typeof args.limit === "number" ? args.limit : 25;
    return proxyJson(goalStoreUrl, `/goal-store/plans?limit=${encodeURIComponent(String(limit))}`, { method: "GET" });
  }
  if (name === "coat_plan_draft") {
    return proxyJson(goalStoreUrl, "/goal-store/plans", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    });
  }
  if (name === "coat_plan_get") {
    const planId = String(args.plan_id ?? "");
    if (!planId) {
      throw new Error("plan_id is required");
    }
    return proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}`, { method: "GET" });
  }
  if (name === "coat_plan_compile") {
    const planId = String(args.plan_id ?? "");
    if (!planId) {
      throw new Error("plan_id is required");
    }
    return proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}/compile`, {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify({ ...args, plan_id: planId }),
    });
  }
  if (name === "coat_plan_revise") {
    const planId = String(args.plan_id ?? "");
    if (!planId) {
      throw new Error("plan_id is required");
    }
    return proxyJson(goalStoreUrl, `/goal-store/plans/${encodeURIComponent(planId)}/revisions`, {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    });
  }
  if (name === "coat_plan_continuity") {
    const planId = String(args.plan_id ?? "");
    if (!planId) {
      throw new Error("plan_id is required");
    }
    return planContinuity(planId);
  }
  if (name === "coat_operator_goal_steer") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return operatorGoalActionEnvelope(goalId, "steer", args.directive ?? {}, "mcp_steer_goal");
  }
  if (name === "coat_operator_goal_submit") {
    return submitOperatorGoalSpec(args, "mcp_goal_submit");
  }
  if (name === "coat_chat_assist") {
    return controlChat(args);
  }
  if (name === "coat_subagent_policy") {
    return {
      mode: "coordinator_durable_tasks",
      native_subagent_spawn: "disabled",
      child_request_channel: "AgentRunResult.child_requests",
      runner_context_requirements: [
        "initialize Codex, Claude Code, SDK, MCP-client, or local-model contexts with this rule",
        "return proposed child work as ChildTaskRequest objects",
        "let the coordinator apply budget, approval, runner routing, memory, and sandbox policy",
      ],
    };
  }
  if (name === "coat_checkpoint_history") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return proxyJson(goalStoreUrl, `/goal-store/goals/${encodeURIComponent(goalId)}/checkpoints`, { method: "GET" });
  }
  if (name === "coat_runner_register") {
    return proxyJson(runnerRegistryUrl, "/runners", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    });
  }
  if (name === "coat_memory_search") {
    return proxyJson(memoryGatewayUrl, "/memory/search", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_web_search") {
    return coatWebSearch(args);
  }
  if (name === "coat_memory_context") {
    return proxyJson(memoryGatewayUrl, "/memory/context", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_write") {
    return proxyJson(memoryGatewayUrl, "/memory/write", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_join") {
    return proxyJson(memoryGatewayUrl, "/memory/join", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_retract") {
    return proxyJson(memoryGatewayUrl, "/memory/retract", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_edit") {
    return proxyJson(memoryGatewayUrl, "/memory/edit", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_edit_preview") {
    return proxyJson(memoryGatewayUrl, "/memory/edit/preview", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_repair") {
    return proxyJson(memoryGatewayUrl, "/memory/repair", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_memory_events") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return proxyJson(memoryGatewayUrl, `/memory/events/${encodeURIComponent(goalId)}`, { method: "GET" }, bearer(memoryGatewayToken));
  }
  if (name === "coat_apply_research_output") {
    const goalId = String(args.goal_id ?? "");
    return applyResearchOutput(goalId, args);
  }
  if (name === "coat_event_sources") {
    return proxyJson(eventGatewayUrl, "/event-sources", { method: "GET" }, bearer(eventGatewayToken));
  }
  throw new Error(`unknown tool: ${name}`);
}

const server = http.createServer((req, res) => {
  void (async () => {
    const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "localhost"}`);
    const startedAt = Date.now();
    log("debug", "http_request", { method: req.method, path: url.pathname });
    res.once("finish", () => {
      log("debug", "http_response", {
        method: req.method,
        path: url.pathname,
        status_code: res.statusCode,
        duration_ms: Date.now() - startedAt,
      });
    });
    if (req.method === "GET" && url.pathname === "/healthz") {
      sendJson(res, 200, { ok: true, service: "coat-control-plane-web" });
      return;
    }
    if (url.pathname.startsWith("/api/")) {
      await routeApi(req, res, url);
      return;
    }
    if (req.method === "POST" && url.pathname === "/mcp") {
      await routeMcp(req, res);
      return;
    }
    if (req.method === "GET" || req.method === "HEAD") {
      await serveSpa(req, res, url);
      return;
    }
    sendJson(res, 404, { error: "not found" });
  })().catch((error) => {
    log("error", "http_request_failed", { method: req.method, url: req.url, error: errorMessage(error) });
    sendJson(res, 500, { error: error instanceof Error ? error.message : String(error) });
  });
});

server.listen(port, host, () => {
  log("info", "listening", { host, port });
});
