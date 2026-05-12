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
  freezeChatSession: boolean;
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

  test("creates a goal draft, submits it, and shows projected subgoals", async ({ page }) => {
    const state = await installGatewayFixtures(page);

    await page.goto("/");
    await expectNoCriticalOrSeriousAxeViolations(page, "initial operator console");
    await page.getByRole("group", { name: "Draft target" }).getByRole("button", { name: "Goal" }).click();
    await sendComposerMessage(page, "Submit browser E2E lane with deterministic mocked gateway evidence.");

    await expect(page.getByText("Goal draft ready")).toBeVisible();
    await page.getByRole("button", { name: "Submit goal" }).click();

    await expect(page).toHaveURL(new RegExp(`goal=${submittedGoal}`));
    await expect(page.getByRole("heading", { name: "Technicolor task graph", exact: true })).toBeVisible();
    await expect(page.locator(".goal-context-trigger")).toContainText("Submitted browser E2E lane");
    await expect(page.getByText("Validate mocked gateway fixtures")).toBeVisible();
    expect(state.submittedGoalSpec?.objective).toContain("Submit browser E2E lane");
  });

  test("switches selected goals from the top bar and updates subgoal visibility", async ({ page }) => {
    await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    await page.getByRole("button", { name: "Task Graph" }).click();
    await expect(page.getByText("Coordinator truth boundary")).toBeVisible();

    await page.locator(".goal-context-trigger").click();
    await page.getByText("Branch visibility goal").click();

    await expect(page).toHaveURL(new RegExp(`goal=${goalB}`));
    await expect(page.locator(".goal-context-trigger")).toContainText("Branch visibility goal");
    await expect(page.getByText("Branch fanout review")).toBeVisible();
    await expect(page.getByText("Coordinator truth boundary")).toBeHidden();
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
    await expect(page.getByRole("button", { name: /Branch visibility goal waiting-input/ })).toBeVisible();

    const goalDraftButton = page.getByRole("group", { name: "Draft target" }).getByRole("button", { name: "Goal" });
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
    await expect(page.getByText("Goal draft ready")).toBeVisible();

    await page.getByRole("button", { name: "Flow Control" }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { name: "Flow control" })).toBeVisible();

    const flowCard = page.locator(".control-card").filter({ has: page.getByRole("heading", { name: "Flow", exact: true }) });
    const flowAction = flowCard.locator("select").first();
    await flowAction.focus();
    await expect(flowAction).toBeFocused();
    await flowAction.selectOption("branch");
    await expect(flowCard.locator('label:has-text("Candidates") input')).toBeEnabled();

    const runFlowAction = flowCard.getByRole("button", { name: "Run flow action" });
    await runFlowAction.focus();
    await expect(runFlowAction).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => state.actions.map((action) => action.handler)).toContain("branch");
  });

  test("shows action-needed thunks, branch/fork nodes, and flow-control actions", async ({ page }) => {
    const state = await installGatewayFixtures(page);

    await page.goto(`/?goal=${goalA}`);
    await page.getByRole("button", { name: "Task Graph" }).click();

    await expect(page.getByTestId("rf__node-thunk-approval-1")).toContainText("Action needed: approve sandbox profile");
    await expect(page.getByTestId("rf__node-fork-reviewer")).toContainText("Forked reviewer branch");
    await expect(page.getByLabel("Continuations")).toContainText("1 waiting");
    await expect(page.getByText("thunk thunk-approval-1")).toBeVisible();

    await page.getByRole("button", { name: "Flow Control" }).click();
    await expect(page.getByRole("heading", { name: "Flow control" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Vote" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Steer" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Flow", exact: true })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Thunk" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Mechanism round" })).toBeVisible();

    const flowCard = page.locator(".control-card").filter({ has: page.getByRole("heading", { name: "Flow", exact: true }) });
    await flowCard.locator("select").first().selectOption("branch");
    await expect(flowCard.locator('label:has-text("Candidates") input')).toBeEnabled();
    await expect(flowCard.locator('label:has-text("Candidate roles") input')).toHaveValue("codex,reviewer");
    await flowCard.getByRole("button", { name: "Run flow action" }).focus();
    await page.keyboard.press("Enter");
    await expect.poll(() => state.actions.map((action) => action.handler)).toContain("branch");

    const createWaitState = page.getByRole("button", { name: "Create wait state" });
    await expect(createWaitState).toBeEnabled();
    await createWaitState.click();
    await expect.poll(() => state.actions.map((action) => action.handler)).toContain("create_thunk");
    expect(state.actions.find((action) => action.handler === "branch")?.body.candidate_roles).toEqual([
      "codex",
      "reviewer",
    ]);
    await expectNoCriticalOrSeriousAxeViolations(page, "flow control view");
  });
});

async function expectNoCriticalOrSeriousAxeViolations(page: Page, contextLabel: string): Promise<void> {
  const results = await new AxeBuilder({ page })
    .exclude(".cs-button--send")
    .analyze();
  const violations = results.violations.filter((violation) => violation.impact === "critical" || violation.impact === "serious");
  expect(violations, `${contextLabel} has critical or serious accessibility violations`).toEqual([]);
}

async function installGatewayFixtures(page: Page): Promise<FixtureState> {
  const state: FixtureState = { actions: [], freezeChatSession: false, submittedGoalSpec: null };

  await page.route("**/api/**", async (route) => {
    await fulfillApi(route, state);
  });

  return state;
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

  if (method === "GET" && path === "/api/overview") {
    await json(route, overviewFixture());
    return;
  }

  if (method === "GET" && path === "/api/goals") {
    await json(route, { ok: true, status: 200, data: { goals: goalRows() } });
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
    state.freezeChatSession = true;
    await json(route, chatResponseFixture(body));
    return;
  }

  if (method === "POST" && path === "/api/goals/submit") {
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

  const goalMatch = path.match(/^\/api\/goals\/([^/]+)(?:\/([^/]+))?$/);
  if (goalMatch && method === "GET" && !goalMatch[2]) {
    await json(route, goalSnapshotFixture(decodeURIComponent(goalMatch[1])));
    return;
  }

  if (goalMatch && method === "POST" && goalMatch[2]) {
    const handler = decodeURIComponent(goalMatch[2]);
    const body = await requestBody(request);
    state.actions.push({ handler, body });
    await json(route, {
      ok: true,
      status: 202,
      url: `http://fixture.local/GoalWorkflow/${decodeURIComponent(goalMatch[1])}/${handler}`,
      data: { accepted: true, handler, body },
    });
    return;
  }

  if (method === "GET" && path === "/api/approvals") {
    await json(route, { ok: true, status: 200, data: { approvals: approvalsFixture() } });
    return;
  }

  if (method === "GET" && path === "/api/human/threads") {
    await json(route, { ok: true, status: 200, data: { threads: [] } });
    return;
  }

  if (method === "GET" && path === "/api/plans") {
    await json(route, { ok: true, status: 200, data: { plans: [] } });
    return;
  }

  await json(route, { error: `unhandled e2e fixture route: ${method} ${path}` }, 404);
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

function overviewFixture(): JsonRecord {
  return {
    generated_at: "2026-05-12T12:00:00.000Z",
    control_surface: "coat-control-plane-web",
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
          display_name: "Codex fixture lane",
          status: "active",
          max_concurrency: 2,
          endpoint: "http://127.0.0.1:19091",
        },
      ],
    },
    approvals: { ok: true, status: 200, data: { approvals: approvalsFixture() } },
    recent_events: { ok: true, status: 200, data: { events: [{ event_id: "evt-1", type: "goal.updated" }] } },
    event_sources: { ok: true, status: 200, data: { event_sources: [{ source_id: "fixture", kind: "generic", enabled: true }] } },
    goals: { ok: true, status: 200, data: { goals: goalRows() } },
    agents: { ok: true, status: 200, data: { tasks: goalATasks() } },
    plans: { ok: true, status: 200, data: { plans: [] } },
    follow_ups: { items: [] },
  };
}

