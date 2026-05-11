import { spawn } from "node:child_process";
import http from "node:http";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer as createViteServer } from "vite";
import react from "@vitejs/plugin-react";

const defaultPort = 19000 + (process.pid % 1000);
const port = Number(process.env.COAT_CONTROL_SMOKE_PORT ?? String(defaultPort));
const baseUrl = `http://127.0.0.1:${port}`;
const packageRoot = fileURLToPath(new URL("..", import.meta.url));
const chatJournalPath = `/tmp/coat-control-chat-smoke-${process.pid}-${Date.now()}.jsonl`;

class SmokeSkip extends Error {}

let smokeSkipped = false;
const server = spawn(process.execPath, ["dist/server.js"], {
  cwd: packageRoot,
  env: {
    ...process.env,
    HOST: "127.0.0.1",
    PORT: String(port),
    COAT_CONTROL_GATEWAY_TOKEN: "",
    COAT_CONTROL_MCP_TOKEN: "",
    COAT_RESTATE_INGRESS: "http://127.0.0.1:9",
    COAT_RESTATE_ADMIN_URL: "http://127.0.0.1:9",
    COAT_GOAL_STORE_URL: "http://127.0.0.1:9",
    COAT_EVENT_GATEWAY_URL: "http://127.0.0.1:9",
    COAT_NOTIFIER_URL: "http://127.0.0.1:9",
    COAT_RUNNER_REGISTRY_URL: "http://127.0.0.1:9",
    COAT_MEMORY_GATEWAY_URL: "http://127.0.0.1:9",
    COAT_CONTROL_CHAT_JOURNAL_PATH: chatJournalPath,
  },
  stdio: ["ignore", "pipe", "pipe"],
});

let stdout = "";
let stderr = "";
server.stdout.on("data", (chunk) => {
  stdout += chunk;
});
server.stderr.on("data", (chunk) => {
  stderr += chunk;
});

try {
  await waitForHealth();
  await assertHtmlSurface();
  await assertBrandAssets();
  await assertStylesheet();
  await assertClientScript();
  await assertMemoryReplacementDiffRender();
  await assertOperatorWorkflowRender();
  await assertOverviewApi();
  await assertFollowUpsApi();
  await assertFollowUpDraftApi();
  await assertChatApi();
  await assertDurableChatSessionApi();
  await assertChatApiDiscoversRegisteredModel();
  await assertGoalSubmitAssignsWorkflowId();
  await assertUnsupportedWorkflowHandlerGuard();
  await assertBackendBackedControlSurfaces();
  await assertMcpTools();
  await assertMcpChatAssistBehavior();
  await assertMcpFollowUpDraftBehavior();
  await assertMcpApplyResearchOutputBehavior();
  console.log("coat-control-plane-web smoke passed");
} catch (error) {
  if (error instanceof SmokeSkip) {
    smokeSkipped = true;
    console.log(`coat-control-plane-web smoke skipped: ${error.message}`);
  } else {
    throw error;
  }
} finally {
  server.kill("SIGTERM");
}

if (smokeSkipped) {
  process.exit(0);
}

async function waitForHealth() {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    if (server.exitCode !== null) {
      skipOnBindPermissionFailure(stderr);
      throw new Error(`server exited early\nstdout:\n${stdout}\nstderr:\n${stderr}`);
    }
    try {
      const response = await fetch(`${baseUrl}/healthz`);
      if (response.ok) {
        const body = await response.json();
        assertEqual(body.service, "coat-control-plane-web", "health service");
        return;
      }
    } catch {
      await delay(100);
    }
  }
  throw new Error(`timed out waiting for gateway\nstdout:\n${stdout}\nstderr:\n${stderr}`);
}

async function assertHtmlSurface() {
  const response = await fetch(`${baseUrl}/`);
  assert(response.ok, "root html returns ok");
  const html = await response.text();
  for (const expected of [
    "COAT Task Graph Manager",
    "coat.theme",
    "theme-color",
    "/brand/coat-mark.svg",
    "/brand/coat-icon-32.png",
    "/site.webmanifest",
    "<div id=\"root\"></div>",
    "/assets/",
  ]) {
    assert(html.includes(expected), `html includes ${expected}`);
  }
}

async function assertBrandAssets() {
  const assets = [
    { path: "/brand/coat-mark.svg", contentType: "image/svg+xml" },
    { path: "/brand/coat-logo.png", contentType: "image/png" },
    { path: "/brand/coat-icon-32.png", contentType: "image/png" },
    { path: "/brand/coat-icon-180.png", contentType: "image/png" },
    { path: "/site.webmanifest", contentType: "application/json" },
  ];
  for (const asset of assets) {
    const response = await fetch(`${baseUrl}${asset.path}`);
    assert(response.ok, `${asset.path} returns ok`);
    assert(
      response.headers.get("content-type")?.startsWith(asset.contentType),
      `${asset.path} content type starts with ${asset.contentType}`,
    );
    const body = new Uint8Array(await response.arrayBuffer());
    assert(body.length > 100, `${asset.path} has non-empty content`);
  }
}

async function assertClientScript() {
  const html = await (await fetch(`${baseUrl}/`)).text();
  const match = html.match(/src="([^"]+\.js)"/);
  assert(match, "root html references a built SPA asset");
  const response = await fetch(`${baseUrl}${match[1]}`);
  assert(response.ok, "client script returns ok");
  const script = await response.text();
  for (const expected of ["Task Graph Manager", "Start work", "Plan first", "Goal draft", "Search request", "Chat activity", "Next outcomes", "Shared Memory", "Technicolor", "Planning queue", "theme-control", "Graph legend", "No urgent task states", "Attention", "Completed", "Auto"]) {
    assert(script.includes(expected), `client script includes ${expected}`);
  }
}

async function assertStylesheet() {
  const html = await (await fetch(`${baseUrl}/`)).text();
  const match = html.match(/href="([^"]+\.css)"/);
  assert(match, "root html references a built SPA stylesheet");
  const response = await fetch(`${baseUrl}${match[1]}`);
  assert(response.ok, "stylesheet returns ok");
  const css = await response.text();
  for (const expected of ["data-theme=dark", "--status-running", ".theme-control", ".mode-toggle", ".coat-chat-container", ".outcome-list", ".graph-filter", ".graph-status-panel", ".react-flow__node.task-node"]) {
    assert(css.includes(expected), `stylesheet includes ${expected}`);
  }
}

