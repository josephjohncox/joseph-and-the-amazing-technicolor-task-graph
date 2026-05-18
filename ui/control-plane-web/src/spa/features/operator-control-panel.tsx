import { useMutation, useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import {
  CheckCircle2,
  FileJson,
  GitBranch,
  PauseCircle,
  RotateCcw,
  Search,
  ShieldCheck,
  Split,
  ThumbsDown,
  ThumbsUp,
  Vote,
  XCircle,
} from "lucide-react";
import { useMemo, useState } from "react";

import {
  branchGoal,
  cancelGoal,
  createThunk,
  mechanismBallot,
  mechanismStart,
  restartGoal,
  selectBranch,
  steer,
  voteGoal,
} from "../api";
import { AdvancedInspect, EmptyState, InspectButton } from "../components/operator-primitives";
import type { ComposedGoalSnapshot, JsonRecord } from "../types";
import {
  ComputeGraphDetails,
  GraphStatusPanel,
  TaskSummary,
  countForStatusToken,
  taskId,
  taskRowsFromComposedSnapshot,
  taskStatusCounts,
} from "./goal-graph-panel";
import {
  ActionNeededPanel,
  EvidenceNextActionPanel,
  actionNeededItemsFromComposedSnapshot,
  applyActionEnvelopeToCache,
  continuationRowsFromComposedSnapshot,
  nextActionSummary,
} from "./operator-action-panels";
import { createRunId, statusLabel, tokenList } from "./workbench-format";

type OtherGoalAction = "review" | "research" | "priority" | "steer" | "restart_branch" | "wait" | "decision_round" | "ballot";

export function CompilerControlView(props: { goalId: string; snapshot?: ComposedGoalSnapshot; loading: boolean; onOpenGoalPicker: () => void }) {
  const tasks = taskRowsFromComposedSnapshot(props.snapshot);
  const counts = taskStatusCounts(tasks);
  const actionNeeded = useMemo(() => actionNeededItemsFromComposedSnapshot(props.snapshot, props.goalId), [props.snapshot, props.goalId]);
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>Operator actions</h2>
          <span className="muted-small">Review evidence, recover blocked work, and choose one follow-up action when needed</span>
        </div>
      </div>
      {!props.goalId ? (
        <EmptyState title="Select a goal" detail="Use the top-bar goal switcher." actionLabel="Choose goal" onAction={props.onOpenGoalPicker} />
      ) : props.loading ? (
        <EmptyState title="Loading actions" detail="Fetching workflow projection." />
      ) : (
        <>
          {props.snapshot && <ActionNeededPanel items={actionNeeded} />}
          {props.snapshot && <EvidenceNextActionPanel snapshot={props.snapshot} counts={counts} taskCount={tasks.length} />}
          {props.snapshot && <GraphStatusPanel counts={counts} taskCount={tasks.length} />}
          <CompilerControlPanel goalId={props.goalId} snapshot={props.snapshot} />
          {props.snapshot && (
            <details className="advanced-details">
              <summary>
                <span>Advanced projection details</span>
                <small>Task counts and compute graph rows</small>
              </summary>
              <TaskSummary snapshot={props.snapshot} counts={counts} />
              <ComputeGraphDetails snapshot={props.snapshot} />
            </details>
          )}
        </>
      )}
    </section>
  );
}

