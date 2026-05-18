import * as Popover from "@radix-ui/react-popover";
import { MarkerType, type Edge, type Node } from "@xyflow/react";
import clsx from "clsx";
import { Command } from "cmdk";
import { ChevronDown, Network, RefreshCw, XCircle } from "lucide-react";
import { useState } from "react";

import { at, isRecord, rowsFrom } from "../api";
import { EmptyState, InspectButton, SimpleTable } from "../components/operator-primitives";
import type { ColorRef, ComputeGraphNode, ComposedGoalSnapshot, GoalRow, JsonRecord, OperatorGoalDetail, TaskRow } from "../types";
import {
  friendlyRef,
  goalNextAction,
  goalOperatorState,
  normalizeStatus,
  numberValue,
  operatorStateDefinitions,
  operatorStateForStatus,
  safeTestId,
  stateTone,
  statusColorVar,
  statusLabel,
  statusLegend,
  statusPriority,
  statusToken,
  statusTone,
  stringValue,
} from "./workbench-format";

export type GraphFilter = "all" | "attention" | "active" | "completed";

export type SubmittedGoalDraft = {
  draft: JsonRecord;
  submittedAt: number;
  projected: boolean;
};

export type GoalSummary = {
  id: string;
  title: string;
  objective: string;
  status: string;
  progress: number;
  openTasks: number;
  blockedTasks: number;
  failedTasks: number;
  updatedAt: string;
};

export const graphFilterOptions: Array<{ key: GraphFilter; label: string; detail: string }> = [
  { key: "all", label: "All", detail: "all projected tasks" },
  { key: "attention", label: "Action needed", detail: "failed, blocked, approvals, continuations" },
  { key: "active", label: "Active", detail: "running, runnable, validation" },
  { key: "completed", label: "Completed", detail: "done or cancelled" },
];

export function GoalContextBar(props: {
  goals: GoalSummary[];
  selectedGoal: GoalSummary | null;
  selectedGoalId: string;
  open: boolean;
  loading: boolean;
  cancelBusy: boolean;
  cancelError: Error | null;
  onOpenChange: (open: boolean) => void;
  onSelectGoal: (goalId: string) => void;
  onCancelGoal: () => void;
  onRefreshGoals: () => void;
  onOpenGraph: () => void;
}) {
  const done = Math.round((props.selectedGoal?.progress ?? 0) * 100);
  const selectedState = props.selectedGoal ? operatorStateForStatus(props.selectedGoal.status) : null;
  const selectedStatus = statusToken(props.selectedGoal?.status);
  const canCancelSelectedGoal = Boolean(props.selectedGoalId && selectedStatus !== "cancelled" && selectedStatus !== "done");
  return (
    <Popover.Root open={props.open} onOpenChange={props.onOpenChange}>
      <div className="goal-context-bar">
        <Popover.Trigger asChild>
          <button
            type="button"
            className="goal-context-trigger"
            aria-expanded={props.open}
            aria-label={props.selectedGoal ? `Current goal: ${props.selectedGoal.title}, ${selectedState?.label ?? "state pending"}` : "Select current goal"}
            data-testid="goal-context-trigger"
          >
            <div>
              <span className="goal-context-kicker">Current goal</span>
              <strong>{props.selectedGoal?.title || "Select a goal"}</strong>
              {props.selectedGoal ? (
                <small>{done}% · {props.selectedGoal.openTasks} open · {props.selectedGoal.blockedTasks} blocked · {selectedState?.label}</small>
              ) : (
                <small>Chat, graph, actions, memory, and the action queue use this context</small>
              )}
            </div>
            <span className={clsx("operator-state-pill", selectedState ? stateTone(selectedState.key) : "muted")}>
              {selectedState?.label ?? (props.loading ? "Loading" : "Select")}
            </span>
            <ChevronDown size={16} />
          </button>
        </Popover.Trigger>
        {canCancelSelectedGoal && (
          <button
            type="button"
            className="danger-button goal-context-cancel"
            disabled={props.cancelBusy}
            title="Stop this durable goal through the coordinator."
            onClick={props.onCancelGoal}
          >
            <XCircle size={15} />
            Cancel goal
          </button>
        )}
        <Popover.Portal>
          <Popover.Content className="goal-picker" align="end" sideOffset={8}>
            <Command shouldFilter className="goal-command">
              <div className="goal-picker-actions">
                <label>
                  Search goals
                  <Command.Input placeholder="Title, status, or id" />
                </label>
                <button type="button" className="secondary-button" onClick={props.onRefreshGoals}>
                  <RefreshCw size={15} />
                  Refresh
                </button>
              </div>
              <Command.List className="goal-picker-list">
                <Command.Empty>
                  <EmptyState title="Goal match pending" detail="Refresh goals or submit one from chat." />
                </Command.Empty>
                <Command.Group>
                  {props.goals.map((goal) => (
                    <Command.Item
                      key={goal.id}
                      value={`${goal.title} ${goal.objective} ${goal.status} ${goal.id}`}
                      className={clsx("goal-picker-item", props.selectedGoalId === goal.id && "active")}
                      onSelect={() => {
                        props.onSelectGoal(goal.id);
                        props.onOpenChange(false);
                      }}
                    >
                      <span>
                        <strong>{goal.title || friendlyRef(goal.id) || "Untitled goal"}</strong>
                        <small>{goal.objective || friendlyRef(goal.id) || "Objective pending"}</small>
                      </span>
                      <span className={clsx("status-pill", statusTone(goal.status))}>{statusLabel(goal.status)}</span>
                    </Command.Item>
                  ))}
                </Command.Group>
              </Command.List>
              <div className="goal-picker-footer">
                <button
                  type="button"
                  className="secondary-button"
                  disabled={!props.selectedGoalId}
                  title="Clear the local goal selection. This does not cancel the goal."
                  onClick={() => {
                    props.onSelectGoal("");
                    props.onOpenChange(false);
                  }}
                >
                  <XCircle size={15} />
                  Clear selection
                </button>
                <button
                  type="button"
                  className="danger-button"
                  disabled={!canCancelSelectedGoal || props.cancelBusy}
                  title="Stop this durable goal through the coordinator."
                  onClick={props.onCancelGoal}
                >
                  <XCircle size={15} />
                  Cancel goal
                </button>
                <button type="button" className="primary-button" disabled={!props.selectedGoalId} onClick={props.onOpenGraph}>
                  <Network size={15} />
                  Open graph
                </button>
              </div>
              {props.cancelError && <span className="error-text">{props.cancelError.message}</span>}
            </Command>
          </Popover.Content>
        </Popover.Portal>
      </div>
    </Popover.Root>
  );
}