async function assertMemoryReplacementDiffRender() {
  const viteServer = await createViteServer({
    configFile: false,
    root: packageRoot,
    logLevel: "silent",
    plugins: [
      react(),
      {
        name: "coat-memory-diff-render-smoke",
        enforce: "post",
        transform(code, id) {
          if (id.split("?")[0].endsWith("/src/spa/App.tsx")) {
            return `${code}\nexport { MemoryDiffTable, PreviewStatus, memoryEditPayload, previewReady };`;
          }
          return null;
        },
      },
    ],
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, hmr: false },
    appType: "custom",
  });

  try {
    const {
      MemoryDiffTable,
      PreviewStatus,
      memoryEditPayload,
      previewReady,
    } = await viteServer.ssrLoadModule("/src/spa/App.tsx");

    assert(typeof MemoryDiffTable === "function", "memory diff render smoke can load MemoryDiffTable");
    assert(typeof PreviewStatus === "function", "memory diff render smoke can load PreviewStatus");
    assert(typeof memoryEditPayload === "function", "memory diff render smoke can load memory edit payload builder");
    assert(typeof previewReady === "function", "memory diff render smoke can load preview readiness helper");

    const editPayload = memoryEditPayload({
      goalId: "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
      replaceKeys: ["mem-control-boundary", "mem-runner-policy"],
      replacementKey: "mem-replacement",
      replacementTitle: "Reviewed control boundary",
      replacementContent: "The coordinator owns truth; runners return structured results with evidence.",
      replacementReason: "replace stale wording after operator review",
      replacementTags: ["operator", "reviewed"],
    });
    assertEqual(editPayload.goal_id, "018f8f2f-1fd8-7688-bb12-8bfb6b756602", "memory edit payload preserves goal id");
    assertEqual(editPayload.replace_keys.length, 2, "memory edit payload preserves all replacement keys");
    assertEqual(editPayload.replacement_key, "mem-replacement", "memory edit payload preserves stable replacement key");
    assertEqual(editPayload.replacement_episode.title, "Reviewed control boundary", "memory edit payload preserves replacement title");
    assertEqual(editPayload.replacement_episode.source.actor, "operator", "memory edit payload marks operator source");
    assertEqual(editPayload.reason, "replace stale wording after operator review", "memory edit payload preserves review reason");

    const readyPreview = {
      data: {
        ready_to_edit: true,
        replacement_key: "mem-replacement",
        missing_keys: [],
        diffs: [
          {
            key: "mem-control-boundary",
            before_title: "Control boundary",
            before_excerpt: "Coordinator owns truth; runners return structured results.",
            after_title: "Reviewed control boundary",
            after_excerpt: "The coordinator owns truth; runners return structured results with evidence.",
          },
          {
            key: "mem-runner-policy",
            before_title: "Runner policy",
            before_excerpt: "Workers may spawn local subagents from prompt text.",
            after_title: "Reviewed control boundary",
            after_excerpt: "Workers request durable child tasks; the coordinator queues and routes them.",
          },
        ],
      },
    };
    assertEqual(previewReady(readyPreview), true, "memory preview helper treats ready payload as editable");
    const readyStatusMarkup = renderToStaticMarkup(React.createElement(PreviewStatus, { value: readyPreview }));
    assert(readyStatusMarkup.includes("status-done"), "ready memory preview renders done status tone");
    assert(readyStatusMarkup.includes(">Ready<"), "ready memory preview renders Ready label");
    const readyDiffMarkup = renderToStaticMarkup(React.createElement(MemoryDiffTable, { value: readyPreview }));
    for (const expected of [
      "Replacement mem-replacement",
      "mem-control-boundary",
      "Control boundary: Coordinator owns truth; runners return structured results.",
      "Reviewed control boundary: The coordinator owns truth; runners return structured results with evidence.",
      "mem-runner-policy",
      "Runner policy: Workers may spawn local subagents from prompt text.",
      "Reviewed control boundary: Workers request durable child tasks; the coordinator queues and routes them.",
      "Inspect preview",
    ]) {
      assert(readyDiffMarkup.includes(expected), `ready memory diff markup includes ${expected}`);
    }
    assert(!readyDiffMarkup.includes("Missing"), "ready memory diff does not render missing-key warning");

    const blockedPreview = {
      data: {
        ready_to_edit: false,
        replacement_key: "mem-replacement",
        missing_keys: ["mem-absent"],
        diffs: [],
      },
    };
    assertEqual(previewReady(blockedPreview), false, "memory preview helper blocks missing-key payload");
    const blockedStatusMarkup = renderToStaticMarkup(React.createElement(PreviewStatus, { value: blockedPreview }));
    assert(blockedStatusMarkup.includes("status-blocked"), "blocked memory preview renders blocked status tone");
    assert(blockedStatusMarkup.includes(">Blocked<"), "blocked memory preview renders Blocked label");
    const blockedDiffMarkup = renderToStaticMarkup(React.createElement(MemoryDiffTable, { value: blockedPreview }));
    assert(blockedDiffMarkup.includes("Missing mem-absent"), "blocked memory diff renders missing key warning");
    assert(blockedDiffMarkup.includes("No diff rows."), "blocked memory diff renders empty diff rows state");

    const emptyMarkup = renderToStaticMarkup(React.createElement(MemoryDiffTable, { value: null }));
    assert(emptyMarkup.includes("No replacement preview"), "empty memory diff renders no-preview state");
    assert(emptyMarkup.includes("Preview a memory edit."), "empty memory diff renders preview guidance");
  } finally {
    await viteServer.close();
  }
}

async function assertOperatorWorkflowRender() {
  const viteServer = await createViteServer({
    configFile: false,
    root: packageRoot,
    logLevel: "silent",
    plugins: [
      react(),
      {
        name: "coat-operator-workflow-render-smoke",
        enforce: "post",
        transform(code, id) {
          if (id.split("?")[0].endsWith("/src/spa/App.tsx")) {
            return `${code}\nexport { ApprovalList, Dashboard, EventSourcesPanel, GoalList, GraphStatusPanel, MemoryEventsTable, RunnersView, eventSourceTableRow, runnerTableRow, taskMatchesGraphFilter, taskStatusCounts };`;
          }
          return null;
        },
      },
    ],
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, hmr: false },
    appType: "custom",
  });

  try {
    const {
      ApprovalList,
      Dashboard,
      EventSourcesPanel,
      GoalList,
      GraphStatusPanel,
      MemoryEventsTable,
      RunnersView,
      eventSourceTableRow,
      runnerTableRow,
      taskMatchesGraphFilter,
      taskStatusCounts,
    } = await viteServer.ssrLoadModule("/src/spa/App.tsx");

    const goalId = "018f8f2f-1fd8-7688-bb12-8bfb6b756602";
    const goals = [
      {
        goal_id: goalId,
        title: "Operator workflow coverage",
        objective: "Exercise goal selection, task progress, and human gates.",
        status: "running",
        percent_done: 0.42,
        open_tasks: 4,
        blocked_tasks: 1,
      },
      {
        goal_id: "018f8f2f-1fd8-7688-bb12-8bfb6b756603",
        title: "Completed validation",
        objective: "Keep done goals visible but lower urgency.",
        status: "done",
        percent_done: 1,
        open_tasks: 0,
        blocked_tasks: 0,
      },
    ];
    const runnerRows = [
      {
        registration: {
          runner_id: "codex-runner-a",
          node_id: "local-dev-a",
          endpoint: "http://runner-a.example:9093",
          labels: { runtime: "model-provider", lane: "work" },
          max_concurrency: 3,
        },
        running_tasks: 1,
        capacity_remaining: 2,
        dispatchable: true,
        stale: false,
        full: false,
      },
    ];
    const approvals = [
      {
        approval_id: "approval-prod-deploy",
        goal_id: goalId,
        status: "pending",
        risk: "deployment",
      },
    ];
    const eventSources = [
      {
        source_id: "github-pr-webhook",
        kind: "webhook",
        enabled: true,
        approval_status: "pending",
        approval_id: "approval-source-github",
      },
    ];
    const overview = {
      runner_status: { data: runnerRows },
      approvals: { data: approvals },
      recent_events: {
        data: [
          { event_type: "TaskStarted", goal_id: goalId },
          { event_type: "ApprovalRequested", goal_id: goalId },
        ],
      },
      event_sources: { sources: eventSources },
    };

    const dashboardMarkup = renderToStaticMarkup(
      React.createElement(Dashboard, {
        overview,
        goals,
        selectedGoalId: goalId,
        onSelectGoal: () => {},
      }),
    );
    for (const expected of [
      "Active goals",
      ">2<",
      "Human queue",
      "Event sources",
      "github-pr-webhook · webhook",
      "pending · approval-source-github",
      "Goal attention",
      "Runner lanes",
    ]) {
      assert(dashboardMarkup.includes(expected), `operator dashboard markup includes ${expected}`);
    }

    const goalListMarkup = renderToStaticMarkup(React.createElement(GoalList, { goals, selectedGoalId: goalId, onSelect: () => {} }));
    for (const expected of [
      "goal-card active",
      "Operator workflow coverage",
      "Exercise goal selection, task progress, and human gates.",
      "42% complete · 4 open · 1 blocked",
      "Completed validation",
      "100% complete · 0 open · 0 blocked",
    ]) {
      assert(goalListMarkup.includes(expected), `goal list markup includes ${expected}`);
    }

    const taskRows = [
      { task_id: "task-failed", status: "failed" },
      { task_id: "task-blocked", status: "blocked" },
      { task_id: "task-approval", status: "waiting_approval" },
      { task_id: "task-running", status: "running" },
      { task_id: "task-done", status: "done" },
    ];
    const counts = taskStatusCounts(taskRows);
    assertEqual(counts.get("waiting_approval"), 1, "task status counts preserve raw status spelling");
    assertEqual(taskMatchesGraphFilter(taskRows[0], "attention"), true, "failed task matches attention filter");
    assertEqual(taskMatchesGraphFilter(taskRows[3], "active"), true, "running task matches active filter");
    assertEqual(taskMatchesGraphFilter(taskRows[4], "completed"), true, "done task matches completed filter");
    assertEqual(taskMatchesGraphFilter(taskRows[4], "attention"), false, "done task does not match attention filter");
    const graphStatusMarkup = renderToStaticMarkup(React.createElement(GraphStatusPanel, { counts, taskCount: taskRows.length }));
    for (const expected of [
      "3 need attention",
      "5 tasks · 1 running · 1 done",
      "Graph legend",
      "Approval",
      "human gate open",
    ]) {
      assert(graphStatusMarkup.includes(expected), `graph status markup includes ${expected}`);
    }

    const approvalMarkup = renderToStaticMarkup(
      React.createElement(ApprovalList, {
        rows: approvals,
        selectedGoalId: "",
        busy: false,
        onApprove: () => {},
      }),
    );
    assert(approvalMarkup.includes("deployment"), "approval list renders risk");
    assert(approvalMarkup.includes(`pending · ${goalId}`), "approval list renders goal-scoped pending state");
    assert(approvalMarkup.includes(">Approve<"), "approval list renders approval command");
    assert(!approvalMarkup.includes("disabled=\"\""), "approval command is enabled when approval and goal ids exist");

    const disabledApprovalMarkup = renderToStaticMarkup(
      React.createElement(ApprovalList, {
        rows: [{ approval_id: "", status: "pending", risk: "unknown gate" }],
        selectedGoalId: "",
        busy: false,
        onApprove: () => {},
      }),
    );
    assert(disabledApprovalMarkup.includes("disabled=\"\""), "approval command is disabled without a goal id");

    const runnersMarkup = renderToStaticMarkup(React.createElement(RunnersView, { overview }));
    for (const expected of [
      "model-provider / work (codex-runner-a)",
      "local-dev-a",
      "active",
      "2/3 free, 1 running",
      "http://runner-a.example:9093",
    ]) {
      assert(runnersMarkup.includes(expected), `runner fleet markup includes ${expected}`);
    }
    const runnerRow = runnerTableRow(runnerRows[0]);
    assertEqual(runnerRow[0], "model-provider / work (codex-runner-a)", "runner row derives display name from labels");
    assertEqual(runnerRow[3], "2/3 free, 1 running", "runner row derives remaining capacity");

    const eventsMarkup = renderToStaticMarkup(
      React.createElement(MemoryEventsTable, {
        selectedGoalId: goalId,
        value: {
          events: [
            {
              action: "edit",
              key: "mem-replacement",
              scope: "goal",
              summary: "operator replaced stale memory after preview",
            },
          ],
        },
        loading: false,
      }),
    );
    for (const expected of ["edit", "mem-replacement", "goal", "operator replaced stale memory after preview"]) {
      assert(eventsMarkup.includes(expected), `memory events markup includes ${expected}`);
    }

    const sourceMarkup = renderToStaticMarkup(React.createElement(EventSourcesPanel, { rows: eventSources }));
    assert(sourceMarkup.includes("github-pr-webhook · webhook"), "event source panel renders source identity");
    assert(sourceMarkup.includes("pending · approval-source-github"), "event source panel renders activation approval");
    const sourceRow = eventSourceTableRow(eventSources[0]);
    assertEqual(sourceRow[0], "github-pr-webhook · webhook", "event source row derives source label");
    assertEqual(sourceRow[1], "enabled", "event source row derives enabled state");
    assertEqual(sourceRow[2], "pending · approval-source-github", "event source row derives approval state");
  } finally {
    await viteServer.close();
  }
}

