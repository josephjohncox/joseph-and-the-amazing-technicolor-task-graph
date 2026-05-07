/**
 * Browser client for the optional COAT control surface.
 *
 * Purpose: render backend state and send explicit steering/approval/planning
 * commands through the control gateway APIs. The browser stores only UI token
 * input locally and never owns coordinator truth.
 *
 * Architecture reference: docs/design-docs/110-control-gateway-spa.md
 */
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
bind("refreshFollowUps", refreshFollowUps);
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
  setJson("followUpsJson", payload);
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
  renderFollowUpsOverview(data);
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
  await loadPlanById(planId);
}

async function loadPlanById(planId: string): Promise<void> {
  const data = await api(`/api/plans/${encodeURIComponent(planId)}`);
  setJson("planJson", data);
  const continuity = await api(`/api/plans/${encodeURIComponent(planId)}/continuity`);
  renderPlanContinuity(continuity);
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
  if (planId) {
    await loadPlanById(planId);
  }
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
  await renderCurrentPlanContinuity();
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
  await renderCurrentPlanContinuity();
}

async function refreshFollowUps(): Promise<void> {
  const data = await api("/api/follow-ups");
  setJson("followUpsJson", data);
  renderFollowUps("followUpsView", data);
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

function renderFollowUpsOverview(data: JsonValue): void {
  renderFollowUps("followUpsOverview", at(data, ["follow_ups"]) ?? data, 8);
}

function renderFollowUps(targetId: string, data: JsonValue, maxItems?: number): void {
  const root = byId<HTMLElement>(targetId);
  const plans = arrayAt(data, ["plans"]);
  const rows: string[][] = [];
  for (const item of plans) {
    const plan = item as Record<string, unknown>;
    const title = String(plan.title ?? plan.path ?? "");
    const followUps = Array.isArray(plan.follow_ups) ? plan.follow_ups : [];
    for (const followUp of followUps) {
      rows.push([title, String(followUp)]);
    }
  }
  const total = Number(at(data, ["follow_up_count"]) ?? rows.length);
  const visibleRows = typeof maxItems === "number" ? rows.slice(0, maxItems) : rows;
  if (!rows.length) {
    root.innerHTML = `<p class="muted">No active execution-plan follow-ups.</p>`;
    return;
  }
  const suffix = visibleRows.length < rows.length ? ` Showing ${visibleRows.length}.` : "";
  root.innerHTML = `<p class="muted">${String(total)} open follow-up item${total === 1 ? "" : "s"}.${escapeHtml(suffix)}</p>${table(["Plan", "Follow-Up"], visibleRows)}`;
}

function renderPlanList(data: JsonValue): void {
  const rows = extractRows(at(data, ["data"]) ?? data);
  renderPlanTable("plansView", rows);
}

async function renderCurrentPlanContinuity(): Promise<void> {
  const planId = byId<HTMLInputElement>("planId").value.trim();
  if (!planId) {
    return;
  }
  renderPlanContinuity(await api(`/api/plans/${encodeURIComponent(planId)}/continuity`));
}

function renderPlanContinuity(data: JsonValue): void {
  const root = byId<HTMLElement>("planContinuityView");
  const record = data && typeof data === "object" ? data as Record<string, unknown> : {};
  if (record.found === false) {
    root.innerHTML = `<p class="muted">Plan continuity is unavailable.</p>`;
    return;
  }
  const continuity = at(data, ["continuity"]) as Record<string, unknown> | null;
  if (!continuity || typeof continuity !== "object") {
    root.innerHTML = `<p class="muted">Load a plan to inspect continuity state.</p>`;
    return;
  }
  const nextActions = arrayAt(continuity, ["next_actions"]);
  const openQuestions = arrayAt(continuity, ["open_questions"]);
  const authoringOpenQuestions = arrayAt(continuity, ["authoring_open_questions"]);
  const decisions = arrayAt(continuity, ["decisions"]);
  const subgoals = arrayAt(continuity, ["subgoals"]);
  const initialTasks = arrayAt(continuity, ["initial_tasks"]);
  const revisions = arrayAt(continuity, ["revisions"]);
  const html: string[] = [];
  html.push(`<p class="muted">${escapeHtml(String(record.title ?? record.plan_id ?? "plan"))} · ${escapeHtml(String(record.status ?? ""))} · v${escapeHtml(String(record.version ?? ""))}</p>`);
  html.push("<h3>Next Actions</h3>");
  html.push(listBlock(nextActions));
  html.push("<h3>Open Questions</h3>");
  html.push(openQuestions.length || authoringOpenQuestions.length
    ? table(["Question", "Required", "Answer"], [
      ...openQuestions.map((item) => {
        const row = item as Record<string, unknown>;
        return [String(row.question ?? ""), String(Boolean(row.required)), String(row.answer ?? "")];
      }),
      ...authoringOpenQuestions.map((item) => [String(item), "authoring", ""]),
    ])
    : `<p class="muted">No open planning questions.</p>`);
  html.push("<h3>Subgoals</h3>");
  html.push(subgoals.length
    ? table(["Color", "Subgoal", "Owner", "Priority", "Objective"], subgoals.map((item) => {
      const row = item as Record<string, unknown>;
      return [
        colorLabel(row.color),
        String(row.id ?? row.title ?? ""),
        String(row.owner_role ?? ""),
        String(row.priority ?? ""),
        truncate(String(row.objective ?? ""), 160),
      ];
    }))
    : `<p class="muted">No stable subgoals defined.</p>`);
  html.push("<h3>Initial Tasks</h3>");
  html.push(initialTasks.length
    ? table(["Task", "Role", "Subgoal", "Reason"], initialTasks.map((item) => {
      const row = item as Record<string, unknown>;
      return [
        String(row.title ?? ""),
        String(row.role ?? ""),
        String(row.subgoal_id ?? ""),
        truncate(String(row.reason ?? row.prompt ?? ""), 160),
      ];
    }))
    : `<p class="muted">No coordinator-owned initial tasks seeded.</p>`);
  html.push("<h3>Decisions</h3>");
  html.push(decisions.length
    ? table(["Decision", "Rationale"], decisions.map((item) => {
      const row = item as Record<string, unknown>;
      return [String(row.title ?? row.id ?? ""), truncate(String(row.rationale ?? row.decision ?? ""), 180)];
    }))
    : `<p class="muted">No planning decisions recorded.</p>`);
  html.push("<h3>Revisions</h3>");
  html.push(revisions.length
    ? table(["Version", "Author", "Summary", "Open Qs"], revisions.map((item) => {
      const row = item as Record<string, unknown>;
      return [
        String(row.version ?? ""),
        String(row.author ?? ""),
        truncate(String(row.summary ?? ""), 150),
        String(row.open_question_count ?? ""),
      ];
    }))
    : `<p class="muted">No revision history projected.</p>`);
  root.innerHTML = html.join("");
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
        await loadPlanById(planId);
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

function listBlock(items: JsonValue[]): string {
  if (!items.length) {
    return `<p class="muted">No next actions projected.</p>`;
  }
  return `<ul>${items.map((item) => `<li>${escapeHtml(String(item))}</li>`).join("")}</ul>`;
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
  header.innerHTML = `<tr>${["Color", "Task", "Role", "Purpose", "Status", "Prompt"].map((value) => `<th>${escapeHtml(value)}</th>`).join("")}</tr>`;
  tableEl.appendChild(header);
  const body = document.createElement("tbody");
  for (const item of rows) {
    const row = item as Record<string, unknown>;
    const payload = taskPayload(row);
    const taskId = String(row.task_id ?? payload.id ?? "");
    const prompt = String(row.current_prompt ?? row.prompt ?? payload.prompt ?? "");
    const tr = document.createElement("tr");
    tr.innerHTML = `<td>${colorChip(row.color ?? payload.color)}</td>${[
      taskId,
      String(row.role ?? payload.role ?? ""),
      String(row.purpose ?? row.purpose_kind ?? payload.purpose ?? ""),
      String(row.status ?? payload.status ?? ""),
      compact ? truncate(prompt, 110) : prompt,
    ].map((value) => `<td>${escapeHtml(value)}</td>`).join("")}`;
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

function colorLabel(value: JsonValue): string {
  const color = colorRecord(value);
  return color ? `${String(color.label ?? color.key ?? "")} ${String(color.hex ?? "")}`.trim() : "";
}

function colorChip(value: JsonValue): string {
  const color = colorRecord(value);
  if (!color) {
    return "";
  }
  const label = String(color.label ?? color.key ?? "");
  const meaning = String(color.meaning ?? "");
  const hex = safeHex(String(color.hex ?? ""));
  return `<span class="color-chip" title="${escapeAttr(meaning)}"><span class="color-dot" style="background:${escapeAttr(hex)}"></span>${escapeHtml(label)}</span>`;
}

function colorRecord(value: JsonValue): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function safeHex(value: string): string {
  return /^#[0-9a-fA-F]{6}$/.test(value) ? value : "#9aa6ad";
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

function escapeAttr(value: string): string {
  return escapeHtml(value).replace(/`/g, "&#096;");
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
