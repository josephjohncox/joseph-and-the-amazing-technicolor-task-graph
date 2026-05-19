import * as Popover from "@radix-ui/react-popover";
import clsx from "clsx";
import { Command } from "cmdk";
import {
  CheckCircle2,
  ChevronDown,
  ClipboardCheck,
  FilePenLine,
  FilePlus2,
  ListPlus,
  MessageSquareText,
  Search,
  Send,
  Trash2,
} from "lucide-react";

import { at, isRecord, rowsFrom } from "../api";
import { EmptyState } from "../components/operator-primitives";
import type { JsonRecord } from "../types";
import { friendlyRef, normalizeStatus, statusLabel, statusTone, stringValue } from "./workbench-format";

export type PlanPhase =
  | "asking"
  | "drafting_plan"
  | "drafting_goals"
  | "accepting"
  | "executing"
  | "reviewing"
  | "satisfied"
  | "cancelled";

export type PlanSummary = {
  id: string;
  title: string;
  objective: string;
  status: string;
  phase: PlanPhase;
  openQuestions: number;
  goalCount: number;
  updatedAt: string;
  source: "projection" | "workspace";
};

export type AcceptedPlanDraft = {
  draftId: string;
  draft: JsonRecord;
  acceptedAt: string;
  sourceSessionId: string;
};

export type StagedGoalDraft = {
  draftId: string;
  draft: JsonRecord;
  acceptedAt: string;
  sourcePlanId: string;
  sourceGoalId?: string;
};

export type DraftSummary = {
  title: string;
  objective: string;
  detail: string;
};

export const workspacePlanId = "workspace-plan";

const planPhases: Array<{ key: PlanPhase; label: string; detail: string }> = [
  { key: "asking", label: "Ask", detail: "Clarify direction and context." },
  { key: "drafting_plan", label: "Draft plan", detail: "Shape the workflow." },
  { key: "drafting_goals", label: "Draft goals", detail: "Create satisfiable work." },
  { key: "accepting", label: "Accept", detail: "Commit staged work." },
  { key: "executing", label: "Execute", detail: "Run accepted goals." },
  { key: "reviewing", label: "Review", detail: "Inspect evidence." },
  { key: "satisfied", label: "Satisfied", detail: "Outcome accepted." },
  { key: "cancelled", label: "Cancelled", detail: "Stopped by operator." },
];

export function planSummariesFromRows(rows: JsonRecord[]): PlanSummary[] {
  return rows.map(planSummaryFromRow).filter((plan): plan is PlanSummary => Boolean(plan));
}

export function defaultWorkspacePlan(goalCount: number, selectedGoalTitle = ""): PlanSummary {
  return {
    id: workspacePlanId,
    title: selectedGoalTitle ? `Workspace plan for ${selectedGoalTitle}` : "Workspace intake plan",
    objective: selectedGoalTitle
      ? "Plan-scoped workspace for the selected goal, follow-on drafts, actions, evidence, and memory."
      : "Ask questions, draft a plan, then accept goals into execution.",
    status: "asking",
    phase: "asking",
    openQuestions: 0,
    goalCount,
    updatedAt: "",
    source: "workspace",
  };
}

export function selectedPlanFromSummaries(plans: PlanSummary[], selectedPlanId: string, fallback: PlanSummary): PlanSummary {
  return plans.find((plan) => plan.id === selectedPlanId) ?? plans[0] ?? fallback;
}

export function derivePlanPhase(input: {
  selectedPlan: PlanSummary;
  selectedGoalStatus?: string;
  activeDraftKind?: string;
  hasPlanDraft: boolean;
  hasAcceptedPlanDraft?: boolean;
  hasGoalDraft: boolean;
  stagedGoalCount: number;
  actionCount: number;
}): PlanPhase {
  const goalStatus = normalizeStatus(input.selectedGoalStatus ?? "");
  if (goalStatus === "cancelled" || input.selectedPlan.phase === "cancelled") {
    return "cancelled";
  }
  if (goalStatus === "done" || goalStatus === "satisfied" || input.selectedPlan.phase === "satisfied") {
    return "satisfied";
  }
  if (input.actionCount > 0) {
    return "executing";
  }
  if (input.hasGoalDraft || input.stagedGoalCount > 0 || input.activeDraftKind === "goal") {
    return input.stagedGoalCount > 0 ? "accepting" : "drafting_goals";
  }
  if (input.hasAcceptedPlanDraft) {
    return "drafting_goals";
  }
  if (input.hasPlanDraft || input.activeDraftKind === "plan") {
    return "drafting_plan";
  }
  if (goalStatus && !["selected", "submitted", "unknown"].includes(goalStatus)) {
    return goalStatus === "needs-validation" ? "reviewing" : "executing";
  }
  return input.selectedPlan.phase || "asking";
}

