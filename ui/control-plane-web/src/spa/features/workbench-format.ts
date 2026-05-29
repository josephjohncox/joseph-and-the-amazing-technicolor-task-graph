import type { GoalRow } from "../types";

export type OperatorStateKey = "action-needed" | "running" | "waiting" | "reviewing" | "satisfied";

export type OperatorStateDefinition = {
  key: OperatorStateKey;
  label: string;
  detail: string;
  statuses: string[];
};

const knownStatusTones = new Set([
  "pending",
  "runnable",
  "running",
  "needs-validation",
  "waiting-approval",
  "waiting-input",
  "done",
  "submitted",
  "blocked",
  "failed",
  "cancelled",
]);

export const operatorStateDefinitions: OperatorStateDefinition[] = [
  {
    key: "action-needed",
    label: "Action needed",
    detail: "failed, blocked, or approval work",
    statuses: ["failed", "blocked", "waiting-approval"],
  },
  {
    key: "running",
    label: "Running",
    detail: "active agents and runnable frontier",
    statuses: ["running", "runnable"],
  },
  {
    key: "waiting",
    label: "Waiting",
    detail: "queued, submitted, or paused continuations",
    statuses: ["waiting-input", "pending", "submitted", "unknown"],
  },
  {
    key: "reviewing",
    label: "Reviewing",
    detail: "validation and evidence checks",
    statuses: ["needs-validation"],
  },
  {
    key: "satisfied",
    label: "Satisfied",
    detail: "accepted task evidence",
    statuses: ["done"],
  },
];

export const statusLegend = [
  { token: "failed", label: "Action needed", detail: "failed task" },
  { token: "blocked", label: "Action needed", detail: "blocked task" },
  { token: "waiting-approval", label: "Action needed", detail: "approval gate" },
  { token: "waiting-input", label: "Waiting", detail: "human prompt" },
  { token: "running", label: "Running", detail: "agent is active" },
  { token: "needs-validation", label: "Reviewing", detail: "evidence check" },
  { token: "runnable", label: "Running", detail: "ready frontier" },
  { token: "done", label: "Satisfied", detail: "accepted evidence" },
  { token: "submitted", label: "Waiting", detail: "projection sync" },
  { token: "pending", label: "Waiting", detail: "queued work" },
  { token: "cancelled", label: "Stopped", detail: "intentionally stopped" },
] as const;

export const statusPriority = new Map<string, number>(statusLegend.map((item, index) => [item.token, index]));

export function operatorStateForStatus(status: unknown): OperatorStateDefinition {
  const token = statusToken(status);
  return operatorStateDefinitions.find((definition) => definition.statuses.includes(token))
    ?? operatorStateDefinitions.find((definition) => definition.key === "waiting")
    ?? { key: "waiting", label: "Waiting", detail: "projection pending", statuses: ["unknown"] };
}

export function stateTone(state: OperatorStateKey): string {
  return `state-${state}`;
}

export function statusTone(status: unknown): string {
  const normalized = statusToken(status);
  return `status-${normalized}`;
}

export function statusColorVar(status: unknown): string {
  return `var(--${statusTone(status)})`;
}

export function statusToken(status: unknown): string {
  const normalized = normalizeStatus(status);
  return knownStatusTones.has(normalized) ? normalized : "unknown";
}

export function normalizeStatus(status: unknown): string {
  return String(status ?? "unknown")
    .trim()
    .toLowerCase()
    .replace(/_/g, "-")
    .replace(/[^a-z0-9-]/g, "") || "unknown";
}

export function statusLabel(status: unknown): string {
  return normalizeStatus(status).split("-").map((part) => part ? `${part[0]?.toUpperCase()}${part.slice(1)}` : part).join(" ");
}

export function goalOperatorState(goal: GoalRow): OperatorStateDefinition {
  const failed = numberValue(goal.failed_tasks) ?? 0;
  const blocked = numberValue(goal.blocked_tasks) ?? 0;
  if (failed > 0 || blocked > 0) {
    return operatorStateDefinitions.find((definition) => definition.key === "action-needed") ?? operatorStateForStatus("blocked");
  }
  return operatorStateForStatus(goal.status);
}

export function goalNextAction(goal: GoalRow): string {
  const failed = numberValue(goal.failed_tasks) ?? 0;
  const blocked = numberValue(goal.blocked_tasks) ?? 0;
  const openTasks = numberValue(goal.open_tasks) ?? 0;
  const state = goalOperatorState(goal);
  if (failed > 0 || blocked > 0) {
    return "Review blockers";
  }
  if (state.key === "reviewing") {
    return "Review evidence";
  }
  if (state.key === "running") {
    return "Monitor active work";
  }
  if (state.key === "satisfied") {
    return "Confirm satisfaction";
  }
  return openTasks > 0 ? "Dispatch or wait" : "Refresh projection";
}

export function friendlyRef(value: unknown): string {
  const ref = shortRef(value);
  return ref ? `Ref ${ref}` : "";
}

export function shortRef(value: unknown): string {
  const raw = stringValue(value);
  if (!raw) {
    return "";
  }
  const uuid = raw.match(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i)?.[0];
  const segments = raw.split(/[/:#?]+/).filter(Boolean);
  const candidate = uuid ?? segments[segments.length - 1] ?? raw;
  if (candidate.length <= 16) {
    return candidate;
  }
  return `${candidate.slice(0, 8)}...${candidate.slice(-4)}`;
}

export function safeTestId(value: unknown): string {
  return (shortRef(value) || "item").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

export function stringValue(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

export function numberValue(value: unknown): number | null {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function tokenList(value: string): string[] {
  return value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function createRunId(prefix = "run"): string {
  return globalThis.crypto?.randomUUID?.() ?? `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function excerpt(value: unknown): string {
  const text = String(value ?? "").replace(/\s+/g, " ").trim();
  return text.length > 180 ? `${text.slice(0, 177)}...` : text;
}
