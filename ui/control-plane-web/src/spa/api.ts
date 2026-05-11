/**
 * Browser API client for the COAT control gateway.
 *
 * Purpose: keep all SPA reads and mutations behind the gateway API so React
 * components never become an engine, projection store, or runner dispatcher.
 *
 * Architecture reference: docs/design-docs/110-control-gateway-spa.md
 */
import type { ChatMessage, ChatResponse, ChatRunTrace, GoalSnapshot, JsonRecord, Overview } from "./types";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly payload: unknown,
  ) {
    super(message);
  }
}

export function authToken(): string {
  return localStorage.getItem("coat.control.token") ?? "";
}

export function setAuthToken(value: string): void {
  localStorage.setItem("coat.control.token", value);
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const token = authToken();
  const response = await fetch(path, {
    ...init,
    headers: {
      ...(init.headers ?? {}),
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
  });
  const text = await response.text();
  const payload = text ? safeJson(text) : null;
  if (!response.ok) {
    const message = payload && typeof payload === "object" && "error" in payload
      ? String((payload as JsonRecord).error)
      : text || response.statusText;
    throw new ApiError(message, response.status, payload);
  }
  return payload as T;
}

export function overview(): Promise<Overview> {
  return api<Overview>("/api/overview");
}

export function goals(): Promise<unknown> {
  return api("/api/goals?limit=100");
}

export function goalSnapshot(goalId: string): Promise<GoalSnapshot> {
  return api<GoalSnapshot>(`/api/goals/${encodeURIComponent(goalId)}`);
}

export function plans(): Promise<unknown> {
  return api("/api/plans?limit=100");
}

export function approvals(): Promise<unknown> {
  return api("/api/approvals?limit=100");
}

export function runners(): Promise<unknown> {
  return api("/api/runners");
}

export function threads(): Promise<unknown> {
  return api("/api/human/threads");
}

export function followUps(): Promise<unknown> {
  return api("/api/follow-ups");
}

export function draftFollowUpPlan(body: JsonRecord): Promise<{ mode?: string; prompt?: string; item?: JsonRecord }> {
  return api("/api/follow-ups/draft-plan", jsonPost(body));
}

export function memorySearch(body: JsonRecord): Promise<unknown> {
  return api("/api/memory/search", jsonPost(body));
}

export function memoryContext(body: JsonRecord): Promise<unknown> {
  return api("/api/memory/context", jsonPost(body));
}

export function memoryWrite(body: JsonRecord): Promise<unknown> {
  return api("/api/memory/write", jsonPost(body));
}

export function memoryEditPreview(body: JsonRecord): Promise<unknown> {
  return api("/api/memory/edit-preview", jsonPost(body));
}

export function memoryEdit(body: JsonRecord): Promise<unknown> {
  return api("/api/memory/edit", jsonPost(body));
}

export function memoryEvents(goalId: string): Promise<unknown> {
  return api(`/api/memory/events/${encodeURIComponent(goalId)}`);
}

export function chat(sessionId: string, mode: string, goalId: string, messages: ChatMessage[], runId?: string): Promise<ChatResponse> {
  return api<ChatResponse>("/api/chat", jsonPost({ session_id: sessionId, run_id: runId, mode, goal_id: goalId || undefined, messages }));
}

export function chatSession(sessionId: string): Promise<{ session_id: string; messages: ChatMessage[] }> {
  return api(`/api/chat/session?session_id=${encodeURIComponent(sessionId)}`);
}

export function chatRun(runId: string): Promise<ChatRunTrace> {
  return api<ChatRunTrace>(`/api/chat/runs/${encodeURIComponent(runId)}`);
}

export function steer(goalId: string, body: JsonRecord): Promise<unknown> {
  return api(`/api/goals/${encodeURIComponent(goalId)}/steer`, jsonPost(body));
}

export function approve(goalId: string, body: JsonRecord): Promise<unknown> {
  return api(`/api/goals/${encodeURIComponent(goalId)}/approve`, jsonPost(body));
}

function jsonPost(body: unknown): RequestInit {
  return {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  };
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

export function rowsFrom(value: unknown): JsonRecord[] {
  if (Array.isArray(value)) {
    return value.filter(isRecord);
  }
  if (isRecord(value)) {
    for (const key of ["goals", "tasks", "plans", "approvals", "threads", "items", "records", "events", "event_sources", "sources", "triggers"]) {
      const candidate = value[key];
      if (Array.isArray(candidate)) {
        return candidate.filter(isRecord);
      }
    }
    const data = value.data;
    if (data !== value) {
      return rowsFrom(data);
    }
  }
  return [];
}

export function isRecord(value: unknown): value is JsonRecord {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function at(value: unknown, path: string[]): unknown {
  let current = value;
  for (const key of path) {
    if (!isRecord(current)) {
      return null;
    }
    current = current[key];
  }
  return current;
}