export function planDraftFromChatResponse(response?: { drafts?: JsonRecord; draft_summary?: JsonRecord }): JsonRecord | null {
  const draft = firstRecord([
    response?.drafts?.plan,
    response?.drafts?.plan_spec,
    response?.drafts?.execution_plan,
    response?.draft_summary?.plan,
  ]);
  if (!draft) {
    return null;
  }
  const title = stringValue(draft.title) || stringValue(draft.name) || stringValue(draft.summary);
  const objective = stringValue(draft.objective) || stringValue(draft.summary) || stringValue(draft.description);
  return title || objective ? draft : null;
}

export function planDraftSummary(draft?: JsonRecord | null): DraftSummary {
  return {
    title: stringValue(draft?.title) || stringValue(draft?.name) || "Untitled plan draft",
    objective: stringValue(draft?.objective) || stringValue(draft?.summary) || stringValue(draft?.description),
    detail: stringValue(draft?.detail) || stringValue(draft?.rationale) || "Review and accept this plan before staging goals.",
  };
}

export function goalDraftSummary(draft?: JsonRecord | null): DraftSummary {
  return {
    title: stringValue(draft?.title) || "Untitled goal draft",
    objective: stringValue(draft?.objective),
    detail: stringValue(at(draft, ["authoring", "intake_summary"])) || stringValue(at(draft, ["plan", "summary"])) || "Goal draft is ready to stage or submit.",
  };
}

export function draftIdFor(prefix: string, draft: JsonRecord): string {
  const stable = stringValue(draft.draft_id) || stringValue(draft.id) || stringValue(draft.title) || stringValue(draft.objective) || Date.now().toString(36);
  return `${prefix}-${stable.toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-|-$/g, "").slice(0, 44) || Date.now().toString(36)}`;
}

export function PlanContextBar(props: {
  plans: PlanSummary[];
  selectedPlan: PlanSummary;
  selectedPlanId: string;
  open: boolean;
  loading: boolean;
  onOpenChange: (open: boolean) => void;
  onSelectPlan: (planId: string) => void;
  onRefreshPlans: () => void;
  onOpenPlans: () => void;
}) {
  return (
    <Popover.Root open={props.open} onOpenChange={props.onOpenChange}>
      <div className="plan-context-bar">
        <Popover.Trigger asChild>
          <button
            type="button"
            className="plan-context-trigger"
            aria-expanded={props.open}
            aria-label={`Current plan: ${props.selectedPlan.title}`}
            data-testid="plan-context-trigger"
          >
            <div>
              <span className="goal-context-kicker">Current plan</span>
              <strong>{props.selectedPlan.title}</strong>
              <small>{props.selectedPlan.objective}</small>
            </div>
            <span className={clsx("status-pill", statusTone(props.selectedPlan.status))}>
              {props.loading ? "Loading" : statusLabel(props.selectedPlan.phase)}
            </span>
            <ChevronDown size={16} />
          </button>
        </Popover.Trigger>
        <Popover.Portal>
          <Popover.Content className="goal-picker plan-picker" align="end" sideOffset={8}>
            <Command shouldFilter className="goal-command">
              <div className="goal-picker-actions">
                <label>
                  Search plans
                  <Command.Input placeholder="Plan title, status, or id" />
                </label>
                <button type="button" className="secondary-button" onClick={props.onRefreshPlans}>
                  Refresh
                </button>
              </div>
              <Command.List className="goal-picker-list">
                <Command.Empty>
                  <EmptyState title="Plan match pending" detail="Draft a plan from Ask, or refresh projections." />
                </Command.Empty>
                <Command.Group>
                  {props.plans.map((plan) => (
                    <Command.Item
                      key={plan.id}
                      value={`${plan.title} ${plan.objective} ${plan.status} ${plan.id}`}
                      className={clsx("goal-picker-item", props.selectedPlanId === plan.id && "active")}
                      onSelect={() => {
                        props.onSelectPlan(plan.id);
                        props.onOpenChange(false);
                      }}
                    >
                      <span>
                        <strong>{plan.title}</strong>
                        <small>{plan.objective || friendlyRef(plan.id)}</small>
                      </span>
                      <span className={clsx("status-pill", statusTone(plan.status))}>{statusLabel(plan.phase)}</span>
                    </Command.Item>
                  ))}
                </Command.Group>
              </Command.List>
              <div className="goal-picker-footer">
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => {
                    props.onSelectPlan(workspacePlanId);
                    props.onOpenChange(false);
                  }}
                >
                  Workspace plan
                </button>
                <button type="button" className="primary-button" onClick={props.onOpenPlans}>
                  Open plan
                </button>
              </div>
            </Command>
          </Popover.Content>
        </Popover.Portal>
      </div>
    </Popover.Root>
  );
}

