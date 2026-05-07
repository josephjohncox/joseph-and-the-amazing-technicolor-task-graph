type JsonValue = unknown;

const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>("nav button[data-tab]"));
const tokenInput = byId<HTMLInputElement>("apiToken");
const goalIdInput = byId<HTMLInputElement>("goalId");

tokenInput.value = localStorage.getItem("coat.control.token") ?? "";
tokenInput.addEventListener("input", () => localStorage.setItem("coat.control.token", tokenInput.value));

for (const tab of tabs) {
  tab.addEventListener("click", () => {
    for (const other of tabs) {
      other.classList.toggle("primary", other === tab);
    }
    for (const section of Array.from(document.querySelectorAll<HTMLElement>("section"))) {
      section.classList.toggle("active", section.id === tab.dataset.tab);
    }
  });
}

bind("refreshOverview", refreshOverview);
bind("refreshPlans", refreshPlans);
bind("loadPlan", loadPlan);
bind("draftPlan", draftPlan);
bind("revisePlan", revisePlan);
bind("compilePlan", compilePlan);
bind("refreshAgents", refreshAgents);
bind("loadGoal", loadGoal);
bind("submitGoal", submitGoal);
bind("sendSteer", () => workflowAction("steer", parseTextarea("steerJson")));
bind("goalProgress", () => workflowAction("progress", {}));
bind("goalStatus", () => workflowAction("status", {}));
bind("sendApprove", () => workflowAction("approve", parseTextarea("approvalJson")));
bind("cancelGoal", () => workflowAction("cancel", { reason: "cancelled from control gateway" }));
bind("refreshThreads", refreshThreads);
bind("refreshApprovals", refreshApprovals);
bind("refreshEvents", refreshEvents);
bind("memorySearch", () => memoryAction("search", parseTextarea("memorySearchJson")));
bind("memoryContext", () => memoryAction("context", parseTextarea("memorySearchJson")));
bind("memoryWrite", () => memoryAction("write", parseTextarea("memoryWriteJson")));

void refreshOverview();

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`missing #${id}`);
  }
  return element as T;
}

function bind(id: string, handler: () => Promise<void> | void): void {
  byId<HTMLButtonElement>(id).addEventListener("click", () => {
    Promise.resolve(handler()).catch((error) => showError(error));
  });
}

async function api(path: string, init: RequestInit = {}): Promise<JsonValue> {
  const response = await fetch(path, {
    ...init,
    headers: {
      ...(init.headers ?? {}),
      ...(tokenInput.value ? { authorization: `Bearer ${tokenInput.value}` } : {}),
    },
  });
  const text = await response.text();
  const data = text ? safeJson(text) : null;
  if (!response.ok) {
    throw new Error(typeof data === "object" && data && "error" in data ? String((data as Record<string, unknown>).error) : text);
  }
  return data;
}