export function GoalsView(props: { goals: GoalRow[]; selectedGoalId: string; onSelectGoal: (goalId: string) => void }) {
  return (
    <section className="panel">
      <div className="section-heading">
        <h2>Goal progress</h2>
        <span className="muted-small">Progress, blockers, and evidence</span>
      </div>
      <GoalList goals={props.goals} selectedGoalId={props.selectedGoalId} onSelect={props.onSelectGoal} />
    </section>
  );
}

export function GoalList({ goals, selectedGoalId, onSelect }: { goals: GoalRow[]; selectedGoalId: string; onSelect: (goalId: string) => void }) {
  if (!goals.length) {
    return <EmptyState title="Start a goal" detail="Draft or submit work from chat." />;
  }
  return (
    <div className="goal-list">
      {goals.map((goal) => {
        const goalId = String(goal.goal_id ?? goal.id ?? "");
        const done = Math.round(numericProgress(goal.percent_done) * 100);
        const status = stringValue(goal.status) || "unknown";
        const state = goalOperatorState(goal);
        const nextAction = goalNextAction(goal);
        const openTasks = numberValue(goal.open_tasks) ?? 0;
        const blockedTasks = numberValue(goal.blocked_tasks) ?? 0;
        const failedTasks = numberValue(goal.failed_tasks) ?? 0;
        const title = stringValue(goal.title) || friendlyRef(goalId) || "Untitled goal";
        return (
          <button
            key={goalId || goal.title}
            type="button"
            className={clsx("goal-card", selectedGoalId === goalId && "active")}
            aria-label={`Open ${title} work graph. ${state.label}. ${nextAction}`}
            data-testid={`goal-card-${safeTestId(goalId || title)}`}
            onClick={() => goalId && onSelect(goalId)}
          >
            <div className="goal-card-header">
              <span>
                <strong>{title}</strong>
                {goalId && <small>{friendlyRef(goalId)}</small>}
              </span>
              <span className={clsx("operator-state-pill", stateTone(state.key))}>{state.label}</span>
            </div>
            <p>{goal.objective || "Objective pending."}</p>
            <div className="progress-line" aria-label={`${done}% complete`}>
              <span style={{ width: `${Math.max(0, Math.min(100, done))}%` }} />
            </div>
            <div className="goal-metrics" aria-label="Goal state summary">
              <span><strong>{done}%</strong><small>complete</small></span>
              <span><strong>{openTasks}</strong><small>open</small></span>
              <span className={blockedTasks || failedTasks ? "attention" : ""}><strong>{blockedTasks + failedTasks}</strong><small>blocked or failed</small></span>
              <span><strong>{statusLabel(status)}</strong><small>backend status</small></span>
            </div>
            <div className="goal-next-row">
              <span>{nextAction}</span>
              <strong>Open graph</strong>
            </div>
          </button>
        );
      })}
    </div>
  );
}