async function assertChatApi() {
  const objective = "Draft a smoke-test plan for the control gateway chat UI.";
  const response = await fetch(`${baseUrl}/api/chat`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      mode: "draft_plan",
      messages: [{ role: "user", content: objective }],
    }),
  });
  assert(response.ok, "chat api returns ok");
  const body = await response.json();
  assertEqual(body.provider, "stub", "chat provider");
  const draft = body.drafts.plan_draft;
  assert(draft, "chat returns plan draft");
  assertEqual(draft.objective, objective, "plan draft objective preserves operator request");
  assertEqual(draft.authoring.intake_summary, objective, "plan draft intake summary preserves operator request");
  assert(
    draft.authoring.acceptance_evidence.includes("plan can compile into a GoalSpec"),
    "plan draft requires compile-to-goal acceptance evidence",
  );
  assert(
    draft.authoring.acceptance_evidence.includes("initial tasks are coordinator-owned"),
    "plan draft requires coordinator-owned task acceptance evidence",
  );
  assert(
    draft.plan.distribution_notes.some((note) => note.includes("coordinator task tree")),
    "plan draft routes subagent work through the coordinator",
  );
  assert(
    Array.isArray(draft.initial_tasks) && draft.initial_tasks.length === 0,
    "draft plan does not invent executable initial tasks before planning review",
  );
  assertEqual(body.session_id, "operator:default", "chat response returns durable session id");
  assert(body.run_id, "chat response returns a run id");
  assertEqual(body.chat_run.status, "done", "chat response includes completed operational trace");
  assert(
    body.chat_run.steps.some((step) => step.stage === "journaling_turns"),
    "chat operational trace records journaling stage",
  );
  assertEqual(body.chat_log.backend, "jsonl", "chat falls back to local JSONL when goal-store is unavailable");
  assertEqual(body.chat_log.durable, true, "chat JSONL fallback is durable for the local server");

  const traceResponse = await fetch(`${baseUrl}/api/chat/runs/${encodeURIComponent(body.run_id)}`);
  assert(traceResponse.ok, "chat run trace api returns ok");
  const trace = await traceResponse.json();
  assertEqual(trace.run_id, body.run_id, "chat run trace preserves run id");
  assertEqual(trace.status, "done", "chat run trace reports done");
  assert(trace.steps.some((step) => step.stage === "using_stub"), "chat run trace exposes backend/stub stage");

  const searchResponse = await fetch(`${baseUrl}/api/chat`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      mode: "draft_search",
      messages: [{ role: "user", content: "Find current evidence for standard web search tooling." }],
    }),
  });
  assert(searchResponse.ok, "search chat api returns ok");
  const searchBody = await searchResponse.json();
  assert(searchBody.drafts.search_request, "search chat returns a structured search request");
  assertEqual(searchBody.drafts.search_request.intent, "operator_search", "search request is marked for operator search");
  assert(searchBody.drafts.steering_directive.kind.prompt.includes("<role>research</role>"), "search chat returns a coordinator-owned research task proposal");
}

async function assertDurableChatSessionApi() {
  const response = await fetch(`${baseUrl}/api/chat/session?session_id=${encodeURIComponent("operator:default")}`);
  assert(response.ok, "chat session api returns ok");
  const body = await response.json();
  assertEqual(body.session_id, "operator:default", "chat session id is preserved");
  assertEqual(body.chat_log.backend, "jsonl", "chat session reads from JSONL fallback");
  assertEqual(body.durable, true, "chat session reports durable storage");
  assert(Array.isArray(body.messages), "chat session returns messages");
  assert(body.messages.length >= 2, "chat session returns user and assistant turns");
  assertEqual(body.messages[0].role, "user", "chat session first turn is user");
  assertEqual(body.messages[0].content, "Draft a smoke-test plan for the control gateway chat UI.", "chat session preserves user prompt");
  assertEqual(body.messages[1].role, "assistant", "chat session second turn is assistant");
  assert(String(body.messages[1].content).includes("durable plan payload"), "chat session preserves assistant response");
}

