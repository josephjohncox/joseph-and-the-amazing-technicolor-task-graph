import http from "node:http";
import { spawn } from "node:child_process";

type AgentRunRequest = {
  goal_id: string;
  task: {
    id: string;
    role: string;
    prompt: string;
    execution?: {
      model?: {
        candidates?: unknown[];
      };
      mcp?: unknown;
    };
  };
};

type AgentRunResult = {
  task_id: string;
  status: "done" | "partial" | "blocked" | "failed";
  summary: string;
  runner_id: string | null;
  model_used: unknown | null;
  mcp_context_used: unknown | null;
  artifacts: Array<{ kind: string; uri: string; description: string; sha256?: string | null }>;
  child_requests: unknown[];
  confidence: number;
  next_actions: string[];
  diagnostics: string[];
  notification_reports: unknown[];
};

const port = Number(process.env.PORT ?? "9091");
const mode = process.env.CODEX_RUNNER_MODE ?? "stub";

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET" && req.url === "/healthz") {
      return json(res, 200, { status: "ok", mode });
    }
    if (req.method === "POST" && req.url === "/run-task") {
      const body = (await readJson(req)) as AgentRunRequest;
      return json(res, 200, await runTask(body));
    }
    json(res, 404, { error: "not_found" });
  } catch (error) {
    json(res, 500, { error: error instanceof Error ? error.message : String(error) });
  }
});

server.listen(port, () => {
  console.log(`codex-runner-ts listening on :${port} (${mode})`);
});

async function runTask(request: AgentRunRequest): Promise<AgentRunResult> {
  if (mode === "mcp-healthcheck") {
    await ensureCodexMcpStarts();
  }

  return {
    task_id: request.task.id,
    status: "done",
    summary:
      mode === "stub"
        ? `stub Codex runner accepted ${request.task.role} task ${request.task.id}`
        : `Codex runner ${mode} accepted ${request.task.id}; live integration is intentionally gated by environment`,
    runner_id: process.env.RUNNER_ID ?? "codex-runner-ts",
    model_used: request.task.execution?.model?.candidates?.[0] ?? null,
    mcp_context_used: request.task.execution?.mcp ?? null,
    artifacts: [
      {
        kind: "report",
        uri: `memory://codex-runner/${request.task.id}`,
        description: "Codex runner placeholder artifact",
        sha256: null,
      },
    ],
    child_requests: [],
    confidence: 0.9,
    next_actions: [],
    diagnostics: [`mode=${mode}`],
    notification_reports: [],
  };
}

async function ensureCodexMcpStarts(): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn("codex", ["mcp-server"], { stdio: ["ignore", "ignore", "pipe"] });
    const timer = setTimeout(() => {
      child.kill();
      resolve();
    }, 1000);
    child.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      if (code === 0 || code === null) resolve();
      else reject(new Error(`codex mcp-server exited with ${code}`));
    });
  });
}

function json(res: http.ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body, null, 2));
}

async function readJson(req: http.IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  const raw = Buffer.concat(chunks).toString("utf8");
  return raw ? JSON.parse(raw) : {};
}
