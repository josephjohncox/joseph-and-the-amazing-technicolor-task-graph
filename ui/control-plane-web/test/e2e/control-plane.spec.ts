import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page, type Route } from "@playwright/test";

const selectedGoalStorageKey = "coat.selectedGoalId";
const themeStorageKey = "coat.theme";
const goalA = "018f8f2f-1fd8-7688-bb12-8bfb6b756602";
const goalB = "018f8f2f-1fd8-7688-bb12-8bfb6b756603";
const submittedGoal = "018f8f2f-1fd8-7688-bb12-8bfb6b756604";

type JsonRecord = Record<string, unknown>;

type FixtureState = {
  actions: Array<{ handler: string; body: JsonRecord }>;
  cancelledGoals: Set<string>;
  freezeChatSession: boolean;
  requestCounts: Record<string, number>;
  submittedGoalSpec: JsonRecord | null;
};

test.describe("COAT control-plane browser flows", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(([storageKey, themeKey]) => {
      window.localStorage.clear();
      window.localStorage.removeItem(storageKey);
      window.localStorage.setItem(themeKey, "dark");
    }, [selectedGoalStorageKey, themeStorageKey]);
  });

  test("asks by default, edits or discards goal drafts, submits, and refreshes the selected goal", async ({ page }) => {
    const state = await installGatewayFixtures(page);

    await page.goto("/");
    await expectNoAmbiguousLaneCopy(page);
    await expect(page.locator(".outcome-meta")).toContainText("Assistant");
    await expect(page.locator(".outcome-meta")).toContainText("Context: workspace");
    await expect(page.locator(".outcome-meta")).toContainText("History: Workspace chat");
    await expect(page.getByRole("heading", { name: "Runs" })).toBeVisible();
    await expectNoCriticalOrSeriousAxeViolations(page, "initial operator console");

    await expect(page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Ask" })).toHaveClass(/active/);
    await expect(page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Draft plan" })).toBeVisible();
    await expect(page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Search" })).toBeVisible();
    await sendComposerMessage(page, "What is blocked and what should I do next?");
    await expect(page.getByText("The selected goal has one action needed item.")).toBeVisible();
    await expect(page.locator(".draft-review-dock")).toBeHidden();
    await expect(page.getByText("Goal draft ready", { exact: true })).toHaveCount(0);

    await page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Draft goal" }).click();
    await sendComposerMessage(page, "Discard this browser E2E goal draft after review.");

    await expect(page.locator(".outcome-meta")).toContainText("Draft: Goal draft");
    await expect(page.locator(".draft-review-dock")).toContainText("Submitted browser E2E task");
    await expect(page.getByText("Goal draft ready", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Review draft" })).toHaveCount(0);
    await expect(page.locator(".draft-summary-card")).toContainText("Submitted browser E2E task");
    await expect(page.locator(".draft-summary-card")).toContainText("Ref browser-e2e");
    await expect(page.locator(".draft-summary-card")).not.toContainText("fixture://drafts/browser-e2e");
    await expect(page.locator(".draft-summary-card")).toContainText("2 evidence items");
    await expect(page.locator(".draft-summary-card")).toContainText("1 constraint");
    await expectNoCliCoverageOrDebugInventory(page);
    await expect(page.getByText("acceptance_evidence")).toHaveCount(0);
    await page.getByLabel("Active draft").getByRole("button", { name: "Discard draft" }).click();
    await expect(page.locator(".draft-review-dock")).toBeHidden();

    await page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Draft goal" }).click();
    await sendComposerMessage(page, "Submit browser E2E task with deterministic mocked gateway evidence.");
    await expect(page.getByText("Goal draft ready", { exact: true })).toBeVisible();
    const draftEditor = page.locator(".goal-draft-editor");
    await expect(draftEditor).toContainText("Draft ready");
    await draftEditor.getByLabel("Objective").fill("Edited browser E2E task with deterministic mocked gateway evidence.");
    await page.getByLabel("Draft actions").getByRole("button", { name: "Accept draft" }).click();

    await expect(page).toHaveURL(new RegExp(`goal=${submittedGoal}`));
    await expect.poll(() => state.requestCounts.goals ?? 0).toBeGreaterThanOrEqual(2);
    await expect.poll(() => state.requestCounts[`goal:${submittedGoal}`] ?? 0).toBeGreaterThanOrEqual(1);
    await expect(page.locator(".outcome-meta")).toContainText("Context: Submitted browser E2E task");
    await expect(page.locator(".draft-review-dock")).toBeHidden();
    await expect(page.getByText("Submitted goal is syncing")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Work graph", exact: true })).toBeVisible();
    await expect(page.getByText(submittedGoal, { exact: true })).toHaveCount(0);
    await expect(page.locator(".goal-context-trigger")).toContainText("Submitted browser E2E task");
    await expect.poll(() => state.requestCounts[`goal:${submittedGoal}`] ?? 0).toBeGreaterThanOrEqual(3);
    await openPrimaryNav(page, "Work Graph");
    await expect(subgoalCard(page, "Validate mocked gateway fixtures")).toBeVisible();
    await page.evaluate((storageKey) => {
      window.localStorage.removeItem(storageKey);
      window.history.pushState({}, "", "/");
      window.dispatchEvent(new PopStateEvent("popstate"));
    }, selectedGoalStorageKey);
    await expect(page.locator(".outcome-meta")).toContainText("History: Workspace chat");
    await expect(page.locator(".draft-review-dock")).toBeHidden();
    expect(state.submittedGoalSpec?.objective).toContain("Edited browser E2E task");
  });

  test("switches selected goals from the top bar and updates subgoal visibility", async ({ page }) => {
    await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    await expect(page.locator(".outcome-meta")).toContainText("Context: Baseline durable goal");
    await expect(page.locator(".outcome-meta")).toContainText("History: Selected goal chat");
    await openPrimaryNav(page, "Work Graph");
    await expect(subgoalCard(page, "Coordinator truth boundary")).toBeVisible();

    await page.locator(".goal-context-trigger").click();
    await page.getByText("Branch visibility goal").click();

    await expect(page).toHaveURL(new RegExp(`goal=${goalB}`));
    await expect(page.locator(".goal-context-trigger")).toContainText("Branch visibility goal");
    await expect(subgoalCard(page, "Branch fanout review")).toBeVisible();
    await expect(subgoalCard(page, "Coordinator truth boundary")).toBeHidden();
  });

  test("supports keyboard focus for goal picker, chat drafting, and flow controls", async ({ page }) => {
    const state = await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    const goalPickerTrigger = page.locator(".goal-context-trigger");
    await goalPickerTrigger.focus();
    await expect(goalPickerTrigger).toBeFocused();
    await page.keyboard.press("Enter");

    const searchGoals = page.getByPlaceholder("Title, status, or id");
    await expect(searchGoals).toBeVisible();
    await expect(searchGoals).toBeFocused();
    await searchGoals.pressSequentially("Branch visibility");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");

    await expect(page).toHaveURL(new RegExp(`goal=${goalB}`));
    await expect(goalPickerTrigger).toContainText("Branch visibility goal");
    await expect(page.locator(".outcome-meta")).toContainText("Context: Branch visibility goal");

    const goalDraftButton = page.getByRole("group", { name: "Draft type" }).getByRole("button", { name: "Draft goal" });
    await goalDraftButton.focus();
    await expect(goalDraftButton).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(goalDraftButton).toHaveAttribute("aria-pressed", "true");

    const composer = page.locator(".cs-message-input__content-editor");
    await composer.focus();
    await expect(composer).toBeFocused();
    await composer.pressSequentially("Keyboard operator path should produce a reviewable goal draft.");

    const sendButton = page.locator(".cs-message-input__tools button:not([disabled])");
    await sendButton.focus();
    await expect(sendButton).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.getByText("Goal draft ready", { exact: true })).toBeVisible();

    await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name: "Operator Actions", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { level: 1, name: "Actions" })).toBeVisible();
    await expectNoAdvancedControlInventory(page);
    const reviewEvidence = page.getByTestId("primary-review-evidence");
    await reviewEvidence.focus();
    await expect(reviewEvidence).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => state.actions.map((action) => action.handler)).toContain("steer");
    const reviewAction = state.actions.find((action) => action.handler === "steer");
    expect(steeringKind(reviewAction?.body)).toBe("evaluate_goal_completion");
  });

  test("shows action-needed thunks, branch/fork nodes, and flow-control actions", async ({ page }) => {
    const state = await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    await openPrimaryNav(page, "Work Graph");
    await expectNoAmbiguousLaneCopy(page);

    await expect(page.getByTestId("rf__node-thunk-approval-1")).toContainText("Action needed: approve sandbox profile");
    await expect(page.getByTestId("rf__node-fork-reviewer")).toContainText("Forked reviewer branch");
    await expect(page.locator(".graph-status-panel")).toContainText("need attention");
    await expect(page.locator(".evidence-next-panel")).toContainText("Evidence");
    await expect(page.locator(".evidence-next-panel")).toContainText("Next action");
    await expect(page.locator(".evidence-next-panel")).toContainText("Review action-needed work");
    await expect(page.getByLabel("Why blocked")).toContainText("Review action-needed thunk");
    await expect(page.getByLabel("Why blocked")).toContainText("Provide the missing input and resume the continuation.");
    await expect(page.getByLabel("Action needed")).toContainText("Review action-needed thunk");
    await expect(page.locator(".evidence-next-panel")).toContainText("Wait evidence");
    await expect(page.locator(".evidence-next-panel")).toContainText("1 continuations");
    await expect(page.getByLabel("Why blocked")).toContainText("continuation Ref thunk-approval-1");

    await openPrimaryNav(page, "Action Queue");
    await expect(page.getByRole("heading", { level: 1, name: "Action Queue" })).toBeVisible();
    await expect(page.locator(".approval-list")).toContainText("Review action-needed thunk");
    await expect(page.locator(".approval-list")).toContainText("Action needed: approve sandbox profile");
    const approvalCard = page.locator(".approval-card").filter({ hasText: "sandbox profile approval" }).first();
    await expect(approvalCard).toContainText("Approval prompt");
    await expect(approvalCard).toContainText("Approve this gate and continue?");
    await approvalCard.getByRole("button", { name: "Approve and continue" }).click();
    await expect.poll(() => state.actions.map((action) => action.handler)).toContain("approve");
    expect(state.actions.find((action) => action.handler === "approve")?.body.approval_id).toBe("approval-sandbox-1");

    const continuationCard = page.locator(".approval-card").filter({ hasText: "Action needed: approve sandbox profile" }).first();
    await expect(continuationCard).toContainText("Human prompt");
    await expect(continuationCard.getByPlaceholder("Add context for the agent, or leave blank and press Continue.")).toBeVisible();
    await continuationCard.getByLabel("Context").fill("Approved sandbox profile for the fixture continuation.");
    await continuationCard.getByRole("button", { name: "Add context" }).click();
    await expect.poll(() => state.actions.map((action) => action.handler)).toContain("resume_thunk");
    const contextAction = state.actions.find((action) => action.handler === "resume_thunk");
    expect(contextAction?.body.thunk_id).toBe("thunk-approval-1");
    expect(contextAction?.body.response_summary).toBe("Approved sandbox profile for the fixture continuation.");

    await continuationCard.getByRole("button", { name: "Continue" }).click();
    await expect.poll(() => state.actions.filter((action) => action.handler === "resume_thunk").length).toBeGreaterThanOrEqual(2);
    const continueAction = state.actions.filter((action) => action.handler === "resume_thunk").at(-1);
    expect(continueAction?.body.thunk_id).toBe("thunk-approval-1");
    expect(continueAction?.body.response_summary).toBe("Operator chose Continue.");

    await openPrimaryNav(page, "Operator Actions");
    await expect(page.getByRole("heading", { level: 1, name: "Actions" })).toBeVisible();
    await expectNoAdvancedControlInventory(page);
    await expect(page.getByRole("button", { name: "Review evidence" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Research gap" })).toBeVisible();
    await expectNoCriticalOrSeriousAxeViolations(page, "flow control view");
  });

  test("cancels a selected goal and removes stale action-needed controls", async ({ page }) => {
    const state = await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    await openPrimaryNav(page, "Action Queue");
    await expect(page.locator(".approval-card").filter({ hasText: "Action needed: approve sandbox profile" }).first()).toBeVisible();

    await page.locator(".goal-context-cancel").click();
    await expect.poll(() => state.actions.some((action) => action.handler === "cancel")).toBe(true);
    await openPrimaryNav(page, "Action Queue");
    await expect(page.locator(".approval-card")).toHaveCount(0);
    await expect(page.getByText("Cancelled task retained for operator queue history.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Approve and continue" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(0);
  });

  test("keeps normal operator UI direct and hides CLI coverage inventory", async ({ page }) => {
    await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    await openPrimaryNav(page, "Work Graph");

    await expect(page.locator(".graph-panel .compiler-control-panel.compact")).toHaveCount(0);
    await expect(page.locator(".evidence-next-panel")).toContainText("Next action");
    await expectNoAdvancedControlInventory(page);
    await expectNoCliCoverageOrDebugInventory(page);

    await openPrimaryNav(page, "Operator Actions");
    await expect(page.getByRole("heading", { level: 1, name: "Actions" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Review evidence" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Research gap" })).toBeVisible();
    await expectNoAdvancedControlInventory(page);
    await expectNoCliCoverageOrDebugInventory(page);
  });
});

async function expectNoCriticalOrSeriousAxeViolations(page: Page, contextLabel: string): Promise<void> {
  const results = await new AxeBuilder({ page })
    .exclude(".cs-button--send")
    .analyze();
  const violations = results.violations.filter((violation) => violation.impact === "critical" || violation.impact === "serious");
  expect(violations, `${contextLabel} has critical or serious accessibility violations`).toEqual([]);
}

async function openPrimaryNav(page: Page, name: string): Promise<void> {
  await page.getByRole("navigation", { name: "Primary" }).getByRole("button", { name, exact: true }).click();
}

function subgoalCard(page: Page, text: string) {
  return page.locator('[data-testid^="subgoal-card-"]').filter({ hasText: text });
}

async function installGatewayFixtures(page: Page): Promise<FixtureState> {
  const state: FixtureState = { actions: [], cancelledGoals: new Set(), freezeChatSession: false, requestCounts: {}, submittedGoalSpec: null };

  await page.route("**/api/**", async (route) => {
    await fulfillApi(route, state);
  });

  return state;
}

async function expectNoAmbiguousLaneCopy(page: Page): Promise<void> {
  await expect(page.getByText(/\b(?:runner|agent|task|model|provider|work|research|chat|embedding|implementation|smoke|fast|deep review|xhigh reasoning|speed tier)[ -]lanes?\b/i)).toHaveCount(0);
}

async function expectNoAdvancedControlInventory(page: Page): Promise<void> {
  for (const label of ["Advanced controls", "Restart or branch", "Wait state", "Decision round", "Decision ballot", "Create wait state"]) {
    await expect(page.getByText(label, { exact: true })).toHaveCount(0);
  }
}

async function expectNoCliCoverageOrDebugInventory(page: Page): Promise<void> {
  await expect(page.getByText("Every canonical CLI group")).toHaveCount(0);
  await expect(page.getByText("CLI coverage", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Inspect coverage" })).toHaveCount(0);
  await expect(page.getByText("Inspect compact JSON", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Inspect JSON" })).toHaveCount(0);
  await expect(page.getByText("Use CLI", { exact: true })).toHaveCount(0);
  await expect(page.getByText(/\bcoat (?:plan|goal|deploy|runner|tool|memory|event|store|scenario|setup|tui)\b/)).toHaveCount(0);
}

function steeringKind(body: JsonRecord | undefined): unknown {
  const kind = body?.kind;
  return isRecord(kind) ? kind.kind : kind;
}

async function sendComposerMessage(page: Page, text: string): Promise<void> {
  const editor = page.locator(".cs-message-input__content-editor");
  await editor.click();
  await editor.pressSequentially(text);
  await page.locator(".cs-message-input__tools button:not([disabled])").click();
}

async function fulfillApi(route: Route, state: FixtureState): Promise<void> {
  const request = route.request();
  const url = new URL(request.url());
  const method = request.method();
  const path = url.pathname;

  if (method === "GET" && path === "/api/operator/goals") {
    countRequest(state, "goals");
    await json(route, { ok: true, status: 200, data: { goals: goalRows(state) } });
    return;
  }

  if (method === "GET" && path === "/api/chat/session") {
    if (state.freezeChatSession) {
      await json(route, { error: "fixture keeps the generated draft available for submission" }, 503);
      return;
    }
    await json(route, { session_id: url.searchParams.get("session_id") ?? "operator:default", messages: [] });
    return;
  }

  if (method === "GET" && path.startsWith("/api/chat/runs/")) {
    await json(route, { found: true, status: "done", stage: "fixture" });
    return;
  }

  if (method === "POST" && path === "/api/chat") {
    const body = await requestBody(request);
    state.freezeChatSession = String(body.kind ?? body.draft_kind ?? body.mode ?? "") !== "ask";
    await json(route, chatResponseFixture(body));
    return;
  }

  if (method === "POST" && path === "/api/operator/goals") {
    state.submittedGoalSpec = await requestBody(request);
    await json(route, {
      ok: true,
      status: 202,
      url: `http://fixture.local/GoalWorkflow/${submittedGoal}/run`,
      goal_id: submittedGoal,
      data: { accepted: true, goal_id: submittedGoal },
    });
    return;
  }

  if (method === "GET" && path === "/api/operator/stream") {
    const goalId = url.searchParams.get("goal_id") ?? goalA;
    await route.fulfill({
      status: 200,
      contentType: "text/event-stream",
      body: `event: goal.updated\ndata: ${JSON.stringify(operatorWorkspaceFixture(goalId, state))}\n\n`,
    });
    return;
  }

  const goalMatch = path.match(/^\/api\/operator\/goals\/([^/]+)(?:\/([^/]+))?$/);
  if (goalMatch && method === "GET" && !goalMatch[2]) {
    const goalId = decodeURIComponent(goalMatch[1]);
    countRequest(state, `goal:${goalId}`);
    await json(route, operatorGoalDetailFixture(composedGoalProjectionFixture(goalId, state), state));
    return;
  }

  if (goalMatch && method === "GET" && goalMatch[2] === "graph") {
    const goalId = decodeURIComponent(goalMatch[1]);
    const snapshot = composedGoalProjectionFixture(goalId, state);
    await json(route, {
      generated_at: "2026-05-12T12:00:00.000Z",
      goal_id: goalId,
      graph: (snapshot.workflow_compute_graph as JsonRecord)?.data ?? {},
      tasks: rowsFrom((snapshot.tasks as JsonRecord)?.data ?? snapshot.tasks),
      actions: operatorActionsFixture(goalId, state),
    });
    return;
  }

  if (goalMatch && method === "POST" && goalMatch[2]) {
    const goalId = decodeURIComponent(goalMatch[1]);
    const handler = decodeURIComponent(goalMatch[2]);
    const body = await requestBody(request);
    if (handler === "cancel") {
      state.cancelledGoals.add(goalId);
    }
    state.actions.push({ handler, body });
    await json(route, {
      ok: true,
      status: 202,
      url: `http://fixture.local/GoalWorkflow/${goalId}/${handler}`,
      data: { accepted: true, handler, body },
    });
    return;
  }

  if (method === "GET" && path === "/api/operator/actions") {
    await json(route, { actions: operatorActionsFixture(url.searchParams.get("goal_id") ?? undefined, state) });
    return;
  }

  const actionResolveMatch = path.match(/^\/api\/operator\/actions\/([^/]+)\/resolve$/);
  if (method === "POST" && actionResolveMatch) {
    const body = await requestBody(request);
    const actionId = decodeURIComponent(actionResolveMatch[1]);
    const action = operatorActionsFixture(undefined, state).find((item) => item.action_id === actionId);
    const handler = action?.handler ? String(action.handler) : actionId.startsWith("approval:")
      ? "approve"
      : actionId.startsWith("thunk:")
        ? "resume_thunk"
        : "steer";
    state.actions.push({ handler, body });
    await json(route, {
      action_id: actionId,
      goal_id: body.goal_id ?? goalA,
      resolution: body.resolution ?? "continue",
      result: {
        ok: true,
        status: 202,
        url: `http://fixture.local/GoalWorkflow/${body.goal_id ?? goalA}/${handler}`,
        data: { accepted: true, handler, body },
      },
      active_state: composedGoalProjectionFixture(String(body.goal_id ?? goalA), state),
    });
    return;
  }

  if (method === "GET" && path === "/api/plans") {
    await json(route, { ok: true, status: 200, data: { plans: [] } });
    return;
  }

  await json(route, { error: `unhandled e2e fixture route: ${method} ${path}` }, 404);
}

function countRequest(state: FixtureState, key: string): void {
  state.requestCounts[key] = (state.requestCounts[key] ?? 0) + 1;
}

async function requestBody(request: ReturnType<Route["request"]>): Promise<JsonRecord> {
  const text = request.postData() ?? "{}";
  return JSON.parse(text || "{}") as JsonRecord;
}

async function json(route: Route, body: unknown, status = 200): Promise<void> {
  await route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}

function backendProjectionFixture(state?: FixtureState): JsonRecord {
  return {
    generated_at: "2026-05-12T12:00:00.000Z",
    services: [
      { name: "goal-store", ok: true, status: 200 },
      { name: "runner-registry", ok: true, status: 200 },
      { name: "memory-gateway", ok: true, status: 200 },
    ],
    runner_status: {
      ok: true,
      status: 200,
      data: [
        {
          runner_id: "runner-codex-1",
          node_id: "local-e2e",
          display_name: "Codex fixture runner",
          status: "active",
          max_concurrency: 2,
          endpoint: "http://127.0.0.1:19091",
        },
      ],
    },
    approvals: { ok: true, status: 200, data: { approvals: approvalsFixture() } },
    recent_events: { ok: true, status: 200, data: { events: [{ event_id: "evt-1", type: "goal.updated" }] } },
    event_sources: { ok: true, status: 200, data: { event_sources: [{ source_id: "fixture", kind: "generic", enabled: true }] } },
    goals: { ok: true, status: 200, data: { goals: goalRows(state) } },
    agents: { ok: true, status: 200, data: { tasks: goalATasks() } },
    plans: { ok: true, status: 200, data: { plans: [] } },
  };
}

function goalRows(state?: FixtureState): JsonRecord[] {
  const rows: JsonRecord[] = [
    {
      goal_id: goalA,
      title: "Baseline durable goal",
      objective: "Exercise goal projection, action-needed state, and branch fork UI.",
      status: state?.cancelledGoals.has(goalA) ? "cancelled" : "running",
      percent_done: state?.cancelledGoals.has(goalA) ? 1 : 0.35,
      open_tasks: state?.cancelledGoals.has(goalA) ? 0 : 3,
      blocked_tasks: state?.cancelledGoals.has(goalA) ? 0 : 1,
      failed_tasks: 0,
      updated_at: "2026-05-12T12:00:00.000Z",
    },
    {
      goal_id: goalB,
      title: "Branch visibility goal",
      objective: "Verify the goal switcher replaces graph and subgoal context.",
      status: "waiting-input",
      percent_done: 0.2,
      open_tasks: 2,
      blocked_tasks: 0,
      failed_tasks: 0,
      updated_at: "2026-05-12T12:05:00.000Z",
    },
  ];
  if (state?.submittedGoalSpec) {
    rows.unshift({
      goal_id: submittedGoal,
      title: String(state.submittedGoalSpec.title ?? "Submitted browser E2E task"),
      objective: String(state.submittedGoalSpec.objective ?? "Submit browser E2E task with deterministic mocked gateway evidence."),
      status: "running",
      percent_done: 0.1,
      open_tasks: 1,
      blocked_tasks: 0,
      failed_tasks: 0,
      updated_at: "2026-05-12T12:10:00.000Z",
    });
  }
  return rows;
}

function operatorWorkspaceFixture(goalId: string, state?: FixtureState): JsonRecord {
  const snapshot = composedGoalProjectionFixture(goalId, state);
  return {
    generated_at: "2026-05-12T12:00:00.000Z",
    goals: goalRows(state),
    selected_goal: operatorGoalDetailFixture(snapshot, state),
    actions: operatorActionsFixture(goalId, state),
    events: [],
    worker_runs: rowsFrom((snapshot.tasks as JsonRecord)?.data ?? snapshot.tasks),
    evidence: [],
    services: backendProjectionFixture(state).services,
    runners: (backendProjectionFixture(state).runner_status as JsonRecord)?.data ?? [],
  };
}

function operatorGoalDetailFixture(snapshot: JsonRecord, state?: FixtureState): JsonRecord {
  const goal = (((snapshot.goal_store_goal as JsonRecord)?.data as JsonRecord)?.goal ?? {}) as JsonRecord;
  return {
    summary: {
      goal_id: goal.goal_id ?? snapshot.goal_id,
      id: goal.goal_id ?? snapshot.goal_id,
      title: ((goal.payload_json as JsonRecord)?.title ?? "Untitled goal"),
      objective: ((goal.payload_json as JsonRecord)?.objective ?? ""),
      status: goal.status ?? "unknown",
      percent_done: goal.percent_done ?? 0,
      open_tasks: goal.open_tasks ?? 0,
      blocked_tasks: goal.blocked_tasks ?? 0,
      failed_tasks: goal.failed_tasks ?? 0,
      updated_at: goal.updated_at ?? "2026-05-12T12:00:00.000Z",
    },
    progress: (snapshot.workflow_progress as JsonRecord)?.data ?? {},
    graph: (snapshot.workflow_compute_graph as JsonRecord)?.data ?? {},
    tasks: rowsFrom((snapshot.tasks as JsonRecord)?.data ?? snapshot.tasks),
    actions: operatorActionsFixture(String(goal.goal_id ?? snapshot.goal_id ?? ""), state),
    evidence: [],
    snapshot,
  };
}

function composedGoalProjectionFixture(goalId: string, state?: FixtureState): JsonRecord {
  if (state?.cancelledGoals.has(goalId)) {
    return snapshot(goalId, {
      title: goalId === goalA ? "Baseline durable goal" : "Cancelled fixture goal",
      objective: "Cancelled by the operator; queue history remains visible without stale action controls.",
      status: "cancelled",
      subgoals: [
        {
          id: "cancelled-history",
          title: "Cancelled queue history",
          objective: "Retain cancellation evidence after action-needed controls are cleared.",
        },
      ],
      tasks: [
        {
          goal_id: goalId,
          task_id: "root-task",
          title: "Cancelled task retained for operator queue history.",
          role: "planner",
          status: "cancelled",
          subgoal_id: "cancelled-history",
        },
      ],
      computeGraph: {
        open_thunks: 0,
        nodes: [
          { id: "root-task", kind: "task", label: "Cancelled task retained for operator queue history.", status: "cancelled", task_id: "root-task" },
        ],
        edges: [],
      },
    });
  }

  if (goalId === submittedGoal) {
    if ((state?.requestCounts[`goal:${submittedGoal}`] ?? 0) <= 2) {
      return submittedProjectionShell();
    }
    return snapshot(goalId, {
      title: "Submitted browser E2E task",
      objective: "Submit browser E2E task with deterministic mocked gateway evidence.",
      status: "running",
      subgoals: [
        {
          id: "validate-fixtures",
          title: "Validate mocked gateway fixtures",
          objective: "Show the submitted goal projection and graph path without live services.",
        },
      ],
      tasks: [
        {
          goal_id: submittedGoal,
          task_id: "submitted-root",
          title: "Plan submitted E2E task",
          role: "planner",
          status: "running",
          subgoal_id: "validate-fixtures",
        },
      ],
      computeGraph: {
        nodes: [
          { id: "submitted-root", kind: "task", label: "Plan submitted E2E task", status: "running", task_id: "submitted-root" },
        ],
        edges: [],
        open_thunks: 0,
      },
    });
  }

  if (goalId === goalB) {
    return snapshot(goalId, {
      title: "Branch visibility goal",
      objective: "Verify the goal switcher replaces graph and subgoal context.",
      status: "waiting-input",
      subgoals: [
        {
          id: "branch-fanout-review",
          title: "Branch fanout review",
          objective: "Inspect branch and fork candidates for the selected goal.",
        },
      ],
      tasks: [
        {
          goal_id: goalB,
          task_id: "goal-b-root",
          title: "Branch fanout root task",
          role: "planner",
          status: "waiting-input",
          subgoal_id: "branch-fanout-review",
        },
      ],
      computeGraph: {
        nodes: [
          { id: "goal-b-root", kind: "task", label: "Branch fanout root task", status: "waiting-input", task_id: "goal-b-root" },
        ],
        edges: [],
        open_thunks: 1,
      },
    });
  }

  return snapshot(goalA, {
    title: "Baseline durable goal",
    objective: "Exercise goal projection, action-needed state, and branch fork UI.",
    status: "running",
    subgoals: [
      {
        id: "coordinator-truth-boundary",
        title: "Coordinator truth boundary",
        objective: "Keep durable state and branching authority in the coordinator.",
      },
      {
        id: "human-thunk-review",
        title: "Action-needed review",
        objective: "Expose delayed compute thunks that need operator input.",
      },
    ],
    tasks: goalATasks(),
    computeGraph: {
      open_thunks: 1,
      nodes: [
        { id: "root-task", kind: "task", label: "Plan launch task", status: "running", task_id: "root-task" },
        {
          id: "thunk-approval-1",
          kind: "delayed-compute-thunk",
          label: "Action needed: approve sandbox profile",
          status: "waiting-input",
          task_id: "root-task",
          thunk_id: "thunk-approval-1",
          continuation_id: "continue-sandbox-approval",
          wait_ref: { kind: "human_thread", reference: `goal://${goalA}/root-task` },
        },
        { id: "fork-reviewer", kind: "branch-fork", label: "Forked reviewer branch", status: "runnable", task_id: "fork-reviewer" },
        { id: "branch-codex", kind: "branch-candidate", label: "Branch candidate: Codex path", status: "pending", task_id: "branch-codex" },
      ],
      edges: [
        { from: "root-task", to: "thunk-approval-1", kind: "waits_for" },
        { from: "root-task", to: "fork-reviewer", kind: "forks" },
        { from: "root-task", to: "branch-codex", kind: "branch_candidate" },
      ],
    },
  });
}

function submittedProjectionShell(): JsonRecord {
  return {
    generated_at: "2026-05-12T12:00:00.000Z",
    goal_id: submittedGoal,
    goal_store_goal: {
      ok: true,
      status: 200,
      data: {
        goal: {
          goal_id: submittedGoal,
          status: "submitted",
          percent_done: 0,
          open_tasks: 0,
          blocked_tasks: 0,
          failed_tasks: 0,
          payload_json: {
            title: "Submitted browser E2E task",
            objective: "Submit browser E2E task with deterministic mocked gateway evidence.",
          },
        },
      },
    },
    workflow_status: { ok: true, status: 200, data: { status: "submitted" } },
    workflow_progress: { ok: true, status: 200, data: {} },
    workflow_compute_graph: { ok: true, status: 200, data: { goal_id: submittedGoal, nodes: [], edges: [], open_thunks: 0 } },
    tasks: { ok: true, status: 200, data: { tasks: [] } },
    approvals: { ok: true, status: 200, data: { approvals: [] } },
    agent_activity: [],
  };
}

function snapshot(goalId: string, input: {
  title: string;
  objective: string;
  status: string;
  subgoals: JsonRecord[];
  tasks: JsonRecord[];
  computeGraph: JsonRecord;
}): JsonRecord {
  return {
    generated_at: "2026-05-12T12:00:00.000Z",
    goal_id: goalId,
    goal_store_goal: {
      ok: true,
      status: 200,
      data: {
        goal: {
          goal_id: goalId,
          status: input.status,
          percent_done: 0.42,
          open_tasks: input.tasks.length,
          blocked_tasks: input.tasks.filter((task) => task.status === "blocked").length,
          failed_tasks: 0,
          payload_json: {
            title: input.title,
            objective: input.objective,
            plan: { subgoals: input.subgoals },
          },
        },
      },
    },
    workflow_status: { ok: true, status: 200, data: { status: input.status } },
    workflow_progress: {
      ok: true,
      status: 200,
      data: {
        ranking: { vote_count: 2, score: 1, upvotes: 2, downvotes: 1, latest_decision: { outcome: "promote" } },
        open_mechanism_rounds: 1,
        ratification_required_mechanism_rounds: 0,
      },
    },
    workflow_compute_graph: { ok: true, status: 200, data: { goal_id: goalId, ...input.computeGraph } },
    tasks: { ok: true, status: 200, data: { tasks: input.tasks } },
    events: { ok: true, status: 200, data: { events: [] } },
    artifacts: { ok: true, status: 200, data: { artifacts: [] } },
    checkpoints: { ok: true, status: 200, data: { checkpoints: [] } },
    approvals: { ok: true, status: 200, data: { approvals: input.status === "cancelled" ? [] : approvalsFixture() } },
    agent_activity: input.tasks,
  };
}

function goalATasks(): JsonRecord[] {
  return [
    {
      goal_id: goalA,
      task_id: "root-task",
      title: "Plan launch task",
      role: "planner",
      status: "running",
      subgoal_id: "coordinator-truth-boundary",
      color: { key: "planning", label: "Planning", hex: "#2563eb" },
    },
    {
      goal_id: goalA,
      task_id: "review-task",
      parent_task_id: "root-task",
      title: "Review action-needed thunk",
      role: "reviewer",
      status: "waiting-approval",
      subgoal_id: "human-thunk-review",
      color: { key: "review", label: "Review", hex: "#dc2626" },
    },
    {
      goal_id: goalA,
      task_id: "fork-reviewer",
      parent_task_id: "root-task",
      title: "Forked reviewer branch",
      role: "reviewer",
      status: "runnable",
      subgoal_id: "coordinator-truth-boundary",
      color: { key: "branch", label: "Branch", hex: "#16a34a" },
    },
  ];
}

function approvalsFixture(): JsonRecord[] {
  return [
    {
      approval_id: "approval-sandbox-1",
      goal_id: goalA,
      risk: "sandbox profile approval",
      status: "pending",
    },
  ];
}

function operatorActionsFixture(goalId?: string, state?: FixtureState): JsonRecord[] {
  if (goalId && state?.cancelledGoals.has(goalId)) {
    return [];
  }
  const approvalActions = approvalsFixture()
    .filter((approval) => !state?.cancelledGoals.has(String(approval.goal_id)))
    .filter((approval) => !goalId || approval.goal_id === goalId)
    .map((approval) => ({
      action_id: `approval:${approval.goal_id}:${approval.approval_id}`,
      kind: "resolve_approval",
      handler: "approve",
      goal_id: approval.goal_id,
      task_id: approval.task_id ?? "review-task",
      title: "Approval required",
      question: "Approve this gate and continue?",
      status: "pending",
      allowed_resolutions: ["approve", "reject", "add_context", "cancel_goal"],
      approval,
      payload_json: approval,
    }));
  const thunkActions = (!goalId || goalId === goalA) && !state?.cancelledGoals.has(goalA)
    ? [{
        action_id: `thunk:${goalA}:thunk-approval-1`,
        kind: "resume_thunk",
        handler: "resume_thunk",
        goal_id: goalA,
        task_id: "root-task",
        title: "Action needed: approve sandbox profile",
        question: "Provide the missing input and resume the continuation.",
        status: "pending",
        allowed_resolutions: ["continue", "add_context", "replan", "cancel_goal"],
        thunk_id: "thunk-approval-1",
        payload_json: {
          id: "thunk-approval-1",
          status: "pending",
          question: "Provide the missing input and resume the continuation.",
        },
      }]
    : [];
  return [...approvalActions, ...thunkActions];
}

function chatResponseFixture(body: JsonRecord): JsonRecord {
  const messages = Array.isArray(body.messages) ? body.messages.filter(isRecord) : [];
  const latest = [...messages].reverse().find((message) => message.role === "user")?.content;
  const objective = typeof latest === "string" && latest.trim()
    ? latest.trim()
    : "Submit browser E2E task with deterministic mocked gateway evidence.";
  const requestedKind = String(body.kind ?? body.draft_kind ?? "");
  const requestedMode = String(body.mode ?? "");
  if (requestedKind === "ask" || requestedMode === "ask") {
    return {
      provider: "fixture",
      model: null,
      mode: "ask",
      assistant: "The selected goal has one action needed item. Open Action Queue to continue, add context, replan, or cancel.",
      session_id: body.session_id,
      run_id: body.run_id,
      chat_run: { run_id: body.run_id, status: "done", stage: "fixture" },
      related_state: {
        selected_goal_id: body.goal_id ?? null,
        answer_kind: "operator_guidance",
        draft_created: false,
      },
    };
  }
  return {
    provider: "fixture",
    model: null,
    mode: "draft_goal",
    assistant: "Goal draft ready. Review the fields, then accept or discard it.",
    draft_ref: "fixture://drafts/browser-e2e",
    draft_summary: {
      title: "Submitted browser E2E task",
      objective,
      summary: "Compact fixture draft summary.",
      evidence_count: 2,
      constraint_count: 1,
    },
    drafts: {
      goal_spec: {
        title: "Submitted browser E2E task",
        objective,
        authoring: {
          intake_summary: objective,
          acceptance_evidence: ["Playwright verifies create and submit flow", "Mocked gateway fixtures project a task graph"],
          constraints: ["No live backend required"],
          out_of_scope: [],
          assumptions: [],
          open_questions: [],
        },
        plan: {
          summary: "E2E workflow for the control-plane SPA.",
          subgoals: [
            {
              id: "validate-fixtures",
              title: "Validate mocked gateway fixtures",
              objective: "Show submitted goal projection and graph path without live services.",
            },
          ],
        },
        root_budget: { max_attempts: 1, max_runtime_seconds: 600 },
        done_criteria: { tests_pass: true, artifact_exists: true },
        initial_tasks: [
          {
            role: "planner",
            title: "Plan submitted E2E task",
            subgoal_id: "validate-fixtures",
            prompt: objective,
          },
        ],
      },
    },
    session_id: body.session_id,
    run_id: body.run_id,
    chat_run: { run_id: body.run_id, status: "done", stage: "fixture" },
  };
}

function isRecord(value: unknown): value is JsonRecord {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function rowsFrom(value: unknown): JsonRecord[] {
  if (Array.isArray(value)) {
    return value.filter(isRecord);
  }
  if (!isRecord(value)) {
    return [];
  }
  for (const key of ["tasks", "nodes", "goals", "actions", "items", "records", "rows", "data"]) {
    const rows = rowsFrom(value[key]);
    if (rows.length) {
      return rows;
    }
  }
  return [];
}
