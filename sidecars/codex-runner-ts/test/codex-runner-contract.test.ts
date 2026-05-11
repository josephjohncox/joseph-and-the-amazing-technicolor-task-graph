import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import {
  loadMcpReplayFixture,
  loadReplayFixture,
  runTask,
  traceFromMcpReplayFixture,
  traceFromReplayFixture,
  verifyCodexIntegration,
} from "../src/index.ts";

const fixturePath = fileURLToPath(new URL("../../../examples/codex-app-server-replay.json", import.meta.url));
const mcpFixturePath = fileURLToPath(new URL("../../../examples/codex-mcp-fallback-replay.json", import.meta.url));
const requestPath = new URL("../../../examples/agent-run-smoke.json", import.meta.url);
const baseEnv = { ...process.env };

afterEach(() => {
  for (const key of Object.keys(process.env)) {
    if (!(key in baseEnv)) delete process.env[key];
  }
  for (const [key, value] of Object.entries(baseEnv)) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
});

function agentRunRequest(): Parameters<typeof runTask>[0] {
  return JSON.parse(readFileSync(requestPath, "utf8")) as Parameters<typeof runTask>[0];
}

test("replay mode deterministically returns structured Codex App Server evidence", async () => {
  process.env.CODEX_RUNNER_MODE = "replay";
  process.env.CODEX_REPLAY_FIXTURE = fixturePath;

  const first = await runTask(agentRunRequest());
  const second = await runTask(agentRunRequest());

  assert.deepEqual(first, second);
  assert.equal(first.status, "done");
  assert.equal(first.summary, "Replay Codex App Server turn produced structured code-worker evidence.");
  assert.equal(first.runner_id, "codex-runner-ts");
  assert.equal(first.confidence, 0.91);
  assert.equal(first.diagnostics.includes("mode=replay"), true);
  assert.equal(first.diagnostics.includes("codex_app_server_thread_id=thr_codex_replay_001"), true);
  assert.equal(first.diagnostics.includes("codex_app_server_turn_id=turn_codex_replay_001"), true);

  assert.ok(first.git_result && typeof first.git_result === "object");
  assert.equal(
    (first.git_result as { branch?: string }).branch,
    "jattg/task/018f8f2f-1fd8-7688-bb12-8bfb6b756601/018f8f2f-1fd8-7688-bb12-8bfb6b756602",
  );
  assert.equal((first.test_evidence[0] as { command?: string }).command, "npm run --prefix sidecars/codex-runner-ts build");
  assert.equal(first.test_evidence[0].passed, true);
  assert.equal((first.child_requests[0] as { role?: string }).role, "tester");
  assert.ok(first.object_artifacts.some((artifact) => JSON.stringify(artifact).includes("codex-app-server-replay.json")));
  assert.ok(
    first.checkpoints.some((checkpoint) =>
      JSON.stringify(checkpoint).includes("\"thread_id\":\"thr_codex_replay_001\""),
    ),
  );
  assert.equal(JSON.stringify(first).toLowerCase().includes("stub"), false);
});

test("replay fixture parser extracts thread, turn, items, and final structured output", () => {
  const trace = traceFromReplayFixture(loadReplayFixture(fixturePath));

  assert.equal(trace.source, "replay");
  assert.equal(trace.thread.id, "thr_codex_replay_001");
  assert.equal(trace.turn.id, "turn_codex_replay_001");
  assert.equal(trace.items.length, 3);
  assert.match(trace.final_response, /structured code-worker evidence/);
});

test("MCP fallback replay mode returns structured callable-tool evidence", async () => {
  process.env.CODEX_RUNNER_MODE = "mcp-replay";
  process.env.CODEX_MCP_REPLAY_FIXTURE = mcpFixturePath;

  const result = await runTask(agentRunRequest());

  assert.equal(result.status, "done");
  assert.equal(result.summary, "Replay Codex MCP fallback produced structured code-worker evidence.");
  assert.equal(result.confidence, 0.86);
  assert.equal(result.diagnostics.includes("mode=mcp-replay"), true);
  assert.equal(result.diagnostics.includes("codex_mcp_source=replay"), true);
  assert.equal(result.diagnostics.includes("codex_mcp_tool_calls=1"), true);
  assert.equal(result.test_evidence[0].command, "codex mcp-server replay: coat_codex_run_task");
  assert.equal(result.test_evidence[0].passed, true);
  assert.ok(result.artifacts.some((artifact) => artifact.uri.includes("codex-mcp://fallback-replay/")));
  assert.ok(
    result.checkpoints.some((checkpoint) => JSON.stringify(checkpoint).includes("\"tool\":\"coat_codex_run_task\"")),
  );
  assert.equal(JSON.stringify(result).toLowerCase().includes("stub"), false);
});

test("MCP fallback fixture parser extracts server, tool calls, and final output", () => {
  const trace = traceFromMcpReplayFixture(loadMcpReplayFixture(mcpFixturePath));

  assert.equal(trace.source, "replay");
  assert.equal(trace.server.command, "codex mcp-server");
  assert.equal(trace.tool_calls.length, 1);
  assert.match(trace.final_response, /MCP fallback produced structured/);
});