async function assertChatApiDiscoversRegisteredModel() {
  const chatRequests = [];
  const chatServer = http.createServer(async (req, res) => {
    if (req.method !== "POST" || req.url !== "/v1/chat/completions") {
      res.writeHead(404, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "not found" }));
      return;
    }
    const request = JSON.parse(await readIncomingBody(req));
    chatRequests.push(request);
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({
      choices: [
        {
          message: {
            content: [
              "<coat_chat_assistant>",
              "  <context_json>{\"echoed_prompt_context\":true}</context_json>",
              "  {\"assistant\":\"live registry model handled the chat request\",\"drafts\":{\"plan_draft\":{\"objective\":\"registry-backed chat\"}}}",
              "</coat_chat_assistant>",
            ].join("\n"),
          },
        },
      ],
    }));
  });
  const chatPort = await listenLocal(chatServer);

  const registryServer = http.createServer((req, res) => {
    if (req.method !== "GET" || req.url !== "/runners/status") {
      res.writeHead(404, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: "not found" }));
      return;
    }
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify([
      {
        registration: {
          runner_id: "unlabeled-work-runner",
          node_id: "worker-node-a",
          endpoint: "http://work-runner.example:9093",
          labels: {
            runtime: "model-provider",
            lane: "work",
          },
          models: [
            {
              provider: "ollama",
              model: "work-model:latest",
              endpoint: `http://127.0.0.1:${chatPort}/v1`,
              params: { latency_class: "fast", max_output_tokens: 256 },
            },
          ],
          max_concurrency: 4,
          lease_ttl_seconds: 60,
        },
        running_tasks: 1,
        capacity_remaining: 3,
        dispatchable: true,
        stale: false,
        full: false,
      },
      {
        registration: {
          runner_id: "local-model-runner",
          node_id: "local-model-node",
          endpoint: "http://runner.example:9093",
          labels: {
            runtime: "model-provider",
            lane: "local-model",
            control_chat: "true",
            "chat.intent": "user_request",
            routing_scope: "operator_chat",
          },
          models: [
            {
              provider: "ollama",
              model: "llama3.2:latest",
              endpoint: `http://127.0.0.1:${chatPort}/v1`,
              params: { latency_class: "fast", max_output_tokens: 256 },
            },
          ],
          max_concurrency: 2,
          lease_ttl_seconds: 60,
        },
        running_tasks: 0,
        capacity_remaining: 2,
        dispatchable: true,
        stale: false,
        full: false,
      },
    ]));
  });
  const registryPort = await listenLocal(registryServer);

  const defaultDiscoveryPort = 21000 + (process.pid % 1000);
  const defaultDiscoveryBaseUrl = `http://127.0.0.1:${defaultDiscoveryPort}`;
  const defaultDiscoveryServer = spawn(process.execPath, ["dist/server.js"], {
    cwd: packageRoot,
    env: {
      ...process.env,
      HOST: "127.0.0.1",
      PORT: String(defaultDiscoveryPort),
      COAT_CONTROL_GATEWAY_TOKEN: "",
      COAT_CONTROL_MCP_TOKEN: "",
      COAT_CONTROL_CHAT_BACKEND: "",
      COAT_CONTROL_CHAT_COMPLETIONS_URL: "",
      COAT_CONTROL_CHAT_MODEL: "",
      COAT_LLM_GATEWAY_CHAT_MODEL: "",
      COAT_LLM_GATEWAY_DEFAULT_MODEL: "",
      OPENAI_API_KEY: "",
      COAT_GOAL_STORE_URL: "http://127.0.0.1:9",
      COAT_CONTROL_CHAT_STORE_BACKEND: "disabled",
      COAT_RUNNER_REGISTRY_URL: `http://127.0.0.1:${registryPort}`,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let defaultDiscoveryStdout = "";
  let defaultDiscoveryStderr = "";
  defaultDiscoveryServer.stdout.on("data", (chunk) => {
    defaultDiscoveryStdout += chunk;
  });
  defaultDiscoveryServer.stderr.on("data", (chunk) => {
    defaultDiscoveryStderr += chunk;
  });

  const discoveryPort = 20000 + (process.pid % 1000);
  const discoveryBaseUrl = `http://127.0.0.1:${discoveryPort}`;
  const discoveryServer = spawn(process.execPath, ["dist/server.js"], {
    cwd: packageRoot,
    env: {
      ...process.env,
      HOST: "127.0.0.1",
      PORT: String(discoveryPort),
      COAT_CONTROL_GATEWAY_TOKEN: "",
      COAT_CONTROL_MCP_TOKEN: "",
      COAT_CONTROL_CHAT_BACKEND: "runner_registry",
      COAT_CONTROL_CHAT_COMPLETIONS_URL: "",
      COAT_CONTROL_CHAT_MODEL: "",
      COAT_LLM_GATEWAY_CHAT_MODEL: "",
      COAT_LLM_GATEWAY_DEFAULT_MODEL: "",
      OPENAI_API_KEY: "",
      COAT_GOAL_STORE_URL: "http://127.0.0.1:9",
      COAT_CONTROL_CHAT_STORE_BACKEND: "disabled",
      COAT_RUNNER_REGISTRY_URL: `http://127.0.0.1:${registryPort}`,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let discoveryStdout = "";
  let discoveryStderr = "";
  discoveryServer.stdout.on("data", (chunk) => {
    discoveryStdout += chunk;
  });
  discoveryServer.stderr.on("data", (chunk) => {
    discoveryStderr += chunk;
  });

  try {
    await waitForProcessHealth(
      defaultDiscoveryBaseUrl,
      defaultDiscoveryServer,
      () => ({ stdout: defaultDiscoveryStdout, stderr: defaultDiscoveryStderr }),
    );
    const defaultResponse = await fetch(`${defaultDiscoveryBaseUrl}/api/chat`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        mode: "draft_plan",
        messages: [{ role: "user", content: "Do not use a registered runner unless opted in." }],
      }),
    });
    assert(defaultResponse.ok, "default chat api returns stub when only runners are configured");
    const defaultBody = await defaultResponse.json();
    assertEqual(defaultBody.provider, "stub", "default chat backend does not opportunistically use runners");
    assertEqual(defaultBody.chat_backend.backend_mode, "configured", "default chat backend mode is configured");
    assert(
      String(defaultBody.chat_backend.reason).includes("runner-registry discovery is disabled"),
      "default chat response explains runner discovery is disabled",
    );
    assertEqual(chatRequests.length, 0, "default chat backend does not call registered runner models");

    await waitForProcessHealth(discoveryBaseUrl, discoveryServer, () => ({ stdout: discoveryStdout, stderr: discoveryStderr }));
    const runnersResponse = await fetch(`${discoveryBaseUrl}/api/runners`);
    assert(runnersResponse.ok, "runners api returns normalized registry rows");
    const runnersBody = await runnersResponse.json();
    const runnerRows = Array.isArray(runnersBody.data) ? runnersBody.data : [];
    assertEqual(runnerRows.length, 2, "runners api preserves registered runner count");
    assertEqual(runnerRows[0].runner_id, "unlabeled-work-runner", "runners api flattens nested runner id");
    assertEqual(runnerRows[0].node_id, "worker-node-a", "runners api flattens nested node id");
    assertEqual(runnerRows[0].endpoint, "http://work-runner.example:9093", "runners api flattens nested endpoint");
    assertEqual(runnerRows[0].display_name, "model-provider / work", "runners api derives readable runner display name");
    assertEqual(runnerRows[0].status, "active", "runners api derives active status");
    assertEqual(runnerRows[0].capacity_remaining, 3, "runners api preserves remaining capacity");
    assertEqual(runnerRows[0].max_concurrency, 4, "runners api exposes registration max concurrency");
    const response = await fetch(`${discoveryBaseUrl}/api/chat`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        mode: "draft_plan",
        messages: [{ role: "user", content: "Use the registered local model." }],
      }),
    });
    assert(response.ok, "registry-discovered chat api returns ok");
    const body = await response.json();
    assertEqual(body.provider, "ollama", "registry-discovered chat provider");
    assertEqual(body.model, "llama3.2:latest", "registry-discovered chat model");
    assertEqual(body.chat_backend.source, "runner_registry", "registry-discovered backend source");
    assertEqual(body.chat_backend.backend_mode, "runner_registry", "registry-discovered backend mode is explicit");
    assertEqual(body.chat_backend.resolution_purpose, "operator_chat", "registry-discovered backend purpose");
    assertEqual(body.chat_backend.durable_task_dispatch, false, "registry chat discovery is not task dispatch");
    assertEqual(body.chat_backend.user_request, true, "registry chat discovery is marked as a user request");
    assertEqual(body.chat_backend.runner_id, "local-model-runner", "registry-discovered backend runner");
    assertEqual(body.chat_backend.runner_labels.control_chat, "true", "registry chat discovery requires chat labels");
    assertEqual(body.assistant, "live registry model handled the chat request", "registry model assistant response");
    assertEqual(body.drafts.plan_draft.objective, "registry-backed chat", "registry model tagged drafts are parsed");
    assertEqual(chatRequests.length, 1, "registry-discovered chat makes one model request");
    assertEqual(chatRequests[0].model, "llama3.2:latest", "registry-discovered chat request uses registered model");
  } finally {
    defaultDiscoveryServer.kill("SIGTERM");
    discoveryServer.kill("SIGTERM");
    await closeServer(registryServer);
    await closeServer(chatServer);
  }
}

