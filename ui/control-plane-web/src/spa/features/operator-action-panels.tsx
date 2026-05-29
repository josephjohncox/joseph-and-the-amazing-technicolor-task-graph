import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { ShieldCheck, XCircle } from "lucide-react";
import { useState } from "react";

import { at, createThunk, isRecord, resolveOperatorAction, rowsFrom } from "../api";
import { AdvancedInspect, EmptyState } from "../components/operator-primitives";
import type { ComposedGoalSnapshot, ComputeGraphNode, JsonRecord, TaskRow } from "../types";
import { taskId, taskRowsFromComposedSnapshot } from "./goal-graph-panel";
import { friendlyRef, normalizeStatus, stateTone, statusLabel, statusToken, statusTone, stringValue, numberValue, type OperatorStateKey } from "./workbench-format";

export type QueueFilter = "all" | "approvals" | "blocked" | "thunks" | "cancelled";
export type QueueGroup = Exclude<QueueFilter, "all">;
export type ActionNeededKind = "approval" | "blocked-task" | "waiting-task" | "thunk" | "cancelled";
export type ActionNeededItem = {
  actionId: string;
  key: string;
  kind: ActionNeededKind;
  label: string;
  status: string;
  detail: string;
  requestedInput: string;
  goalId: string;
  taskId: string;
  approvalId: string;
  thunkId: string;
  continuationId: string;
  risk: string;
  actionLabel: string;
};
export type ContinuationRow = {
  key: string;
  thunkId: string;
  continuationId: string;
  taskId: string;
  reason: string;
  requestedInput: string;
  status: string;
  waitKind: string;
  waitReference: string;
};
type HumanPromptSpec = {
  title: string;
  question: string;
  detail: string;
  primaryLabel: string;
  contextLabel: string;
  rejectLabel?: string;
  inputLabel: string;
  placeholder: string;
  showInput: boolean;
  defaultResponseSummary: string;
  createPromptLabel?: string;
  cancelLabel?: string;
};
export type ActionMutationInput = {
  item: ActionNeededItem;
  responseSummary?: string;
  intent?: ActionIntent;
};
export type ActionIntent = "primary" | "context" | "reject" | "create-human-prompt" | "cancel-goal";

const defaultContinuationSummary = "Operator chose Continue.";
const queueFilterOptions: Array<{ key: QueueFilter; label: string; detail: string }> = [
  { key: "all", label: "All", detail: "all operator actions and stopped history" },
  { key: "approvals", label: "Approvals", detail: "approval gates that need a human decision" },
  { key: "blocked", label: "Recovery", detail: "blocked or failed work that can be retried, replanned, or turned into a prompt" },
  { key: "thunks", label: "Human prompts", detail: "questions or decisions that need an operator" },
  { key: "cancelled", label: "Stopped", detail: "stopped work kept as read-only history" },
];
const queueGroupLabels: Record<QueueGroup, string> = {
  approvals: "Approvals",
  blocked: "Recovery",
  thunks: "Human prompts",
  cancelled: "Stopped history",
};
const queueGroupOrder: QueueGroup[] = ["approvals", "blocked", "thunks", "cancelled"];

export function applyActionEnvelopeToCache(queryClient: QueryClient, response: unknown, fallbackGoalId = ""): void {
  const snapshot = activeStateFromActionResponse(response);
  const goalId = stringValue(at(response, ["action", "goal_id"])) || stringValue(snapshot?.goal_id) || fallbackGoalId;
  if (goalId) {
    void queryClient.invalidateQueries({ queryKey: ["operator-goal", goalId] });
    void queryClient.invalidateQueries({ queryKey: ["operator-actions", goalId] });
  }
  void queryClient.invalidateQueries({ queryKey: ["operator-actions"] });
  void queryClient.invalidateQueries({ queryKey: ["operator-workspace"] });
  void queryClient.invalidateQueries({ queryKey: ["goals"] });
}

function activeStateFromActionResponse(response: unknown): ComposedGoalSnapshot | null {
  if (!activeStateEnvelopeIsFresh(response)) {
    return null;
  }
  const activeState = at(response, ["active_state"]);
  if (isRecord(activeState)) {
    return activeState as ComposedGoalSnapshot;
  }
  const nestedActiveState = at(response, ["result", "data", "active_state"]);
  return isRecord(nestedActiveState) ? nestedActiveState as ComposedGoalSnapshot : null;
}

function activeStateEnvelopeIsFresh(response: unknown): boolean {
  if (!isRecord(response)) {
    return false;
  }
  if ("active_state_available" in response) {
    return response.active_state_available === true;
  }
  const status = stringValue(response.active_state_status) || stringValue(at(response, ["observability", "active_state_status"]));
  return status ? status === "fresh" : true;
}

export function ActionNeededPanel({ items }: { items: ActionNeededItem[] }) {
  if (!items.length) {
    return null;
  }
  return (
    <section className="evidence-card action-needed-panel" aria-label="Action needed">
      <div className="section-heading">
        <h3>Action needed</h3>
        <span className="muted-small">{items.length} waiting</span>
      </div>
      <OperatorActionList items={items.slice(0, 5)} compact />
    </section>
  );
}