test("live mode blocks instead of fabricating work when App Server gates are missing", async () => {
  process.env.CODEX_RUNNER_MODE = "live";
  process.env.CODEX_AUTH_MODE = "app_server";
  delete process.env.CODEX_APP_SERVER_URL;
  delete process.env.CODEX_APP_SERVER_CWD;
  delete process.env.CODEX_WORKSPACE_DIR;
  delete process.env.CODEX_ALLOW_REPO_CWD_LIVE;

  const result = await runTask(agentRunRequest());

  assert.equal(result.status, "blocked");
  assert.match(result.summary, /Codex App Server live execution is not enabled/);
  assert.equal(result.diagnostics.includes("mode=live"), true);
  assert.ok(result.diagnostics.some((line) => line.includes("CODEX_APP_SERVER_URL is required")));
  assert.equal(result.test_evidence.length, 0);
  assert.equal(JSON.stringify(result).toLowerCase().includes("stub"), false);
});

test("stub mode remains explicit for local smoke", async () => {
  process.env.CODEX_RUNNER_MODE = "stub";

  const result = await runTask(agentRunRequest());

  assert.equal(result.status, "done");
  assert.match(result.summary, /^stub Codex actor accepted/);
  assert.equal(result.diagnostics.includes("mode=stub"), true);
});

test("provider verification profiles report skipped lanes without live credentials", async () => {
  process.env.RUNNER_MODELS_JSON = JSON.stringify([
    {
      provider: "codex",
      model: "codex-default",
      endpoint: null,
      priority: 100,
      weight: 1,
      context_window: null,
      features: ["tool_use", "json_schema", "streaming"],
      labels: {},
    },
    {
      provider: "vllm",
      model: "qwen2.5-coder",
      endpoint: "http://127.0.0.1:18000/v1",
      priority: 80,
      weight: 1,
      context_window: 32768,
      features: ["tool_use", "json_schema"],
      labels: {},
    },
    {
      provider: "hugging_face",
      model: "meta-llama/Llama-3.1-8B-Instruct",
      endpoint: "https://api-inference.huggingface.co/models/meta-llama/Llama-3.1-8B-Instruct",
      priority: 60,
      weight: 1,
      context_window: null,
      features: ["streaming"],
      labels: {},
    },
    {
      provider: "bedrock",
      model: "anthropic.claude-3-5-sonnet-20241022-v2:0",
      endpoint: null,
      priority: 50,
      weight: 1,
      context_window: null,
      features: ["tool_use"],
      labels: {},
    },
    {
      provider: "open_ai_compatible",
      model: "gateway-default",
      endpoint: "https://gateway.example.invalid/v1",
      priority: 40,
      weight: 1,
      context_window: null,
      features: ["tool_use", "json_schema"],
      labels: {},
    },
  ]);
  delete process.env.CODEX_APP_SERVER_URL;
  delete process.env.CODEX_VERIFY_APP_SERVER;
  delete process.env.CODEX_VERIFY_MCP;
  delete process.env.CODEX_VERIFY_PROVIDER_NETWORK;
  delete process.env.HF_TOKEN;
  delete process.env.HUGGING_FACE_HUB_TOKEN;
  delete process.env.AWS_PROFILE;
  delete process.env.AWS_ACCESS_KEY_ID;
  delete process.env.AWS_WEB_IDENTITY_TOKEN_FILE;
  delete process.env.COAT_LLM_GATEWAY_API_KEY;
  delete process.env.OPENAI_COMPATIBLE_API_KEY;
  delete process.env.OPENAI_API_KEY;

  const verification = await verifyCodexIntegration();
  const profiles = verification.provider_profiles as Array<Record<string, unknown>>;
  const byLane = new Map(profiles.map((profile) => [profile.lane_id, profile]));
  const profileFor = (provider: string, model: string) =>
    profiles.find((profile) => profile.provider === provider && profile.model === model);

  assert.equal(byLane.get("codex:app_server")?.status, "skipped");
  assert.match(String(byLane.get("codex:app_server")?.skipped_reason), /CODEX_APP_SERVER_URL/);
  assert.equal(byLane.get("codex:mcp")?.status, "skipped");
  assert.equal(profileFor("vllm", "qwen2.5-coder")?.status, "skipped");
  assert.match(String(profileFor("hugging_face", "meta-llama/Llama-3.1-8B-Instruct")?.skipped_reason), /Hugging Face token/);
  assert.match(String(profileFor("bedrock", "anthropic.claude-3-5-sonnet-20241022-v2:0")?.skipped_reason), /AWS Bedrock/);
  assert.match(String(profileFor("open_ai_compatible", "gateway-default")?.skipped_reason), /gateway API key/);
  assert.equal(profiles.every((profile) => profile.secret_values_exposed === false), true);
});