export function SubgoalPlanPanel({ subgoals, source }: { subgoals: JsonRecord[]; source: string }) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const selected = subgoals[selectedIndex] ?? subgoals[0];
  return (
    <section className="subgoal-panel">
      <div className="section-heading">
        <div>
          <h3>Subgoals</h3>
          <span className="muted-small">{source}</span>
        </div>
      </div>
      <div className="subgoal-list">
        {subgoals.map((subgoal, index) => {
          const id = subgoalId(subgoal);
          const title = subgoalTitle(subgoal, index);
          const objective = subgoalObjective(subgoal);
          const status = stringValue(subgoal.status) || stringValue(subgoal.state) || "planned";
          return (
            <button
              key={id || `subgoal-${index}`}
              type="button"
              className={clsx("subgoal-card", selectedIndex === index && "active")}
              aria-pressed={selectedIndex === index}
              aria-label={`Open subgoal ${title}`}
              data-testid={`subgoal-card-${safeTestId(id || title)}`}
              onClick={() => setSelectedIndex(index)}
            >
              <span className={clsx("status-pill", statusTone(status))}>{statusLabel(status)}</span>
              <strong>{title}</strong>
              <p>{objective}</p>
              {id && <small>{friendlyRef(id)}</small>}
            </button>
          );
        })}
      </div>
      {selected && <SubgoalDetail subgoal={selected} index={selectedIndex} />}
    </section>
  );
}

function SubgoalDetail({ subgoal, index }: { subgoal: JsonRecord; index: number }) {
  const id = subgoalId(subgoal);
  const doneCriteria = textList(subgoal.done_criteria ?? subgoal.acceptance_criteria ?? subgoal.evidence);
  const constraints = textList(subgoal.constraints);
  const initialTasks = rowsFrom(subgoal.initial_tasks ?? subgoal.tasks);
  return (
    <article className="subgoal-detail" data-testid="subgoal-detail">
      <div>
        <span className="goal-context-kicker">Selected subgoal</span>
        <h4>{subgoalTitle(subgoal, index)}</h4>
        <p>{subgoalObjective(subgoal)}</p>
      </div>
      <div className="subgoal-detail-grid">
        <DetailList title="Evidence or done criteria" items={doneCriteria.length ? doneCriteria : ["Evidence requirements pending."]} />
        <DetailList title="Constraints" items={constraints.length ? constraints : ["No explicit constraints projected."]} />
        <DetailList
          title="Seed tasks"
          items={initialTasks.length ? initialTasks.map((task, taskIndex) => stringValue(task.title) || stringValue(task.role) || `Task ${taskIndex + 1}`) : ["No seed tasks projected."]}
        />
      </div>
      {id && <span className="status-pill muted">{friendlyRef(id)}</span>}
    </article>
  );
}

function DetailList({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="detail-list">
      <strong>{title}</strong>
      <ul>
        {items.slice(0, 4).map((item, index) => <li key={`${item}-${index}`}>{item}</li>)}
      </ul>
    </div>
  );
}

