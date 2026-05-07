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
import { readFile, readdir } from "node:fs/promises";

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

type FollowUpItem = {
  plan: string;
  path: string;
  index: number;
  text: string;
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
  process.env.COAT_RUNNER_REGISTRY_URL ?? process.env.COAT_RUNNER_REGISTRY ?? "http://localhost:9085",
);
const memoryGatewayUrl = trimSlash(process.env.COAT_MEMORY_GATEWAY_URL ?? "http://localhost:9087");
const memoryGatewayToken = process.env.COAT_MEMORY_GATEWAY_TOKEN ?? process.env.MEMORY_GATEWAY_TOKEN ?? "";
const controlMcpToken = process.env.COAT_CONTROL_MCP_TOKEN ?? gatewayToken;
const chatCompletionsUrl = process.env.COAT_CONTROL_CHAT_COMPLETIONS_URL
  ?? (process.env.OPENAI_API_KEY && process.env.COAT_CONTROL_CHAT_MODEL ? "https://api.openai.com/v1/chat/completions" : "");
const chatModel = process.env.COAT_CONTROL_CHAT_MODEL ?? "";
const chatApiKey = process.env.COAT_CONTROL_CHAT_API_KEY ?? process.env.OPENAI_API_KEY ?? "";
const chatTemperature = Number(process.env.COAT_CONTROL_CHAT_TEMPERATURE ?? "0.2");
const executionPlanDirs = process.env.COAT_EXEC_PLAN_DIR
  ? [process.env.COAT_EXEC_PLAN_DIR]
  : ["docs/exec-plans/active", "../../docs/exec-plans/active", "/app/docs/exec-plans/active"];

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
const workflowReadHandlers = new Set(["status", "progress", "tasks"]);

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
  if (pathname.endsWith(".json")) return "application/json; charset=utf-8";
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
  const [plans, approvals, followUps] = await Promise.all([
    proxyJson(goalStoreUrl, "/goal-store/plans?limit=25", { method: "GET" }),
    proxyJson(goalStoreUrl, "/goal-store/approvals?limit=50", { method: "GET" }),
    executionPlanFollowUps(executionPlanDirs, false),
  ]);

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
    follow_ups: followUps,
  };
}

async function executionPlanFollowUps(planDirs: string[], includeEmpty: boolean): Promise<JsonMap> {
  const errors: JsonMap[] = [];
  for (const planDir of planDirs) {
    const result = await readExecutionPlanFollowUps(planDir, includeEmpty);
    if (!("error" in result)) {
      return { ...result, checked_plan_dirs: planDirs };
    }
    errors.push({
      plan_dir: result.plan_dir,
      error: result.error,
    });
  }
  return {
    plan_dir: planDirs[0] ?? "",
    checked_plan_dirs: planDirs,
    plan_count: 0,
    follow_up_count: 0,
    items: [],
    plans: [],
    error: errors.map((item) => `${item.plan_dir}: ${item.error}`).join("; "),
    errors,
  };
}