async function assertOverviewApi() {
  const response = await fetch(`${baseUrl}/api/overview`);
  assert(response.ok, "overview api returns ok even when backend services are absent");
  const body = await response.json();
  assertEqual(body.control_surface, "coat-control-plane-web", "overview control surface");
  assert(
    String(body.authority_note).includes("Restate and the Rust services remain authoritative"),
    "overview states that the gateway is not the durable engine",
  );
  assert(Array.isArray(body.services), "overview includes service health rows");
  assert(
    body.services.some((service) => service.name === "goal-store" && typeof service.url === "string"),
    "overview reports goal-store backend health probe target",
  );
  assert(body.follow_ups && Array.isArray(body.follow_ups.items), "overview includes flattened follow-up queue");
  assert(body.goals && typeof body.goals.status === "number", "overview includes goal-store proxy result");
  assert(body.runner_status && typeof body.runner_status.status === "number", "overview includes runner registry proxy result");
}

async function assertGoalSubmitAssignsWorkflowId() {
  const response = await fetch(`${baseUrl}/api/goals/submit`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      title: "Gateway-assigned goal id smoke",
      objective: "Submitting without an id should still target a durable workflow instance.",
    }),
  });
  assert(response.ok, "goal submit endpoint returns a gateway result");
  const body = await response.json();
  assertEqual(body.status, 0, "goal submit reports unavailable local Restate as proxy status 0");
  const match = String(body.url).match(/\/GoalWorkflow\/([0-9a-f-]{36})\/run$/);
  assert(match, "goal submit assigns a workflow id in the Restate URL");
  assert(
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(match[1]),
    "assigned workflow id is UUID-shaped",
  );
}

async function assertUnsupportedWorkflowHandlerGuard() {
  const goalId = "018f8f2f-1fd8-7688-bb12-8bfb6b756602";
  const response = await fetch(`${baseUrl}/api/goals/${goalId}/native-subagent-spawn`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({}),
  });
  assertEqual(response.status, 400, "unsupported workflow handler is rejected before proxying");
  const body = await response.json();
  assert(
    String(body.error).includes("unsupported workflow handler"),
    "unsupported workflow handler response explains the guardrail",
  );
}

