#!/usr/bin/env node
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

import { runTask, verifyCodexIntegration } from "../dist/index.js";

const outputDir = resolve(process.env.COAT_CODEX_APP_SERVER_PROOF_DIR ?? "target/coat-runtime-live-scaffold/proofs/codex_app_server");
const requestPath = resolve(process.env.COAT_CODEX_APP_SERVER_PROOF_REQUEST ?? "examples/agent-run-smoke.json");

function fail(message) {
  console.error(`[coat-codex-app-server-live-proof] ${message}`);
  process.exit(1);
}

function writeJson(name, value) {
  writeFileSync(resolve(outputDir, name), `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

mkdirSync(outputDir, { recursive: true });

const request = JSON.parse(readFileSync(requestPath, "utf8"));
const verify = await verifyCodexIntegration();
writeJson("verify.json", verify);

if (verify?.codex_app_server?.available !== true) {
  fail("Codex App Server verification failed; see verify.json");
}

const result = await runTask(request);
writeJson("run-task-result.json", result);

if (result.status !== "done") {
  fail(`Codex App Server live task returned status=${result.status}; see run-task-result.json`);
}

if (!Array.isArray(result.checkpoints) || result.checkpoints.length === 0) {
  fail("Codex App Server live task did not return checkpoints");
}

if (!Array.isArray(result.test_evidence)) {
  fail("Codex App Server live task did not return test_evidence");
}

const summary = {
  status: "passed",
  task_id: result.task_id,
  runner_id: result.runner_id,
  confidence: result.confidence,
  checkpoint_count: result.checkpoints.length,
  test_evidence_count: result.test_evidence.length,
  diagnostics: Array.isArray(result.diagnostics) ? result.diagnostics : [],
};
writeJson("summary.json", summary);
console.log(JSON.stringify(summary, null, 2));