async function readExecutionPlanFollowUps(planDir: string, includeEmpty: boolean): Promise<JsonMap> {
  const normalizedDir = planDir.replace(/\/+$/, "") || ".";
  try {
    const entries = await readdir(normalizedDir, { withFileTypes: true });
    const files = entries
      .filter((entry) => entry.isFile() && entry.name.endsWith(".md"))
      .map((entry) => `${normalizedDir}/${entry.name}`)
      .sort();
    const plans: JsonMap[] = [];
    const items: FollowUpItem[] = [];
    let followUpCount = 0;
    for (const file of files) {
      const text = await readFile(file, "utf8");
      const followUps = extractFollowUps(text);
      const title = extractMarkdownTitle(text) ?? file;
      followUpCount += followUps.length;
      followUps.forEach((item, index) => {
        items.push({ plan: title, path: file, index, text: item });
      });
      if (includeEmpty || followUps.length > 0) {
        plans.push({
          path: file,
          title,
          follow_ups: followUps,
        });
      }
    }
    return {
      plan_dir: normalizedDir,
      plan_count: plans.length,
      follow_up_count: followUpCount,
      items,
      plans,
    };
  } catch (error) {
    return {
      plan_dir: normalizedDir,
      plan_count: 0,
      follow_up_count: 0,
      items: [],
      plans: [],
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function extractMarkdownTitle(text: string): string | null {
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("# ")) {
      const title = line.slice(2).trim();
      return title || null;
    }
  }
  return null;
}

function extractFollowUps(text: string): string[] {
  const items: string[] = [];
  let inSection = false;
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("## ")) {
      inSection = line.trim() === "## Follow-Ups";
      continue;
    }
    if (!inSection) {
      continue;
    }
    const trimmed = line.trim();
    if (trimmed.startsWith("- ") || trimmed.startsWith("* ")) {
      const item = trimmed.slice(2).trim();
      if (item) {
        items.push(item);
      }
    }
  }
  return items;
}

function followUpDraftPlan(payload: unknown): JsonMap {
  const item = followUpItemFromPayload(payload);
  return {
    mode: "draft_plan",
    item,
    prompt: followUpDraftPrompt(item),
  };
}

function followUpItemFromPayload(payload: unknown): FollowUpItem {
  const body = asRecord(payload);
  const source = Object.keys(asRecord(body.item)).length > 0 ? asRecord(body.item) : body;
  const text = String(source.text ?? source.follow_up ?? source.followup ?? "").trim();
  if (!text) {
    throw new Error("follow-up text is required");
  }
  const rawIndex = Number(source.index ?? source.follow_up_index ?? 0);
  return {
    plan: String(source.plan ?? source.title ?? source.source_plan ?? "Execution plan"),
    path: String(source.path ?? source.source_path ?? ""),
    index: Number.isFinite(rawIndex) && rawIndex >= 0 ? Math.floor(rawIndex) : 0,
    text,
  };
}

function followUpDraftPrompt(item: FollowUpItem): string {
  return `<task>
  <mode>draft_durable_plan</mode>
  <instruction>MUST turn this execution-plan follow-up into a concrete durable plan draft for COAT. MUST preserve the source plan and path. MUST propose subgoals, evidence requirements, budget/sandbox assumptions, review gates, and next implementation steps. MUST identify any questions that block execution.</instruction>
  <source_plan>${escapeXml(item.plan)}</source_plan>
  <source_path>${escapeXml(item.path)}</source_path>
  <follow_up_index>${item.index}</follow_up_index>
  <follow_up>${escapeXml(item.text)}</follow_up>
</task>`;
}