async function assertBackendBackedControlSurfaces() {
  const goalId = "018f8f2f-1fd8-7688-bb12-8bfb6b756602";
  const compiledGoalId = "018f8f2f-1fd8-7688-bb12-8bfb6b756603";
  const planId = "plan-smoke-compile";
  const approvalId = "approval-prod-deploy";
  const calls = {
    goalStore: [],
    restate: [],
    notifier: [],
    memory: [],
  };
  const taskPrompt = "Compile the backend-backed smoke plan and preserve coordinator ownership.";
  const taskRows = [
    {
      goal_id: goalId,
      task_id: "task-plan",
      parent_task_id: null,
      subgoal_id: "sg-control-plane",
      title: "Compile plan before execution",
      role: "planner",
      status: "running",
      purpose_kind: "work",
      depth: 0,
      priority: 5,
      attempts: 1,
      payload_json: {
        prompt: taskPrompt,
        execution: { profile: "strict-local", local_tools: ["npm"], runner: "codex" },
        budget: { max_attempts: 2, max_runtime_seconds: 600 },
        sandbox: { profile: "workspace-write", network: "disabled" },
        done_criteria: { tests_pass: true, evidence: ["behavioral smoke assertions pass"] },
        dependencies: [],
        children: ["task-review"],
      },
    },
  ];
  const eventRows = [
    {
      goal_id: goalId,
      task_id: "task-plan",
      event_type: "TaskStarted",
      message: "planner accepted durable task",
      created_at: "2026-05-09T12:00:00Z",
    },
    {
      goal_id: goalId,
      task_id: "task-review",
      event_type: "TaskQueued",
      message: "review waits for checkpoint evidence",
      created_at: "2026-05-09T12:01:00Z",
    },
  ];
  const artifactRows = [
    {
      goal_id: goalId,
      task_id: "task-plan",
      artifact_id: "artifact-diff",
      kind: "git_diff",
      uri: "git:smoke-diff",
    },
  ];
  const checkpointRows = [
    {
      checkpoint_id: "cp-git-1",
      goal_id: goalId,
      task_id: "task-plan",
      kind: "git",
      git: { branch: "codex/smoke", commit: "abc1234" },
      snapshot_uri: "s3://coat-smoke/checkpoints/cp-git-1.tar.zst",
      created_at: "2026-05-09T12:02:00Z",
    },
  ];
  const approvalRows = [
    {
      approval_id: approvalId,
      goal_id: goalId,
      status: "pending",
      risk: "deployment",
      reason: "requires explicit human gate before execution",
    },
  ];
  const threadRows = [
    {
      thread_key: `approval:${approvalId}`,
      goal_id: goalId,
      entries: 2,
      latest_status: "waiting_operator",
      latest_message: "Approve before execution",
    },
  ];

  const goalStoreServer = http.createServer(async (req, res) => {
    const request = await captureRequest(req);
    calls.goalStore.push(request);
    if (request.method === "GET" && request.path === "/healthz") {
      respondJson(res, 200, { service: "goal-store", ok: true });
      return;
    }
    if (request.method === "GET" && request.path === `/goal-store/goals/${goalId}`) {
      respondJson(res, 200, {
        goal_id: goalId,
        title: "Backend-backed smoke goal",
        status: "running",
        objective: "Exercise real control-plane projections",
      });
      return;
    }
    if (request.method === "GET" && request.path === `/goal-store/goals/${goalId}/tasks`) {
      respondJson(res, 200, { tasks: taskRows });
      return;
    }
    if (request.method === "GET" && request.path === `/goal-store/goals/${goalId}/events`) {
      respondJson(res, 200, { events: eventRows });
      return;
    }
    if (request.method === "GET" && request.path === `/goal-store/goals/${goalId}/artifacts`) {
      respondJson(res, 200, { artifacts: artifactRows });
      return;
    }
    if (request.method === "GET" && request.path === `/goal-store/goals/${goalId}/checkpoints`) {
      respondJson(res, 200, { checkpoints: checkpointRows });
      return;
    }
    if (request.method === "GET" && request.path === `/goal-store/goals/${goalId}/approvals`) {
      respondJson(res, 200, { approvals: approvalRows });
      return;
    }
    if (request.method === "GET" && request.path === "/goal-store/approvals") {
      respondJson(res, 200, { data: approvalRows, queue_owner: "goal-store-projection" });
      return;
    }
    if (request.method === "GET" && request.path === "/goal-store/plans") {
      respondJson(res, 200, {
        data: [
          {
            plan_id: planId,
            title: "Backend-backed smoke plan",
            status: "ready_for_review",
            mode: "durable_plan",
          },
        ],
      });
      return;
    }
    if (request.method === "POST" && request.path === `/goal-store/plans/${planId}/compile`) {
      respondJson(res, 200, {
        compiled_goal_id: compiledGoalId,
        received: request.body,
        goal_spec: {
          id: compiledGoalId,
          objective: "Compiled from durable smoke plan",
          plan: { subgoals: [{ id: "sg-control-plane", title: "Exercise backend-backed control surfaces" }] },
          initial_tasks: [{ task_id: "task-plan", role: "planner" }],
        },
      });
      return;
    }
    respondJson(res, 404, { error: "not found", path: request.path });
  });
  const restateServer = http.createServer(async (req, res) => {
    const request = await captureRequest(req);
    calls.restate.push(request);
    if (request.method === "GET" && request.path === "/health") {
      respondJson(res, 200, { service: "restate", ok: true });
      return;
    }
    if (request.method === "POST" && request.path === `/GoalWorkflow/${goalId}/status`) {
      respondJson(res, 200, {
        goal_id: goalId,
        status: "running",
        durable_owner: "restate",
        current_task_id: "task-plan",
      });
      return;
    }
    if (request.method === "POST" && request.path === `/GoalWorkflow/${goalId}/progress`) {
      respondJson(res, 200, {
        goal_id: goalId,
        satisfaction_score: 0.42,
        next_tasks: [{ task_id: "task-plan", runnable: true, reason: "checkpoint evidence pending" }],
      });
      return;
    }
    if (request.method === "POST" && request.path === `/GoalWorkflow/${goalId}/steer`) {
      respondJson(res, 200, { accepted: true, handler: "steer", directive: request.body });
      return;
    }
    if (request.method === "POST" && request.path === `/GoalWorkflow/${goalId}/approve`) {
      respondJson(res, 200, { accepted: true, handler: "approve", approval: request.body });
      return;
    }
    respondJson(res, 404, { error: "not found", path: request.path });
  });
  const notifierServer = http.createServer(async (req, res) => {
    const request = await captureRequest(req);
    calls.notifier.push(request);
    if (request.method === "GET" && request.path === "/healthz") {
      respondJson(res, 200, { service: "notifier", ok: true });
      return;
    }
    if (request.method === "GET" && request.path === "/threads") {
      respondJson(res, 200, { data: threadRows });
      return;
    }
    if (request.method === "GET" && request.path === `/threads/${encodeURIComponent(`approval:${approvalId}`)}`) {
      respondJson(res, 200, { thread: threadRows[0], messages: ["waiting for operator approval"] });
      return;
    }
    respondJson(res, 404, { error: "not found", path: request.path });
  });
  const memoryServer = http.createServer(async (req, res) => {
    const request = await captureRequest(req);
    calls.memory.push(request);
    if (request.method === "GET" && request.path === "/healthz") {
      respondJson(res, 200, { service: "memory-gateway", ok: true });
      return;
    }
    if (request.method === "POST" && request.path === "/memory/search") {
      respondJson(res, 200, {
        query: request.body?.query ?? "",
        results: [
          {
            key: "mem-control-boundary",
            goal_id: goalId,
            score: 0.91,
            text: "Coordinator owns truth; runners return structured results.",
          },
        ],
      });
      return;
    }
    if (request.method === "POST" && request.path === "/memory/context") {
      respondJson(res, 200, {
        pack: {
          goal_id: request.body?.goal_id ?? null,
          query: request.body?.query ?? "",
          facts: ["budget and sandbox policy travel with the durable task"],
        },
        context: [
          {
            key: "ctx-budget-sandbox",
            text: "Every worker task has a budget, sandbox profile, and done criteria.",
            provenance: "fake-memory-gateway",
          },
        ],
      });
      return;
    }
    if (request.method === "POST" && request.path === "/memory/edit/preview") {
      respondJson(res, 200, {
        ready_to_edit: true,
        replacement_key: request.body?.replacement_key ?? "mem-replacement",
        missing_keys: [],
        diffs: [
          {
            key: request.body?.replace_keys?.[0] ?? "mem-control-boundary",
            before_title: "Control boundary",
            before_excerpt: "Coordinator owns truth; runners return structured results.",
            after_title: request.body?.replacement_episode?.title ?? "Reviewed control boundary",
            after_excerpt: request.body?.replacement_episode?.content ?? "Reviewed replacement memory.",
          },
        ],
      });
      return;
    }
    if (request.method === "POST" && request.path === "/memory/edit") {
      respondJson(res, 200, {
        status: "edited",
        goal_id: request.body?.goal_id ?? null,
        replacement_key: request.body?.replacement_key ?? "mem-replacement",
        retracted_keys: request.body?.replace_keys ?? [],
        reason: request.body?.reason ?? "",
      });
      return;
    }
    if (request.method === "GET" && request.path === `/memory/events/${encodeURIComponent(goalId)}`) {
      respondJson(res, 200, {
        goal_id: goalId,
        events: [
          {
            action: "edit",
            key: "mem-replacement",
            scope: "goal",
            summary: "operator replaced stale control-boundary memory",
          },
        ],
      });
      return;
    }
    respondJson(res, 404, { error: "not found", path: request.path });
  });

  const goalStorePort = await listenLocal(goalStoreServer);
  const restatePort = await listenLocal(restateServer);
  const notifierPort = await listenLocal(notifierServer);
  const memoryPort = await listenLocal(memoryServer);
  const backendPort = 22000 + (process.pid % 1000);
  const backendBaseUrl = `http://127.0.0.1:${backendPort}`;
  const backendServer = spawn(process.execPath, ["dist/server.js"], {
    cwd: packageRoot,
    env: {
      ...process.env,
      HOST: "127.0.0.1",
      PORT: String(backendPort),
      COAT_CONTROL_GATEWAY_TOKEN: "",
      COAT_CONTROL_MCP_TOKEN: "",
      COAT_CONTROL_CHAT_STORE_BACKEND: "disabled",
      COAT_GOAL_STORE_URL: `http://127.0.0.1:${goalStorePort}`,
      COAT_RESTATE_INGRESS: `http://127.0.0.1:${restatePort}`,
      COAT_RESTATE_ADMIN_URL: `http://127.0.0.1:${restatePort}`,
      COAT_NOTIFIER_URL: `http://127.0.0.1:${notifierPort}`,
      COAT_MEMORY_GATEWAY_URL: `http://127.0.0.1:${memoryPort}`,
      COAT_MEMORY_GATEWAY_TOKEN: "memory-smoke-token",
      COAT_EVENT_GATEWAY_URL: "http://127.0.0.1:9",
      COAT_RUNNER_REGISTRY_URL: "http://127.0.0.1:9",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let backendStdout = "";
  let backendStderr = "";
  backendServer.stdout.on("data", (chunk) => {
    backendStdout += chunk;
  });
  backendServer.stderr.on("data", (chunk) => {
    backendStderr += chunk;
  });

  try {
    await waitForProcessHealth(backendBaseUrl, backendServer, () => ({ stdout: backendStdout, stderr: backendStderr }));

    const snapshotResponse = await fetch(`${backendBaseUrl}/api/goals/${goalId}`);
    assert(snapshotResponse.ok, "backend-backed goal snapshot returns ok");
    const snapshot = await snapshotResponse.json();
    assertEqual(snapshot.goal_id, goalId, "goal snapshot preserves requested goal id");
    assertEqual(snapshot.goal_store_goal.status, 200, "goal snapshot includes successful goal projection status");
    assertEqual(snapshot.goal_store_goal.data.objective, "Exercise real control-plane projections", "goal snapshot preserves backend objective");
    assertEqual(snapshot.workflow_status.data.durable_owner, "restate", "goal snapshot includes Restate workflow status");
    assertEqual(snapshot.workflow_progress.data.satisfaction_score, 0.42, "goal snapshot includes workflow progress score");
    assertEqual(snapshot.checkpoints.data.checkpoints[0].snapshot_uri, "s3://coat-smoke/checkpoints/cp-git-1.tar.zst", "goal snapshot includes checkpoint refs");
    assertEqual(snapshot.approvals.data.approvals[0].approval_id, approvalId, "goal snapshot includes human approval gates");
    assertEqual(snapshot.agent_activity.length, 1, "goal snapshot derives one agent activity row from projected tasks");
    const activity = snapshot.agent_activity[0];
    assertEqual(activity.task_id, "task-plan", "agent activity preserves task id");
    assertEqual(activity.current_prompt, taskPrompt, "agent activity exposes current prompt payload");
    assertEqual(activity.runnable, true, "agent activity merges Restate progress runnable state");
    assertEqual(activity.progress.reason, "checkpoint evidence pending", "agent activity preserves progress reason");
    assertEqual(activity.recent_events[0].event_type, "TaskStarted", "agent activity attaches task-local events");
    assertEqual(activity.artifacts[0].uri, "git:smoke-diff", "agent activity attaches task-local artifacts");
    assertEqual(activity.execution.local_tools[0], "npm", "agent activity exposes execution local tools");
    assertEqual(activity.budget.max_attempts, 2, "agent activity exposes task budget");
    assertEqual(activity.sandbox.network, "disabled", "agent activity exposes sandbox policy");
    assertEqual(activity.done_criteria.tests_pass, true, "agent activity exposes done criteria");

    const mcpSnapshot = await callMcpAt(backendBaseUrl, "coat_goal_snapshot", { goal_id: goalId });
    assertEqual(mcpSnapshot.agent_activity[0].task_id, "task-plan", "mcp goal snapshot uses the same backend aggregation path");

    const checkpointHistory = await callMcpAt(backendBaseUrl, "coat_checkpoint_history", { goal_id: goalId });
    assertEqual(checkpointHistory.data.checkpoints[0].git.commit, "abc1234", "checkpoint history returns git checkpoint commit");
    assertEqual(checkpointHistory.data.checkpoints[0].task_id, "task-plan", "checkpoint history preserves task ownership");

    const threadsResponse = await fetch(`${backendBaseUrl}/api/human/threads`);
    assert(threadsResponse.ok, "human threads api returns ok");
    const threads = await threadsResponse.json();
    assertEqual(threads.data.data[0].thread_key, `approval:${approvalId}`, "human threads preserve notifier thread key");
    assertEqual(threads.data.data[0].latest_status, "waiting_operator", "human threads preserve waiting status");
    const approvalQueue = await callMcpAt(backendBaseUrl, "coat_approval_queue", { limit: 3 });
    assertEqual(approvalQueue.data.data[0].approval_id, approvalId, "approval queue reads projected human gate");
    assert(
      calls.goalStore.some((call) => call.url === "/goal-store/approvals?limit=3"),
      "mcp approval queue forwards requested limit to goal-store",
    );
    const approvalResponse = await fetch(`${backendBaseUrl}/api/goals/${goalId}/approve`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ approval_id: approvalId, approved: true, note: "smoke approval" }),
    });
    assert(approvalResponse.ok, "goal approval api returns ok");
    const approvalBody = await approvalResponse.json();
    assertEqual(approvalBody.data.handler, "approve", "goal approval routes through workflow approve handler");
    const approvalCall = calls.restate.find((call) => call.url === `/GoalWorkflow/${goalId}/approve`);
    assertEqual(approvalCall?.body?.approval_id, approvalId, "goal approval forwards approval id");
    assertEqual(approvalCall?.body?.approved, true, "goal approval forwards operator decision");

    const compileResponse = await fetch(`${backendBaseUrl}/api/plans/${planId}/compile`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ operator: "smoke", emit_initial_tasks: true }),
    });
    assert(compileResponse.ok, "plan compile api returns ok");
    const compileBody = await compileResponse.json();
    assertEqual(compileBody.data.compiled_goal_id, compiledGoalId, "plan compile returns compiled GoalSpec id");
    assertEqual(compileBody.data.goal_spec.initial_tasks[0].role, "planner", "plan compile preserves coordinator-owned initial task");
    const apiCompileCall = calls.goalStore.find((call) => call.url === `/goal-store/plans/${planId}/compile` && call.body?.operator === "smoke");
    assertEqual(apiCompileCall?.body?.emit_initial_tasks, true, "plan compile forwards operator compile options");
    const mcpCompile = await callMcpAt(backendBaseUrl, "coat_plan_compile", { plan_id: planId, operator: "mcp-smoke" });
    assertEqual(mcpCompile.data.received.plan_id, planId, "mcp plan compile includes durable plan id in forwarded body");

    const directive = {
      id: "directive-smoke",
      kind: { kind: "add_constraint", constraint: "Keep fake backend assertions behavior-level." },
      message: "Pin behavioral smoke coverage.",
    };
    const steerResponse = await fetch(`${backendBaseUrl}/api/goals/${goalId}/steer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(directive),
    });
    assert(steerResponse.ok, "goal steering api returns ok");
    const steerBody = await steerResponse.json();
    assertEqual(steerBody.data.handler, "steer", "goal steering routes through workflow steer handler");
    assertEqual(steerBody.data.directive.kind.kind, "add_constraint", "goal steering forwards directive kind");
    const steerCall = calls.restate.find((call) => call.url === `/GoalWorkflow/${goalId}/steer`);
    assertEqual(steerCall?.body?.message, "Pin behavioral smoke coverage.", "goal steering forwards operator message");

    const memorySearchResponse = await fetch(`${backendBaseUrl}/api/memory/search`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ goal_id: goalId, query: "coordinator truth", limit: 2 }),
    });
    assert(memorySearchResponse.ok, "memory search api returns ok");
    const memorySearch = await memorySearchResponse.json();
    assertEqual(memorySearch.data.results[0].key, "mem-control-boundary", "memory search returns ranked memory evidence");
    const memorySearchCall = calls.memory.find((call) => call.url === "/memory/search" && call.body?.query === "coordinator truth");
    assertEqual(memorySearchCall?.authorization, "Bearer memory-smoke-token", "memory search forwards memory gateway bearer token");
    assertEqual(memorySearchCall?.body?.limit, 2, "memory search forwards scoped search limit");

    const memoryContext = await callMcpAt(backendBaseUrl, "coat_memory_context", {
      goal_id: goalId,
      query: "task graph policy",
      limit: 3,
    });
    assertEqual(memoryContext.data.pack.goal_id, goalId, "memory context preserves scoped goal id");
    assert(
      memoryContext.data.context[0].text.includes("budget, sandbox profile, and done criteria"),
      "memory context returns durable task policy evidence",
    );
    const memoryContextCall = calls.memory.find((call) => call.url === "/memory/context" && call.body?.query === "task graph policy");
    assertEqual(memoryContextCall?.authorization, "Bearer memory-smoke-token", "memory context forwards memory gateway bearer token");
    assertEqual(memoryContextCall?.body?.limit, 3, "memory context forwards requested context limit");

    const memoryEditPayload = {
      goal_id: goalId,
      replace_keys: ["mem-control-boundary"],
      replacement_key: "mem-replacement",
      replacement_episode: {
        title: "Reviewed control boundary",
        content: "The coordinator owns truth; runners return structured results with evidence.",
        source: { source_type: "human", uri: null, actor: "operator" },
        artifacts: [],
        tags: ["operator", "reviewed"],
      },
      reason: "replace stale wording after operator review",
    };
    const memoryPreviewResponse = await fetch(`${backendBaseUrl}/api/memory/edit-preview`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(memoryEditPayload),
    });
    assert(memoryPreviewResponse.ok, "memory edit preview api returns ok");
    const memoryPreview = await memoryPreviewResponse.json();
    assertEqual(memoryPreview.data.ready_to_edit, true, "memory edit preview confirms edit readiness");
    assertEqual(memoryPreview.data.diffs[0].key, "mem-control-boundary", "memory edit preview returns replaced key diff");
    const memoryPreviewCall = calls.memory.find((call) => call.url === "/memory/edit/preview" && call.body?.replacement_key === "mem-replacement");
    assertEqual(memoryPreviewCall?.authorization, "Bearer memory-smoke-token", "memory edit preview forwards bearer token");

    const memoryEditResponse = await fetch(`${backendBaseUrl}/api/memory/edit`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...memoryEditPayload, task_id: null, scope: "goal", store: null }),
    });
    assert(memoryEditResponse.ok, "memory edit api returns ok");
    const memoryEdit = await memoryEditResponse.json();
    assertEqual(memoryEdit.data.status, "edited", "memory edit applies through backend proxy");
    assertEqual(memoryEdit.data.retracted_keys[0], "mem-control-boundary", "memory edit forwards replacement keys");
    const memoryEditCall = calls.memory.find((call) => call.url === "/memory/edit" && call.body?.replacement_key === "mem-replacement");
    assertEqual(memoryEditCall?.authorization, "Bearer memory-smoke-token", "memory edit forwards bearer token");

    const memoryEventsResponse = await fetch(`${backendBaseUrl}/api/memory/events/${encodeURIComponent(goalId)}`);
    assert(memoryEventsResponse.ok, "memory events api returns ok");
    const memoryEvents = await memoryEventsResponse.json();
    assertEqual(memoryEvents.data.events[0].key, "mem-replacement", "memory events return replacement history");
    const memoryEventsCall = calls.memory.find((call) => call.url === `/memory/events/${encodeURIComponent(goalId)}`);
    assertEqual(memoryEventsCall?.authorization, "Bearer memory-smoke-token", "memory events forwards bearer token");

    const mcpMemoryPreview = await callMcpAt(backendBaseUrl, "coat_memory_edit_preview", memoryEditPayload);
    assertEqual(mcpMemoryPreview.data.ready_to_edit, true, "mcp memory preview returns edit readiness");
    const mcpMemoryEdit = await callMcpAt(backendBaseUrl, "coat_memory_edit", {
      ...memoryEditPayload,
      task_id: null,
      scope: "goal",
      store: null,
    });
    assertEqual(mcpMemoryEdit.data.status, "edited", "mcp memory edit applies through backend proxy");
    const mcpMemoryEvents = await callMcpAt(backendBaseUrl, "coat_memory_events", { goal_id: goalId });
    assertEqual(mcpMemoryEvents.data.events[0].action, "edit", "mcp memory events return durable event history");
  } finally {
    backendServer.kill("SIGTERM");
    await closeServer(goalStoreServer);
    await closeServer(restateServer);
    await closeServer(notifierServer);
    await closeServer(memoryServer);
  }
}

async function assertFollowUpsApi() {
  const response = await fetch(`${baseUrl}/api/follow-ups`);
  assert(response.ok, "follow-ups api returns ok");
  const body = await response.json();
  assert(Array.isArray(body.plans), "follow-ups response includes plans");
  assert(Array.isArray(body.items), "follow-ups response includes flattened items");
  assert(typeof body.follow_up_count === "number", "follow-ups response includes count");
}

async function assertFollowUpDraftApi() {
  const response = await fetch(`${baseUrl}/api/follow-ups/draft-plan`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      plan: "Smoke Plan",
      path: "docs/exec-plans/active/smoke.md",
      index: 0,
      text: "Make follow-ups actionable from the control surface.",
    }),
  });
  assert(response.ok, "follow-up draft api returns ok");
  const body = await response.json();
  assertEqual(body.mode, "draft_plan", "follow-up draft mode");
  assert(body.prompt.includes("<task>"), "follow-up draft prompt is structured");
  assert(body.prompt.includes("<instruction>MUST turn this durable continuation item"), "follow-up draft uses hard instruction text");
  assert(body.prompt.includes("MUST propose subgoals, evidence requirements, budget/sandbox assumptions, review gates"), "follow-up draft requests durable execution criteria");
  assert(body.prompt.includes("<source_plan>Smoke Plan</source_plan>"), "follow-up draft preserves source plan");
  assert(body.prompt.includes("<source_path>docs/exec-plans/active/smoke.md</source_path>"), "follow-up draft preserves source path");
  assert(body.prompt.includes("<follow_up>Make follow-ups actionable from the control surface.</follow_up>"), "follow-up draft prompt includes source item");
}

async function assertMcpTools() {
  const response = await fetch(`${baseUrl}/mcp`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list", params: {} }),
  });
  assert(response.ok, "mcp tools/list returns ok");
  const body = await response.json();
  const names = new Set(body.result.tools.map((tool) => tool.name));
  for (const expected of [
    "coat_goal_snapshot",
    "coat_chat_assist",
    "coat_steer_goal",
    "coat_follow_up_draft_plan",
    "coat_memory_edit_preview",
    "coat_apply_research_output",
  ]) {
    assert(names.has(expected), `mcp tools include ${expected}`);
  }
}

async function assertMcpChatAssistBehavior() {
  const objective = "Author a behavioral testing goal for the SPA control gateway.";
  const body = await callMcp("coat_chat_assist", {
    mode: "draft_goal",
    messages: [{ role: "user", content: objective }],
  });
  assertEqual(body.provider, "stub", "mcp chat provider");
  const goal = body.drafts.goal_spec;
  assert(goal, "mcp chat returns goal draft");
  assertEqual(goal.objective, objective, "mcp chat draft preserves objective");
  assertEqual(goal.authoring.intake_summary, objective, "mcp chat draft preserves intake");
  assert(goal.done_criteria.tests_pass === true, "mcp chat draft asks validator for passing tests");
  assert(goal.done_criteria.validator_score_min >= 0.85, "mcp chat draft sets validation threshold");
  assert(goal.initial_tasks.length === 1, "mcp chat seeds exactly one coordinator-owned decomposition task");
  assertEqual(goal.initial_tasks[0].role, "planner", "mcp chat seed task is planner-owned");
  assert(
    goal.plan.distribution_notes.some((note) => note.includes("Coordinator owns task creation")),
    "mcp chat draft preserves durable subagent routing rule",
  );
}

async function assertMcpFollowUpDraftBehavior() {
  const body = await callMcp("coat_follow_up_draft_plan", {
    plan: "Behavioral Coverage Plan",
    path: "docs/exec-plans/active/060-test-review-validator.md",
    index: 2,
    text: "Replace presence-only smoke tests with workflow tests that fail on broken goal authoring.",
  });
  assertEqual(body.mode, "draft_plan", "mcp follow-up draft mode");
  assertEqual(body.item.plan, "Behavioral Coverage Plan", "mcp follow-up preserves source plan");
  assertEqual(body.item.path, "docs/exec-plans/active/060-test-review-validator.md", "mcp follow-up preserves source path");
  assertEqual(body.item.index, 2, "mcp follow-up preserves source index");
  assert(
    body.prompt.includes("<follow_up>Replace presence-only smoke tests with workflow tests that fail on broken goal authoring.</follow_up>"),
    "mcp follow-up prompt preserves actionable text",
  );
  assert(
    body.prompt.includes("MUST identify any questions that block execution"),
    "mcp follow-up prompt requires blocking questions",
  );
}

async function assertMcpApplyResearchOutputBehavior() {
  const body = await callMcp("coat_apply_research_output", {
    goal_id: "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
    research_output: {
      use_plan: {
        facts_to_use: ["Use the supported framework adapter instead of inventing a custom runner bus."],
        proposed_task_updates: [
          {
            role: "research",
            title: "Validate standard runner integration libraries",
            prompt: "Prefer standard SDKs and typed contracts for new runner integrations.",
            reason: "research use plan identified an implementation dependency",
          },
        ],
      },
    },
  });
  assertEqual(body.goal_id, "018f8f2f-1fd8-7688-bb12-8bfb6b756602", "research apply preserves goal id");
  assert(Array.isArray(body.applied_directives), "research apply returns generated steering directives");
  assertEqual(body.applied_directives.length, 2, "research apply creates one constraint and one goal-update directive");
  assertEqual(body.applied_directives[0].kind.kind, "add_constraint", "research fact becomes constraint directive");
  assertEqual(body.applied_directives[1].kind.kind, "inject_task", "goal update becomes coordinator-owned task directive");
  assert(
    body.applied_directives[1].kind.prompt.includes("Prefer standard SDKs"),
    "research goal update is preserved in child task prompt",
  );
  assert(
    body.responses.every((response) => response.status === 0),
    "research apply reports backend proxy failures without losing generated directives",
  );
}

async function callMcp(name, args) {
  return callMcpAt(baseUrl, name, args);
}

async function callMcpAt(targetBaseUrl, name, args) {
  const response = await fetch(`${targetBaseUrl}/mcp`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: `smoke-${name}`,
      method: "tools/call",
      params: { name, arguments: args },
    }),
  });
  assert(response.ok, `${name} mcp call returns ok`);
  const envelope = await response.json();
  if (envelope.error) {
    throw new Error(`${name} mcp error: ${envelope.error.message}`);
  }
  const text = envelope.result?.content?.[0]?.text;
  assert(typeof text === "string" && text.length > 0, `${name} mcp call returns text content`);
  return JSON.parse(text);
}

async function waitForProcessHealth(targetBaseUrl, targetServer, readLogs) {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    if (targetServer.exitCode !== null) {
      const logs = readLogs();
      skipOnBindPermissionFailure(logs.stderr);
      throw new Error(`server exited early\nstdout:\n${logs.stdout}\nstderr:\n${logs.stderr}`);
    }
    try {
      const response = await fetch(`${targetBaseUrl}/healthz`);
      if (response.ok) {
        return;
      }
    } catch {
      await delay(100);
    }
  }
  const logs = readLogs();
  throw new Error(`timed out waiting for gateway\nstdout:\n${logs.stdout}\nstderr:\n${logs.stderr}`);
}

async function listenLocal(server) {
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  }).catch((error) => {
    if (isBindPermissionFailure(error)) {
      throw new SmokeSkip(`localhost bind denied (${error.code})`);
    }
    throw error;
  });
  return server.address().port;
}

async function closeServer(server) {
  if (!server.listening) {
    return;
  }
  await new Promise((resolve, reject) => {
    server.close((error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

async function readIncomingBody(req) {
  let body = "";
  for await (const chunk of req) {
    body += chunk;
  }
  return body;
}

async function captureRequest(req) {
  const url = new URL(req.url ?? "/", "http://127.0.0.1");
  const bodyText = await readIncomingBody(req);
  return {
    method: req.method ?? "",
    path: url.pathname,
    url: `${url.pathname}${url.search}`,
    authorization: String(req.headers.authorization ?? ""),
    body: bodyText.trim() ? JSON.parse(bodyText) : null,
  };
}

function respondJson(res, status, body) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

function skipOnBindPermissionFailure(text) {
  const match = String(text).match(/\b(EACCES|EPERM)\b[\s\S]*\b(?:listen|bind|127\.0\.0\.1|localhost)\b/i);
  if (match) {
    throw new SmokeSkip(`localhost bind denied (${match[1]})`);
  }
}

function isBindPermissionFailure(error) {
  return (
    error &&
    (error.code === "EACCES" || error.code === "EPERM") &&
    String(error.syscall ?? "").toLowerCase() === "listen"
  );
}

function assert(value, message) {
  if (!value) {
    throw new Error(message);
  }
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${expected}, got ${actual}`);
  }
}
