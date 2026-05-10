import { spawn } from "node:child_process";
import http from "node:http";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const defaultPort = 19000 + (process.pid % 1000);
const port = Number(process.env.COAT_CONTROL_SMOKE_PORT ?? String(defaultPort));
const baseUrl = `http://127.0.0.1:${port}`;
const packageRoot = fileURLToPath(new URL("..", import.meta.url));
const chatJournalPath = `/tmp/coat-control-chat-smoke-${process.pid}-${Date.now()}.jsonl`;
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
  await assertOverviewApi();
  await assertFollowUpsApi();
  await assertFollowUpDraftApi();
  await assertChatApi();
  await assertDurableChatSessionApi();
  await assertChatApiDiscoversRegisteredModel();
  await assertGoalSubmitAssignsWorkflowId();
  await assertUnsupportedWorkflowHandlerGuard();
  await assertMcpTools();
  await assertMcpChatAssistBehavior();
  await assertMcpFollowUpDraftBehavior();
  await assertMcpApplyResearchOutputBehavior();
  console.log("coat-control-plane-web smoke passed");
} finally {
  server.kill("SIGTERM");
}

async function waitForHealth() {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    if (server.exitCode !== null) {
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
  const response = await fetch(`${baseUrl}/mcp`, {
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