export function PlanPhaseRail(props: {
  phase: PlanPhase;
  selectedPlan: PlanSummary;
  selectedGoalTitle?: string;
  actionCount: number;
  stagedGoalCount: number;
  hasPlanDraft: boolean;
  hasGoalDraft: boolean;
  onAsk: () => void;
  onDraftPlan: () => void;
  onDraftGoal: () => void;
  onReviewAccept: () => void;
  onOpenActions: () => void;
}) {
  return (
    <section className="plan-phase-rail" aria-label="Plan workflow">
      <div className="phase-rail-heading">
        <div>
          <p className="eyebrow">Plan workflow</p>
          <h2>{props.selectedPlan.title}</h2>
          <span>{props.selectedGoalTitle ? `Goal focus: ${props.selectedGoalTitle}` : "No nested goal focus selected."}</span>
        </div>
        <span className={clsx("operator-state-pill", phaseTone(props.phase))}>{phaseLabel(props.phase)}</span>
      </div>
      <div className="phase-stepper">
        {planPhases.map((phase) => (
          <span key={phase.key} className={clsx("phase-step", phase.key === props.phase && "active", phaseOrder(phase.key) < phaseOrder(props.phase) && "complete")}>
            {phaseOrder(phase.key) < phaseOrder(props.phase) ? <CheckCircle2 size={14} /> : <span className="phase-dot" />}
            <strong>{phase.label}</strong>
            <small>{phase.detail}</small>
          </span>
        ))}
      </div>
      <div className="phase-action-row" aria-label="Plan phase actions">
        <button type="button" className={clsx("phase-action-button", props.phase === "asking" && "active")} onClick={props.onAsk}>
          <MessageSquareText size={15} />
          Ask
        </button>
        <button type="button" className={clsx("phase-action-button", props.phase === "drafting_plan" && "active")} onClick={props.onDraftPlan}>
          <FilePenLine size={15} />
          Draft plan
        </button>
        <button type="button" className={clsx("phase-action-button", props.phase === "drafting_goals" && "active")} onClick={props.onDraftGoal}>
          <ListPlus size={15} />
          Draft goal
        </button>
        <button type="button" className={clsx("phase-action-button", props.phase === "accepting" && "active")} onClick={props.onReviewAccept}>
          <ClipboardCheck size={15} />
          Accept
        </button>
        <button type="button" className={clsx("phase-action-button", props.actionCount > 0 && "attention")} onClick={props.onOpenActions}>
          <CheckCircle2 size={15} />
          {props.actionCount ? `${props.actionCount} actions` : "Actions"}
        </button>
      </div>
      <div className="phase-action-items" aria-label="Current plan action items">
        <PhaseActionItem
          title={props.hasPlanDraft ? "Review plan draft" : "Draft or ask"}
          detail={props.hasPlanDraft ? "A staged plan is waiting for acceptance." : "Ask a question or draft the high-level plan."}
          action={props.hasPlanDraft ? "Accept plan" : "Draft plan"}
        />
        <PhaseActionItem
          title={props.hasGoalDraft || props.stagedGoalCount ? "Review goal drafts" : "Create goal contracts"}
          detail={props.stagedGoalCount ? `${props.stagedGoalCount} goal draft${props.stagedGoalCount === 1 ? "" : "s"} staged in this plan.` : "Draft goals after the plan shape is clear."}
          action={props.hasGoalDraft || props.stagedGoalCount ? "Accept or submit" : "Draft goal"}
        />
        <PhaseActionItem
          title={props.actionCount ? "Resolve plan actions" : "No action blockers"}
          detail={props.actionCount ? "Plan-scoped actions are waiting for a decision." : "No approvals, prompts, or recovery actions are waiting."}
          action={props.actionCount ? "Open actions" : "Continue"}
        />
      </div>
    </section>
  );
}