async function goalSnapshot(goalId: string): Promise<JsonMap> {
  const encodedGoalId = encodeURIComponent(goalId);
  const [goal, tasks, events, artifacts, checkpoints, approvals, workflowStatus, progress] = await Promise.all([
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/tasks`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/events`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/artifacts`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/checkpoints`, { method: "GET" }),
    proxyJson(goalStoreUrl, `/goal-store/goals/${encodedGoalId}/approvals`, { method: "GET" }),
    workflowReadPost(goalId, "status", {}),
    workflowReadPost(goalId, "progress", {}),
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
    checkpoints,
    approvals,
    agent_activity: buildAgentActivity(tasks.data, progress.data, events.data, artifacts.data),
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

function asRecord(value: unknown): JsonMap {
  return value && typeof value === "object" && !Array.isArray(value) ? value as JsonMap : {};
}

function arrayField(record: JsonMap, key: string): unknown[] {
  const value = record[key];
  return Array.isArray(value) ? value : [];
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

async function workflowReadPost(goalId: string, handler: string, body: unknown): Promise<ProxyResult> {
  return normalizeWorkflowReadResult(await workflowPost(goalId, handler, body), handler);
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
  if (result.status !== 404) {
    return result;
  }
  return {
    ...result,
    data: {
      unavailable: true,
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
  const messages = chatMessagesFrom(request.messages);
  if (!messages.length) {
    throw new Error("chat request requires at least one message");
  }
  const context = await chatContext(goalId);
  if (chatCompletionsUrl && chatModel) {
    return callChatModel(mode, messages, context);
  }
  return stubChat(mode, messages, context);
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
    context.goal_snapshot = await goalSnapshot(goalId);
  } catch (error) {
    context.goal_snapshot_error = error instanceof Error ? error.message : String(error);
  }
  return context;
}

async function callChatModel(mode: string, messages: ChatMessage[], context: JsonMap): Promise<JsonMap> {
  const response = await fetch(chatCompletionsUrl, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(chatApiKey ? { authorization: `Bearer ${chatApiKey}` } : {}),
    },
    body: JSON.stringify({
      model: chatModel,
      temperature: chatTemperature,
      messages: [
        {
          role: "system",
          content: controlChatSystemPrompt(mode, context),
        },
        ...messages,
      ],
    }),
  });
  const text = await response.text();
  const data = text ? safeJsonValue(text) : {};
  if (!response.ok) {
    throw new Error(`chat model request failed with ${response.status}: ${text}`);
  }
  const content = String(atRecord(data, ["choices", "0", "message", "content"]) ?? "");
  const parsed = parseChatAssistantPayload(content);
  return {
    provider: "openai_compatible",
    model: chatModel,
    mode,
    assistant: parsed.assistant,
    drafts: parsed.drafts,
    raw_model_response: parsed.raw_model_response,
    context,
  };
}

function controlChatSystemPrompt(mode: string, context: JsonMap): string {
  return [
    "<coat_chat_assistant>",
    "  <role>You are the COAT control-plane chat assistant.</role>",
    "  <mission>Help the operator author goals, durable plans, steering directives, memory notes, and review requests.</mission>",
    "  <authority>",
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
  const parsed = extractJsonObject(content);
  if (parsed) {
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

function extractJsonObject(content: string): JsonMap | null {
  const fenced = content.match(/```(?:json)?\s*([\s\S]*?)```/i);
  const candidate = fenced ? fenced[1] : content;
  const first = candidate.indexOf("{");
  const last = candidate.lastIndexOf("}");
  if (first < 0 || last <= first) {
    return null;
  }
  const parsed = safeJsonValue(candidate.slice(first, last + 1));
  return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed as JsonMap : null;
}

function stubChat(mode: string, messages: ChatMessage[], context: JsonMap): JsonMap {
  const latest = messages[messages.length - 1]?.content ?? "";
  const drafts = stubDrafts(mode, latest, context);
  const assistant = [
    "I can help draft the structured control-plane payloads from plain language.",
    chatCompletionsUrl && !chatModel
      ? "A chat completions URL is configured, but COAT_CONTROL_CHAT_MODEL is missing, so this response used the local stub."
      : "No live chat model is configured, so this response used the local stub.",
    "Review any draft, then use the existing form buttons to submit or steer durable work.",
  ].join(" ");
  return {
    provider: "stub",
    model: null,
    mode,
    assistant,
    drafts,
    context,
  };
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
      subgoals: [],
      distribution_notes: ["Coordinator owns task creation; workers may only request child tasks."],
    },
    root_budget: defaultBudget(),
    done_criteria: { tests_pass: true, artifact_exists: true, validator_score_min: 0.85 },
    initial_tasks: [
      {
        role: "planner",
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
        restate_admin: restateAdminUrl,
        execution_plan_dir: executionPlanDirs[0],
        execution_plan_dirs: executionPlanDirs,
      },
    });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/overview") {
    sendJson(res, 200, await overview());
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/chat") {
    sendJson(res, 200, await controlChat(await readJson(req)));
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/follow-ups") {
    const includeEmpty = url.searchParams.get("include_empty") === "true";
    sendJson(res, 200, await executionPlanFollowUps(executionPlanDirs, includeEmpty));
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/follow-ups/draft-plan") {
    sendJson(res, 200, followUpDraftPlan(await readJson(req)));
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

  if (req.method === "GET" && url.pathname === "/api/agents") {
    sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/tasks${url.search}`, { method: "GET" }));
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/approvals") {
    sendJson(res, 200, await proxyJson(goalStoreUrl, `/goal-store/approvals${url.search}`, { method: "GET" }));
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/goals/submit") {
    const { goalId, spec } = goalSpecWithId(await readJson(req));
    sendJson(res, 200, await workflowPost(goalId, "run", spec));
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
      const result = workflowReadHandlers.has(handler)
        ? await workflowReadPost(goalId, handler, body)
        : await workflowPost(goalId, handler, body);
      sendJson(res, 200, result);
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
  const record = body as Record<string, unknown>;
  const goalId = goalIdFromSpec(record) ?? crypto.randomUUID();
  return { goalId, spec: { ...record, id: goalId } };
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
      name: "coat_follow_ups",
      description: "List active execution-plan follow-up items that should continue across sessions.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        properties: { include_empty: { type: "boolean" } },
      },
    },
    {
      name: "coat_follow_up_draft_plan",
      description: "Turn one execution-plan follow-up item into the standard structured draft-plan prompt without mutating durable state.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["text"],
        properties: {
          plan: { type: "string" },
          path: { type: "string" },
          index: { type: "number" },
          text: { type: "string" },
        },
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
      name: "coat_goal_submit",
      description: "Submit a GoalSpec to GoalWorkflow/run. If id is omitted, the gateway assigns one before submission.",
      inputSchema: { type: "object", additionalProperties: true },
    },
    {
      name: "coat_approve_goal",
      description: "Approve or reject a durable HumanApproval request for a goal.",
      inputSchema: {
        type: "object",
        additionalProperties: false,
        required: ["goal_id", "approval_id", "approved"],
        properties: {
          goal_id: { type: "string" },
          approval_id: { type: "string" },
          approved: { type: "boolean" },
          note: { type: "string" },
        },
      },
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
      name: "coat_runner_list",
      description: "List registered runners and status from the runner registry.",
      inputSchema: { type: "object", additionalProperties: false, properties: { status: { type: "boolean" } } },
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
  if (name === "coat_follow_ups") {
    return executionPlanFollowUps(executionPlanDirs, args.include_empty === true);
  }
  if (name === "coat_follow_up_draft_plan") {
    return followUpDraftPlan(args);
  }
  if (name === "coat_steer_goal") {
    const goalId = String(args.goal_id ?? "");
    if (!goalId) {
      throw new Error("goal_id is required");
    }
    return workflowPost(goalId, "steer", args.directive ?? {});
  }
  if (name === "coat_goal_submit") {
    const { goalId, spec } = goalSpecWithId(args);
    return workflowPost(goalId, "run", spec);
  }
  if (name === "coat_approve_goal") {
    const goalId = String(args.goal_id ?? "");
    const approvalId = String(args.approval_id ?? "");
    if (!goalId || !approvalId) {
      throw new Error("goal_id and approval_id are required");
    }
    return workflowPost(goalId, "approve", {
      approval_id: approvalId,
      approved: args.approved === true,
      note: typeof args.note === "string" ? args.note : null,
    });
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
  if (name === "coat_runner_list") {
    return proxyJson(runnerRegistryUrl, args.status === false ? "/runners" : "/runners/status", { method: "GET" });
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
    sendJson(res, 500, { error: error instanceof Error ? error.message : String(error) });
  });
});

server.listen(port, host, () => {
  console.log(`coat-control-plane-web listening on http://${host}:${port}`);
});
