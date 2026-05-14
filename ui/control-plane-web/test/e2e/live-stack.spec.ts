import { expect, test, type Page } from "@playwright/test";

test.describe("COAT live deterministic stack", () => {
  test.skip(process.env.COAT_CONTROL_E2E_LIVE !== "1", "set COAT_CONTROL_E2E_LIVE=1 to run against the local Compose gateway");

  test("submits a chat-authored goal and observes the goal-store projection", async ({ page }) => {
    test.setTimeout(90_000);
    const title = `Live UI projection ${Date.now()}`;
    const objective = `${title}: prove the browser can draft, submit, select, and observe a real goal-store projection through the control gateway.`;

    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
    await expect(page.locator(".outcome-meta")).toContainText("Context: workspace");

    await expect(page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Goal" })).toHaveClass(/active/);
    await sendComposerMessage(page, objective);
    await expect(page.getByText("Goal draft ready", { exact: true })).toBeVisible();

    await page.locator(".goal-draft-editor").getByLabel("Title").fill(title);
    await page.locator(".goal-draft-editor").getByLabel("Objective").fill(objective);
    await page.getByRole("button", { name: "Accept draft" }).click();

    const goalId = await selectedGoalIdFromUrl(page);
    expect(goalId).toMatch(/^[0-9a-f-]{36}$/);
    await expect(page.locator(".goal-context-trigger")).toContainText(title);
    await expect(page.getByRole("heading", { name: "Work graph", exact: true })).toBeVisible();

    await expect.poll(async () => goalListContains(page, goalId, title), {
      message: "goal list should contain the submitted goal after goal-store projection",
      timeout: 60_000,
      intervals: [500, 1_000, 2_000],
    }).toBe(true);

    await expect.poll(async () => goalSnapshotHasProjectedWork(page, goalId), {
      message: "goal snapshot should expose projected tasks or compute graph nodes",
      timeout: 60_000,
      intervals: [500, 1_000, 2_000],
    }).toBe(true);

    await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name: "Goals", exact: true }).click();
    await expect(page.getByText(title).first()).toBeVisible();
    await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name: "Work Graph", exact: true }).click();
    await expect(page.locator(".goal-context-trigger")).toContainText(title);
    await expect(page.locator(".graph-status-panel")).toBeVisible();

    const memoryKey = `live-ui-memory-${Date.now()}`;
    const replacementKey = `${memoryKey}-reviewed`;
    await seedMemory(page, goalId, memoryKey, title);

    await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name: "Memory", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Search shared memory" })).toBeVisible();
    await expect.poll(async () => memoryEventsContain(page, goalId, memoryKey), {
      message: "memory write should be visible through the gateway memory events route",
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    }).toBe(true);
    await page.getByLabel("Replace keys").fill(memoryKey);
    await page.getByLabel("Replacement key").fill(replacementKey);
    await page.getByLabel("Replacement title").fill(`Reviewed ${title}`);
    await page.getByLabel("Replacement content").fill("Reviewed memory content created by the live-stack browser proof.");
    await page.getByLabel("Reason").fill("Prove memory preview and apply are routed through the real backend.");
    await page.getByRole("button", { name: "Preview diff" }).click();
    await expect(page.locator(".panel").filter({ has: page.getByRole("heading", { name: "Replacement diff" }) })).toContainText("Ready");
    await page.getByRole("button", { name: "Apply edit" }).click();
    await expect.poll(async () => memoryEventsContain(page, goalId, replacementKey), {
      message: "memory replacement should be visible through the gateway memory events route",
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    }).toBe(true);
    await expect(page.getByText(replacementKey).first()).toBeVisible();

    await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name: "Human Queue", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Approvals" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Feedback threads" })).toBeVisible();

    await expect.poll(async () => runnerCount(page), {
      message: "deterministic stack should expose registered runners through the gateway",
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    }).toBeGreaterThan(0);
    await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name: "Runners", exact: true }).click();
    await expect(page.getByRole("heading", { level: 2, name: "Runner fleet", exact: true })).toBeVisible();
    await expect(page.getByLabel("Runner fleet state")).toContainText("registered");

    const sourceId = `live-ui-ci-${Date.now()}`;
    await registerEventSource(page, sourceId);
    await page.reload();
    await expect(page.locator(".goal-context-trigger")).toContainText(title);
    await expect.poll(async () => eventSourcesContain(page, sourceId), {
      message: "registered event source should be visible through the gateway event route",
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    }).toBe(true);
    await expect(page.getByRole("heading", { name: "Event sources" })).toBeVisible();
    await expect(page.getByText(sourceId).first()).toBeVisible();
  });
});

async function sendComposerMessage(page: Page, text: string): Promise<void> {
  const editor = page.locator(".cs-message-input__content-editor");
  await editor.click();
  await editor.pressSequentially(text);
  await page.locator(".cs-message-input__tools button:not([disabled])").click();
}