function PhaseActionItem(props: { title: string; detail: string; action: string }) {
  return (
    <article className="phase-action-item">
      <strong>{props.title}</strong>
      <span>{props.detail}</span>
      <small>{props.action}</small>
    </article>
  );
}

export function StagedDraftCards(props: {
  planDraft: JsonRecord | null;
  goalDraft: JsonRecord | null;
  acceptedPlanDraft: AcceptedPlanDraft | null;
  stagedGoalDrafts: StagedGoalDraft[];
  onEditDraft: () => void;
  onAcceptPlanDraft: () => void;
  onDiscardPlanDraft: () => void;
  onAcceptGoalIntoPlan: () => void;
  onSubmitGoalDraft: () => void;
  onDiscardGoalDraft: () => void;
  onSubmitStagedGoal: (draftId: string) => void;
  onDiscardStagedGoal: (draftId: string) => void;
}) {
  const hasCards = props.planDraft || props.goalDraft || props.acceptedPlanDraft || props.stagedGoalDrafts.length > 0;
  if (!hasCards) {
    return (
      <section className="staged-drafts-panel" aria-label="Plan drafts">
        <EmptyState title="No staged drafts" detail="Use Ask, Draft plan, or Draft goal to build the plan graph." />
      </section>
    );
  }
  return (
    <section className="staged-drafts-panel" aria-label="Plan drafts">
      {props.acceptedPlanDraft && (
        <DraftActionCard
          eyebrow="Accepted plan"
          summary={planDraftSummary(props.acceptedPlanDraft.draft)}
          tone="status-done"
          actions={[
            { label: "Edit draft", variant: "secondary", onClick: props.onEditDraft },
          ]}
        />
      )}
      {props.planDraft && (
        <DraftActionCard
          eyebrow="Plan draft"
          summary={planDraftSummary(props.planDraft)}
          tone="status-runnable"
          actions={[
            { label: "Edit draft", variant: "secondary", onClick: props.onEditDraft },
            { label: "Discard", variant: "danger", onClick: props.onDiscardPlanDraft },
            { label: "Accept plan", variant: "primary", onClick: props.onAcceptPlanDraft },
          ]}
        />
      )}
      {props.goalDraft && (
        <DraftActionCard
          eyebrow="Goal draft"
          summary={goalDraftSummary(props.goalDraft)}
          tone="status-runnable"
          actions={[
            { label: "Edit draft", variant: "secondary", onClick: props.onEditDraft },
            { label: "Discard", variant: "danger", onClick: props.onDiscardGoalDraft },
            { label: "Accept into plan", variant: "secondary", onClick: props.onAcceptGoalIntoPlan },
            { label: "Submit goal", variant: "primary", onClick: props.onSubmitGoalDraft },
          ]}
        />
      )}
      {props.stagedGoalDrafts.map((draft) => (
        <DraftActionCard
          key={draft.draftId}
          eyebrow="Staged goal"
          summary={goalDraftSummary(draft.draft)}
          tone="status-submitted"
          actions={[
            { label: "Discard", variant: "danger", onClick: () => props.onDiscardStagedGoal(draft.draftId) },
            { label: "Submit goal", variant: "primary", onClick: () => props.onSubmitStagedGoal(draft.draftId) },
          ]}
        />
      ))}
    </section>
  );
}

