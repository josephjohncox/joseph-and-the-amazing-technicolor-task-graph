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
import { readFile } from "node:fs/promises";

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

const port = Number(process.env.PORT ?? "9090");
const host = process.env.HOST ?? "0.0.0.0";
const gatewayToken = process.env.COAT_CONTROL_GATEWAY_TOKEN ?? "";

const restateIngress = trimSlash(process.env.COAT_RESTATE_INGRESS ?? "http://localhost:8080");
const goalStoreUrl = trimSlash(process.env.COAT_GOAL_STORE_URL ?? "http://localhost:9088");
const eventGatewayUrl = trimSlash(process.env.COAT_EVENT_GATEWAY_URL ?? "http://localhost:9089");
const eventGatewayToken = process.env.COAT_EVENT_GATEWAY_TOKEN ?? "";
const notifierUrl = trimSlash(process.env.COAT_NOTIFIER_URL ?? "http://localhost:9086");
const runnerRegistryUrl = trimSlash(
  process.env.COAT_RUNNER_REGISTRY_URL ?? process.env.COAT_RUNNER_REGISTRY ?? "http://localhost:9085",
);
const memoryGatewayUrl = trimSlash(process.env.COAT_MEMORY_GATEWAY_URL ?? "http://localhost:9087");
const memoryGatewayToken = process.env.COAT_MEMORY_GATEWAY_TOKEN ?? process.env.MEMORY_GATEWAY_TOKEN ?? "";
const controlMcpToken = process.env.COAT_CONTROL_MCP_TOKEN ?? gatewayToken;

const workflowHandlers = new Set([
  "status",
  "progress",
  "tasks",
  "steer",
  "restart",
  "branch",
  "select_branch",
  "cancel",
  "approve",
  "inject_feedback",
]);

const services: ServiceRef[] = [
  { name: "restate", baseUrl: restateIngress, healthPath: "/" },
  { name: "goal-store", baseUrl: goalStoreUrl, healthPath: "/healthz" },
  { name: "event-gateway", baseUrl: eventGatewayUrl, healthPath: "/healthz" },
  { name: "notifier", baseUrl: notifierUrl, healthPath: "/healthz" },
  { name: "runner-registry", baseUrl: runnerRegistryUrl, healthPath: "/healthz" },
  { name: "memory-gateway", baseUrl: memoryGatewayUrl, healthPath: "/healthz" },
];

const clientScriptUrl = new URL("./client.js", import.meta.url);