export function graphFromTasks(tasks: TaskRow[]): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = tasks.map((task, index) => {
    const id = taskId(task) || `task-${index}`;
    const color = colorRef(task.color ?? taskPayload(task).color);
    const status = String(task.status ?? "unknown");
    const state = operatorStateForStatus(status);
    return {
      id,
      className: clsx("task-node", statusTone(status)),
      position: { x: (Number(taskPayload(task).depth ?? index) % 4) * 280, y: Math.floor(index / 4) * 150 },
      data: {
        label: `${color?.label ? `${color.label}: ` : ""}${stringValue(task.title) || friendlyRef(id) || "Task"}\n${task.role ?? ""} · ${state.label} · ${statusLabel(status)}`,
      },
      style: {
        borderColor: safeHex(color?.hex),
        borderWidth: 2,
        borderRadius: 8,
        background: `linear-gradient(90deg, ${statusColorVar(status)} 0 5px, var(--node-bg) 5px)`,
        minWidth: 220,
        color: "var(--text)",
        whiteSpace: "pre-line",
        boxShadow: "var(--node-shadow)",
      },
    };
  });
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edges: Edge[] = [];
  for (const task of tasks) {
    const source = String(task.parent_task_id ?? taskPayload(task).parent_id ?? "");
    const target = taskId(task);
    if (source && target && nodeIds.has(source) && nodeIds.has(target)) {
      const edgeColor = statusColorVar(task.status);
      edges.push({
        id: `${source}-${target}`,
        source,
        target,
        animated: normalizeStatus(task.status) === "running",
        markerEnd: { type: MarkerType.ArrowClosed, color: edgeColor },
        style: { stroke: edgeColor, strokeWidth: 1.8 },
      });
    }
  }
  return { nodes, edges };
}

export function graphFromComputeGraph(graph: Record<string, unknown> | undefined, nodesToShow: ComputeGraphNode[]): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = nodesToShow.map((node, index) => {
    const id = stringValue(node.id) || `compute-${index}`;
    const status = String(node.status ?? "unknown");
    const kind = String(node.kind ?? "node");
    const state = operatorStateForStatus(status);
    return {
      id,
      className: clsx("task-node", statusTone(status)),
      position: { x: (index % 4) * 285, y: Math.floor(index / 4) * 150 },
      data: {
        label: `${stringValue(node.label) || friendlyRef(id) || "Compute node"}\n${kind} · ${state.label} · ${statusLabel(status)}`,
      },
      style: {
        borderColor: statusColorVar(status),
        borderWidth: 2,
        borderRadius: 8,
        background: `linear-gradient(90deg, ${statusColorVar(status)} 0 5px, var(--node-bg) 5px)`,
        minWidth: 230,
        color: "var(--text)",
        whiteSpace: "pre-line",
        boxShadow: "var(--node-shadow)",
      },
    };
  });
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edgeRows = Array.isArray(graph?.edges) ? graph.edges.filter(isRecord) : [];
  const edges: Edge[] = edgeRows.flatMap((edge) => {
    const source = stringValue(edge.from);
    const target = stringValue(edge.to);
    if (!source || !target || !nodeIds.has(source) || !nodeIds.has(target)) {
      return [];
    }
    return [{
      id: `${source}-${target}-${stringValue(edge.kind) || "edge"}`,
      source,
      target,
      label: stringValue(edge.kind),
      animated: stringValue(edge.kind).includes("resume") || stringValue(edge.kind).includes("unblock"),
      markerEnd: { type: MarkerType.ArrowClosed, color: "var(--edge)" },
      style: { stroke: "var(--edge)", strokeWidth: 1.8 },
    }];
  });
  return { nodes, edges };
}

export function computeGraphNodes(graph: Record<string, unknown> | undefined): ComputeGraphNode[] {
  const nodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  return nodes.filter(isRecord).map((node) => node as ComputeGraphNode);
}

export function computeNodeMatchesGraphFilter(node: ComputeGraphNode, filter: GraphFilter): boolean {
  if (filter === "all") {
    return true;
  }
  const status = statusToken(node.status);
  if (filter === "attention") {
    return status === "failed" || status === "blocked" || status === "waiting-approval" || status === "waiting-input" || normalizeStatus(node.kind) === "delayed-compute-thunk";
  }
  if (filter === "active") {
    return status === "running" || status === "runnable" || status === "needs-validation" || status === "pending";
  }
  return status === "done" || status === "cancelled";
}

