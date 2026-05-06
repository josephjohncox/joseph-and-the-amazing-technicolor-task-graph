import http from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const port = Number(process.env.PORT ?? "9092");
const mode = process.env.STAFF_ENGINEER_RUNNER_MODE ?? "stub";

type StaffEngineerRequest = {
  goal_id: string;
  task_id: string;
  repo_path?: string;
  issue_ref?: string;
  instruction: string;
  max_minutes: number;
};

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET" && req.url === "/healthz") {
      return json(res, 200, { status: "ok", mode });
    }
    if (req.method === "GET" && req.url === "/verify") {
      return json(res, 200, await verifyCtxrPackage());
    }
    if (req.method === "POST" && req.url === "/run-task") {
      const body = (await readJson(req)) as StaffEngineerRequest;
      return json(res, 200, runTask(body));
    }
    json(res, 404, { error: "not_found" });
  } catch (error) {
    json(res, 500, { error: error instanceof Error ? error.message : String(error) });
  }
});

server.listen(port, () => {
  console.log(`staff-engineer-runner-ts listening on :${port} (${mode})`);
});

async function verifyCtxrPackage(): Promise<Record<string, unknown>> {
  try {
    const { stdout } = await execFileAsync("npm", [
      "view",
      "@ctxr/agent-staff-engineer",
      "version",
      "--json",
    ]);
    return { available: true, version: JSON.parse(stdout), package: "@ctxr/agent-staff-engineer" };
  } catch (error) {
    return {
      available: false,
      package: "@ctxr/agent-staff-engineer",
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function runTask(request: StaffEngineerRequest): Record<string, unknown> {
  return {
    status: "blocked",
    branch: null,
    pr_url: null,
    summary:
      mode === "stub"
        ? `staff-engineer worker contract accepted task ${request.task_id}; live Claude Code execution is not enabled`
        : `staff-engineer runner mode ${mode} accepted ${request.task_id}`,
    unresolved_blockers: [
      "verify @ctxr/kit and @ctxr/agent-staff-engineer in the target environment",
      "configure tracker and Claude Code credentials before live runs",
    ],
  };
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
