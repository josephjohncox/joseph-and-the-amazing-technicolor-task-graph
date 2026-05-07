import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const defaultPort = 19000 + (process.pid % 1000);
const port = Number(process.env.COAT_CONTROL_SMOKE_PORT ?? String(defaultPort));
const baseUrl = `http://127.0.0.1:${port}`;
const packageRoot = fileURLToPath(new URL("..", import.meta.url));
const server = spawn(process.execPath, ["dist/server.js"], {
  cwd: packageRoot,
  env: {
    ...process.env,
    HOST: "127.0.0.1",
    PORT: String(port),
    COAT_CONTROL_GATEWAY_TOKEN: "",
    COAT_CONTROL_MCP_TOKEN: "",
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
  await assertStylesheet();
  await assertClientScript();
  await assertFollowUpsApi();
  await assertFollowUpDraftApi();
  await assertChatApi();
  await assertMcpTools();
  await assertMcpChatAssistBehavior();
  await assertMcpFollowUpDraftBehavior();
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
    "<div id=\"root\"></div>",
    "/assets/",
  ]) {
    assert(html.includes(expected), `html includes ${expected}`);
  }
}

async function assertClientScript() {
  const html = await (await fetch(`${baseUrl}/`)).text();
  const match = html.match(/src="([^"]+\.js)"/);
  assert(match, "root html references a built SPA asset");
  const response = await fetch(`${baseUrl}${match[1]}`);
  assert(response.ok, "client script returns ok");
  const script = await response.text();
  for (const expected of ["Task Graph Manager", "Ask COAT", "Shared Memory", "Technicolor", "Follow-Ups", "Continuation queue", "Draft plan from follow-up", "theme-control", "Graph legend", "No urgent task states", "Attention", "Completed", "Auto"]) {
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
  for (const expected of ["data-theme=dark", "--status-running", ".theme-control", ".graph-filter", ".graph-status-panel", ".followup-card", ".followup-meta", ".react-flow__node.task-node"]) {
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
  assert(body.prompt.includes("<instruction>MUST turn this execution-plan follow-up"), "follow-up draft uses hard instruction text");
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