export function ComputeGraphDetails({ snapshot }: { snapshot: ComposedGoalSnapshot }) {
  const graph = workflowComputeGraph(snapshot);
  const nodes = computeGraphNodes(graph);
  const openRows = nodes.filter((node) => computeNodeMatchesGraphFilter(node, "attention")).slice(0, 8);
  if (!nodes.length) {
    return <EmptyState title="Compute graph pending" detail="Waiting for workflow activity." />;
  }
  return (
    <div className="compute-details">
      <div className="section-heading">
        <h3>Compute graph</h3>
        <span className="muted-small">Graph rows</span>
        <InspectButton title="Compute graph projection" payload={graph} buttonLabel="Debug" />
      </div>
      <SimpleTable
        empty="Active nodes pending."
        headers={["Node", "Kind", "Status", "Wait / Continuation"]}
        rows={(openRows.length ? openRows : nodes.slice(0, 8)).map((node) => [
          stringValue(node.label) || friendlyRef(stringValue(node.id)) || "node",
          stringValue(node.kind) || "unknown",
          statusLabel(stringValue(node.status) || "unknown"),
          [stringValue(node.wait_ref?.kind), friendlyRef(stringValue(node.wait_ref?.reference)), friendlyRef(stringValue(node.continuation_id))].filter(Boolean).join(" · "),
        ])}
      />
    </div>
  );
}

export function TaskSummary({ snapshot, counts }: { snapshot: ComposedGoalSnapshot; counts: Map<string, number> }) {
  const entries = sortedStatusEntries(counts);
  const computeGraph = workflowComputeGraph(snapshot);
  const progress = workflowProgress(snapshot);
  const ranking = progress?.ranking as Record<string, unknown> | undefined;
  const latestRanking = ranking?.latest_decision as Record<string, unknown> | undefined;
  const graphNodeCount = Array.isArray(computeGraph?.nodes) ? computeGraph.nodes.length : 0;
  const graphEdgeCount = Array.isArray(computeGraph?.edges) ? computeGraph.edges.length : 0;
  const openThunkCount = Number(computeGraph?.open_thunks ?? 0);
  const voteCount = Number(ranking?.vote_count ?? 0);
  const rankingScore = Number(ranking?.score ?? 0);
  const upvotes = Number(ranking?.upvotes ?? 0);
  const downvotes = Number(ranking?.downvotes ?? 0);
  const openMechanismRounds = Number(progress?.open_mechanism_rounds ?? 0);
  const ratificationRounds = Number(progress?.ratification_required_mechanism_rounds ?? 0);
  return (
    <div className="summary-row">
      {entries.map(([status, count]) => (
        <span key={status} className={clsx("status-pill", statusTone(status))}>{status}: {count}</span>
      ))}
      {voteCount > 0 && (
        <span className={clsx("status-pill", rankingScore >= 0 ? "status-runnable" : "status-blocked")}>
          ranking: {rankingScore >= 0 ? "+" : ""}{rankingScore} · {upvotes} up · {downvotes} down
          {latestRanking?.outcome ? ` · ${String(latestRanking.outcome)}` : ""}
        </span>
      )}
      {(openMechanismRounds > 0 || ratificationRounds > 0) && (
        <span className={clsx("status-pill", ratificationRounds > 0 ? "status-waiting-approval" : "status-running")}>
          mechanisms: {openMechanismRounds} open · {ratificationRounds} ratify
        </span>
      )}
      {(graphNodeCount > 0 || graphEdgeCount > 0 || openThunkCount > 0) && (
        <span className={clsx("status-pill", openThunkCount > 0 ? "status-waiting-input" : "status-done")}>
          workflow graph: {graphNodeCount} nodes · {graphEdgeCount} edges · {openThunkCount} continuations
        </span>
      )}
      <InspectButton title="Operator goal detail" payload={snapshot} />
    </div>
  );
}

export function GraphStatusPanel({ counts, taskCount }: { counts: Map<string, number>; taskCount: number }) {
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = countForStatusToken(counts, "waiting-input");
  const running = countForStatusToken(counts, "running");
  const done = countForStatusToken(counts, "done");
  const attention = failed + blocked + approvals + continuations;
  return (
    <div className="graph-status-panel" aria-label="Work graph status">
      <div className={clsx("graph-attention", attention > 0 ? "needs-attention" : "stable")}>
        <strong>{attention > 0 ? `${attention} need attention` : "All tasks steady"}</strong>
        <span>{taskCount} tasks · {running} running · {done} done</span>
      </div>
      <OperatorStateStrip counts={counts} />
      <details className="legend-details">
        <summary>Status legend</summary>
        <div className="status-legend" aria-label="Graph legend">
          {statusLegend.map((item) => (
            <span key={item.token} className="legend-item">
              <span className={clsx("legend-swatch", `status-${item.token}`)} />
              <span>
                <strong>{item.label}</strong>
                <small>{item.detail}</small>
              </span>
            </span>
          ))}
        </div>
      </details>
    </div>
  );
}