export function BlockerInsightPanel({ items, onOpenControls }: { items: ActionNeededItem[]; onOpenControls: () => void }) {
  const blockers = items.filter((item) => {
    const status = statusToken(item.status);
    return ["failed", "blocked", "waiting-approval", "waiting-input"].includes(status) || item.kind === "thunk" || item.kind === "approval";
  });
  if (!blockers.length) {
    return (
      <section className="blocker-panel stable" aria-label="Why blocked">
        <div>
          <span className="goal-context-kicker">Why blocked</span>
          <strong>No blockers projected</strong>
          <small>The coordinator has not projected failed, blocked, approval, or waiting-input work for this goal.</small>
        </div>
      </section>
    );
  }
  return (
    <section className="blocker-panel" aria-label="Why blocked">
      <div className="section-heading">
        <div>
          <span className="goal-context-kicker">Why blocked</span>
          <h3>{blockers.length} item{blockers.length === 1 ? "" : "s"} need attention</h3>
        </div>
        <button type="button" className="secondary-button" onClick={onOpenControls}>
          <ShieldCheck size={15} />
          Open actions
        </button>
      </div>
      <div className="blocker-list">
        {blockers.slice(0, 4).map((item) => (
          <article key={`blocker-${item.key}`} className="blocker-card">
            <span className={clsx("status-pill", statusTone(item.status))}>{statusLabel(item.status)}</span>
            <strong>{item.label}</strong>
            <p>{blockerReason(item)}</p>
            <div className="blocker-meta">
              <span>{blockerActionText(item)}</span>
              {item.taskId && <small title={item.taskId}>task {friendlyRef(item.taskId)}</small>}
              {item.approvalId && <small title={item.approvalId}>approval {friendlyRef(item.approvalId)}</small>}
              {item.thunkId && <small title={item.thunkId}>continuation {friendlyRef(item.thunkId)}</small>}
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function blockerReason(item: ActionNeededItem): string {
  if (item.detail) {
    return item.detail;
  }
  const status = statusToken(item.status);
  if (item.kind === "approval" || status === "waiting-approval") {
    return item.risk ? `Approval is waiting because risk was classified as ${item.risk}.` : "A human approval gate is waiting.";
  }
  if (isResumableThunkItem(item)) {
    return "This task needs your input before work can continue.";
  }
  if (status === "waiting-input") {
    return "The task is waiting for input. Create a prompt so the operator can answer.";
  }
  if (status === "failed") {
    return "The last task attempt failed and needs retry or replanning.";
  }
  return "The task is blocked and needs recovery, a dependency, or operator input.";
}

function blockerActionText(item: ActionNeededItem): string {
  const affordance = actionAffordanceForItem(item);
  if (affordance === "approve") return "Approve the gate in the Action queue.";
  if (affordance === "resume") return "Provide the missing input and resume the continuation.";
  const status = statusToken(item.status);
  if (status === "failed") return "Retry failed work or open operator actions for a different recovery path.";
  if (status === "blocked") return "Retry blocked work or open operator actions to replan.";
  if (status === "waiting-input") return "Create a human prompt or replan; no continuation can resume yet.";
  return "Review the item and choose a recovery action.";
}

export function EvidenceNextActionPanel({ snapshot, counts, taskCount, compact = false }: { snapshot: ComposedGoalSnapshot; counts: Map<string, number>; taskCount: number; compact?: boolean }) {
  const highlights = evidenceHighlights(snapshot, counts, taskCount);
  const nextAction = nextActionSummary(counts, taskCount, snapshot);
  return (
    <div className={clsx("evidence-next-panel", compact && "compact")}>
      <section className="evidence-card">
        <div className="section-heading">
          <h3>Evidence</h3>
        </div>
        <ul className="evidence-list">
          {highlights.map((item) => (
            <li key={item.label}>
              <span>{item.label}</span>
              <strong className={clsx("operator-state-pill", stateTone(item.state))}>{item.value}</strong>
            </li>
          ))}
        </ul>
      </section>
      <section className="evidence-card next-action-card">
        <div className="section-heading">
          <h3>Next action</h3>
          <span className={clsx("operator-state-pill", stateTone(nextAction.state))}>{nextAction.stateLabel}</span>
        </div>
        <strong>{nextAction.title}</strong>
        <p>{nextAction.detail}</p>
      </section>
    </div>
  );
}

function evidenceHighlights(snapshot: ComposedGoalSnapshot, counts: Map<string, number>, taskCount: number): Array<{ label: string; value: string; state: OperatorStateKey }> {
  const computeGraph = workflowComputeGraph(snapshot);
  const progress = workflowProgress(snapshot);
  const nodeCount = Array.isArray(computeGraph?.nodes) ? computeGraph.nodes.length : 0;
  const openThunks = Number(computeGraph?.open_thunks ?? 0) || countForStatusToken(counts, "waiting-input");
  const reviewing = countForStatusToken(counts, "needs-validation");
  const satisfied = countForStatusToken(counts, "done");
  const running = countForStatusToken(counts, "running") + countForStatusToken(counts, "runnable");
  const rounds = Number(progress?.open_mechanism_rounds ?? 0);
  return [
    { label: "Task states", value: `${taskCount} visible`, state: running > 0 ? "running" : "waiting" },
    { label: "Satisfied evidence", value: `${satisfied} accepted`, state: "satisfied" },
    { label: "Review evidence", value: `${reviewing} checks`, state: reviewing > 0 ? "reviewing" : "satisfied" },
    { label: "Wait evidence", value: `${openThunks} continuations`, state: openThunks > 0 ? "waiting" : "satisfied" },
    { label: "Compute graph", value: `${nodeCount} nodes`, state: nodeCount > 0 ? "running" : "waiting" },
    { label: "Mechanisms", value: `${rounds} open`, state: rounds > 0 ? "action-needed" : "satisfied" },
  ];
}

export function nextActionSummary(counts: Map<string, number>, taskCount: number, snapshot: ComposedGoalSnapshot): { title: string; detail: string; state: OperatorStateKey; stateLabel: string } {
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = countForStatusToken(counts, "waiting-input") || Number(workflowComputeGraph(snapshot)?.open_thunks ?? 0);
  const reviewing = countForStatusToken(counts, "needs-validation");
  const running = countForStatusToken(counts, "running");
  const runnable = countForStatusToken(counts, "runnable");
  const satisfied = countForStatusToken(counts, "done");
  if (failed + blocked + approvals > 0) {
    return {
      title: "Review action-needed work",
      detail: `${failed} failed · ${blocked} blocked · ${approvals} approvals`,
      state: "action-needed",
      stateLabel: "Action needed",
    };
  }
  if (continuations > 0) {
    return {
      title: "Answer human prompt",
      detail: `${continuations} waiting prompts can accept operator input.`,
      state: "waiting",
      stateLabel: "Waiting",
    };
  }
  if (reviewing > 0) {
    return {
      title: "Review evidence",
      detail: `${reviewing} validation checks are ready for satisfaction review.`,
      state: "reviewing",
      stateLabel: "Reviewing",
    };
  }
  if (running > 0) {
    return {
      title: "Monitor running work",
      detail: `${running} active tasks are producing evidence.`,
      state: "running",
      stateLabel: "Running",
    };
  }
  if (runnable > 0) {
    return {
      title: "Dispatch runnable frontier",
      detail: `${runnable} tasks are ready for an available runner.`,
      state: "running",
      stateLabel: "Running",
    };
  }
  if (taskCount > 0 && satisfied >= taskCount) {
    return {
      title: "Confirm satisfaction",
      detail: "Accepted evidence covers the visible task graph.",
      state: "satisfied",
      stateLabel: "Satisfied",
    };
  }
  return {
    title: "Refresh projection",
    detail: "Goal state is syncing from the coordinator.",
    state: "waiting",
    stateLabel: "Waiting",
  };
}

function countForStatusToken(counts: Map<string, number>, token: string): number {
  let total = 0;
  for (const [status, count] of counts) {
    if (statusToken(status) === token) {
      total += count;
    }
  }
  return total;
}

function workflowComputeGraph(snapshot: ComposedGoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_compute_graph as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

function workflowProgress(snapshot: ComposedGoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_progress as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

export function actionNeededItemsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot | JsonRecord, selectedGoalId = ""): ActionNeededItem[] {
  if (!snapshot) {
    return [];
  }
  const continuationRows = continuationRowsFromComposedSnapshot(snapshot as ComposedGoalSnapshot);
  const continuationTaskIds = new Set(continuationRows.map((row) => row.taskId).filter(Boolean));
  const taskRows = taskRowsFromComposedSnapshot(snapshot).filter((task) => {
    const status = statusToken(task.status ?? at(task, ["payload_json", "status"]));
    const id = taskId(task as TaskRow) || stringValue(task.id);
    return !(status === "waiting-input" && id && continuationTaskIds.has(id));
  });
  return mergeActionNeededItems([
    ...actionNeededItemsFromApprovals(rowsFrom(at(snapshot, ["approvals", "data", "approvals"]) ?? at(snapshot, ["approvals", "approvals"]) ?? at(snapshot, ["approvals", "data"]) ?? snapshot.approvals), selectedGoalId),
    ...actionNeededItemsFromTasks(taskRows, selectedGoalId),
    ...continuationRows.map((row) => actionItemFromContinuation(selectedGoalId || stringValue(snapshot.goal_id), row)),
  ]);
}

export function queueItemsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot | JsonRecord, selectedGoalId = ""): ActionNeededItem[] {
  if (!snapshot) {
    return [];
  }
  return mergeActionNeededItems([
    ...actionNeededItemsFromComposedSnapshot(snapshot, selectedGoalId),
    ...cancelledItemsFromComposedSnapshot(snapshot as ComposedGoalSnapshot, selectedGoalId),
  ]);
}

export function actionNeededItemsFromOperatorActions(value?: unknown): ActionNeededItem[] {
  return rowsFrom(value).map((row, index) => {
    const actionId = stringValue(row.action_id) || `operator-action:${index}`;
    const goalId = stringValue(row.goal_id);
    const taskId = stringValue(row.task_id);
    const kind = stringValue(row.kind);
    const status = normalizeStatus(row.status) || "pending";
    const approval = isRecord(row.approval) ? row.approval : {};
    const thunk = isRecord(row.thunk) ? row.thunk : {};
    const approvalId = stringValue(approval.approval_id) || (kind === "resolve_approval" ? actionId.split(":").at(-1) ?? "" : "");
    const thunkId = stringValue(thunk.id) || stringValue(thunk.thunk_id) || (kind === "resume_thunk" ? actionId.split(":").at(-1) ?? "" : "");
    const derivedKind: ActionNeededKind = kind === "resolve_approval"
      ? "approval"
      : kind === "resume_thunk"
        ? "thunk"
        : status === "cancelled"
          ? "cancelled"
          : status === "waiting_input" || status === "waiting-input"
            ? "waiting-task"
            : "blocked-task";
    return {
      actionId,
      key: actionId,
      kind: derivedKind,
      label: stringValue(row.title) || stringValue(row.question) || "Action needed",
      status,
      detail: stringValue(row.question) || stringValue(at(row, ["payload_json", "reason"])) || "Operator action projected by the backend.",
      requestedInput: stringValue(row.question) || stringValue(thunk.requested_input) || "",
      goalId,
      taskId,
      approvalId,
      thunkId,
      continuationId: stringValue(at(thunk, ["continuation", "continuation_id"])),
      risk: stringValue(approval.risk),
      actionLabel: derivedKind === "approval" ? "Approve" : derivedKind === "thunk" ? "Continue" : "Review",
    };
  });
}

export function actionNeededItemsFromApprovals(rows: JsonRecord[], selectedGoalId = ""): ActionNeededItem[] {
  return rows.filter((row) => {
    const status = normalizeStatus(row.status) || "pending";
    return status === "pending" || status === "waiting-approval";
  }).map((row, index) => {
    const approvalId = stringValue(row.approval_id) || stringValue(row.id) || stringValue(row.approval_ref);
    const goalId = stringValue(row.goal_id) || selectedGoalId;
    const risk = stringValue(row.risk) || stringValue(at(row, ["payload_json", "risk"]));
    const action = stringValue(row.requested_action) || stringValue(row.reason) || stringValue(at(row, ["payload_json", "requested_action"])) || stringValue(at(row, ["payload_json", "reason"])) || risk || "Review approval request";
    return {
      actionId: approvalId && goalId ? `approval:${goalId}:${approvalId}` : "",
      key: approvalId ? `approval:${approvalId}` : `approval:${goalId}:${index}`,
      kind: "approval" as const,
      label: action,
      status: normalizeStatus(row.status) || "pending",
      detail: risk ? `Risk: ${risk}` : "Human approval requested.",
      requestedInput: stringValue(row.requested_input) || stringValue(row.question) || stringValue(at(row, ["payload_json", "requested_input"])) || stringValue(at(row, ["payload_json", "question"])),
      goalId,
      taskId: stringValue(row.task_id) || stringValue(at(row, ["payload_json", "task_id"])),
      approvalId,
      thunkId: "",
      continuationId: "",
      risk,
      actionLabel: "Approve",
    };
  });
}

function actionNeededItemsFromTasks(rows: JsonRecord[], selectedGoalId = ""): ActionNeededItem[] {
  return rows
    .filter((task) => taskNeedsOperatorAttention(task))
    .map((task, index) => {
      const status = normalizeStatus(task.status ?? at(task, ["payload_json", "status"]));
      const id = taskId(task as TaskRow) || stringValue(task.id);
      const goalId = stringValue(task.goal_id) || stringValue(at(task, ["payload_json", "goal_id"])) || selectedGoalId;
      const title = taskTitle(task) || (id ? `Task ${friendlyRef(id)}` : "Task needs operator action");
      return {
        actionId: id && goalId ? `task:${goalId}:${id}` : "",
        key: id ? `task:${id}` : `task:${goalId}:${index}`,
        kind: status === "blocked" || status === "failed" ? "blocked-task" as const : "waiting-task" as const,
        label: title,
        status,
        detail: taskDetail(task, status),
        requestedInput: stringValue(task.requested_input) || stringValue(task.question) || stringValue(at(task, ["payload_json", "requested_input"])) || stringValue(at(task, ["payload_json", "question"])),
        goalId,
        taskId: id,
        approvalId: stringValue(task.approval_id) || stringValue(task.approval_ref) || stringValue(at(task, ["payload_json", "approval_id"])) || stringValue(at(task, ["payload_json", "approval_ref"])),
        thunkId: "",
        continuationId: "",
        risk: "",
        actionLabel: status === "waiting-approval" ? "Approve" : status === "waiting-input" ? "Create prompt" : "Review",
      };
    });
}

export function actionNeededItemsFromThunks(snapshot: ComposedGoalSnapshot, selectedGoalId = ""): ActionNeededItem[] {
  return continuationRowsFromComposedSnapshot(snapshot).map((row) => actionItemFromContinuation(selectedGoalId || stringValue(snapshot.goal_id), row));
}

function cancelledItemsFromComposedSnapshot(snapshot: ComposedGoalSnapshot, selectedGoalId = ""): ActionNeededItem[] {
  const taskItems = taskRowsFromComposedSnapshot(snapshot)
    .filter((task) => statusToken(task.status ?? at(task, ["payload_json", "status"])) === "cancelled")
    .map((task, index) => {
      const id = taskId(task as TaskRow) || stringValue(task.id);
      const goalId = stringValue(at(task, ["goal_id"])) || stringValue(at(task, ["payload_json", "goal_id"])) || selectedGoalId || stringValue(snapshot.goal_id);
      return {
        actionId: id && goalId ? `task:${goalId}:${id}` : "",
        key: id ? `cancelled-task:${id}` : `cancelled-task:${goalId}:${index}`,
        kind: "cancelled" as const,
        label: taskTitle(task) || (id ? `Cancelled task ${friendlyRef(id)}` : "Cancelled task"),
        status: "cancelled",
        detail: taskDetail(task, "cancelled") || "Cancelled task retained for operator queue history.",
        requestedInput: "",
        goalId,
        taskId: id,
        approvalId: "",
        thunkId: "",
        continuationId: "",
        risk: "",
        actionLabel: "Cancelled",
      };
    });
  const thunkItems = continuationRowsFromComposedSnapshot(snapshot, true)
    .filter((row) => row.status === "cancelled")
    .map((row) => ({
      ...actionItemFromContinuation(selectedGoalId || stringValue(snapshot.goal_id), row),
      key: `cancelled-thunk:${row.thunkId}`,
      kind: "cancelled" as const,
      status: "cancelled",
      actionLabel: "Cancelled",
      detail: [row.waitKind ? `Wait: ${row.waitKind}` : "", row.waitReference ? `Ref ${friendlyRef(row.waitReference)}` : "", "Cancelled continuation retained for operator queue history."].filter(Boolean).join(" · "),
    }));
  return [...taskItems, ...thunkItems];
}

export function mergeActionNeededItems(items: ActionNeededItem[]): ActionNeededItem[] {
  const seen = new Set<string>();
  const merged: ActionNeededItem[] = [];
  for (const item of items) {
    const key = item.key || `${item.kind}:${item.goalId}:${item.taskId}:${item.label}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push({ ...item, key });
  }
  return merged.sort((left, right) => actionNeededPriority(left) - actionNeededPriority(right) || left.label.localeCompare(right.label));
}

function actionNeededPriority(item: ActionNeededItem): number {
  const status = statusToken(item.status);
  if (item.kind === "cancelled" || status === "cancelled") return 4;
  if (item.kind === "approval" || status === "waiting-approval") return 0;
  if (status === "blocked" || status === "failed") return 1;
  if (item.kind === "thunk" || status === "waiting-input") return 2;
  return 3;
}

function taskNeedsOperatorAttention(task: JsonRecord): boolean {
  return ["blocked", "failed", "waiting-approval", "waiting-input"].includes(statusToken(task.status ?? at(task, ["payload_json", "status"])));
}

function taskTitle(task: JsonRecord): string {
  return stringValue(task.title) || stringValue(task.current_prompt) || stringValue(task.prompt) || stringValue(at(task, ["payload_json", "title"])) || stringValue(at(task, ["payload_json", "prompt"]));
}

function taskDetail(task: JsonRecord, status: string): string {
  const role = stringValue(task.role) || stringValue(at(task, ["payload_json", "role"]));
  const subgoal = stringValue(task.subgoal_id) || stringValue(at(task, ["payload_json", "subgoal_id"]));
  const reason = taskBlockerReason(task);
  return [statusLabel(status), role, subgoal ? `subgoal ${subgoal}` : "", reason].filter(Boolean).join(" · ");
}

function taskBlockerReason(task: JsonRecord): string {
  const payload = isRecord(task.payload_json) ? task.payload_json : {};
  const result = isRecord(task.result) ? task.result : isRecord(payload.result) ? payload.result : {};
  const candidates = [
    task.reason,
    task.blocker,
    task.blocked_reason,
    task.blocker_reason,
    task.error,
    task.last_error,
    task.summary,
    at(task, ["payload_json", "reason"]),
    at(task, ["payload_json", "blocker"]),
    at(task, ["payload_json", "blocked_reason"]),
    at(task, ["payload_json", "blocker_reason"]),
    at(task, ["payload_json", "error"]),
    at(task, ["payload_json", "last_error"]),
    at(task, ["payload_json", "summary"]),
    result.reason,
    result.error,
    result.summary,
    result.blocker,
    result.blocked_reason,
  ];
  for (const candidate of candidates) {
    const value = stringValue(candidate);
    if (value) {
      return value;
    }
  }
  return "";
}

export function continuationRowsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot, includeClosed = false): ContinuationRow[] {
  if (!snapshot) {
    return [];
  }
  const rowVisible = (row: ContinuationRow | null): row is ContinuationRow => {
    if (!row) {
      return false;
    }
    return includeClosed || isOpenContinuationRow(row);
  };
  const graph = workflowComputeGraph(snapshot);
  const nodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  const thunkRecords = delayedThunkRecordsByIdFromComposedSnapshot(snapshot);
  const graphRows = nodes
    .filter(isRecord)
    .filter((node) => normalizeStatus(node.kind) === "delayed-compute-thunk")
    .map((node) => continuationRowFromNode(node as ComputeGraphNode, thunkRecords.get(stringValue(node.thunk_id))))
    .filter(rowVisible);
  const graphThunkIds = new Set(graphRows.map((row) => row.thunkId));
  const recordRows = [...thunkRecords.values()]
    .filter((record) => !graphThunkIds.has(stringValue(record.id)))
    .map(continuationRowFromThunkRecord)
    .filter(rowVisible);
  return [...graphRows, ...recordRows];
}

function delayedThunkRecordsByIdFromComposedSnapshot(snapshot: ComposedGoalSnapshot): Map<string, JsonRecord> {
  const rows = rowsFrom(
    at(snapshot, ["workflow_status", "data", "delayed_compute_thunks"])
      ?? at(snapshot, ["workflow_status", "delayed_compute_thunks"])
      ?? at(snapshot, ["workflow_progress", "data", "delayed_compute_thunks"])
      ?? at(snapshot, ["workflow_progress", "delayed_compute_thunks"]),
  );
  return new Map(
    rows
      .map((row): [string, JsonRecord] => [stringValue(row.id), row])
      .filter(([id]) => Boolean(id)),
  );
}

function continuationRowFromNode(node: ComputeGraphNode, record?: JsonRecord): ContinuationRow | null {
  const thunkId = stringValue(node.thunk_id);
  if (!thunkId) {
    return null;
  }
  const waitRef = isRecord(record?.wait_ref) ? record.wait_ref : isRecord(node.wait_ref) ? node.wait_ref : {};
  const continuation = isRecord(record?.continuation) ? record.continuation : {};
  return {
    key: thunkId,
    thunkId,
    continuationId: stringValue(node.continuation_id) || stringValue(continuation.continuation_id),
    taskId: stringValue(node.task_id) || stringValue(record?.task_id),
    reason: stringValue(record?.reason) || stringValue(node.label) || "Waiting for input",
    requestedInput: stringValue(record?.requested_input) || stringValue(at(node, ["requested_input"])) || stringValue(at(node, ["payload_json", "requested_input"])) || stringValue(at(node, ["wait_ref", "requested_input"])),
    status: continuationStatus(record?.status ?? node.status),
    waitKind: stringValue(waitRef.kind),
    waitReference: stringValue(waitRef.reference),
  };
}

function continuationRowFromThunkRecord(record: JsonRecord): ContinuationRow | null {
  const thunkId = stringValue(record.id);
  if (!thunkId) {
    return null;
  }
  const waitRef = isRecord(record.wait_ref) ? record.wait_ref : {};
  const continuation = isRecord(record.continuation) ? record.continuation : {};
  return {
    key: thunkId,
    thunkId,
    continuationId: stringValue(continuation.continuation_id),
    taskId: stringValue(record.task_id),
    reason: stringValue(record.reason) || "Waiting for input",
    requestedInput: stringValue(record.requested_input),
    status: continuationStatus(record.status || "waiting-input"),
    waitKind: stringValue(waitRef.kind),
    waitReference: stringValue(waitRef.reference),
  };
}

function isOpenContinuationRow(row: ContinuationRow): boolean {
  return !["resumed", "cancelled", "expired", "done"].includes(row.status);
}

function continuationStatus(value: unknown): string {
  const status = normalizeStatus(value);
  return status === "pending" ? "waiting-input" : status;
}

function actionItemFromContinuation(goalId: string, row: ContinuationRow): ActionNeededItem {
  return {
    actionId: row.thunkId && goalId ? `thunk:${goalId}:${row.thunkId}` : "",
    key: `thunk:${row.thunkId}`,
    kind: "thunk",
    label: row.reason || "Human prompt",
    status: row.status || "waiting-input",
    detail: [row.waitKind ? `Wait: ${row.waitKind}` : "", row.waitReference ? `Ref ${friendlyRef(row.waitReference)}` : ""].filter(Boolean).join(" · ") || "Waiting continuation is paused.",
    requestedInput: row.requestedInput || row.reason || "Continue this work, or add the missing context.",
    goalId,
    taskId: row.taskId,
    approvalId: "",
    thunkId: row.thunkId,
    continuationId: row.continuationId,
    risk: "",
    actionLabel: "Continue",
  };
}

export function ContinuationQueue({ goalId, snapshot }: { goalId: string; snapshot?: ComposedGoalSnapshot }) {
  const queryClient = useQueryClient();
  const rows = continuationRowsFromComposedSnapshot(snapshot);
  const [responses, setResponses] = useState<Record<string, string>>({});
  const actionMutation = useMutation({
    mutationFn: ({ item, responseSummary, intent }: ActionMutationInput) => runOperatorAction(item, responseSummary, intent),
    onSuccess: (value) => {
      applyActionEnvelopeToCache(queryClient, value, goalId);
    },
  });

  if (!rows.length) {
    return <EmptyState title="Human prompts clear" detail="Tasks that need your input will appear here." />;
  }

  return (
    <div className="continuation-list" aria-label="Human prompts">
      <div className="section-heading">
        <h3>Human prompts</h3>
        <span className="muted-small">{rows.length} waiting</span>
      </div>
      {rows.map((row) => {
        const item = actionItemFromContinuation(goalId, row);
        const responseSummary = responses[item.key] ?? "";
        return (
          <div key={row.key} className="continuation-card">
            <div className="continuation-copy">
              <strong>{row.reason}</strong>
              <span title={row.thunkId}>{statusLabel(row.status)} · wait {friendlyRef(row.thunkId)}</span>
              {row.taskId && <small title={row.taskId}>task {friendlyRef(row.taskId)}</small>}
              {row.continuationId && <small title={row.continuationId}>continuation {friendlyRef(row.continuationId)}</small>}
              {row.waitReference && <small title={row.waitReference}>{row.waitKind || "wait_ref"} · {friendlyRef(row.waitReference)}</small>}
            </div>
            <HumanPromptCard
              item={item}
              value={responseSummary}
              onChange={(value) => setResponses((current) => ({ ...current, [item.key]: value }))}
              onAction={(nextItem, nextSummary, intent) => actionMutation.mutate({ item: nextItem, responseSummary: nextSummary, intent })}
              pending={actionMutation.isPending}
              compact
            />
          </div>
        );
      })}
    </div>
  );
}

export function OperatorActionList({ items, compact = false }: { items: ActionNeededItem[]; compact?: boolean }) {
  const queryClient = useQueryClient();
  const [queueFilter, setQueueFilter] = useState<QueueFilter>("all");
  const [responses, setResponses] = useState<Record<string, string>>({});
  const [lastAction, setLastAction] = useState<{ item: ActionNeededItem; response: unknown } | null>(null);
  const actionMutation = useMutation({
    mutationFn: ({ item, responseSummary, intent }: ActionMutationInput) => runOperatorAction(item, responseSummary, intent),
    onSuccess: (result, variables) => {
      setResponses((current) => {
        const next = { ...current };
        delete next[variables.item.key];
        return next;
      });
      setLastAction({ item: variables.item, response: result });
      applyActionEnvelopeToCache(queryClient, result, variables.item.goalId);
    },
  });
  const showQueueControls = !compact;
  const visibleItems = showQueueControls ? items.filter((item) => queueItemMatchesFilter(item, queueFilter)) : items;
  const groupedItems = showQueueControls ? queueGroupsForItems(visibleItems) : [];
  const renderItem = (item: ActionNeededItem) => {
    const responseSummary = responses[item.key] ?? "";
    if (queueGroupForItem(item) === "cancelled") {
      return <QueueHistoryCard key={item.key} item={item} />;
    }
    return (
      <HumanPromptCard
        key={item.key}
        item={item}
        value={responseSummary}
        onChange={(value) => setResponses((current) => ({ ...current, [item.key]: value }))}
        onAction={(nextItem, nextSummary, intent) => actionMutation.mutate({ item: nextItem, responseSummary: nextSummary, intent })}
        pending={actionMutation.isPending}
        compact={compact}
      />
    );
  };
  return (
    <div className="approval-list">
      {showQueueControls && <QueueFilterBar items={items} active={queueFilter} onChange={setQueueFilter} />}
      {lastAction && <ActionResultCard item={lastAction.item} response={lastAction.response} onClear={() => setLastAction(null)} />}
      {visibleItems.length ? (
        showQueueControls ? groupedItems.map((group) => (
          <section key={group.group} className="queue-group" aria-label={`${queueGroupLabels[group.group]} queue`}>
            <div className="queue-group-heading">
              <strong>{queueGroupLabels[group.group]}</strong>
              <span className="filter-count">{group.items.length} item{group.items.length === 1 ? "" : "s"}</span>
            </div>
            {group.items.map(renderItem)}
          </section>
        )) : visibleItems.map(renderItem)
      ) : (
        <EmptyState
          title={queueFilter === "all" ? "Action queue clear" : `${queueGroupLabels[queueFilter]} queue clear`}
          detail={queueFilter === "all" ? "Approvals, recovery work, continuations, and stopped history appear here." : `No ${queueGroupLabels[queueFilter].toLowerCase()} rows match this filter.`}
        />
      )}
      {actionMutation.error && <span className="error-text">{(actionMutation.error as Error).message}</span>}
    </div>
  );
}

export function QueueFilterBar({ items, active, onChange }: { items: ActionNeededItem[]; active: QueueFilter; onChange: (value: QueueFilter) => void }) {
  return (
    <div className="queue-toolbar" aria-label="Action queue filters">
      <div className="graph-filter queue-filter" role="group" aria-label="Queue filter">
        {queueFilterOptions.map((option) => {
          const count = option.key === "all" ? items.length : items.filter((item) => queueItemMatchesFilter(item, option.key)).length;
          return (
            <button
              key={option.key}
              type="button"
              className={clsx("graph-filter-button", active === option.key && "active")}
              aria-pressed={active === option.key}
              title={option.detail}
              data-testid={`queue-filter-${option.key}`}
              onClick={() => onChange(option.key)}
            >
              {option.label}
              <span>{count}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function ActionResultCard({ item, response, onClear }: { item: ActionNeededItem; response: unknown; onClear: () => void }) {
  const summary = actionEnvelopeSummary(response);
  if (!summary) {
    return null;
  }
  return (
    <div className="action-result-card">
      <div>
        <strong>{summary.label}</strong>
        <span>{item.label}</span>
        <small>{summary.detail}</small>
        <small>State will refresh automatically.</small>
      </div>
      <div className="action-result-controls">
        <AdvancedInspect summaryLabel="Details" title="Action result and active state" payload={response} buttonLabel="Inspect JSON" />
        <button type="button" className="secondary-button" title="Dismiss this local notice only." onClick={onClear}>
          Clear notice
        </button>
      </div>
    </div>
  );
}

function actionEnvelopeSummary(response: unknown): { label: string; detail: string; status: string } | null {
  if (!isRecord(response)) {
    return null;
  }
  const handler = stringValue(at(response, ["action", "handler"])) || "action";
  const ok = response.ok !== false;
  const observability = isRecord(response.observability) ? response.observability : {};
  const attempts = numberValue(observability.active_state_attempts) ?? 0;
  const elapsed = numberValue(observability.active_state_after_ms) ?? 0;
  const stateStatus = stringValue(response.active_state_status) || stringValue(observability.active_state_status) || "fresh";
  const unavailableReads = (Array.isArray(response.active_state_unavailable_reads)
    ? response.active_state_unavailable_reads
    : Array.isArray(observability.active_state_unavailable_reads)
      ? observability.active_state_unavailable_reads
      : [])
    .map((value) => String(value))
    .filter(Boolean);
  const activeStateDetail = stateStatus === "fresh"
    ? attempts
      ? `Active state read after ${attempts} attempt${attempts === 1 ? "" : "s"} in ${elapsed}ms.`
      : "Active state read pending."
    : `Active state ${stateStatus}; ${unavailableReads.length ? `unavailable reads: ${unavailableReads.join(", ")}. ` : ""}Projection will refresh from the stream or next poll.`;
  return {
    label: ok ? `${humanActionName(handler)} accepted` : `${humanActionName(handler)} failed`,
    detail: activeStateDetail,
    status: ok ? "done" : "failed",
  };
}

function humanActionName(handler: string): string {
  return handler
    .replace(/_/g, " ")
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

function QueueHistoryCard({ item }: { item: ActionNeededItem }) {
  return (
    <article className="queue-history-card">
      <div>
        <span className="goal-context-kicker">Stopped history</span>
        <strong>{item.label}</strong>
        <span>{statusLabel(item.status)} · {item.goalId ? friendlyRef(item.goalId) : "goal unknown"}</span>
        <small>{item.detail || "This stopped row is kept for queue history."}</small>
        <small>Read-only history.</small>
      </div>
      <span className="status-pill status-cancelled">Read-only</span>
    </article>
  );
}

export function queueGroupsForItems(items: ActionNeededItem[]): Array<{ group: QueueGroup; items: ActionNeededItem[] }> {
  return queueGroupOrder
    .map((group) => ({ group, items: items.filter((item) => queueGroupForItem(item) === group) }))
    .filter((group) => group.items.length > 0);
}

function queueItemMatchesFilter(item: ActionNeededItem, filter: QueueFilter): boolean {
  return filter === "all" || queueGroupForItem(item) === filter;
}

export function queueGroupForItem(item: ActionNeededItem): QueueGroup {
  const status = statusToken(item.status);
  if (item.kind === "cancelled" || status === "cancelled") return "cancelled";
  if (item.kind === "approval" || status === "waiting-approval") return "approvals";
  if (isResumableThunkItem(item)) return "thunks";
  return "blocked";
}

export function HumanPromptCard(props: {
  item: ActionNeededItem;
  value: string;
  onChange: (value: string) => void;
  onAction: (item: ActionNeededItem, responseSummary: string, intent: ActionIntent) => void;
  pending: boolean;
  compact?: boolean;
}) {
  const prompt = humanPromptForItem(props.item);
  const affordance = actionAffordanceForItem(props.item);
  const context = props.value.trim();
  const missingTarget = !props.item.goalId || (affordance === "approve" && !props.item.approvalId) || (affordance === "resume" && !props.item.thunkId);
  const primaryDisabled = props.pending || missingTarget;
  const contextDisabled = primaryDisabled || !context;
  const rejectDisabled = primaryDisabled || !prompt.rejectLabel;
  const createPromptDisabled = props.pending || !props.item.goalId || !prompt.createPromptLabel;
  const cancelDisabled = props.pending || !props.item.goalId || !prompt.cancelLabel;
  return (
    <article className={clsx("approval-card", "human-prompt-card", props.compact && "compact")}>
      <div className="human-prompt-copy">
        <span className="goal-context-kicker">{prompt.title}</span>
        <strong>{prompt.question}</strong>
        <span>{statusLabel(props.item.status)} · {props.item.goalId ? friendlyRef(props.item.goalId) : "no goal selected"}</span>
        {prompt.detail && <small>{prompt.detail}</small>}
      </div>
      {prompt.showInput && (
        <label className="inline-action-input human-prompt-input">
          {prompt.inputLabel}
          <textarea
            value={props.value}
            onChange={(event) => props.onChange(event.target.value)}
            placeholder={prompt.placeholder}
            rows={props.compact ? 2 : 3}
          />
        </label>
      )}
      <div className="human-prompt-actions">
        <button
          type="button"
          className={affordance === "approve" || affordance === "resume" ? "primary-button" : "secondary-button"}
          disabled={primaryDisabled}
          onClick={() => props.onAction(props.item, prompt.defaultResponseSummary, "primary")}
        >
          {prompt.primaryLabel}
        </button>
        {prompt.showInput && (
          <button type="button" className="secondary-button" disabled={contextDisabled} onClick={() => props.onAction(props.item, context, "context")}>
            {prompt.contextLabel}
          </button>
        )}
        {prompt.rejectLabel && (
          <button type="button" className="secondary-button" disabled={rejectDisabled} onClick={() => props.onAction(props.item, context, "reject")}>
            {prompt.rejectLabel}
          </button>
        )}
        {prompt.createPromptLabel && (
          <button type="button" className="secondary-button" disabled={createPromptDisabled} onClick={() => props.onAction(props.item, context, "create-human-prompt")}>
            {prompt.createPromptLabel}
          </button>
        )}
        {prompt.cancelLabel && (
          <button
            type="button"
            className="danger-button"
            disabled={cancelDisabled}
            title="Cancel this goal."
            onClick={() => props.onAction(props.item, context, "cancel-goal")}
          >
            <XCircle size={15} />
            {prompt.cancelLabel}
          </button>
        )}
      </div>
      {prompt.cancelLabel && <small className="danger-note">Cancel stops the goal. Clear only dismisses local notices or selection.</small>}
    </article>
  );
}

function humanPromptForItem(item: ActionNeededItem): HumanPromptSpec {
  const status = statusToken(item.status);
  const question = item.requestedInput || item.label || "What should the coordinator do next?";
  if ((item.kind === "approval" || status === "waiting-approval") && item.approvalId) {
    const approvalQuestion = item.requestedInput || "Approve this gate and continue?";
    return {
      title: "Approval prompt",
      question: approvalQuestion,
      detail: item.detail || blockerReason(item),
      primaryLabel: "Approve and continue",
      contextLabel: "Approve with note",
      rejectLabel: "Reject",
      inputLabel: "Approval note",
      placeholder: "Add context for the approval record.",
      showInput: true,
      defaultResponseSummary: "Approved by operator.",
    };
  }
  if (isResumableThunkItem(item)) {
    return {
      title: "Human prompt",
      question,
      detail: item.detail || blockerReason(item),
      primaryLabel: "Continue",
      contextLabel: "Add context",
      inputLabel: "Context",
      placeholder: "Add context for the agent, or leave blank and press Continue.",
      showInput: true,
      defaultResponseSummary: defaultContinuationSummary,
    };
  }
  const failed = status === "failed";
  const waitingWithoutThunk = status === "waiting-input";
  return {
    title: failed ? "Failed work" : waitingWithoutThunk ? "Waiting task" : "Recovery",
    question: item.label || (failed ? "Retry failed work?" : waitingWithoutThunk ? "Create an operator prompt for this waiting task?" : "Recover blocked work?"),
    detail: item.detail || blockerReason(item),
    primaryLabel: waitingWithoutThunk ? "Create prompt" : "Retry work",
    contextLabel: "Replan",
    inputLabel: "Recovery context",
    placeholder: waitingWithoutThunk ? "Describe the operator input this task needs." : "Add what changed, what to avoid, or what the next attempt should consider.",
    showInput: true,
    defaultResponseSummary: "",
    createPromptLabel: waitingWithoutThunk ? undefined : "Ask for input",
    cancelLabel: "Cancel goal",
  };
}

function actionAffordanceForItem(item: ActionNeededItem): "approve" | "resume" | "replan" {
  const status = statusToken(item.status);
  if ((item.kind === "approval" || status === "waiting-approval") && item.approvalId) return "approve";
  if (isResumableThunkItem(item)) return "resume";
  return "replan";
}

function isResumableThunkItem(item: ActionNeededItem): boolean {
  return item.kind === "thunk" && Boolean(item.thunkId);
}

export function runOperatorAction(item: ActionNeededItem, responseSummary = "", intent: ActionIntent = "primary"): Promise<unknown> {
  if (!item.goalId) {
    throw new Error("This action is missing a goal id.");
  }
  const actionId = item.actionId || actionIdForItem(item);
  const baseResolutionPayload = {
    goal_id: item.goalId,
    task_id: item.taskId || undefined,
    approval_id: item.approvalId || undefined,
    thunk_id: item.thunkId || undefined,
    operator: "operator",
    response_summary: responseSummary.trim(),
    answer: responseSummary.trim() || undefined,
    artifact_refs: [],
  };
  if (intent === "cancel-goal") {
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: "cancel_goal",
      response_summary: blockedTaskCancelReason(item, responseSummary.trim()),
    });
  }
  const affordance = actionAffordanceForItem(item);
  if (affordance === "approve") {
    if (!item.approvalId) throw new Error("This approval is missing an approval id.");
    if (intent === "reject") {
      return resolveOperatorAction(actionId, {
        ...baseResolutionPayload,
        resolution: "reject",
        response_summary: responseSummary.trim() || "Rejected from Task Graph Manager",
      });
    }
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: "approve",
      response_summary: responseSummary.trim() || "Approved from Task Graph Manager",
    });
  }
  if (affordance === "resume") {
    if (!item.thunkId) throw new Error("This continuation is missing a thunk id.");
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: intent === "context" ? "add_context" : "continue",
      response_summary: responseSummary.trim() || defaultContinuationSummary,
    });
  }
  if (intent === "create-human-prompt" || (statusToken(item.status) === "waiting-input" && !isResumableThunkItem(item) && intent === "primary")) {
    return createThunk(item.goalId, blockedTaskHumanPromptPayload(item, responseSummary.trim()));
  }
  const status = statusToken(item.status);
  if (status === "blocked" || status === "failed") {
    if (intent === "context" || responseSummary.trim()) {
      return resolveOperatorAction(actionId, {
        ...baseResolutionPayload,
        resolution: "replan",
        response_summary: responseSummary.trim() || "Operator requested replan.",
      });
    }
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: "retry",
      response_summary: "Operator requested retry.",
    });
  }
  return resolveOperatorAction(actionId, {
    ...baseResolutionPayload,
    resolution: "replan",
    response_summary: responseSummary.trim() || "Operator requested replan.",
  });
}

function blockedTaskHumanPromptPayload(item: ActionNeededItem, operatorContext = ""): JsonRecord {
  const subject = item.taskId ? `task ${friendlyRef(item.taskId)}` : "blocked work";
  const requestedInput = operatorContext
    || item.requestedInput
    || item.detail
    || `What operator input is required to unblock ${subject}?`;
  return thunkPayload({
    goalId: item.goalId,
    taskId: item.taskId,
    kind: "human_input",
    reason: `Operator requested a human prompt for ${subject}: ${item.detail || item.label}`,
    requestedInput,
    timeoutSeconds: 0,
  });
}

function blockedTaskCancelReason(item: ActionNeededItem, operatorContext = ""): string {
  const subject = item.taskId ? `task ${friendlyRef(item.taskId)}` : "the selected goal";
  return [
    `Operator cancelled the goal from the action queue while reviewing ${subject}.`,
    item.detail ? `Blocked detail: ${item.detail}.` : "",
    operatorContext ? `Operator context: ${operatorContext}.` : "",
  ].filter(Boolean).join(" ");
}

function thunkPayload(input: { goalId: string; taskId: string; kind: string; reason: string; requestedInput: string; timeoutSeconds: number }): JsonRecord {
  const taskRef = input.taskId || "goal";
  return {
    goal_id: input.goalId,
    task_id: input.taskId || null,
    kind: input.kind,
    reason: input.reason,
    requested_input: input.requestedInput || null,
    wait_ref: {
      kind: input.kind === "timer" ? "durable_timer" : input.kind === "model_availability" ? "model_route" : input.kind === "resource_availability" ? "runner_capacity" : "human_thread",
      reference: `goal://${input.goalId}/${taskRef}`,
    },
    continuation: {
      continuation_id: `${input.goalId}/${taskRef}/${Date.now()}`,
      boundary: "task_dispatch",
      state_ref: `goal/${input.goalId}/task/${taskRef}`,
      resume_actions: ["apply_feedback", "mark_runnable"],
    },
    timeout_seconds: input.timeoutSeconds > 0 ? input.timeoutSeconds : null,
  };
}

function actionIdForItem(item: ActionNeededItem): string {
  if (item.kind === "approval" && item.approvalId) return `approval:${item.goalId}:${item.approvalId}`;
  if (item.kind === "thunk" && item.thunkId) return `thunk:${item.goalId}:${item.thunkId}`;
  if (item.taskId) return `task:${item.goalId}:${item.taskId}`;
  return `goal:${item.goalId}:action`;
}