function trimSlash(value: string): string {
  return value.replace(/\/+$/, "");
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

function sendText(res: any, status: number, body: string, contentType = "text/plain; charset=utf-8"): void {
  res.writeHead(status, {
    "content-type": contentType,
    "cache-control": "no-store",
  });
  res.end(body);
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

async function overview(): Promise<JsonMap> {
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
  const plans = await proxyJson(goalStoreUrl, "/goal-store/plans?limit=25", { method: "GET" });
  const approvals = await proxyJson(goalStoreUrl, "/goal-store/approvals?limit=50", { method: "GET" });

  return {
    generated_at: new Date().toISOString(),
    control_surface: "coat-control-plane-web",
    authority_note:
      "This gateway reads projections and submits workflow signals; Restate and the Rust services remain authoritative.",
    services: health,
    runner_status: runnerStatus,
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

async function goalSnapshot(goalId: string): Promise<JsonMap> {
  const encodedGoalId = encodeURIComponent(goalId);
  const [goal, tasks, events, artifacts, approvals, workflowStatus, progress] = await Promise.all([
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/tasks`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/events`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/artifacts`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/approvals`, { method: "GET" }),
    workflowPost(goalId, "status", {}),
    workflowPost(goalId, "progress", {}),
  ]);
  return {
    generated_at: new Date().toISOString(),
    goal_id: goalId,
    workflow_status: workflowStatus,
    workflow_progress: progress,
    goal_store_goal: goal,
    tasks,
    events,
    artifacts,
    approvals,
    agent_activity: buildAgentActivity(tasks.data, progress.data, events.data, artifacts.data),
  };
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
      role: task.role ?? payload.role ?? "",
      purpose: task.purpose_kind ?? task.purpose ?? payload.purpose ?? null,
      status: task.status ?? payload.status ?? "",
      depth: task.depth ?? payload.depth ?? 0,
      priority: task.priority ?? payload.priority ?? null,
      attempts: task.attempts ?? payload.attempts ?? 0,
      runnable: task.runnable ?? progress?.runnable ?? false,
      prompt: payload.prompt ?? null,
      current_prompt: payload.prompt ?? null,
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

async function routeApi(req: any, res: any, url: URL): Promise<void> {
  if (!isAuthorized(req)) {
    sendJson(res, 401, { error: "missing or invalid COAT_CONTROL_GATEWAY_TOKEN bearer token" });
    return;
  }

  const segments = url.pathname.split("/").filter(Boolean);

  if (req.method === "GET" && url.pathname === "/api/config") {
    sendJson(res, 200, {
      gateway_token_required: Boolean(gatewayToken),
      endpoints: {
        restate_ingress: restateIngress,
        goal_store: goalStoreUrl,
        event_gateway: eventGatewayUrl,
        notifier: notifierUrl,
        runner_registry: runnerRegistryUrl,
        memory_gateway: memoryGatewayUrl,
      },
    });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/overview") {
    sendJson(res, 200, await overview());
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/goals") {
    sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/goals${url.search}`, { method: "GET" }));
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

  if (req.method === "GET" && url.pathname === "/api/agents") {
    sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/tasks${url.search}`, { method: "GET" }));
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/approvals") {
    sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/approvals${url.search}`, { method: "GET" }));
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/goals/submit") {
    const body = await readJson(req);
    const goalId = goalIdFromSpec(body);
    if (!goalId) {
      sendJson(res, 400, { error: "goal spec requires id or goal_id" });
      return;
    }
    sendJson(res, 200, await workflowPost(goalId, "run", body));
    return;
  }

  if (segments[0] === "api" && segments[1] === "goals" && segments[2]) {
    const goalId = decodeURIComponent(segments[2]);
    if (req.method === "GET" && segments.length === 3) {
      sendJson(res, 200, await goalSnapshot(goalId));
      return;
    }
    if (req.method === "POST" && segments.length === 4) {
      const handler = segments[3];
      if (!workflowHandlers.has(handler)) {
        sendJson(res, 400, { error: `unsupported workflow handler: ${handler}` });
        return;
      }
      const body = await readJson(req);
      sendJson(res, 200, await workflowPost(goalId, handler, body));
      return;
    }
  }

  if (req.method === "GET" && url.pathname === "/api/human/threads") {
    sendJson(res, 200, await proxyJson(notifierUrl, "/threads", { method: "GET" }));
    return;
  }

  if (req.method === "GET" && segments[0] === "api" && segments[1] === "human" && segments[2] === "threads" && segments[3]) {
    sendJson(res, 200, await proxyJson(notifierUrl, `/threads/${encodeURIComponent(decodeURIComponent(segments[3]))}`, { method: "GET" }));
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
      name: "coat_overview",
      description: "Read service health, runner status, notification threads, event sources, recent events, and triggers.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
    },
    {
      name: "coat_goal_snapshot",
      description: "Read a goal snapshot from Restate workflow handlers and the goal-store projection.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id"],
        properties: { goal_id: { type: "string" } },
      },
    },
    {
      name: "coat_human_threads",
      description: "List local human feedback and approval notification threads.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
    },
    {
      name: "coat_approval_queue",
      description: "List projected durable approval records across goals.",
      inputSchema: { type: "object", additionalProperties: false, properties: { limit: { type: "integer", minimum: 1 } } },
    },
    {
      name: "coat_agent_activity",
      description: "Read projected task/agent rows globally or for one goal, including prompt payloads when projected.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        properties: { goal_id: { type: "string" }, limit: { type: "integer", minimum: 1 } },
      },
    },
    {
      name: "coat_plan_list",
      description: "List durable planning-mode drafts and compiled plans.",
      inputSchema: { type: "object", additionalProperties: false, properties: { limit: { type: "integer", minimum: 1 } } },
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
      name: "coat_steer_goal",
      description: "Submit a SteeringDirective to GoalWorkflow/steer.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id", "directive"],
        properties: { goal_id: { type: "string" }, directive: { type: "object" } },
      },
    },
    {
      name: "coat_memory_search",
      description: "Search the memory gateway using the standard MemorySearchRequest payload.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_event_sources",
      description: "List event sources visible to the event gateway.",
      inputSchema: { type: "object", additionalProperties: false, properties: {} },
    },
  ];
}

async function callMcpTool(name: string, args: Record<string, unknown>): Promise<unknown> {
  if (name === "coat_overview") {
    return overview();
  }
  if (name === "coat_goal_snapshot") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return goalSnapshot(goalId);
  }
  if (name === "coat_human_threads") {
    return proxyJson(notifierUrl, "/threads", { method: "GET" });
  }
  if (name === "coat_approval_queue") {
    const limit = typeof args.limit === "number" ? args.limit : 50;
    return proxyJson(goalStoreUrl, `/goal-store/approvals?limit=${encodeURIComponent(String(limit))}`, { method: "GET" });
  }
  if (name === "coat_agent_activity") {
    const goalId = typeof args.goal_id === "string" ? args.goal_id : "";
    const limit = typeof args.limit === "number" ? args.limit : 100;
    if (goalId) {
      return goalSnapshot(goalId);
    }
    return proxyJson(goalStoreUrl, `/goal-store/tasks?limit=${encodeURIComponent(String(limit))}`, { method: "GET" });
  }
  if (name === "coat_plan_list") {
    const limit = typeof args.limit === "number" ? args.limit : 25;
    return proxyJson(goalStoreUrl, `/goal-store/plans?limit=${encodeURIComponent(String(limit))}`, { method: "GET" });
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
  if (name === "coat_steer_goal") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return workflowPost(goalId, "steer", args.directive ?? {});
  }
  if (name === "coat_memory_search") {
    return proxyJson(memoryGatewayUrl, "/memory/search", {
      method: "POST",
      headers: jsonHeaders(),
      body: JSON.stringify(args),
    }, bearer(memoryGatewayToken));
  }
  if (name === "coat_event_sources") {
    return proxyJson(eventGatewayUrl, "/event-sources", { method: "GET" }, bearer(eventGatewayToken));
  }
  throw new Error(`unknown tool: ${name}`);
}

function appHtml(): string {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>COAT Control Plane</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f6f7f8;
      --surface: #ffffff;
      --surface-2: #eef1f3;
      --text: #172026;
      --muted: #53616a;
      --line: #cad1d6;
      --accent: #0d6b5f;
      --accent-2: #8b3d58;
      --bad: #a13a2f;
      --warn: #9b6500;
      --good: #24714a;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font: 14px/1.45 Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    header {
      position: sticky;
      top: 0;
      z-index: 2;
      display: grid;
      grid-template-columns: minmax(220px, 1fr) auto;
      gap: 16px;
      align-items: center;
      padding: 12px 18px;
      border-bottom: 1px solid var(--line);
      background: rgba(255,255,255,0.96);
      backdrop-filter: blur(8px);
    }
    h1 {
      margin: 0;
      font-size: 18px;
      font-weight: 680;
      letter-spacing: 0;
    }
    h2 {
      margin: 0 0 10px;
      font-size: 14px;
      font-weight: 680;
      letter-spacing: 0;
    }
    h3 {
      margin: 12px 0 8px;
      font-size: 13px;
      font-weight: 680;
      letter-spacing: 0;
    }
    main {
      display: grid;
      grid-template-columns: 260px minmax(0, 1fr);
      min-height: calc(100vh - 58px);
    }
    nav {
      border-right: 1px solid var(--line);
      background: var(--surface);
      padding: 14px;
    }
    nav button {
      width: 100%;
      display: block;
      margin: 0 0 6px;
      text-align: left;
    }
    section {
      display: none;
      padding: 16px;
    }
    section.active { display: block; }
    .toolbar {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      align-items: center;
      margin-bottom: 12px;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 12px;
    }
    .grid.three {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .panel {
      min-width: 0;
      border: 1px solid var(--line);
      background: var(--surface);
      border-radius: 6px;
      padding: 12px;
    }
    .wide { grid-column: 1 / -1; }
    label {
      display: grid;
      gap: 5px;
      color: var(--muted);
      font-size: 12px;
      font-weight: 600;
    }
    input, textarea, select, button {
      font: inherit;
      letter-spacing: 0;
    }
    input, textarea, select {
      width: 100%;
      border: 1px solid var(--line);
      border-radius: 5px;
      background: #fff;
      color: var(--text);
      padding: 8px 9px;
    }
    textarea {
      min-height: 160px;
      resize: vertical;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 12px;
      line-height: 1.4;
    }
    button {
      border: 1px solid #9aa6ad;
      border-radius: 5px;
      background: #fff;
      color: var(--text);
      padding: 7px 10px;
      cursor: pointer;
      min-height: 34px;
    }
    button.primary {
      border-color: var(--accent);
      background: var(--accent);
      color: #fff;
    }
    button.warn {
      border-color: var(--warn);
      color: var(--warn);
    }
    .status-strip {
      display: flex;
      gap: 6px;
      flex-wrap: wrap;
      justify-content: flex-end;
    }
    .pill {
      display: inline-flex;
      align-items: center;
      min-height: 24px;
      padding: 3px 8px;
      border-radius: 999px;
      border: 1px solid var(--line);
      background: var(--surface-2);
      color: var(--muted);
      font-size: 12px;
      white-space: nowrap;
    }
    .pill.good { color: var(--good); border-color: #9dc7ad; background: #eef8f1; }
    .pill.bad { color: var(--bad); border-color: #dfaea8; background: #fff1ef; }
    .pill.warn { color: var(--warn); border-color: #d7bd7a; background: #fff8df; }
    table {
      width: 100%;
      border-collapse: collapse;
      font-size: 12px;
    }
    th, td {
      border-bottom: 1px solid var(--line);
      padding: 6px;
      text-align: left;
      vertical-align: top;
      word-break: break-word;
    }
    th { color: var(--muted); font-weight: 680; }
    pre {
      overflow: auto;
      max-height: 460px;
      margin: 0;
      border: 1px solid var(--line);
      border-radius: 5px;
      background: #11171c;
      color: #e6eef3;
      padding: 10px;
      font-size: 12px;
      line-height: 1.45;
    }
    .muted { color: var(--muted); }
    .split {
      display: grid;
      grid-template-columns: minmax(260px, 390px) minmax(0, 1fr);
      gap: 12px;
    }
    @media (max-width: 920px) {
      header { grid-template-columns: 1fr; }
      main { grid-template-columns: 1fr; }
      nav {
        border-right: 0;
        border-bottom: 1px solid var(--line);
        display: flex;
        gap: 6px;
        overflow-x: auto;
      }
      nav button { width: auto; white-space: nowrap; }
      .grid, .grid.three, .split { grid-template-columns: 1fr; }
      section { padding: 12px; }
      .status-strip { justify-content: flex-start; }
    }
  </style>
</head>
<body>
  <header>
    <div>
      <h1>COAT Control Plane</h1>
      <div class="muted">Joseph and the Amazing Technicolor Task Graph</div>
    </div>
    <div id="serviceStrip" class="status-strip"></div>
  </header>
  <main>
    <nav>
      <button data-tab="overview" class="primary">Overview</button>
      <button data-tab="plans">Plans</button>
      <button data-tab="agents">Agents</button>
      <button data-tab="goal">Goals</button>
      <button data-tab="human">Human Queue</button>
      <button data-tab="events">Events</button>
      <button data-tab="memory">Memory</button>
      <button data-tab="mcp">MCP</button>
    </nav>
    <div>
      <section id="overview" class="active">
        <div class="toolbar">
          <button id="refreshOverview" class="primary">Refresh</button>
          <label style="max-width: 300px;">Gateway token <input id="apiToken" type="password" autocomplete="off"></label>
        </div>
        <div class="grid three">
          <div class="panel"><h2>Services</h2><div id="servicesView"></div></div>
          <div class="panel"><h2>Runners</h2><div id="runnersView"></div></div>
          <div class="panel"><h2>Human Threads</h2><div id="threadsSummary"></div></div>
          <div class="panel wide"><h2>Approval Queue</h2><div id="approvalsOverview"></div></div>
          <div class="panel wide"><h2>Durable Plans</h2><div id="plansOverview"></div></div>
          <div class="panel wide"><h2>Goal List</h2><div id="goalsView"></div></div>
          <div class="panel wide"><h2>Agent Activity</h2><div id="agentsOverview"></div></div>
          <div class="panel wide"><h2>Overview JSON</h2><pre id="overviewJson"></pre></div>
        </div>
      </section>
      <section id="plans">
        <div class="split">
          <div class="panel">
            <h2>Planning Mode</h2>
            <div class="toolbar">
              <button id="refreshPlans" class="primary">Refresh Plans</button>
              <label>Plan ID <input id="planId" placeholder="plan UUID"></label>
              <button id="loadPlan">Load</button>
            </div>
            <h3>Draft Plan</h3>
            <textarea id="planDraftJson" spellcheck="false">{
  "title": "Durable planning draft",
  "objective": "Turn a rough operator request into a typed, durable plan.",
  "repo": null,
  "prompt": "Capture the planning conversation, questions, decisions, subgoals, and first task frontier.",
  "mode": "interactive",
  "author": "operator",
  "authoring": {
    "intake_summary": "Initial planning-mode draft.",
    "acceptance_evidence": ["plan can compile into a GoalSpec"],
    "constraints": [],
    "out_of_scope": [],
    "assumptions": [],
    "open_questions": []
  },
  "plan": {
    "summary": "Draft before compilation.",
    "subgoals": [],
    "distribution_notes": []
  },
  "initial_tasks": [],
  "questions": [],
  "decisions": []
}</textarea>
            <div class="toolbar"><button id="draftPlan" class="primary">Create Durable Plan</button></div>
            <h3>Revise Plan</h3>
            <textarea id="planRevisionJson" spellcheck="false">{
  "author": "operator",
  "summary": "Refine plan from planning-mode discussion.",
  "operator_message": "Update subgoals, questions, and first task frontier.",
  "status": "ready_for_review"
}</textarea>
            <div class="toolbar">
              <button id="revisePlan">Revise</button>
              <button id="compilePlan">Compile GoalSpec</button>
            </div>
          </div>
          <div class="panel">
            <h2>Plan State</h2>
            <div id="plansView"></div>
            <pre id="planJson"></pre>
          </div>
        </div>
      </section>
      <section id="agents">
        <div class="toolbar">
          <button id="refreshAgents" class="primary">Refresh Agents</button>
          <label style="max-width: 360px;">Filter goal ID <input id="agentGoalFilter" placeholder="optional goal UUID"></label>
        </div>
        <div class="grid">
          <div class="panel wide"><h2>Agent Progress And Prompts</h2><div id="agentsView"></div></div>
          <div class="panel wide"><h2>Selected Agent State</h2><pre id="agentDetailJson"></pre></div>
        </div>
      </section>
      <section id="goal">
        <div class="split">
          <div class="panel">
            <h2>Goal Control</h2>
            <div class="toolbar">
              <label>Goal ID <input id="goalId" placeholder="018f8f2f-..."></label>
              <button id="loadGoal" class="primary">Load</button>
            </div>
            <h3>Submit GoalSpec</h3>
            <textarea id="goalSubmitJson" spellcheck="false">{}</textarea>
            <div class="toolbar"><button id="submitGoal" class="primary">Submit</button></div>
            <h3>SteeringDirective</h3>
            <textarea id="steerJson" spellcheck="false">{
  "kind": "request_research",
  "message": "Find current evidence before continuing.",
  "created_by": "operator"
}</textarea>
            <div class="toolbar">
              <button id="sendSteer" class="primary">Steer</button>
              <button id="goalProgress">Progress</button>
              <button id="goalStatus">Status</button>
            </div>
            <h3>Approval / Feedback</h3>
            <textarea id="approvalJson" spellcheck="false">{
  "approval_id": "",
  "approved": true,
  "comment": "Approved from control gateway"
}</textarea>
            <div class="toolbar">
              <button id="sendApprove">Approve</button>
              <button id="cancelGoal" class="warn">Cancel</button>
            </div>
          </div>
          <div class="panel">
            <h2>Goal Snapshot</h2>
            <div id="goalTables"></div>
            <h3>Agent Activity</h3>
            <div id="goalAgentActivity"></div>
            <pre id="goalJson"></pre>
          </div>
        </div>
      </section>
      <section id="human">
        <div class="toolbar">
          <button id="refreshThreads" class="primary">Refresh Threads</button>
          <button id="refreshApprovals">Refresh Approvals</button>
        </div>
        <div class="grid">
          <div class="panel"><h2>Threads</h2><div id="threadsView"></div></div>
          <div class="panel"><h2>Approvals</h2><div id="approvalsView"></div></div>
          <div class="panel wide"><h2>Thread / Approval Detail</h2><pre id="threadDetail"></pre></div>
        </div>
      </section>
      <section id="events">
        <div class="toolbar">
          <button id="refreshEvents" class="primary">Refresh Events</button>
        </div>
        <div class="grid">
          <div class="panel"><h2>Sources</h2><pre id="eventSourcesJson"></pre></div>
          <div class="panel"><h2>Triggers</h2><pre id="eventTriggersJson"></pre></div>
          <div class="panel wide"><h2>Recent Events</h2><pre id="eventsJson"></pre></div>
        </div>
      </section>
      <section id="memory">
        <div class="split">
          <div class="panel">
            <h2>Memory Search</h2>
            <textarea id="memorySearchJson" spellcheck="false">{
  "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
  "query": "current goal context and steering constraints",
  "limit": 8
}</textarea>
            <div class="toolbar">
              <button id="memorySearch" class="primary">Search</button>
              <button id="memoryContext">Context</button>
            </div>
            <h3>Memory Write</h3>
            <textarea id="memoryWriteJson" spellcheck="false">{
  "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
  "scope": "goal",
  "kind": "operator_note",
  "text": "Reviewed through the control gateway.",
  "tags": ["operator", "dashboard"]
}</textarea>
            <div class="toolbar"><button id="memoryWrite">Write</button></div>
          </div>
          <div class="panel">
            <h2>Memory Result</h2>
            <pre id="memoryJson"></pre>
          </div>
        </div>
      </section>
      <section id="mcp">
        <div class="grid">
          <div class="panel">
            <h2>MCP Surface</h2>
            <p class="muted">POST JSON-RPC to <code>/mcp</code>. Use <code>Authorization: Bearer $COAT_CONTROL_MCP_TOKEN</code> when configured.</p>
            <pre>{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/list",
  "params": {}
}</pre>
          </div>
          <div class="panel">
            <h2>Engine Boundary</h2>
            <p class="muted">The SPA and MCP gateway only read projections and submit workflow signals. Durable state, task creation, validation, approval waits, and runner dispatch stay in the Rust/Restate engine.</p>
          </div>
        </div>
      </section>
    </div>
  </main>
  <script type="module" src="/app.js"></script>
</body>
</html>`;
}

const server = http.createServer((req, res) => {
  void (async () => {
    const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "localhost"}`);
    if (req.method === "GET" && url.pathname === "/healthz") {
      sendJson(res, 200, { ok: true, service: "coat-control-plane-web" });
      return;
    }
    if (req.method === "GET" && url.pathname === "/") {
      sendText(res, 200, appHtml(), "text/html; charset=utf-8");
      return;
    }
    if (req.method === "GET" && url.pathname === "/app.js") {
      const script = await readFile(clientScriptUrl, "utf8");
      sendText(res, 200, script, "text/javascript; charset=utf-8");
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
    sendJson(res, 404, { error: "not found" });
  })().catch((error) => {
    sendJson(res, 500, { error: error instanceof Error ? error.message : String(error) });
  });
});

server.listen(port, host, () => {
  console.log(`coat-control-plane-web listening on http://${host}:${port}`);
});