export function CompilerControlPanel({ goalId, snapshot, compact = false }: { goalId: string; snapshot?: ComposedGoalSnapshot; compact?: boolean }) {
  const queryClient = useQueryClient();
  const tasks = taskRowsFromComposedSnapshot(snapshot);
  const firstTaskId = taskId(tasks[0] ?? {});
  const [operator, setOperator] = useState("operator");
  const [result, setResult] = useState<unknown>(null);
  const [otherAction, setOtherAction] = useState<OtherGoalAction>("review");
  const [voteReason, setVoteReason] = useState("Promote or demote this goal based on current priority.");
  const [voteWeight, setVoteWeight] = useState(1);
  const [suggestedRole, setSuggestedRole] = useState("peer_goal");
  const [steerKind, setSteerKind] = useState("evaluate_goal_completion");
  const [steerTaskId, setSteerTaskId] = useState("");
  const [steerTopic, setSteerTopic] = useState("");
  const [steerReason, setSteerReason] = useState("Evaluate whether the durable evidence satisfies the current objective.");
  const [reviewCheck, setReviewCheck] = useState("behavioral_testing");
  const [flowMode, setFlowMode] = useState("restart");
  const [flowReason, setFlowReason] = useState("Operator requested a follow-up action.");
  const [restartScope, setRestartScope] = useState("goal");
  const [restartTaskId, setRestartTaskId] = useState("");
  const [branchTargetTaskId, setBranchTargetTaskId] = useState("");
  const [branchCandidates, setBranchCandidates] = useState(2);
  const [branchRoles, setBranchRoles] = useState("codex,reviewer");
  const [branchGroupId, setBranchGroupId] = useState("");
  const [selectedTaskId, setSelectedTaskId] = useState("");
  const [thunkKind, setThunkKind] = useState("human_input");
  const [thunkReason, setThunkReason] = useState("Need an operator decision before resuming the task graph.");
  const [thunkInput, setThunkInput] = useState("Provide the decision or missing input.");
  const [thunkTimeoutSeconds, setThunkTimeoutSeconds] = useState(3600);
  const [mechanismTitle, setMechanismTitle] = useState("Choose the next implementation path");
  const [mechanismKind, setMechanismKind] = useState("approval_vote");
  const [mechanismTarget, setMechanismTarget] = useState("subgoal_selection");
  const [mechanismReason, setMechanismReason] = useState("Use a coordinator-owned round to choose the next task graph move.");
  const [mechanismProposals, setMechanismProposals] = useState("codex-fast | Fast Codex implementation path | planner\nreview-deep | Deep reviewer-first path | planner");
  const [ballotRoundId, setBallotRoundId] = useState("");
  const [ballotProposalId, setBallotProposalId] = useState("");
  const [ballotRationale, setBallotRationale] = useState("Best fit for the current goal evidence.");
  const mutation = useMutation({
    mutationFn: async (action: { label: string; run: () => Promise<unknown> }) => {
      const value = await action.run();
      return { label: action.label, value };
    },
    onSuccess: (value) => {
      setResult(value);
      applyActionEnvelopeToCache(queryClient, value.value, goalId);
    },
  });
  const disabled = !goalId || mutation.isPending;
  const run = (label: string, action: () => Promise<unknown>) => mutation.mutate({ label, run: action });
  const counts = taskStatusCounts(tasks);
  const nextAction = snapshot ? nextActionSummary(counts, tasks.length, snapshot) : null;
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = snapshot ? continuationRowsFromComposedSnapshot(snapshot).length : 0;
  const accepted = countForStatusToken(counts, "done");
  const primaryTaskId = steerTaskId || firstTaskId;
  const runSteering = (label: string, kind: string, reason: string, topic = "", check = reviewCheck) => run(
    label,
    () => steer(goalId, steeringPayload({ goalId, operator, taskId: primaryTaskId, kind, topic, reason, reviewCheck: check })),
  );

  return (
    <div className={clsx("compiler-control-panel", compact && "compact")}>
      <div className="section-heading">
        <div>
          <h3>Actions</h3>
          <span className="muted-small">Use a direct action, or configure one focused follow-up.</span>
        </div>
        {result ? (
          <AdvancedInspect summaryLabel="Last action" title="Last goal action" payload={result} buttonLabel="Inspect JSON" />
        ) : (
          <span className="status-pill muted">Action result pending</span>
        )}
      </div>
      <div className="control-primary-grid" aria-label="Primary operator actions">
        <section className="primary-action-card">
          <span className="goal-context-kicker">Current state</span>
          <h4>{nextAction?.stateLabel ?? "Waiting"}</h4>
          <p>{nextAction?.title ?? "Projection pending"}</p>
          <small>{nextAction?.detail ?? "The coordinator has not projected task state yet."}</small>
        </section>
        <section className="primary-action-card">
          <span className="goal-context-kicker">Blockers</span>
          <h4>{failed + blocked + approvals}</h4>
          <p>{failed} failed · {blocked} blocked · {approvals} approvals</p>
          <small>{continuations} waiting continuations</small>
        </section>
        <section className="primary-action-card">
          <span className="goal-context-kicker">Evidence</span>
          <h4>{accepted}/{tasks.length || 0}</h4>
          <p>accepted task evidence</p>
          <small>{tasks.length ? "Use review when evidence is ready." : "Task evidence pending."}</small>
        </section>
        <section className="primary-action-card action-card">
          <span className="goal-context-kicker">Primary actions</span>
          <div className="button-row">
            <button
              type="button"
              className="primary-button"
              disabled={disabled}
              data-testid="primary-review-evidence"
              onClick={() => runSteering("review evidence", "evaluate_goal_completion", "Review whether current durable evidence satisfies the goal done criteria.")}
            >
              <ShieldCheck size={16} />
              Review evidence
            </button>
            {(failed + blocked) > 0 && (
              <button
                type="button"
                className="primary-button"
                disabled={disabled}
                data-testid="primary-recover-blockers"
                onClick={() => run("recover blockers", () => restartGoal(goalId, {
                  goal_id: goalId,
                  scope: failed > 0 ? "failed" : "blocked",
                  reason: "operator_requested",
                  message: `Recover ${failed > 0 ? "failed" : "blocked"} work from primary operator actions.`,
                  task_id: null,
                  reset_attempts: false,
                  preserve_artifacts: true,
                  operator,
                }))}
              >
                <RotateCcw size={16} />
                Recover blockers
              </button>
            )}
            <button
              type="button"
              className="secondary-button"
              disabled={disabled}
              data-testid="primary-request-research"
              onClick={() => runSteering("request research", "request_research", "Find the highest-risk missing information before the next control action.", "highest-risk missing information")}
            >
              <Search size={16} />
              Research gap
            </button>
          </div>
        </section>
      </div>
      <div className="control-grid">
        <details className="advanced-control-panel span-2">
          <summary>
            <span>More operator actions</span>
            <small>Choose one focused action to configure</small>
          </summary>
          <div className="other-action-picker">
            <label>
              Action
              <select value={otherAction} onChange={(event) => setOtherAction(event.target.value as OtherGoalAction)}>
                <option value="review">Request standard review</option>
                <option value="research">Ask for research</option>
                <option value="priority">Promote or demote priority</option>
                <option value="steer">Steer the coordinator</option>
              </select>
            </label>
          </div>
          <div className="control-grid nested">
            {otherAction === "priority" && <section className="control-card">
              <div className="section-heading">
                <h4>Goal priority</h4>
                <Vote size={17} />
              </div>
              <div className="form-grid two">
                <label>
                  Voter
                  <input value={operator} onChange={(event) => setOperator(event.target.value)} />
                </label>
                <label>
                  Role
                  <select value={suggestedRole} onChange={(event) => setSuggestedRole(event.target.value)}>
                    <option value="overarching_goal">Overarching goal</option>
                    <option value="peer_goal">Peer goal</option>
                    <option value="subgoal">Subgoal</option>
                  </select>
                </label>
                <label>
                  Weight
                  <input type="number" min={1} max={100} value={voteWeight} onChange={(event) => setVoteWeight(Number(event.target.value) || 1)} />
                </label>
                <label>
                  Reason
                  <input value={voteReason} onChange={(event) => setVoteReason(event.target.value)} />
                </label>
              </div>
              <div className="button-row">
                <button type="button" className="primary-button" disabled={disabled} onClick={() => run("upvote", () => voteGoal(goalId, votePayload(goalId, operator, "up", voteWeight, voteReason, suggestedRole)))}>
                  <ThumbsUp size={16} />
                  Upvote
                </button>
                <button type="button" className="secondary-button" disabled={disabled} onClick={() => run("downvote", () => voteGoal(goalId, votePayload(goalId, operator, "down", voteWeight, voteReason, suggestedRole)))}>
                  <ThumbsDown size={16} />
                  Downvote
                </button>
              </div>
            </section>}

            {otherAction === "steer" && <section className="control-card">
              <div className="section-heading">
                <h4>Steering directive</h4>
                <ShieldCheck size={17} />
              </div>
              <div className="form-grid two">
                <label>
                  Directive
                  <select value={steerKind} onChange={(event) => setSteerKind(event.target.value)}>
                    <option value="evaluate_goal_completion">Evaluate completion</option>
                    <option value="request_research">Request research</option>
                    <option value="request_standard_review">Standard review</option>
                    <option value="expand_done_criteria">Expand done criteria</option>
                    <option value="pause">Pause</option>
                    <option value="resume">Resume</option>
                    <option value="inject_task">Inject task</option>
                  </select>
                </label>
                <label>
                  Task
                  <input value={steerTaskId} onChange={(event) => setSteerTaskId(event.target.value)} placeholder="optional task id" title={firstTaskId || undefined} />
                </label>
                <label>
                  Review check
                  <select value={reviewCheck} onChange={(event) => setReviewCheck(event.target.value)} disabled={steerKind !== "request_standard_review"}>
                    {standardReviewChecks.map((check) => <option key={check} value={check}>{check}</option>)}
                  </select>
                </label>
                <label>
                  Topic or question
                  <input value={steerTopic} onChange={(event) => setSteerTopic(event.target.value)} placeholder="what to inspect, research, or inject" />
                </label>
              </div>
              <label>
                Reason
                <textarea value={steerReason} onChange={(event) => setSteerReason(event.target.value)} />
              </label>
              <button type="button" className="primary-button" disabled={disabled || !steerReason.trim()} onClick={() => run("steer", () => steer(goalId, steeringPayload({ goalId, operator, taskId: steerTaskId, kind: steerKind, topic: steerTopic, reason: steerReason, reviewCheck })))}>
                Apply steering
              </button>
            </section>}

            {otherAction === "restart_branch" && <section className="control-card">
              <div className="section-heading">
                <h4>Restart or branch</h4>
                <Split size={17} />
              </div>
              <div className="form-grid two">
                <label>
                  Action
                  <select value={flowMode} onChange={(event) => setFlowMode(event.target.value)}>
                    <option value="restart">Restart</option>
                    <option value="branch">Branch</option>
                    <option value="select_branch">Select branch</option>
                    <option value="cancel">Cancel</option>
                  </select>
                </label>
                <label>
                  Restart scope
                  <select value={restartScope} onChange={(event) => setRestartScope(event.target.value)} disabled={flowMode !== "restart"}>
                    <option value="goal">Goal</option>
                    <option value="task">Task</option>
                    <option value="failed">Failed tasks</option>
                    <option value="blocked">Blocked tasks</option>
                    <option value="timed_out">Timed out tasks</option>
                  </select>
                </label>
                <label>
                  Task / target task
                  <input value={flowMode === "branch" ? branchTargetTaskId : restartTaskId} onChange={(event) => flowMode === "branch" ? setBranchTargetTaskId(event.target.value) : setRestartTaskId(event.target.value)} placeholder="optional task id" title={firstTaskId || undefined} />
                </label>
                <label>
                  Candidates
                  <input type="number" min={1} max={8} value={branchCandidates} onChange={(event) => setBranchCandidates(Number(event.target.value) || 1)} disabled={flowMode !== "branch"} />
                </label>
                <label>
                  Candidate roles
                  <input value={branchRoles} onChange={(event) => setBranchRoles(event.target.value)} disabled={flowMode !== "branch"} />
                </label>
                <label>
                  Branch group
                  <input value={branchGroupId} onChange={(event) => setBranchGroupId(event.target.value)} disabled={flowMode !== "select_branch"} placeholder="branch group id" />
                </label>
                <label>
                  Selected task
                  <input value={selectedTaskId} onChange={(event) => setSelectedTaskId(event.target.value)} disabled={flowMode !== "select_branch"} placeholder="candidate task id" />
                </label>
                <label>
                  Reason
                  <input value={flowReason} onChange={(event) => setFlowReason(event.target.value)} />
                </label>
              </div>
              <div className="button-row">
                <button type="button" className={flowMode === "cancel" ? "danger-button" : "primary-button"} disabled={disabled || !flowReason.trim()} onClick={() => run(flowMode, () => flowAction({ goalId, flowMode, operator, reason: flowReason, restartScope, restartTaskId, branchTargetTaskId, branchCandidates, branchRoles, branchGroupId, selectedTaskId }))}>
                  {flowMode === "restart" && <RotateCcw size={16} />}
                  {flowMode === "branch" && <GitBranch size={16} />}
                  {flowMode === "select_branch" && <CheckCircle2 size={16} />}
                  {flowMode === "cancel" && <XCircle size={16} />}
                  {flowActionButtonLabel(flowMode)}
                </button>
              </div>
            </section>}

            {otherAction === "wait" && <section className="control-card">
              <div className="section-heading">
                <h4>Wait state</h4>
                <PauseCircle size={17} />
              </div>
              <div className="form-grid two">
                <label>
                  Kind
                  <select value={thunkKind} onChange={(event) => setThunkKind(event.target.value)}>
                    <option value="human_input">Human input</option>
                    <option value="approval">Approval</option>
                    <option value="external_event">External event</option>
                    <option value="timer">Timer</option>
                    <option value="resource_availability">Runner capacity</option>
                    <option value="model_availability">Model availability</option>
                  </select>
                </label>
                <label>
                  Timeout seconds
                  <input type="number" min={0} value={thunkTimeoutSeconds} onChange={(event) => setThunkTimeoutSeconds(Number(event.target.value) || 0)} />
                </label>
              </div>
              <label>
                Requested input
                <input value={thunkInput} onChange={(event) => setThunkInput(event.target.value)} />
              </label>
              <label>
                Reason
                <textarea value={thunkReason} onChange={(event) => setThunkReason(event.target.value)} />
              </label>
              <button type="button" className="secondary-button" disabled={disabled || !thunkReason.trim()} onClick={() => run("create wait state", () => createThunk(goalId, thunkPayload({ goalId, taskId: firstTaskId, kind: thunkKind, reason: thunkReason, requestedInput: thunkInput, timeoutSeconds: thunkTimeoutSeconds })))}>
                <PauseCircle size={16} />
                Create wait state
              </button>
            </section>}

            {otherAction === "decision_round" && <section className="control-card span-2">
              <div className="section-heading">
                <h4>Decision round</h4>
                <Vote size={17} />
              </div>
              <div className="form-grid three">
                <label>
                  Title
                  <input value={mechanismTitle} onChange={(event) => setMechanismTitle(event.target.value)} />
                </label>
                <label>
                  Decision type
                  <select value={mechanismKind} onChange={(event) => setMechanismKind(event.target.value)}>
                    {mechanismKinds.map((kind) => <option key={kind} value={kind}>{statusLabel(kind)}</option>)}
                  </select>
                </label>
                <label>
                  Target
                  <select value={mechanismTarget} onChange={(event) => setMechanismTarget(event.target.value)}>
                    {mechanismTargets.map((target) => <option key={target} value={target}>{statusLabel(target)}</option>)}
                  </select>
                </label>
              </div>
              <label>
                Proposals
                <textarea value={mechanismProposals} onChange={(event) => setMechanismProposals(event.target.value)} />
              </label>
              <label>
                Reason
                <input value={mechanismReason} onChange={(event) => setMechanismReason(event.target.value)} />
              </label>
              <div className="button-row">
                <button type="button" className="primary-button" disabled={disabled || !mechanismTitle.trim()} onClick={() => run("start mechanism", () => mechanismStart(goalId, mechanismStartPayload({ goalId, title: mechanismTitle, mechanism: mechanismKind, target: mechanismTarget, reason: mechanismReason, proposals: mechanismProposals })))}>
                  Start round
                </button>
                <InspectButton title="Decision proposal format" payload={{ format: "label | description | proposer", example: mechanismProposals }} buttonLabel="Format" />
              </div>
            </section>}

            {otherAction === "ballot" && <section className="control-card">
              <div className="section-heading">
                <h4>Decision ballot</h4>
                <FileJson size={17} />
              </div>
              <label>
                Round ID
                <input value={ballotRoundId} onChange={(event) => setBallotRoundId(event.target.value)} />
              </label>
              <label>
                Proposal ID
                <input value={ballotProposalId} onChange={(event) => setBallotProposalId(event.target.value)} />
              </label>
              <label>
                Rationale
                <textarea value={ballotRationale} onChange={(event) => setBallotRationale(event.target.value)} />
              </label>
              <button type="button" className="secondary-button" disabled={disabled || !ballotRoundId || !ballotProposalId} onClick={() => run("cast ballot", () => mechanismBallot(goalId, mechanismBallotPayload({ goalId, roundId: ballotRoundId, proposalId: ballotProposalId, participant: operator, rationale: ballotRationale })))}>
                Cast ballot
              </button>
            </section>}

            {otherAction === "review" && <section className="control-card">
              <div className="section-heading">
                <h4>Request standard review</h4>
                <CheckCircle2 size={17} />
              </div>
              <label>
                Review focus
                <select value={reviewCheck} onChange={(event) => setReviewCheck(event.target.value)}>
                  {standardReviewChecks.map((check) => <option key={check} value={check}>{statusLabel(check)}</option>)}
                </select>
              </label>
              <label>
                Topic
                <input value={steerTopic} onChange={(event) => setSteerTopic(event.target.value)} placeholder="current evidence, a task id, or a risk" />
              </label>
              <label>
                Reason
                <textarea value={steerReason} onChange={(event) => setSteerReason(event.target.value)} />
              </label>
              <button
                type="button"
                className="primary-button"
                disabled={disabled || !steerReason.trim()}
                onClick={() => runSteering("request review", "request_standard_review", steerReason, steerTopic || "current task evidence", reviewCheck)}
              >
                Request review
              </button>
            </section>}

            {otherAction === "research" && <section className="control-card">
              <div className="section-heading">
                <h4>Ask for research</h4>
                <Search size={17} />
              </div>
              <label>
                Question
                <input value={steerTopic} onChange={(event) => setSteerTopic(event.target.value)} placeholder="what should the research task answer?" />
              </label>
              <label>
                Reason
                <textarea value={steerReason} onChange={(event) => setSteerReason(event.target.value)} />
              </label>
              <button
                type="button"
                className="primary-button"
                disabled={disabled || !steerReason.trim()}
                onClick={() => runSteering("request research", "request_research", steerReason, steerTopic || "highest-risk missing information")}
              >
                Ask for research
              </button>
            </section>}
          </div>
        </details>
      </div>
      {mutation.error && <span className="error-text">{mutation.error.message}</span>}
    </div>
  );
}