async function selectedGoalIdFromUrl(page: Page): Promise<string> {
  await expect(page).toHaveURL(/goal=/, { timeout: 60_000 });
  const url = new URL(page.url());
  return url.searchParams.get("goal") ?? "";
}

async function goalListContains(page: Page, goalId: string, title: string): Promise<boolean> {
  const response = await page.evaluate(async () => {
    const result = await fetch("/api/goals?limit=100");
    return result.json();
  });
  const text = JSON.stringify(response);
  return text.includes(goalId) && text.includes(title);
}

async function goalSnapshotHasProjectedWork(page: Page, goalId: string): Promise<boolean> {
  const response = await page.evaluate(async (selectedGoalId) => {
    const result = await fetch(`/api/goals/${encodeURIComponent(selectedGoalId)}`);
    return result.json();
  }, goalId);
  const text = JSON.stringify(response);
  if (!text.includes(goalId)) {
    return false;
  }
  const record = response && typeof response === "object" ? response as Record<string, unknown> : {};
  const tasks = rowsFrom((record.tasks as Record<string, unknown> | undefined)?.data ?? record.tasks);
  const agentActivity = Array.isArray(record.agent_activity) ? record.agent_activity : [];
  const workflowGraph = record.workflow_compute_graph && typeof record.workflow_compute_graph === "object"
    ? record.workflow_compute_graph as Record<string, unknown>
    : {};
  const graphData = workflowGraph.data && typeof workflowGraph.data === "object"
    ? workflowGraph.data as Record<string, unknown>
    : workflowGraph;
  const nodes = rowsFrom(graphData.nodes);
  return tasks.length > 0 || agentActivity.length > 0 || nodes.length > 0;
}

function rowsFrom(value: unknown): unknown[] {
  if (Array.isArray(value)) {
    return value;
  }
  if (!value || typeof value !== "object") {
    return [];
  }
  const record = value as Record<string, unknown>;
  for (const key of ["tasks", "nodes", "goals", "items", "records", "rows", "data"]) {
    const rows = rowsFrom(record[key]);
    if (rows.length) {
      return rows;
    }
  }
  return [];
}

async function seedMemory(page: Page, goalId: string, key: string, title: string): Promise<void> {
  await apiJson(page, "/api/memory/write", {
    method: "POST",
    body: {
      goal_id: goalId,
      task_id: null,
      scope: "goal",
      key,
      episode: {
        title: `Candidate ${title}`,
        content: "Candidate memory content created by the live-stack browser proof.",
        source: {
          source_type: "human",
          uri: null,
          actor: "playwright",
        },
        artifacts: [],
        tags: ["live-stack", "ui-e2e"],
      },
      store: null,
    },
  });
}

async function registerEventSource(page: Page, sourceId: string): Promise<void> {
  await apiJson(page, "/api/events/sources", {
    method: "POST",
    body: {
      id: sourceId,
      kind: "ci",
      enabled: true,
      description: "Live UI E2E CI signal source.",
      namespace: "coat-live-ui",
      webhook: null,
      generic: {
        auth: {
          kind: "none",
          secret_ref: null,
          header_name: null,
        },
        accepts_cloudevents: true,
        max_payload_bytes: 1048576,
        allowed_event_types: ["ci.workflow.failed", "ci.workflow.completed"],
        id_json_pointer: "/id",
        type_json_pointer: "/type",
        subject_json_pointer: "/subject",
        dedupe_json_pointer: "/delivery_id",
        dedupe_header: "ce-id",
        payload_schema: null,
        mcp_context: null,
      },
      schedule: null,
      calendar: null,
      route: {
        mode: "human_review",
        goal_template: null,
        target_goal_id: null,
        steering_directive: null,
        require_approval: true,
        dedupe_window_seconds: 3600,
      },
    },
  });
}

async function apiJson(page: Page, path: string, options: { method?: string; body?: unknown } = {}): Promise<unknown> {
  return page.evaluate(async ({ requestPath, method, body }) => {
    const response = await fetch(requestPath, {
      method,
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const payload = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new Error(`${requestPath} returned ${response.status}: ${JSON.stringify(payload)}`);
    }
    return payload;
  }, { requestPath: path, method: options.method ?? "GET", body: options.body });
}

async function memoryEventsContain(page: Page, goalId: string, key: string): Promise<boolean> {
  const response = await apiJson(page, `/api/memory/events/${encodeURIComponent(goalId)}`);
  return JSON.stringify(response).includes(key);
}

async function runnerCount(page: Page): Promise<number> {
  const response = await apiJson(page, "/api/runners");
  return rowsFrom((response as Record<string, unknown>).data ?? response).length;
}

async function eventSourcesContain(page: Page, sourceId: string): Promise<boolean> {
  const response = await apiJson(page, "/api/events/sources");
  return JSON.stringify(response).includes(sourceId);
}