export function OperatorStateStrip({ counts }: { counts: Map<string, number> }) {
  return (
    <div className="operator-state-row" aria-label="Operator state labels">
      {operatorStateDefinitions.map((definition) => {
        const count = definition.statuses.reduce((sum, status) => sum + countForStatusToken(counts, status), 0);
        return (
          <span key={definition.key} className={clsx("operator-state-chip", stateTone(definition.key), count > 0 && "active")}>
            <strong>{definition.label}</strong>
            <small>{count} · {definition.detail}</small>
          </span>
        );
      })}
    </div>
  );
}

export function mergeSubmittedGoalRows(projectedRows: GoalRow[], submittedDrafts: Record<string, SubmittedGoalDraft | JsonRecord>): GoalRow[] {
  const projectedIds = new Set(projectedRows.map((goal) => String(goal.goal_id ?? goal.id ?? "")).filter(Boolean));
  const pendingRows = Object.entries(submittedDrafts)
    .filter(([goalId, pending]) => {
      const submitted = submittedGoalDraftState(pending);
      return !submitted.projected || !projectedIds.has(goalId);
    })
    .map(([goalId, pending]) => pendingGoalRow(goalId, submittedGoalDraftState(pending).draft));
  return [...pendingRows, ...projectedRows];
}

export function goalRowsWithSelected(rows: GoalRow[], selectedGoal: GoalSummary | null): GoalSummary[] {
  const summaries = rows.map(goalSummaryFromRow).filter((goal): goal is GoalSummary => Boolean(goal));
  if (selectedGoal && !summaries.some((goal) => goal.id === selectedGoal.id)) {
    return [selectedGoal, ...summaries];
  }
  return summaries;
}

export function selectedGoalSummary(goalId: string, rows: GoalRow[], snapshot?: ComposedGoalSnapshot, submittedDraft?: JsonRecord | null): GoalSummary | null {
  if (!goalId) {
    return null;
  }
  const fromRows = rows.map(goalSummaryFromRow).find((goal): goal is GoalSummary => Boolean(goal && goal.id === goalId));
  if (fromRows) {
    return fromRows;
  }
  const payload = at(snapshot, ["goal_store_goal", "data", "goal", "payload_json"]);
  const source = isRecord(payload) ? payload : submittedDraft;
  return {
    id: goalId,
    title: stringValue(source?.title) || friendlyRef(goalId) || "Selected goal",
    objective: stringValue(source?.objective),
    status: stringValue(at(snapshot, ["goal_store_goal", "data", "goal", "status"])) || (submittedDraft ? "submitted" : "selected"),
    progress: numericProgress(at(snapshot, ["goal_store_goal", "data", "goal", "percent_done"])),
    openTasks: numberValue(at(snapshot, ["goal_store_goal", "data", "goal", "open_tasks"])) ?? rowsFrom(source?.initial_tasks).length,
    blockedTasks: numberValue(at(snapshot, ["goal_store_goal", "data", "goal", "blocked_tasks"])) ?? 0,
    failedTasks: numberValue(at(snapshot, ["goal_store_goal", "data", "goal", "failed_tasks"])) ?? 0,
    updatedAt: stringValue(at(snapshot, ["goal_store_goal", "data", "goal", "updated_at"])),
  };
}

export function composedSnapshotFromOperatorGoalDetail(detail?: OperatorGoalDetail | null): ComposedGoalSnapshot | undefined {
  if (!detail) {
    return undefined;
  }
  if (isRecord(detail.snapshot)) {
    return detail.snapshot as ComposedGoalSnapshot;
  }
  const summary = isRecord(detail.summary) ? detail.summary : {};
  const goalId = stringValue(summary.goal_id) || stringValue(summary.id);
  const tasks = rowsFrom(detail.tasks);
  return {
    goal_id: goalId,
    goal_store_goal: {
      data: {
        goal: {
          goal_id: goalId,
          status: summary.status,
          percent_done: summary.percent_done,
          open_tasks: summary.open_tasks,
          blocked_tasks: summary.blocked_tasks,
          failed_tasks: summary.failed_tasks,
          updated_at: summary.updated_at,
          payload_json: {
            title: summary.title,
            objective: summary.objective,
          },
        },
      },
    },
    workflow_progress: { data: detail.progress ?? {} },
    workflow_compute_graph: { data: detail.graph ?? {} },
    tasks: { data: { tasks } },
    approvals: { data: { approvals: [] } },
    checkpoints: { data: { checkpoints: [] } },
    agent_activity: tasks as TaskRow[],
  };
}

