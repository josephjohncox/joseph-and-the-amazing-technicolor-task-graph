import {
  ChatContainer,
  MainContainer,
  Message,
  MessageInput,
  MessageList,
  TypingIndicator,
} from "@chatscope/chat-ui-kit-react";
import clsx from "clsx";
import { GitBranch, ListChecks, MessageSquareText, Network, Search, ShieldCheck } from "lucide-react";
import { useEffect, useRef } from "react";

import { at, isRecord, rowsFrom } from "../api";
import { AdvancedInspect } from "../components/operator-primitives";
import type { ChatMessage, ChatResponse, ChatRunTrace, JsonRecord } from "../types";
import { planDraftSummary, type PlanPhase } from "./plan-workflow";
import { friendlyRef, statusTone, stringValue } from "./workbench-format";

export type DraftKind = "ask" | "plan" | "goal" | "search";
export type GoalDraftEditField = "title" | "objective" | "acceptance_evidence" | "constraints";

export type ActiveDraftState = {
  kind: DraftKind;
  mode: string;
  sessionId: string;
  selectedGoalId: string;
  savedAt: string;
  response: ChatResponse;
  goalDraft: JsonRecord | null;
  planDraft: JsonRecord | null;
  runId: string | null;
  selectedPlanId?: string;
};

export type DraftReviewSummary = {
  title: string;
  objective: string;
  summary: string;
  reference: string;
  source: string;
  evidenceCount: number;
  constraintCount: number;
};

type SelectedGoalContext = {
  title: string;
  status: string;
};

type SelectedPlanContext = {
  title: string;
  phase: PlanPhase;
};