const standardReviewChecks = [
  "abstraction",
  "readability",
  "compile",
  "test_evidence",
  "behavioral_testing",
  "hypothesis_testing",
  "type_soundness",
  "formal_verification",
  "clean_code",
  "ddd",
  "functional_ddd",
  "denotational_semantics",
  "canonical_style",
  "library_fit",
  "reference_search",
  "web_search",
  "deep_research",
  "simplicity",
  "security",
  "output_safety",
];
const mechanismKinds = ["approval_vote", "majority_vote", "ranked_choice", "delphi_round", "sealed_bid_auction", "vickrey_auction", "contract_net"];
const mechanismTargets = ["goal_priority", "goal_promotion", "subgoal_selection", "branch_selection", "runner_allocation", "budget_allocation", "work_auction", "review_panel", "custom"];

function votePayload(goalId: string, voter: string, direction: "up" | "down", weight: number, reason: string, suggestedRole: string): JsonRecord {
  return {
    goal_id: goalId,
    voter: voter.trim() || "operator",
    source: "human",
    direction,
    weight: Math.max(1, Math.round(weight)),
    reason,
    suggested_role: suggestedRole || null,
  };
}

function steeringPayload(input: { goalId: string; operator: string; taskId: string; kind: string; topic: string; reason: string; reviewCheck: string }): JsonRecord {
  const topic = input.topic.trim();
  const reason = input.reason.trim();
  return {
    id: createRunId(),
    goal_id: input.goalId,
    task_id: input.taskId.trim() || null,
    operator: input.operator.trim() || "operator",
    message: reason,
    kind: steeringKindPayload(input.kind, topic, reason, input.reviewCheck),
  };
}