function safeJson(text: string): JsonValue {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function parseTextarea(id: string): JsonValue {
  const text = byId<HTMLTextAreaElement>(id).value.trim();
  return text ? JSON.parse(text) : {};
}

function setJson(id: string, value: JsonValue): void {
  byId<HTMLElement>(id).textContent = JSON.stringify(value, null, 2);
}

function showError(error: unknown): void {
  const payload = { error: error instanceof Error ? error.message : String(error) };
  setJson("overviewJson", payload);
  setJson("goalJson", payload);
}

async function refreshOverview(): Promise<void> {
  const data = await api("/api/overview");
  setJson("overviewJson", data);
  renderServiceStrip(data);
  renderServices(data);
  renderRunners(data);
  renderThreadsSummary(data);
  renderApprovalsOverview(data);
  renderPlansOverview(data);
  renderGoals(data);
  renderAgentsOverview(data);
}

async function refreshPlans(): Promise<void> {
  const data = await api("/api/plans?limit=100");
  setJson("planJson", data);
  renderPlanList(data);
}

async function loadPlan(): Promise<void> {
  const planId = byId<HTMLInputElement>("planId").value.trim();
  if (!planId) {
    throw new Error("plan ID is required");
  }
  const data = await api(`/api/plans/${encodeURIComponent(planId)}`);
  setJson("planJson", data);
}

async function draftPlan(): Promise<void> {
  const body = parseTextarea("planDraftJson") as Record<string, unknown>;
  const result = await api("/api/plans", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const planId = String(at(result, ["data", "plan", "id"]) ?? at(result, ["plan", "id"]) ?? "");
  if (planId) {
    byId<HTMLInputElement>("planId").value = planId;
  }
  setJson("planJson", result);
  await refreshPlans();
}

async function revisePlan(): Promise<void> {
  const planId = byId<HTMLInputElement>("planId").value.trim();
  if (!planId) {
    throw new Error("plan ID is required");
  }
  const result = await api(`/api/plans/${encodeURIComponent(planId)}/revisions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(parseTextarea("planRevisionJson")),
  });
  setJson("planJson", result);
}

async function compilePlan(): Promise<void> {
  const planId = byId<HTMLInputElement>("planId").value.trim();
  if (!planId) {
    throw new Error("plan ID is required");
  }
  const result = await api(`/api/plans/${encodeURIComponent(planId)}/compile`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      plan_id: planId,
      strict_review: true,
      human_steered: true,
      enable_branching: false,
    }),
  });
  setJson("planJson", result);
}

async function refreshAgents(): Promise<void> {
  const goalId = byId<HTMLInputElement>("agentGoalFilter").value.trim();
  const path = goalId ? `/api/agents?goal_id=${encodeURIComponent(goalId)}&limit=200` : "/api/agents?limit=200";
  const data = await api(path);
  const rows = extractRows(at(data, ["data"]) ?? data);
  renderAgentTable("agentsView", rows);
  setJson("agentDetailJson", data);
}

async function loadGoal(): Promise<void> {
  const goalId = goalIdInput.value.trim();
  if (!goalId) {
    throw new Error("goal ID is required");
  }
  const data = await api(`/api/goals/${encodeURIComponent(goalId)}`);
  setJson("goalJson", data);
  renderGoalTables(data);
  renderGoalAgents(data);
}

async function submitGoal(): Promise<void> {
  const body = parseTextarea("goalSubmitJson") as Record<string, unknown>;
  const goalId = String(body.goal_id ?? body.id ?? "");
  if (goalId) {
    goalIdInput.value = goalId;
  }
  const result = await api("/api/goals/submit", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  setJson("goalJson", result);
  if (goalId) {
    await loadGoal();
  }
}

async function workflowAction(handler: string, body: JsonValue): Promise<void> {
  const goalId = goalIdInput.value.trim();
  if (!goalId) {
    throw new Error("goal ID is required");
  }
  const result = await api(`/api/goals/${encodeURIComponent(goalId)}/${handler}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  setJson("goalJson", result);
  if (handler !== "cancel") {
    await loadGoal();
  }
}

async function refreshThreads(): Promise<void> {
  const data = await api("/api/human/threads");
  renderThreads(data);
  setJson("threadDetail", data);
}

async function refreshApprovals(): Promise<void> {
  const data = await api("/api/approvals?limit=100");
  renderApprovals("approvalsView", extractRows(at(data, ["data"]) ?? data));
  setJson("threadDetail", data);
}

async function refreshEvents(): Promise<void> {
  const [sources, triggers, events] = await Promise.all([
    api("/api/events/sources"),
    api("/api/events/triggers"),
    api("/api/events"),
  ]);
  setJson("eventSourcesJson", sources);
  setJson("eventTriggersJson", triggers);
  setJson("eventsJson", events);
}

async function memoryAction(action: string, body: JsonValue): Promise<void> {
  const result = await api(`/api/memory/${action}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  setJson("memoryJson", result);
}

function renderServiceStrip(data: JsonValue): void {
  const root = byId<HTMLElement>("serviceStrip");
  const services = arrayAt(data, ["services"]);
  root.innerHTML = "";
  for (const service of services) {
    const record = service as Record<string, unknown>;
    const pill = document.createElement("span");
    const ok = Boolean(record.ok);
    pill.className = `pill ${ok ? "good" : "bad"}`;
    pill.textContent = `${record.name ?? "service"} ${ok ? "ok" : "down"}`;
    root.appendChild(pill);
  }
}

function renderServices(data: JsonValue): void {
  const services = arrayAt(data, ["services"]);
  byId<HTMLElement>("servicesView").innerHTML = table(["Service", "Status", "Code"], services.map((item) => {
    const row = item as Record<string, unknown>;
    return [String(row.name ?? ""), row.ok ? "ok" : "down", String(row.status ?? "")];
  }));
}

function renderRunners(data: JsonValue): void {
  const runnerStatus = at(data, ["runner_status", "data"]);
  const runners = Array.isArray(runnerStatus) ? runnerStatus : arrayAt(runnerStatus, ["runners"]);
  byId<HTMLElement>("runnersView").innerHTML = runners.length
    ? table(["Runner", "Node", "Capacity"], runners.map((item) => {
      const row = item as Record<string, unknown>;
      return [
        String(row.runner_id ?? row.id ?? ""),
        String(row.node_id ?? ""),
        String(row.capacity_remaining ?? row.running_tasks ?? ""),
      ];
    }))
    : `<p class="muted">No runner status projection available.</p>`;
}

function renderThreadsSummary(data: JsonValue): void {
  const threads = extractThreadList(at(data, ["human_threads", "data"]));
  byId<HTMLElement>("threadsSummary").innerHTML = threads.length
    ? table(["Thread", "Entries"], threads.map((item) => {
      const row = item as Record<string, unknown>;
      return [String(row.thread_key ?? row.key ?? ""), String(row.entries ?? row.reports ?? "")];
    }))
    : `<p class="muted">No local human-feedback threads.</p>`;
}

function renderApprovalsOverview(data: JsonValue): void {
  const rows = extractRows(at(data, ["approvals", "data"]) ?? at(data, ["approvals"]));
  renderApprovals("approvalsOverview", rows.slice(0, 20));
}

function renderApprovals(targetId: string, rows: JsonValue[]): void {
  const root = byId<HTMLElement>(targetId);
  if (!rows.length) {
    root.innerHTML = `<p class="muted">No projected approval records.</p>`;
    return;
  }
  const tableEl = document.createElement("table");
  const header = document.createElement("thead");
  header.innerHTML = `<tr>${["Approval", "Goal", "Task", "Status", "Risk"].map((value) => `<th>${escapeHtml(value)}</th>`).join("")}</tr>`;
  tableEl.appendChild(header);
  const body = document.createElement("tbody");
  for (const item of rows) {
    const row = item as Record<string, unknown>;
    const tr = document.createElement("tr");
    tr.innerHTML = [
      String(row.approval_id ?? ""),
      String(row.goal_id ?? ""),
      String(row.task_id ?? ""),
      String(row.status ?? ""),
      String(row.risk ?? ""),
    ].map((value) => `<td>${escapeHtml(value)}</td>`).join("");
    tr.addEventListener("click", () => setJson("threadDetail", row));
    body.appendChild(tr);
  }
  tableEl.appendChild(body);
  root.innerHTML = "";
  root.appendChild(tableEl);
}

function renderPlansOverview(data: JsonValue): void {
  const rows = extractRows(at(data, ["plans", "data"]) ?? at(data, ["plans"]));
  renderPlanTable("plansOverview", rows.slice(0, 20));
}

function renderPlanList(data: JsonValue): void {
  const rows = extractRows(at(data, ["data"]) ?? data);
  renderPlanTable("plansView", rows);
}

function renderPlanTable(targetId: string, rows: JsonValue[]): void {
  const root = byId<HTMLElement>(targetId);
  if (!rows.length) {
    root.innerHTML = `<p class="muted">No durable plans projected yet.</p>`;
    return;
  }
  const tableEl = document.createElement("table");
  const header = document.createElement("thead");
  header.innerHTML = `<tr>${["Plan", "Status", "Mode", "Subgoals", "Questions"].map((value) => `<th>${escapeHtml(value)}</th>`).join("")}</tr>`;
  tableEl.appendChild(header);
  const body = document.createElement("tbody");
  for (const item of rows) {
    const row = item as Record<string, unknown>;
    const planId = String(row.plan_id ?? row.id ?? "");
    const tr = document.createElement("tr");
    tr.innerHTML = [
      String(row.title ?? planId),
      String(row.status ?? ""),
      String(row.mode ?? ""),
      String(row.subgoal_count ?? ""),
      String(row.open_question_count ?? ""),
    ].map((value) => `<td>${escapeHtml(value)}</td>`).join("");
    tr.addEventListener("click", async () => {
      if (planId) {
        byId<HTMLInputElement>("planId").value = planId;
        setJson("planJson", await api(`/api/plans/${encodeURIComponent(planId)}`));
      } else {
        setJson("planJson", row);
      }
    });
    body.appendChild(tr);
  }
  tableEl.appendChild(body);
  root.innerHTML = "";
  root.appendChild(tableEl);
}

function renderGoals(data: JsonValue): void {
  const rows = extractRows(at(data, ["goals", "data"]) ?? at(data, ["goals"]));
  byId<HTMLElement>("goalsView").innerHTML = rows.length
    ? table(["Goal", "Status", "Done", "Open"], rows.map((item) => {
      const row = item as Record<string, unknown>;
      return [
        String(row.title ?? row.goal_id ?? ""),
        String(row.status ?? ""),
        String(Math.round(Number(row.percent_done ?? 0) * 100)) + "%",
        String(row.open_tasks ?? ""),
      ];
    }))
    : `<p class="muted">No goals projected yet.</p>`;
}

function renderAgentsOverview(data: JsonValue): void {
  const rows = extractRows(at(data, ["agents", "data"]) ?? at(data, ["agents"]));
  renderAgentTable("agentsOverview", rows.slice(0, 20), true);
}

function renderThreads(data: JsonValue): void {
  const threads = extractThreadList(at(data, ["data"]) ?? data);
  const root = byId<HTMLElement>("threadsView");
  root.innerHTML = "";
  if (!threads.length) {
    root.innerHTML = `<p class="muted">No local human-feedback threads.</p>`;
    return;
  }
  for (const thread of threads) {
    const row = thread as Record<string, unknown>;
    const button = document.createElement("button");
    button.textContent = String(row.thread_key ?? row.key ?? "thread");
    button.addEventListener("click", async () => {
      const key = String(row.thread_key ?? row.key ?? "");
      if (!key) {
        setJson("threadDetail", thread);
        return;
      }
      setJson("threadDetail", await api(`/api/human/threads/${encodeURIComponent(key)}`));
    });
    root.appendChild(button);
  }
}

function renderGoalTables(data: JsonValue): void {
  const tasks = extractRows(at(data, ["tasks", "data"]));
  const events = extractRows(at(data, ["events", "data"]));
  const html: string[] = [];
  html.push("<h3>Tasks</h3>");
  html.push(tasks.length
    ? table(["Task", "Role", "Status", "Purpose"], tasks.slice(0, 20).map((item) => {
      const row = item as Record<string, unknown>;
      return [
        String(row.task_id ?? row.id ?? ""),
        String(row.role ?? row.worker_kind ?? ""),
        String(row.status ?? ""),
        String(row.purpose ?? ""),
      ];
    }))
    : `<p class="muted">No task projection found.</p>`);
  html.push("<h3>Events</h3>");
  html.push(events.length
    ? table(["Type", "Task", "At"], events.slice(0, 20).map((item) => {
      const row = item as Record<string, unknown>;
      return [
        String(row.event_type ?? row.kind ?? ""),
        String(row.task_id ?? ""),
        String(row.occurred_at ?? row.created_at ?? ""),
      ];
    }))
    : `<p class="muted">No goal events found.</p>`);
  byId<HTMLElement>("goalTables").innerHTML = html.join("");
}

function renderGoalAgents(data: JsonValue): void {
  const rows = arrayAt(data, ["agent_activity"]);
  renderAgentTable("goalAgentActivity", rows);
}

function renderAgentTable(targetId: string, rows: JsonValue[], compact = false): void {
  const root = byId<HTMLElement>(targetId);
  if (!rows.length) {
    root.innerHTML = `<p class="muted">No projected agent activity.</p>`;
    return;
  }
  const tableEl = document.createElement("table");
  const header = document.createElement("thead");
  header.innerHTML = `<tr>${["Task", "Role", "Purpose", "Status", "Prompt"].map((value) => `<th>${escapeHtml(value)}</th>`).join("")}</tr>`;
  tableEl.appendChild(header);
  const body = document.createElement("tbody");
  for (const item of rows) {
    const row = item as Record<string, unknown>;
    const payload = taskPayload(row);
    const taskId = String(row.task_id ?? payload.id ?? "");
    const prompt = String(row.current_prompt ?? row.prompt ?? payload.prompt ?? "");
    const tr = document.createElement("tr");
    tr.innerHTML = [
      taskId,
      String(row.role ?? payload.role ?? ""),
      String(row.purpose ?? row.purpose_kind ?? payload.purpose ?? ""),
      String(row.status ?? payload.status ?? ""),
      compact ? truncate(prompt, 110) : prompt,
    ].map((value) => `<td>${escapeHtml(value)}</td>`).join("");
    tr.addEventListener("click", () => setJson("agentDetailJson", row));
    body.appendChild(tr);
  }
  tableEl.appendChild(body);
  root.innerHTML = "";
  root.appendChild(tableEl);
}

function taskPayload(row: Record<string, unknown>): Record<string, unknown> {
  const rawTask = row.raw_task;
  if (rawTask && typeof rawTask === "object") {
    const payload = (rawTask as Record<string, unknown>).payload_json;
    if (payload && typeof payload === "object") {
      return payload as Record<string, unknown>;
    }
  }
  const payload = row.payload_json;
  if (payload && typeof payload === "object") {
    return payload as Record<string, unknown>;
  }
  return {};
}

function truncate(value: string, max: number): string {
  if (value.length <= max) {
    return value;
  }
  return `${value.slice(0, max - 3)}...`;
}

function table(headers: string[], rows: string[][]): string {
  return `<table><thead><tr>${headers.map((header) => `<th>${escapeHtml(header)}</th>`).join("")}</tr></thead><tbody>${rows
    .map((row) => `<tr>${row.map((cell) => `<td>${escapeHtml(cell)}</td>`).join("")}</tr>`)
    .join("")}</tbody></table>`;
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#039;",
  }[char] ?? char));
}

function at(value: JsonValue, path: string[]): JsonValue {
  let current = value;
  for (const key of path) {
    if (!current || typeof current !== "object") {
      return null;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

function arrayAt(value: JsonValue, path: string[]): JsonValue[] {
  const item = path.length ? at(value, path) : value;
  return Array.isArray(item) ? item : [];
}

function extractRows(value: JsonValue): JsonValue[] {
  if (Array.isArray(value)) {
    return value;
  }
  if (value && typeof value === "object") {
    for (const key of ["tasks", "goals", "plans", "approvals", "events", "items", "records"]) {
      const maybe = (value as Record<string, unknown>)[key];
      if (Array.isArray(maybe)) {
        return maybe;
      }
    }
  }
  return [];
}

function extractThreadList(value: JsonValue): JsonValue[] {
  if (Array.isArray(value)) {
    return value;
  }
  if (value && typeof value === "object") {
    for (const key of ["threads", "items", "entries"]) {
      const maybe = (value as Record<string, unknown>)[key];
      if (Array.isArray(maybe)) {
        return maybe;
      }
    }
  }
  return [];
}