function DraftActionCard(props: {
  eyebrow: string;
  summary: DraftSummary;
  tone: string;
  actions: Array<{ label: string; variant: "primary" | "secondary" | "danger"; onClick: () => void }>;
}) {
  return (
    <article className="draft-action-card">
      <div>
        <span className={clsx("status-pill", props.tone)}>{props.eyebrow}</span>
        <strong>{props.summary.title}</strong>
        {props.summary.objective && <p>{props.summary.objective}</p>}
        {props.summary.detail && <small>{props.summary.detail}</small>}
      </div>
      <div className="button-row">
        {props.actions.map((action) => (
          <button
            key={action.label}
            type="button"
            className={action.variant === "primary" ? "primary-button" : action.variant === "danger" ? "danger-button" : "secondary-button"}
            onClick={action.onClick}
          >
            {action.label === "Submit goal" && <Send size={15} />}
            {action.label === "Accept plan" && <ClipboardCheck size={15} />}
            {action.label === "Accept into plan" && <FilePlus2 size={15} />}
            {action.label === "Discard" && <Trash2 size={15} />}
            {action.label === "Edit draft" && <FilePenLine size={15} />}
            {action.label}
          </button>
        ))}
      </div>
    </article>
  );
}

export function PlanRouteView(props: {
  plans: PlanSummary[];
  selectedPlan: PlanSummary;
  phase: PlanPhase;
  selectedGoalTitle?: string;
  planDraft: JsonRecord | null;
  goalDraft: JsonRecord | null;
  acceptedPlanDraft: AcceptedPlanDraft | null;
  stagedGoalDrafts: StagedGoalDraft[];
  planActions: JsonRecord[];
  actionBusy: boolean;
  onSelectPlan: (planId: string) => void;
  onDraftPlan: () => void;
  onDraftGoal: () => void;
  onAcceptPlanDraft: () => void;
  onDiscardPlanDraft: () => void;
  onAcceptGoalIntoPlan: () => void;
  onSubmitGoalDraft: () => void;
  onDiscardGoalDraft: () => void;
  onSubmitStagedGoal: (draftId: string) => void;
  onDiscardStagedGoal: (draftId: string) => void;
  onResolvePlanAction: (actionId: string, resolution: string) => void;
  onEditDraft: () => void;
}) {
  return (
    <section className="plan-route-grid">
      <div className="panel span-2">
        <div className="section-heading">
          <div>
            <h2>Plan workspace</h2>
            <span className="muted-small">Ask, draft the plan, draft goals, then accept staged work.</span>
          </div>
          <span className={clsx("operator-state-pill", phaseTone(props.phase))}>{phaseLabel(props.phase)}</span>
        </div>
        <StagedDraftCards
          planDraft={props.planDraft}
          goalDraft={props.goalDraft}
          acceptedPlanDraft={props.acceptedPlanDraft}
          stagedGoalDrafts={props.stagedGoalDrafts}
          onEditDraft={props.onEditDraft}
          onAcceptPlanDraft={props.onAcceptPlanDraft}
          onDiscardPlanDraft={props.onDiscardPlanDraft}
          onAcceptGoalIntoPlan={props.onAcceptGoalIntoPlan}
          onSubmitGoalDraft={props.onSubmitGoalDraft}
          onDiscardGoalDraft={props.onDiscardGoalDraft}
          onSubmitStagedGoal={props.onSubmitStagedGoal}
          onDiscardStagedGoal={props.onDiscardStagedGoal}
        />
      </div>
      <div className="panel">
        <div className="section-heading">
          <h2>Plan selector</h2>
          <Search size={18} />
        </div>
        <div className="plan-list">
          {props.plans.map((plan) => (
            <button
              key={plan.id}
              type="button"
              className={clsx("plan-list-card", props.selectedPlan.id === plan.id && "active")}
              onClick={() => props.onSelectPlan(plan.id)}
            >
              <strong>{plan.title}</strong>
              <span>{plan.objective}</span>
              <small>{statusLabel(plan.phase)} · {plan.goalCount} goals</small>
            </button>
          ))}
        </div>
        <div className="button-row plan-route-actions">
          <button type="button" className="secondary-button" onClick={props.onDraftPlan}>Draft plan</button>
          <button type="button" className="primary-button" onClick={props.onDraftGoal}>Draft goal</button>
        </div>
        <PlanActionList
          actions={props.planActions}
          busy={props.actionBusy}
          onResolvePlanAction={props.onResolvePlanAction}
        />
      </div>
    </section>
  );
}