function steeringKindPayload(kind: string, topic: string, reason: string, reviewCheck: string): JsonRecord {
  if (kind === "request_research") {
    return { kind, question: topic || reason, reason };
  }
  if (kind === "request_standard_review") {
    return { kind, check: reviewCheck, topic: topic || null, reason };
  }
  if (kind === "expand_done_criteria") {
    return {
      kind,
      tests_pass: true,
      artifact_exists: true,
      validator_score_min: 0.9,
      min_satisfaction_score: 0.9,
      reason,
      apply_to_open_tasks: true,
      reopen_terminal_tasks: false,
    };
  }
  if (kind === "pause" || kind === "resume") {
    return { kind, reason };
  }
  if (kind === "inject_task") {
    return { kind, role: "reviewer", prompt: topic || reason, reason };
  }
  return { kind: "evaluate_goal_completion", reason };
}

function flowActionButtonLabel(flowMode: string): string {
  if (flowMode === "branch") return "Create branch candidates";
  if (flowMode === "select_branch") return "Select branch";
  if (flowMode === "cancel") return "Cancel goal";
  return "Restart work";
}

function flowAction(input: {
  goalId: string;
  flowMode: string;
  operator: string;
  reason: string;
  restartScope: string;
  restartTaskId: string;
  branchTargetTaskId: string;
  branchCandidates: number;
  branchRoles: string;
  branchGroupId: string;
  selectedTaskId: string;
}): Promise<unknown> {
  if (input.flowMode === "branch") {
    return branchGoal(input.goalId, {
      goal_id: input.goalId,
      target_task_id: input.branchTargetTaskId.trim() || null,
      subgoal_id: null,
      reason: input.reason,
      candidate_count: Math.max(1, Math.round(input.branchCandidates)),
      candidate_roles: tokenList(input.branchRoles),
      candidate_executions: [],
      prompt_overrides: [],
      selection_strategy: "voter_quorum",
      operator: input.operator.trim() || "operator",
    });
  }
  if (input.flowMode === "select_branch") {
    return selectBranch(input.goalId, {
      goal_id: input.goalId,
      group_id: input.branchGroupId.trim(),
      selected_task_id: input.selectedTaskId.trim(),
      selector: "human",
      reason: input.reason,
    });
  }
  if (input.flowMode === "cancel") {
    return cancelGoal(input.goalId, input.reason);
  }
  return restartGoal(input.goalId, {
    goal_id: input.goalId,
    scope: input.restartScope,
    reason: "operator_requested",
    message: input.reason,
    task_id: input.restartTaskId.trim() || null,
    reset_attempts: false,
    preserve_artifacts: true,
    operator: input.operator.trim() || "operator",
  });
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

function mechanismStartPayload(input: { goalId: string; title: string; mechanism: string; target: string; reason: string; proposals: string }): JsonRecord {
  return {
    goal_id: input.goalId,
    title: input.title,
    mechanism: input.mechanism,
    target: input.target,
    reason: input.reason,
    proposals: proposalLines(input.proposals),
    quorum: 2,
    min_participants: 2,
    require_human_ratification: false,
  };
}

function proposalLines(value: string): JsonRecord[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [label, description, proposer] = line.split("|").map((part) => part.trim());
      return { label: label || "proposal", description: description || line, proposer: proposer || "operator", metadata: {} };
    });
}

function mechanismBallotPayload(input: { goalId: string; roundId: string; proposalId: string; participant: string; rationale: string }): JsonRecord {
  return {
    goal_id: input.goalId,
    round_id: input.roundId,
    participant: input.participant.trim() || "operator",
    source: "human",
    allocations: [{ proposal_id: input.proposalId, support: 1, rank: 1, bid: null }],
    rationale: input.rationale,
  };
}
