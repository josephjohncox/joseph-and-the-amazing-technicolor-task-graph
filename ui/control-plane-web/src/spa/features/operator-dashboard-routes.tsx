import { useQuery } from "@tanstack/react-query";
import clsx from "clsx";
import { CheckCircle2, CircleAlert, GitBranch, MessageSquareText, Monitor, Network, Server, Sparkles } from "lucide-react";

import { at, isRecord, operatorActions, operatorGoalDetail, plans, rowsFrom } from "../api";
import { AdvancedInspect, SimpleTable } from "../components/operator-primitives";
import { Badge } from "../components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/ui/card";
import { ScrollArea } from "../components/ui/scroll-area";
import type { GoalRow, JsonRecord, OperatorWorkspaceSnapshot, ServiceHealth } from "../types";
import { GoalList, composedSnapshotFromOperatorGoalDetail, numericProgress } from "./goal-graph-panel";
import {
  OperatorActionList,
  actionNeededItemsFromOperatorActions,
  mergeActionNeededItems,
  queueItemsFromComposedSnapshot,
} from "./operator-action-panels";
import { excerpt, friendlyRef, normalizeStatus, numberValue, statusLabel, statusTone, stringValue } from "./workbench-format";

export function ServiceStrip({ services }: { services?: ServiceHealth[] }) {
  if (!services?.length) {
    return <span className="status-pill muted">Services pending</span>;
  }
  return (
    <div className="service-strip">
      {services.slice(0, 6).map((service) => (
        <span key={service.name} className={clsx("status-pill", service.ok ? "good" : "bad")}>
          {service.ok ? <CheckCircle2 size={14} /> : <CircleAlert size={14} />}
          {service.name ?? "service"}
        </span>
      ))}
    </div>
  );
}

export function Dashboard(props: { workspace?: OperatorWorkspaceSnapshot; goals: GoalRow[]; selectedGoalId: string; onSelectGoal: (goalId: string) => void }) {
  const runnerRows = rowsFrom(at(props.workspace?.runners, ["data"]) ?? props.workspace?.runners);
  const eventSourceRows = rowsFrom(at(props.workspace?.event_sources, ["data"]) ?? props.workspace?.event_sources);
  const workerRunRows = rowsFrom(props.workspace?.worker_runs);
  const actionCount = props.workspace?.actions?.length ?? 0;
  const operatorEventCount = props.workspace?.events?.length ?? 0;
  const attentionGoals = props.goals.filter((goal) => {
    const status = String(goal.status ?? "").toLowerCase();
    return status.includes("blocked") || status.includes("failed") || Number(goal.blocked_tasks ?? 0) > 0 || Number(goal.failed_tasks ?? 0) > 0;
  }).length;
  return (
    <div className="dashboard-grid">
      <OperatorWorkspaceCard workspace={props.workspace} />
      <MetricCard label="Active goals" value={String(props.goals.length)} detail="in progress" />
      <MetricCard label="Runners" value={String(runnerRows.length)} detail="available capacity" />
      <MetricCard label="Action queue" value={String(actionCount)} detail="waiting decisions" />
      <MetricCard label="Events" value={String(operatorEventCount)} detail="recent signals" />
      <MetricCard label="Event sources" value={String(eventSourceRows.length)} detail="registered ingress" />
      <section className="panel span-2">
        <div className="section-heading">
          <h2>Recent goals</h2>
          <span className="muted-small">Open the work graph</span>
        </div>
        <GoalList goals={props.goals.slice(0, 6)} selectedGoalId={props.selectedGoalId} onSelect={props.onSelectGoal} />
      </section>
      <section className="panel">
        <div className="section-heading">
          <h2>Next outcomes</h2>
          <Sparkles size={18} />
        </div>
        <ul className="outcome-list">
          <OutcomeRow label="Actions" value={actionCount} tone={actionCount ? "waiting-approval" : "done"} />
          <OutcomeRow label="Events" value={operatorEventCount} tone={operatorEventCount ? "runnable" : "done"} />
          <OutcomeRow label="Goal attention" value={attentionGoals} tone={attentionGoals ? "blocked" : "done"} />
          <OutcomeRow label="Runners" value={runnerRows.length} tone={runnerRows.length ? "running" : "pending"} />
        </ul>
      </section>
      <WorkerRunsPanel rows={workerRunRows} />
      <EventSourcesPanel rows={eventSourceRows} />
    </div>
  );
}