export function composedSnapshotHasProjectedTasks(snapshot?: ComposedGoalSnapshot): boolean {
  const projectedTasks = taskRowsFromComposedSnapshot(snapshot);
  const taskRows = rowsFrom(at(snapshot, ["tasks", "data"]) ?? snapshot?.tasks);
  const computeNodes = rowsFrom(at(snapshot, ["workflow_compute_graph", "data", "nodes"]) ?? at(snapshot, ["workflow_compute_graph", "nodes"]));
  return Boolean(projectedTasks.length || taskRows.length || computeNodes.length);
}

export function taskRowsFromGoalDraft(goalId: string, draft?: JsonRecord | null): TaskRow[] {
  if (!draft) {
    return [];
  }
  const initialTasks = rowsFrom(draft.initial_tasks);
  if (initialTasks.length) {
    return initialTasks.map((task, index) => ({
      goal_id: goalId,
      task_id: stringValue(task.id) || stringValue(task.task_id) || `draft-task-${index + 1}`,
      parent_task_id: index === 0 ? null : "draft-root",
      subgoal_id: stringValue(task.subgoal_id) || null,
      title: stringValue(task.title) || stringValue(task.role) || `Draft task ${index + 1}`,
      role: stringValue(task.role) || "planner",
      purpose: "work",
      status: "submitted",
      current_prompt: stringValue(task.prompt) || stringValue(task.reason) || null,
      prompt: stringValue(task.prompt) || stringValue(task.reason) || null,
      raw_task: task,
    }));
  }
  return goalSubgoalsFromDraft(draft).map((subgoal, index) => ({
    goal_id: goalId,
    task_id: stringValue(subgoal.id) || stringValue(subgoal.subgoal_id) || `draft-subgoal-${index + 1}`,
    parent_task_id: "draft-root",
    subgoal_id: stringValue(subgoal.id) || stringValue(subgoal.subgoal_id) || null,
    title: stringValue(subgoal.title) || `Draft subgoal ${index + 1}`,
    role: "planner",
    purpose: "work",
    status: "submitted",
    current_prompt: stringValue(subgoal.objective) || stringValue(subgoal.summary) || null,
    prompt: stringValue(subgoal.objective) || stringValue(subgoal.summary) || null,
    raw_task: subgoal,
  }));
}

export function goalSubgoalsFromComposedSnapshotOrDraft(snapshot?: ComposedGoalSnapshot, draft?: JsonRecord | null): JsonRecord[] {
  const projected = rowsFrom(at(snapshot, ["goal_store_goal", "data", "goal", "payload_json", "plan", "subgoals"]));
  return projected.length ? projected : goalSubgoalsFromDraft(draft);
}

export function taskStatusCounts(tasks: TaskRow[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const task of tasks) {
    const status = String(task.status ?? "unknown");
    counts.set(status, (counts.get(status) ?? 0) + 1);
  }
  return counts;
}

export function taskMatchesGraphFilter(task: TaskRow, filter: GraphFilter): boolean {
  if (filter === "all") {
    return true;
  }
  const status = statusToken(task.status);
  if (filter === "attention") {
    return status === "failed" || status === "blocked" || status === "waiting-approval" || status === "waiting-input";
  }
  if (filter === "active") {
    return status === "running" || status === "runnable" || status === "needs-validation";
  }
  return status === "done" || status === "cancelled";
}

