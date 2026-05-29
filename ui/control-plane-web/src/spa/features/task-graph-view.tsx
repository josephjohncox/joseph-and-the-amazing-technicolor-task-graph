import { Background, Controls, MiniMap, ReactFlow } from "@xyflow/react";
import clsx from "clsx";
import { ShieldCheck } from "lucide-react";
import { useMemo, useState } from "react";

import { EmptyState } from "../components/operator-primitives";
import type { ComposedGoalSnapshot, JsonRecord } from "../types";
import {
  ComputeGraphDetails,
  GraphStatusPanel,
  SubgoalPlanPanel,
  TaskSummary,
  computeGraphNodes,
  computeNodeMatchesGraphFilter,
  goalSubgoalsFromComposedSnapshotOrDraft,
  graphFilterOptions,
  graphFromComputeGraph,
  graphFromTasks,
  taskMatchesGraphFilter,
  taskRowsFromComposedSnapshot,
  taskRowsFromGoalDraft,
  taskStatusCounts,
  workflowComputeGraph,
  type GraphFilter,
} from "./goal-graph-panel";
import {
  ActionNeededPanel,
  BlockerInsightPanel,
  ContinuationQueue,
  EvidenceNextActionPanel,
  actionNeededItemsFromComposedSnapshot,
  continuationRowsFromComposedSnapshot,
} from "./operator-action-panels";

export function TaskGraphView(props: {
  goalId: string;
  snapshot?: ComposedGoalSnapshot;
  submittedDraft?: JsonRecord | null;
  loading: boolean;
  onOpenGoalPicker: () => void;
  onOpenControls: () => void;
}) {
  const [graphFilter, setGraphFilter] = useState<GraphFilter>("all");
  const projectedTasks = useMemo(() => taskRowsFromComposedSnapshot(props.snapshot), [props.snapshot]);
  const draftTasks = useMemo(() => taskRowsFromGoalDraft(props.goalId, props.submittedDraft), [props.goalId, props.submittedDraft]);
  const tasks = projectedTasks.length ? projectedTasks : draftTasks;
  const computeGraph = useMemo(() => props.snapshot ? workflowComputeGraph(props.snapshot) : undefined, [props.snapshot]);
  const computeNodes = useMemo(() => computeGraphNodes(computeGraph), [computeGraph]);
  const filteredComputeNodes = useMemo(() => computeNodes.filter((node) => computeNodeMatchesGraphFilter(node, graphFilter)), [computeNodes, graphFilter]);
  const filteredTasks = useMemo(() => tasks.filter((task) => taskMatchesGraphFilter(task, graphFilter)), [tasks, graphFilter]);
  const graph = useMemo(() => computeNodes.length ? graphFromComputeGraph(computeGraph, filteredComputeNodes) : graphFromTasks(filteredTasks), [computeGraph, computeNodes.length, filteredComputeNodes, filteredTasks]);
  const counts = useMemo(() => taskStatusCounts(tasks), [tasks]);
  const subgoals = useMemo(() => goalSubgoalsFromComposedSnapshotOrDraft(props.snapshot, props.submittedDraft), [props.snapshot, props.submittedDraft]);
  const showingSubmittedDraft = Boolean(props.submittedDraft && !projectedTasks.length);
  const taskCount = tasks.length;
  const visibleCount = computeNodes.length ? filteredComputeNodes.length : filteredTasks.length;
  const totalCount = computeNodes.length ? computeNodes.length : taskCount;
  const graphUnit = computeNodes.length ? "compute nodes" : "tasks";
  const continuationCount = props.snapshot ? continuationRowsFromComposedSnapshot(props.snapshot).length : 0;
  const actionNeeded = useMemo(() => actionNeededItemsFromComposedSnapshot(props.snapshot, props.goalId), [props.snapshot, props.goalId]);
  return (
    <section className="panel graph-panel">
      <div className="section-heading">
        <div>
          <h2>Work graph</h2>
          <span className="muted-small">Current state, next action, blockers, evidence, and dependencies</span>
        </div>
      </div>
      {props.snapshot && (
        <div className="graph-toolbar">
          <div className="graph-filter" aria-label="Work graph filter">
            {graphFilterOptions.map((option) => (
              <button
                key={option.key}
                type="button"
                className={clsx("graph-filter-button", graphFilter === option.key && "active")}
                aria-pressed={graphFilter === option.key}
                title={option.detail}
                data-testid={`graph-filter-${option.key}`}
                onClick={() => setGraphFilter(option.key)}
              >
                {option.label}
              </button>
            ))}
          </div>
          <span className="filter-count">Showing {visibleCount} of {totalCount} {graphUnit}</span>
        </div>
      )}
      {props.snapshot && <EvidenceNextActionPanel snapshot={props.snapshot} counts={counts} taskCount={taskCount} />}
      {props.snapshot && <BlockerInsightPanel items={actionNeeded} onOpenControls={props.onOpenControls} />}
      {props.snapshot && <ActionNeededPanel items={actionNeeded} />}
      {props.snapshot && <GraphStatusPanel counts={counts} taskCount={taskCount} />}
      {!props.goalId ? (
        <EmptyState title="Select a goal" detail="Use the top-bar goal switcher." actionLabel="Choose goal" onAction={props.onOpenGoalPicker} />
      ) : props.loading && !showingSubmittedDraft ? (
        <EmptyState title="Loading task graph" detail="Loading goal state and task activity." />
      ) : taskCount === 0 ? (
        <EmptyState title="Task activity pending" detail="Waiting for the first projected task." />
      ) : graph.nodes.length ? (
        <div className="flow-canvas">
          <ReactFlow nodes={graph.nodes} edges={graph.edges} fitView>
            <MiniMap nodeColor={(node) => String(node.style?.borderColor ?? "var(--accent)")} maskColor="var(--flow-minimap-mask)" />
            <Controls />
            <Background color="var(--flow-dot)" gap={18} size={1.2} />
          </ReactFlow>
        </div>
      ) : (
        <EmptyState title="Filter has no matches" detail="Try another task state." />
      )}
      {showingSubmittedDraft && <EmptyState title="Goal submitted" detail="Draft tasks are visible while goal state catches up." />}
      {subgoals.length > 0 && <SubgoalPlanPanel subgoals={subgoals} source={showingSubmittedDraft ? "submitted draft" : "goal projection"} />}
      {props.snapshot && (
        <>
          {continuationCount > 0 && (
            <details className="advanced-details">
              <summary>
                <span>Waiting continuations</span>
                <small>Inline resume fields for delayed work</small>
              </summary>
              <ContinuationQueue goalId={props.goalId} snapshot={props.snapshot} />
            </details>
          )}
          <section className="graph-control-redirect">
            <div>
              <strong>Need action?</strong>
              <span>Open operator actions to review evidence, recover blockers, ask for research, or choose another path.</span>
            </div>
            <button type="button" className="secondary-button" onClick={props.onOpenControls}>
              <ShieldCheck size={15} />
              Open actions
            </button>
          </section>
          <details className="advanced-details">
            <summary>
              <span>Advanced graph details</span>
              <small>Task counts and compute projection</small>
            </summary>
            <TaskSummary snapshot={props.snapshot} counts={counts} />
            <ComputeGraphDetails snapshot={props.snapshot} />
          </details>
        </>
      )}
    </section>
  );
}