function OperatorWorkspaceCard({ workspace }: { workspace?: OperatorWorkspaceSnapshot }) {
  const selected = workspace?.selected_goal?.summary;
  const actions = workspace?.actions?.length ?? 0;
  const workerRuns = workspace?.worker_runs?.length ?? 0;
  const evidence = workspace?.evidence?.length ?? 0;
  return (
    <Card className="operator-workspace-card span-2">
      <CardHeader>
        <div className="section-heading">
          <div>
            <p className="eyebrow">Operator workspace</p>
            <CardTitle>{selected?.title || "No goal selected"}</CardTitle>
            <CardDescription>
              {selected?.objective || "Select or submit a goal to see live state, actions, worker runs, and evidence."}
            </CardDescription>
          </div>
          <Badge variant={actions ? "destructive" : "secondary"}>{actions} actions</Badge>
        </div>
      </CardHeader>
      <CardContent>
        <div className="operator-workspace-grid">
          <span><strong>{Math.round(numericProgress(selected?.percent_done) * 100)}%</strong><small>complete</small></span>
          <span><strong>{selected?.open_tasks ?? 0}</strong><small>open tasks</small></span>
          <span><strong>{workerRuns}</strong><small>worker runs</small></span>
          <span><strong>{evidence}</strong><small>evidence</small></span>
        </div>
        <ScrollArea className="operator-action-preview">
          {(workspace?.actions ?? []).slice(0, 4).map((action) => (
            <div key={action.action_id || action.title} className="operator-action-preview-row">
              <Badge variant="outline">{operatorActionKindLabel(action.kind)}</Badge>
              <span>{action.title || "Action required"}</span>
              <small>{action.question || "Review this item."}</small>
            </div>
          ))}
          {!actions && <span className="muted-small">No pending operator actions.</span>}
        </ScrollArea>
      </CardContent>
    </Card>
  );
}

function OutcomeRow({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <li>
      <span>{label}</span>
      <strong className={clsx("status-pill", statusTone(tone))}>{value}</strong>
    </li>
  );
}

function MetricCard({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <section className="metric-card">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{detail}</small>
    </section>
  );
}

function operatorActionKindLabel(kind: unknown): string {
  const token = normalizeStatus(stringValue(kind));
  if (token === "resolve-approval") return "Approval";
  if (token === "resume-thunk") return "Continuation";
  if (token === "create-thunk") return "Prompt";
  if (token === "restart" || token === "retry") return "Recovery";
  if (token === "cancel-goal") return "Cancel";
  return "Action";
}

export function EventSourcesPanel({ rows }: { rows: JsonRecord[] }) {
  return (
    <section className="panel">
      <div className="section-heading">
        <h2>Event sources</h2>
        <Network size={18} />
      </div>
      <SimpleTable
        empty="Event sources pending."
        headers={["Source", "Status", "Activation"]}
        rows={rows.slice(0, 6).map(eventSourceTableRow)}
      />
    </section>
  );
}

function WorkerRunsPanel({ rows }: { rows: JsonRecord[] }) {
  return (
    <section className="panel">
      <div className="section-heading">
        <h2>Runs</h2>
        <Monitor size={18} />
      </div>
      <SimpleTable
        empty="Worker runs pending."
        headers={["Run", "Worker", "Status", "Summary"]}
        rows={rows.slice(0, 6).map(workerRunTableRow)}
      />
    </section>
  );
}