export function workflowComputeGraph(snapshot: ComposedGoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_compute_graph as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

export function workflowProgress(snapshot: ComposedGoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_progress as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

export function taskRowsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot | JsonRecord): TaskRow[] {
  if (!snapshot) {
    return [];
  }
  const agentRows = Array.isArray(snapshot.agent_activity) ? snapshot.agent_activity.filter(isRecord) as TaskRow[] : [];
  if (agentRows.length) {
    return agentRows;
  }
  const taskRows = rowsFrom(at(snapshot, ["tasks", "data", "tasks"]) ?? at(snapshot, ["tasks", "tasks"]) ?? at(snapshot, ["tasks", "data"]) ?? snapshot.tasks);
  return taskRows as TaskRow[];
}

export function countForStatusToken(counts: Map<string, number>, token: string): number {
  let total = 0;
  for (const [status, count] of counts) {
    if (statusToken(status) === token) {
      total += count;
    }
  }
  return total;
}

export function numericProgress(value: unknown): number {
  const numeric = numberValue(value) ?? 0;
  return numeric > 1 ? numeric / 100 : numeric;
}

export function taskId(task: TaskRow): string {
  return stringValue(task.task_id) || stringValue(task.id) || stringValue(taskPayload(task).id);
}

function submittedGoalDraftState(value: SubmittedGoalDraft | JsonRecord): SubmittedGoalDraft {
  if (isRecord(value.draft)) {
    return value as SubmittedGoalDraft;
  }
  return {
    draft: value,
    submittedAt: 0,
    projected: false,
  };
}

function goalSummaryFromRow(goal: GoalRow): GoalSummary | null {
  const id = String(goal.goal_id ?? goal.id ?? "").trim();
  if (!id) {
    return null;
  }
  return {
    id,
    title: stringValue(goal.title) || friendlyRef(id) || "Untitled goal",
    objective: stringValue(goal.objective),
    status: stringValue(goal.status) || "unknown",
    progress: numericProgress(goal.percent_done),
    openTasks: numberValue(goal.open_tasks) ?? 0,
    blockedTasks: numberValue(goal.blocked_tasks) ?? 0,
    failedTasks: numberValue(goal.failed_tasks) ?? 0,
    updatedAt: stringValue(goal.updated_at),
  };
}

function pendingGoalRow(goalId: string, draft: JsonRecord): GoalRow {
  const initialTasks = rowsFrom(draft.initial_tasks);
  const subgoals = goalSubgoalsFromDraft(draft);
  return {
    goal_id: goalId,
    id: goalId,
    title: stringValue(draft.title) || "Submitted goal",
    objective: stringValue(draft.objective) || "Waiting for the coordinator projection.",
    status: "submitted",
    percent_done: 0,
    open_tasks: Math.max(1, initialTasks.length || subgoals.length),
    blocked_tasks: 0,
    failed_tasks: 0,
    updated_at: new Date().toISOString(),
  };
}

function goalSubgoalsFromDraft(draft?: JsonRecord | null): JsonRecord[] {
  return rowsFrom(at(draft, ["plan", "subgoals"]));
}

function sortedStatusEntries(counts: Map<string, number>): Array<[string, number]> {
  return [...counts.entries()].sort(([left], [right]) => {
    const leftPriority = statusPriority.get(statusToken(left)) ?? 99;
    const rightPriority = statusPriority.get(statusToken(right)) ?? 99;
    return leftPriority - rightPriority || left.localeCompare(right);
  });
}

function subgoalId(subgoal: JsonRecord): string {
  return stringValue(subgoal.id) || stringValue(subgoal.subgoal_id);
}

function subgoalTitle(subgoal: JsonRecord, index: number): string {
  return stringValue(subgoal.title) || stringValue(subgoal.name) || friendlyRef(subgoalId(subgoal)) || `Subgoal ${index + 1}`;
}

function subgoalObjective(subgoal: JsonRecord): string {
  return stringValue(subgoal.objective) || stringValue(subgoal.summary) || stringValue(subgoal.description) || "Objective pending.";
}

function textList(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.map((item) => {
      if (typeof item === "string") {
        return item.trim();
      }
      if (isRecord(item)) {
        return stringValue(item.title) || stringValue(item.description) || stringValue(item.summary);
      }
      return String(item ?? "").trim();
    }).filter(Boolean);
  }
  const text = stringValue(value);
  return text ? [text] : [];
}

function taskPayload(task: TaskRow): JsonRecord {
  if (isRecord(task.raw_task) && isRecord(task.raw_task.payload_json)) {
    return task.raw_task.payload_json;
  }
  if (isRecord(task.payload_json)) {
    return task.payload_json;
  }
  return {};
}

function colorRef(value: unknown): ColorRef | null {
  return isRecord(value) ? value as ColorRef : null;
}

function safeHex(value: unknown): string {
  const text = stringValue(value);
  return /^#[0-9a-f]{6}$/i.test(text) ? text : "#7d8b94";
}