function PlanActionList(props: {
  actions: JsonRecord[];
  busy: boolean;
  onResolvePlanAction: (actionId: string, resolution: string) => void;
}) {
  if (!props.actions.length) {
    return (
      <div className="plan-action-list" aria-label="Plan actions">
        <EmptyState title="No plan actions" detail="Draft or accept work to create the next plan action." />
      </div>
    );
  }
  return (
    <div className="plan-action-list" aria-label="Plan actions">
      {props.actions.map((action) => {
        const actionId = stringValue(action.action_id) || stringValue(action.id);
        const kind = stringValue(action.kind);
        const allowed = Array.isArray(action.allowed_actions)
          ? action.allowed_actions.map((item) => stringValue(item)).filter(Boolean)
          : [];
        const resolutions = allowed.length ? allowed : [kind].filter(Boolean);
        return (
          <article key={actionId || `${kind}-${action.title}`} className="plan-action-card">
            <strong>{stringValue(action.title) || statusLabel(kind)}</strong>
            <span>{stringValue(action.reason) || "Resolve this plan action to continue the workflow."}</span>
            <div className="button-row">
              {resolutions.slice(0, 4).map((resolution) => (
                <button
                  key={resolution}
                  type="button"
                  className={resolution === "cancel" ? "danger-button" : resolution.includes("accept") || resolution.includes("submit") ? "primary-button" : "secondary-button"}
                  disabled={props.busy || !actionId}
                  onClick={() => props.onResolvePlanAction(actionId, resolution)}
                >
                  {statusLabel(resolution)}
                </button>
              ))}
            </div>
          </article>
        );
      })}
    </div>
  );
}

export function phaseLabel(phase: PlanPhase): string {
  return planPhases.find((item) => item.key === phase)?.label ?? "Ask";
}

function phaseTone(phase: PlanPhase): string {
  if (phase === "cancelled") return "state-waiting";
  if (phase === "satisfied") return "state-satisfied";
  if (phase === "reviewing") return "state-reviewing";
  if (phase === "executing") return "state-running";
  if (phase === "accepting") return "state-action-needed";
  return "state-waiting";
}

function phaseOrder(phase: PlanPhase): number {
  return planPhases.findIndex((item) => item.key === phase);
}

function planSummaryFromRow(row: JsonRecord): PlanSummary | null {
  const id = stringValue(row.plan_id) || stringValue(row.id);
  if (!id) {
    return null;
  }
  const status = stringValue(row.status) || stringValue(row.phase) || "asking";
  const phase = phaseFromValue(stringValue(row.phase) || status);
  return {
    id,
    title: stringValue(row.title) || stringValue(row.name) || friendlyRef(id) || "Untitled plan",
    objective: stringValue(row.objective) || stringValue(row.summary) || stringValue(row.description),
    status,
    phase,
    openQuestions: Number(row.open_question_count ?? row.open_questions ?? 0) || 0,
    goalCount: Number(row.goal_count ?? row.goals ?? rowsFrom(row.goal_refs).length) || 0,
    updatedAt: stringValue(row.updated_at),
    source: "projection",
  };
}

function phaseFromValue(value: string): PlanPhase {
  const normalized = normalizeStatus(value).replace(/-/g, "_");
  if (normalized === "draft_plan") return "drafting_plan";
  if (normalized === "draft_goal" || normalized === "drafting_goal") return "drafting_goals";
  if (planPhases.some((phase) => phase.key === normalized)) {
    return normalized as PlanPhase;
  }
  if (normalized === "done") return "satisfied";
  if (normalized === "running") return "executing";
  return "asking";
}

function firstRecord(values: unknown[]): JsonRecord | null {
  for (const value of values) {
    if (isRecord(value)) {
      return value;
    }
  }
  return null;
}