function workerRunTableRow(row: JsonRecord): string[] {
  const runId = stringValue(row.run_id) || stringValue(row.id) || "run";
  const worker = stringValue(row.worker) || stringValue(row.worker_kind) || stringValue(row.runner_id) || "worker";
  const status = statusLabel(stringValue(row.status) || "unknown");
  const summary = stringValue(row.summary) || stringValue(at(row, ["payload_json", "summary"])) || "";
  return [friendlyRef(runId) || runId, worker, status, excerpt(summary)];
}

export function eventSourceTableRow(row: JsonRecord): string[] {
  const sourceId = stringValue(row.source_id) || stringValue(row.event_source_id) || stringValue(row.id) || stringValue(row.name) || "event source";
  const kind = stringValue(row.kind) || stringValue(row.type) || stringValue(row.provider) || "generic";
  const enabled = row.enabled === true ? "enabled" : row.enabled === false ? "disabled" : stringValue(row.status) || "unknown";
  const approvalStatus = stringValue(row.approval_status) || stringValue(row.activation_status);
  const approvalId = stringValue(row.approval_id);
  const activation = approvalStatus
    ? approvalId
      ? `${approvalStatus} · ${approvalId}`
      : approvalStatus
    : row.requires_approval === true
      ? "approval required"
      : "direct";
  return [`${sourceId} · ${kind}`, enabled, activation];
}

export function PlansView() {
  const planQuery = useQuery({ queryKey: ["plans"], queryFn: plans });
  const rows = rowsFrom(at(planQuery.data, ["data"]) ?? planQuery.data);
  return (
    <section className="dashboard-grid">
      <div className="panel span-2">
        <div className="section-heading">
          <h2>Durable plans</h2>
          <span className="muted-small">Planning-mode records before GoalSpec submission</span>
        </div>
        <SimpleTable
          empty="Plans pending."
          headers={["Plan", "Status", "Mode", "Questions"]}
          rows={rows.map((row) => [
            String(row.title ?? row.plan_id ?? row.id ?? ""),
            String(row.status ?? ""),
            String(row.mode ?? ""),
            String(row.open_question_count ?? ""),
          ])}
        />
      </div>
      <div className="panel">
        <div className="section-heading">
          <h2>Planning queue</h2>
          <GitBranch size={18} />
        </div>
        <p className="body-copy">
          Open planning continuations and follow-up work.
        </p>
        <AdvancedInspect summaryLabel="Details" title="Plan projection" payload={planQuery.data ?? {}} buttonLabel="Inspect JSON" />
      </div>
    </section>
  );
}

export function HumanQueueView({ selectedGoalId, workspace }: { selectedGoalId: string; workspace?: OperatorWorkspaceSnapshot }) {
  const operatorActionsQuery = useQuery({
    queryKey: ["operator-actions", selectedGoalId],
    queryFn: () => operatorActions(selectedGoalId || undefined),
    refetchInterval: 5_000,
  });
  const selectedGoalQuery = useQuery({
    queryKey: ["operator-goal", selectedGoalId],
    queryFn: () => operatorGoalDetail(selectedGoalId),
    enabled: Boolean(selectedGoalId),
  });
  const actionItems = (() => {
    const operatorItems = actionNeededItemsFromOperatorActions(operatorActionsQuery.data ?? workspace?.actions);
    const selectedGoalItems = selectedGoalId ? queueItemsFromComposedSnapshot(composedSnapshotFromOperatorGoalDetail(selectedGoalQuery.data), selectedGoalId) : [];
    return mergeActionNeededItems([...operatorItems, ...selectedGoalItems]);
  })();
  const threadRows = rowsFrom(at(workspace?.human_threads, ["data"]) ?? workspace?.human_threads);
  return (
    <section className="dashboard-grid">
      <div className="panel span-2">
        <div className="section-heading">
          <h2>Action queue</h2>
          <span className="muted-small">Approvals, recovery work, continuations, and stopped history</span>
        </div>
        <OperatorActionList items={actionItems} />
      </div>
      <div className="panel">
        <div className="section-heading">
          <h2>Feedback threads</h2>
          <MessageSquareText size={18} />
        </div>
        <ul className="compact-list">
          {threadRows.length ? threadRows.map((row) => <li key={String(row.thread_key ?? row.key ?? JSON.stringify(row))}><strong>{String(row.thread_key ?? row.key ?? "thread")}</strong><span>{String(row.entries ?? row.reports ?? "")} entries</span></li>) : <li>Threads pending.</li>}
        </ul>
      </div>
    </section>
  );
}