function goalRows(): JsonRecord[] {
  return [
    {
      goal_id: goalA,
      title: "Baseline durable goal",
      objective: "Exercise goal projection, action-needed state, and branch fork UI.",
      status: "running",
      percent_done: 0.35,
      open_tasks: 3,
      blocked_tasks: 1,
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
}

function goalSnapshotFixture(goalId: string): JsonRecord {
  if (goalId === submittedGoal) {
    return snapshot(goalId, {
      title: "Submitted browser E2E lane",
      objective: "Submit browser E2E lane with deterministic mocked gateway evidence.",
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
          title: "Plan submitted E2E lane",
          role: "planner",
          status: "running",
          subgoal_id: "validate-fixtures",
        },
      ],
      computeGraph: {
        nodes: [
          { id: "submitted-root", kind: "task", label: "Plan submitted E2E lane", status: "running", task_id: "submitted-root" },
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
        { id: "root-task", kind: "task", label: "Plan launch lane", status: "running", task_id: "root-task" },
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
        { id: "branch-codex", kind: "branch-candidate", label: "Branch candidate: codex lane", status: "pending", task_id: "branch-codex" },
      ],
      edges: [
        { from: "root-task", to: "thunk-approval-1", kind: "waits_for" },
        { from: "root-task", to: "fork-reviewer", kind: "forks" },
        { from: "root-task", to: "branch-codex", kind: "branch_candidate" },
      ],
    },
  });
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
    approvals: { ok: true, status: 200, data: { approvals: approvalsFixture() } },
    agent_activity: input.tasks,
  };
}

function goalATasks(): JsonRecord[] {
  return [
    {
      goal_id: goalA,
      task_id: "root-task",
      title: "Plan launch lane",
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

function chatResponseFixture(body: JsonRecord): JsonRecord {
  const messages = Array.isArray(body.messages) ? body.messages.filter(isRecord) : [];
  const latest = [...messages].reverse().find((message) => message.role === "user")?.content;
  const objective = typeof latest === "string" && latest.trim()
    ? latest.trim()
    : "Submit browser E2E lane with deterministic mocked gateway evidence.";
  return {
    provider: "fixture",
    model: null,
    mode: "draft_goal",
    assistant: "Drafted a goal payload with deterministic E2E evidence and coordinator-owned tasks.",
    drafts: {
      goal_spec: {
        title: "Submitted browser E2E lane",
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
          summary: "E2E lane for the control-plane SPA.",
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
            title: "Plan submitted E2E lane",
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