export function ChatDraftPanel(props: {
  messages: ChatMessage[];
  input: string;
  draftKind: DraftKind;
  busy: boolean;
  error: Error | null;
  activeDraft: ActiveDraftState | null;
  latestResponse?: ChatResponse;
  chatRun?: ChatRunTrace;
  goalDraft: JsonRecord | null;
  planDraft: JsonRecord | null;
  goalSubmitBusy: boolean;
  goalSubmitError: Error | null;
  goalSubmitResult?: unknown;
  selectedPlanId: string;
  selectedPlan: SelectedPlanContext;
  selectedGoalId: string;
  selectedGoal: SelectedGoalContext | null;
  sessionId: string;
  mode: string;
  onDraftKindChange: (value: DraftKind) => void;
  onInputChange: (value: string) => void;
  onSend: (content?: string) => void;
  onSubmitGoalDraft: () => void;
  onAcceptGoalIntoPlan: () => void;
  onDiscardGoalDraft: () => void;
  onAcceptPlanDraft: () => void;
  onDiscardPlanDraft: () => void;
  onUpdateGoalDraftField: (field: GoalDraftEditField, value: string) => void;
  onClear: () => void;
}) {
  const draftKeys = Object.keys(props.latestResponse?.drafts ?? {});
  const activityPayload = chatActivityPayload(props);
  const activityLabel = props.busy ? "Activity" : "Run details";
  const submittedGoalId = goalIdFromSubmitResponse(props.goalSubmitResult);
  const draftSummary = draftReviewSummary(props.activeDraft?.response ?? props.latestResponse, props.goalDraft);
  const chatShellRef = useRef<HTMLDivElement>(null);
  const latestMessage = props.messages[props.messages.length - 1];
  const draftFromOtherSession = Boolean(props.activeDraft && props.activeDraft.sessionId !== props.sessionId);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => {
      const root = chatShellRef.current;
      const scrollBox = root?.querySelector<HTMLElement>(".cs-message-list");
      if (scrollBox) {
        scrollBox.scrollTop = scrollBox.scrollHeight;
      }
      root?.querySelector<HTMLElement>(".cs-message:last-child")?.scrollIntoView({ block: "end" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [props.busy, props.sessionId, props.messages.length, latestMessage?.role, latestMessage?.content]);

  return (
    <section className="command-panel" aria-label="Assistant">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Assistant</p>
          <h2>{props.goalDraft ? "Review goal draft" : "Ask"}</h2>
        </div>
        <div className="draft-mode-group">
          <div className="mode-toggle" role="group" aria-label="Draft type">
            <button
              type="button"
              className={clsx("mode-option", props.draftKind === "ask" && "active")}
              aria-pressed={props.draftKind === "ask"}
              onClick={() => props.onDraftKindChange("ask")}
            >
              <MessageSquareText size={15} />
              Ask
            </button>
            <button
              type="button"
              className={clsx("mode-option", props.draftKind === "plan" && "active")}
              aria-pressed={props.draftKind === "plan"}
              onClick={() => props.onDraftKindChange("plan")}
            >
              <GitBranch size={15} />
              Plan
            </button>
            <button
              type="button"
              className={clsx("mode-option", props.draftKind === "goal" && "active")}
              aria-pressed={props.draftKind === "goal"}
              onClick={() => props.onDraftKindChange("goal")}
            >
              <ListChecks size={15} />
              Draft goal
            </button>
          </div>
          <details className="secondary-mode-details">
            <summary>Search</summary>
            <div>
              <button
                type="button"
                className={clsx("mode-option", props.draftKind === "search" && "active")}
                aria-pressed={props.draftKind === "search"}
                onClick={() => props.onDraftKindChange("search")}
              >
                <Search size={15} />
                Search
              </button>
            </div>
          </details>
        </div>
      </div>
      <div className="outcome-meta" aria-label="Chat scope">
        <span className={clsx("status-pill", props.selectedGoal ? statusTone(props.selectedGoal.status) : "muted")}>
          Plan: {props.selectedPlan.title}
        </span>
        {props.selectedGoal && (
          <span className={clsx("status-pill", statusTone(props.selectedGoal.status))}>
            Goal focus: {props.selectedGoal.title}
          </span>
        )}
        {!props.selectedGoal && (
          <span className="status-pill muted">
            Goal focus: none
          </span>
        )}
        <span className="status-pill muted">
          Phase: {phaseDisplay(props.selectedPlan.phase)}
        </span>
        <span className={clsx("status-pill", props.busy ? "status-running" : "muted")}>
          {props.busy ? commandBusyLabel(props.draftKind) : draftModeHeadline(props.draftKind)}
        </span>
        {props.activeDraft && (
          <span className={clsx("status-pill", props.goalDraft ? "status-runnable" : "muted")}>
            {props.goalDraft ? `Draft: ${draftKindLabel(props.activeDraft.kind)}` : "Response saved"}
          </span>
        )}
        {draftFromOtherSession && <span className="status-pill status-waiting-input">Draft from {sessionDisplayLabel(props.activeDraft?.sessionId ?? "")}</span>}
        {!props.activeDraft && (props.busy || props.chatRun || props.latestResponse || draftKeys.length > 0) && (
          <AdvancedInspect summaryLabel={activityLabel} title="Chat activity" payload={activityPayload} buttonLabel="Debug" />
        )}
      </div>
      {props.planDraft && (
        <PlanDraftReview
          draft={props.planDraft}
          disabled={props.busy}
          onAccept={props.onAcceptPlanDraft}
          onDiscard={props.onDiscardPlanDraft}
        />
      )}
      {props.goalDraft && (
        <GoalDraftEditor
          draft={props.goalDraft}
          summary={draftSummary}
          disabled={props.busy || props.goalSubmitBusy || Boolean(submittedGoalId)}
          submitDisabled={props.busy || props.goalSubmitBusy || Boolean(submittedGoalId)}
          submitBusy={props.goalSubmitBusy}
          submittedGoalLabel={submittedGoalId ? friendlyRef(submittedGoalId) : ""}
          onUpdate={props.onUpdateGoalDraftField}
          onSubmit={props.onSubmitGoalDraft}
          onAcceptIntoPlan={props.onAcceptGoalIntoPlan}
          onDiscard={props.onDiscardGoalDraft}
        />
      )}
      {props.activeDraft && !props.goalDraft && (
        <DraftSummaryCard summary={draftSummary} />
      )}
      <details className="quick-prompts">
        <summary>
          <span>Shortcuts</span>
          <small>Common requests</small>
        </summary>
        <div aria-label="Goal action prompts">
          {compilerPromptTemplates(props.selectedGoalId, props.selectedGoal?.title).map((template) => (
            <button key={template.label} type="button" className="secondary-button" disabled={props.busy} onClick={() => props.onSend(template.prompt)}>
              {template.icon === "graph" && <Network size={15} />}
              {template.icon === "control" && <ShieldCheck size={15} />}
              {template.icon === "research" && <Search size={15} />}
              {template.label}
            </button>
          ))}
        </div>
      </details>
      <div className="chat-shell" ref={chatShellRef}>
        <MainContainer className="coat-chat-container" responsive>
          <ChatContainer>
            <MessageList
              autoScrollToBottom
              autoScrollToBottomOnMount
              scrollBehavior="smooth"
              typingIndicator={props.busy ? <TypingIndicator content={commandBusyLabel(props.draftKind)} /> : undefined}
            >
              {props.messages.map((message, index) => (
                <Message
                  key={`${message.role}-${index}`}
                  model={{
                    message: message.content,
                    sender: message.role === "assistant" ? "COAT" : "Operator",
                    direction: message.role === "user" ? "outgoing" : "incoming",
                    position: "single",
                  }}
                />
              ))}
            </MessageList>
            <MessageInput
              value={props.input}
              attachButton={false}
              autoFocus={false}
              sendDisabled={props.busy || !props.input.trim()}
              placeholder={commandPlaceholder(props.draftKind)}
              onChange={(_html, textContent) => props.onInputChange(textContent)}
              onSend={(_html, textContent) => props.onSend(textContent)}
            />
          </ChatContainer>
        </MainContainer>
      </div>
      <div className="composer-actions">
        {props.error && <span className="error-text">{props.error.message}</span>}
        {props.goalSubmitError && <span className="error-text">{props.goalSubmitError.message}</span>}
        <button type="button" className="secondary-button" onClick={props.onClear}>
          Clear chat
        </button>
      </div>
    </section>
  );
}

function GoalDraftEditor(props: {
  draft: JsonRecord;
  summary: DraftReviewSummary;
  disabled: boolean;
  submitDisabled: boolean;
  submitBusy: boolean;
  submittedGoalLabel: string;
  onUpdate: (field: GoalDraftEditField, value: string) => void;
  onSubmit: () => void;
  onAcceptIntoPlan: () => void;
  onDiscard: () => void;
}) {
  return (
    <section className="goal-draft-editor" aria-label="Goal draft review">
      <div className="draft-editor-header">
        <div>
          <span className="goal-context-kicker">Goal draft ready</span>
          <strong>{props.summary.title}</strong>
          {props.summary.objective && <p>{props.summary.objective}</p>}
        </div>
        <div className="button-row" aria-label="Draft actions">
          <button type="button" className="secondary-button" disabled={props.submitBusy} onClick={props.onDiscard}>
            Discard
          </button>
          <button type="button" className="secondary-button" disabled={props.disabled} onClick={props.onAcceptIntoPlan}>
            Accept into plan
          </button>
          <button type="button" className="primary-button" disabled={props.submitDisabled} onClick={props.onSubmit}>
            {props.submittedGoalLabel ? "Submitted" : props.submitBusy ? "Submitting" : "Submit goal"}
          </button>
        </div>
      </div>
      <div className="draft-editor-grid">
        <label>
          Title
          <input
            value={stringValue(props.draft.title)}
            disabled={props.disabled}
            onChange={(event) => props.onUpdate("title", event.target.value)}
          />
        </label>
        <label>
          Objective
          <textarea
            value={stringValue(props.draft.objective)}
            disabled={props.disabled}
            onChange={(event) => props.onUpdate("objective", event.target.value)}
          />
        </label>
        <label>
          Evidence requirements
          <textarea
            value={linesFromList(at(props.draft, ["authoring", "acceptance_evidence"]))}
            disabled={props.disabled}
            onChange={(event) => props.onUpdate("acceptance_evidence", event.target.value)}
          />
        </label>
        <label>
          Constraints
          <textarea
            value={linesFromList(at(props.draft, ["authoring", "constraints"]))}
            disabled={props.disabled}
            onChange={(event) => props.onUpdate("constraints", event.target.value)}
          />
        </label>
      </div>
      <div className="draft-summary-meta" aria-label="Draft summary">
        {props.submittedGoalLabel && <span className="status-pill status-done">Selected {props.submittedGoalLabel}</span>}
        <span className="status-pill status-runnable">Edit draft</span>
        <span className="status-pill muted">{countLabel(props.summary.evidenceCount, "evidence item")}</span>
        <span className="status-pill muted">{countLabel(props.summary.constraintCount, "constraint")}</span>
      </div>
    </section>
  );
}

function PlanDraftReview(props: {
  draft: JsonRecord;
  disabled: boolean;
  onAccept: () => void;
  onDiscard: () => void;
}) {
  const summary = planDraftSummary(props.draft);
  const phases = rowsFrom(props.draft.phases ?? props.draft.steps ?? props.draft.workstreams);
  const goals = rowsFrom(props.draft.goals ?? props.draft.goal_slots ?? props.draft.subgoals);
  return (
    <section className="goal-draft-editor plan-draft-review" aria-label="Plan draft review">
      <div className="draft-editor-header">
        <div>
          <span className="goal-context-kicker">Plan draft ready</span>
          <strong>{summary.title}</strong>
          {summary.objective && <p>{summary.objective}</p>}
        </div>
        <div className="button-row" aria-label="Plan draft actions">
          <button type="button" className="secondary-button" disabled={props.disabled} onClick={props.onDiscard}>
            Discard
          </button>
          <button type="button" className="primary-button" disabled={props.disabled} onClick={props.onAccept}>
            Accept plan
          </button>
        </div>
      </div>
      <div className="draft-summary-meta">
        <span className="status-pill muted">{phases.length} phases</span>
        <span className="status-pill muted">{goals.length} goal slots</span>
        <span className="status-pill status-runnable">Edit draft in chat</span>
      </div>
    </section>
  );
}

function DraftSummaryCard({ summary }: { summary: DraftReviewSummary }) {
  return (
    <div className="draft-summary-card">
      <div>
        <span className="goal-context-kicker">Draft</span>
        <strong>{summary.title}</strong>
        {summary.objective && <p>{summary.objective}</p>}
        {summary.summary && summary.summary !== summary.objective && <small>{summary.summary}</small>}
      </div>
      <div className="draft-summary-meta" aria-label="Draft summary">
        {summary.reference && <span className="status-pill muted">{friendlyRef(summary.reference) || "Saved draft"}</span>}
        <span className="status-pill muted">{summary.source === "GoalSpec payload" ? "Goal draft" : summary.source}</span>
        <span className="status-pill muted">{countLabel(summary.evidenceCount, "evidence item")}</span>
        <span className="status-pill muted">{countLabel(summary.constraintCount, "constraint")}</span>
      </div>
    </div>
  );
}

export function modeForDraftKind(kind: DraftKind): string {
  if (kind === "ask") return "ask";
  if (kind === "goal") return "draft_goal";
  if (kind === "search") return "draft_search";
  return "draft_plan";
}

export function goalDraftFromChatResponse(response?: ChatResponse): JsonRecord | null {
  const draft = response?.drafts?.goal_spec;
  if (!isRecord(draft)) {
    return null;
  }
  const title = typeof draft.title === "string" ? draft.title.trim() : "";
  const objective = typeof draft.objective === "string" ? draft.objective.trim() : "";
  return title && objective ? draft : null;
}

export function draftReviewSummary(response?: ChatResponse, draft?: JsonRecord | null): DraftReviewSummary {
  const compact = firstRecord([
    at(response, ["draft_summary", "goal_spec"]),
    at(response, ["draft_summary"]),
    at(response, ["draft", "summary"]),
    at(response, ["drafts", "goal_spec_summary"]),
    at(response, ["drafts", "goal_summary"]),
  ]);
  const reference = stringValue(at(response, ["draft_ref"]))
    || stringValue(at(response, ["draft", "ref"]))
    || stringValue(at(response, ["drafts", "goal_spec_ref"]))
    || stringValue(at(response, ["drafts", "goal_ref"]))
    || stringValue(compact?.ref)
    || stringValue(compact?.id)
    || stringValue(draft?.draft_ref)
    || stringValue(draft?.draft_id)
    || stringValue(draft?.id);
  const title = stringValue(compact?.title) || stringValue(draft?.title) || "Untitled goal draft";
  const objective = stringValue(compact?.objective)
    || stringValue(compact?.description)
    || stringValue(compact?.detail)
    || stringValue(draft?.objective);
  const summary = stringValue(compact?.summary)
    || stringValue(compact?.body)
    || stringValue(at(draft, ["authoring", "intake_summary"]))
    || stringValue(at(draft, ["plan", "summary"]))
    || objective;
  const evidenceCount = countFromCompact(compact?.evidence_count)
    ?? countFromCompact(compact?.acceptance_evidence_count)
    ?? listCount(compact?.acceptance_evidence)
    ?? listCount(compact?.evidence)
    ?? listCount(at(draft, ["authoring", "acceptance_evidence"]))
    ?? 0;
  const constraintCount = countFromCompact(compact?.constraint_count)
    ?? countFromCompact(compact?.constraints_count)
    ?? listCount(compact?.constraints)
    ?? listCount(at(draft, ["authoring", "constraints"]))
    ?? 0;
  return {
    title,
    objective,
    summary,
    reference,
    source: compact ? "Summary" : draft ? "GoalSpec payload" : "Draft response",
    evidenceCount,
    constraintCount,
  };
}

export function updateGoalDraftField(draft: JsonRecord, field: GoalDraftEditField, value: string): JsonRecord {
  if (field === "title" || field === "objective") {
    return { ...draft, [field]: value };
  }
  const authoring = isRecord(draft.authoring) ? draft.authoring : {};
  return {
    ...draft,
    authoring: {
      ...authoring,
      [field]: listFromLines(value),
    },
  };
}

export function goalIdFromSubmitResponse(response: unknown): string {
  for (const path of [
    ["goal_id"],
    ["id"],
    ["action", "goal_id"],
    ["result", "goal_id"],
    ["active_state", "goal_id"],
    ["goal", "id"],
    ["goal", "spec", "id"],
    ["state", "goal", "id"],
    ["result", "id"],
  ]) {
    const candidate = valueAt(response, path);
    if (typeof candidate === "string" && candidate.trim()) {
      return candidate.trim();
    }
  }
  const url = isRecord(response) && typeof response.url === "string" ? response.url : "";
  const nestedUrl = stringValue(at(response, ["result", "url"]));
  return (url || nestedUrl).match(/\/GoalWorkflow\/([0-9a-f-]{36})\/run$/i)?.[1] ?? "";
}

export function draftKindLabel(kind: DraftKind): string {
  if (kind === "ask") return "Answer";
  if (kind === "goal") return "Goal draft";
  if (kind === "search") return "Search request";
  return "Plan draft";
}

function phaseDisplay(phase: PlanPhase): string {
  if (phase === "drafting_plan") return "Draft plan";
  if (phase === "drafting_goals") return "Draft goals";
  return phase
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function sessionDisplayLabel(sessionId: string): string {
  if (sessionId.startsWith("goal:")) {
    return "Selected goal chat";
  }
  if (sessionId === "operator:default") {
    return "Workspace chat";
  }
  return "Chat history";
}

function firstRecord(values: unknown[]): JsonRecord | null {
  for (const value of values) {
    if (isRecord(value)) {
      return value;
    }
  }
  return null;
}

function countFromCompact(value: unknown): number | null {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function listCount(value: unknown): number | null {
  if (Array.isArray(value)) {
    return value.length;
  }
  const lines = linesFromList(value);
  return lines ? lines.split("\n").length : null;
}

function linesFromList(value: unknown): string {
  const rows = Array.isArray(value) ? value : rowsFrom(value);
  return rows
    .map((item) => typeof item === "string" ? item : isRecord(item) ? stringValue(item.title ?? item.value ?? item.summary) : stringValue(item))
    .filter(Boolean)
    .join("\n");
}

function listFromLines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function valueAt(value: unknown, path: string[]): unknown {
  let cursor = value;
  for (const key of path) {
    if (!isRecord(cursor)) {
      return undefined;
    }
    cursor = cursor[key];
  }
  return cursor;
}

function countLabel(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}

function compilerPromptTemplates(goalId: string, goalTitle?: string): Array<{ label: string; icon: "graph" | "control" | "research"; prompt: string }> {
  const goalClause = goalId ? ` for ${goalTitle ? `"${goalTitle}"` : "the selected goal"}` : "";
  return [
    {
      label: "Summarize work",
      icon: "graph",
      prompt: `Summarize current work${goalClause}: running tasks, blocked tasks, human prompts, and the next action.`,
    },
    {
      label: "Plan next step",
      icon: "control",
      prompt: `Plan the next useful step${goalClause}. Include what should happen, why, and what evidence should prove it worked.`,
    },
    {
      label: "Research gap",
      icon: "research",
      prompt: `Find the highest-risk missing information${goalClause} and draft a bounded research request with evidence requirements.`,
    },
  ];
}

function draftModeHeadline(kind: DraftKind): string {
  if (kind === "ask") return "Ask";
  if (kind === "goal") return "Goal draft";
  if (kind === "search") return "Search request";
  return "Plan draft";
}

function draftModeDetail(kind: DraftKind): string {
  if (kind === "ask") return "Ask about the current goal, blockers, evidence, runners, memory, or next step.";
  if (kind === "goal") return "Draft a goal, review it, then submit it.";
  if (kind === "search") return "Draft a sourced search request for a goal or the workspace.";
  return "Draft a plan before turning it into a goal.";
}

function commandPlaceholder(kind: DraftKind): string {
  if (kind === "ask") return "Ask about the selected goal, blockers, evidence, runners, or next action";
  if (kind === "goal") return "Describe the goal, evidence, constraints, and stop conditions";
  if (kind === "search") return "Ask what to search across memory, references, docs, or web";
  return "Describe the outcome, constraints, and review gates";
}

function commandBusyLabel(kind: DraftKind): string {
  if (kind === "ask") return "Thinking";
  if (kind === "search") return "Preparing search request";
  if (kind === "goal") return "Generating goal draft";
  return "Generating plan draft";
}

function chatActivityPayload(props: {
  busy: boolean;
  draftKind: DraftKind;
  selectedGoalId: string;
  sessionId: string;
  mode: string;
  activeDraft?: ActiveDraftState | null;
  latestResponse?: ChatResponse;
  chatRun?: ChatRunTrace;
  goalDraft?: JsonRecord | null;
  goalSubmitResult?: unknown;
}): JsonRecord {
  return {
    label: "operational trace",
    note: "Gateway and backend stages for this chat run.",
    status: props.busy ? "running" : "idle",
    draft_kind: props.draftKind,
    mode: props.mode,
    selected_goal_id: props.selectedGoalId || null,
    session_id: props.sessionId,
    active_draft: props.activeDraft ? {
      kind: props.activeDraft.kind,
      mode: props.activeDraft.mode,
      session_id: props.activeDraft.sessionId,
      selected_goal_id: props.activeDraft.selectedGoalId || null,
      selected_plan_id: props.activeDraft.selectedPlanId || null,
      saved_at: props.activeDraft.savedAt,
      run_id: props.activeDraft.runId,
    } : null,
    run: props.chatRun ?? props.latestResponse?.chat_run ?? null,
    backend: props.latestResponse?.chat_backend ?? props.chatRun?.backend ?? null,
    model_params: props.latestResponse?.model_params ?? props.chatRun?.model_params ?? null,
    chat_log: props.latestResponse?.chat_log ?? props.chatRun?.chat_log ?? null,
    draft_summary: props.latestResponse?.draft_summary ?? null,
    draft_refs: props.latestResponse?.draft_refs ?? null,
    goal_submit: props.goalSubmitResult ?? null,
  };
}