export function RunnersView({ workspace }: { workspace?: OperatorWorkspaceSnapshot }) {
  const rows = rowsFrom(at(workspace?.runners, ["data"]) ?? workspace?.runners);
  const tableRows = rows.map(runnerTableRow);
  const statuses = rows.map(runnerStatusFromRow);
  const active = statuses.filter((status) => ["active", "running", "dispatchable"].includes(status)).length;
  const constrained = statuses.filter((status) => ["full", "stale", "unavailable", "offline"].includes(status)).length;
  const totalCapacity = rows.reduce((sum, row) => sum + (numberValue(row.capacity_remaining) ?? 0), 0);
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>Runner fleet</h2>
          <span className="muted-small">Current runner state, available capacity, and endpoints</span>
        </div>
        <Server size={18} />
      </div>
      <div className="runner-state-strip" aria-label="Runner fleet state">
        <span className="runner-state-card"><strong>{rows.length}</strong><small>registered</small></span>
        <span className="runner-state-card"><strong>{active}</strong><small>active</small></span>
        <span className={clsx("runner-state-card", constrained > 0 && "attention")}><strong>{constrained}</strong><small>constrained</small></span>
        <span className="runner-state-card"><strong>{totalCapacity}</strong><small>free slots</small></span>
      </div>
      <SimpleTable
        empty="Runners pending."
        headers={["Runner", "Node", "Status", "Capacity", "Endpoint"]}
        rows={tableRows}
      />
    </section>
  );
}

function runnerStatusFromRow(row: JsonRecord): string {
  const registration = isRecord(row.registration) ? row.registration : row;
  return normalizeStatus(stringValue(row.status) || (row.stale ? "stale" : row.full ? "full" : row.dispatchable === false ? "unavailable" : stringValue(registration.status) || "active"));
}

export function runnerTableRow(row: JsonRecord): string[] {
  const registration = isRecord(row.registration) ? row.registration : row;
  const labels = {
    ...(isRecord(registration.labels) ? registration.labels : {}),
    ...(isRecord(row.labels) ? row.labels : {}),
  };
  const runnerId = stringValue(row.runner_id) || stringValue(registration.runner_id) || "unknown-runner";
  const displayName = stringValue(row.display_name)
    || stringValue(labels.display_name)
    || stringValue(labels.name)
    || [stringValue(labels.runtime), stringValue(labels.pool)].filter(Boolean).join(" / ")
    || runnerId;
  const nodeId = stringValue(row.node_id) || stringValue(registration.node_id) || "unknown node";
  const endpoint = stringValue(row.endpoint) || stringValue(registration.endpoint) || "no endpoint advertised";
  const status = runnerStatusFromRow(row);
  const maxConcurrency = numberValue(row.max_concurrency ?? registration.max_concurrency);
  const remaining = numberValue(row.capacity_remaining);
  const running = numberValue(row.running_tasks);
  const capacity = maxConcurrency !== null && remaining !== null
    ? `${remaining}/${maxConcurrency} free${running !== null ? `, ${running} running` : ""}`
    : remaining !== null
      ? `${remaining} free${running !== null ? `, ${running} running` : ""}`
      : running !== null
        ? `${running} running`
        : "unknown";
  return [
    displayName === runnerId ? friendlyRef(runnerId) || "Unnamed runner" : `${displayName} (${friendlyRef(runnerId) || runnerId})`,
    nodeId,
    statusLabel(status),
    capacity,
    endpoint,
  ];
}
