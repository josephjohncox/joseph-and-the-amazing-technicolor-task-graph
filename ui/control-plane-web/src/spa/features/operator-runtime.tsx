import clsx from "clsx";
import { Bell, ListChecks, Network, ShieldCheck, XCircle } from "lucide-react";

import { Button } from "../components/ui/button";

export type ActiveRuntimeViewModel = {
  stateLabel: string;
  title: string;
  streamStatus: "idle" | "connecting" | "live" | "error";
  streamUpdatedLabel: string;
  streamError: string;
  taskCount: number;
  actionCount: number;
  actionBusy: boolean;
};

export function ActiveGoalRuntimeBar(props: {
  view: ActiveRuntimeViewModel | null;
  onOpenGraph: () => void;
  onOpenQueue: () => void;
  onOpenControls: () => void;
}) {
  if (!props.view) {
    return null;
  }
  const streamTone = props.view.streamStatus === "live"
    ? "status-running"
    : props.view.streamStatus === "error"
      ? "status-failed"
      : "status-pending";
  return (
    <section className="active-runtime-bar" aria-label="Selected goal active state">
      <div className="runtime-state">
        <span className="goal-context-kicker">Live state</span>
        <strong>{props.view.stateLabel}</strong>
        <small>{props.view.title}</small>
      </div>
      <div className="runtime-metrics">
        <span className={clsx("status-pill", streamTone)}>
          {props.view.streamStatus === "live"
            ? "Streaming"
            : props.view.streamStatus === "connecting"
              ? "Connecting"
              : props.view.streamStatus === "error"
                ? "Stream error"
                : "Idle"}
        </span>
        <span className="status-pill muted">{props.view.taskCount} tasks</span>
        <span className={clsx("status-pill", props.view.actionCount ? "status-waiting-approval" : "status-done")}>{props.view.actionCount} actions</span>
        <span className="status-pill muted">{props.view.streamUpdatedLabel}</span>
        {props.view.actionBusy && <span className="status-pill status-running">Submitting draft</span>}
      </div>
      <div className="button-row">
        <button type="button" className="secondary-button" onClick={props.onOpenGraph}>
          <Network size={15} />
          Graph
        </button>
        <button type="button" className={props.view.actionCount ? "primary-button" : "secondary-button"} onClick={props.onOpenQueue}>
          <Bell size={15} />
          Action queue
        </button>
        <button type="button" className="secondary-button" onClick={props.onOpenControls}>
          <ShieldCheck size={15} />
          Goal controls
        </button>
      </div>
      {props.view.streamError && <span className="error-text">{props.view.streamError}</span>}
    </section>
  );
}

export type DraftDockViewModel = {
  title: string;
  detail: string;
  kindLabel: string;
  sessionLabel: string;
  hasGoalDraft: boolean;
  submittedGoalLabel: string;
  busy: boolean;
  errorMessage: string;
};

export function DraftReviewDock(props: {
  view: DraftDockViewModel;
  onEditGoalDraft: () => void;
  onSubmitGoalDraft: () => void;
  onDiscardGoalDraft: () => void;
}) {
  const accepted = Boolean(props.view.submittedGoalLabel);
  return (
    <section className="draft-review-dock" aria-label="Active draft">
      <div>
        <span className="goal-context-kicker">Draft</span>
        <strong>{props.view.title}</strong>
        <small>{props.view.detail}</small>
      </div>
      <div className="draft-summary-meta">
        <span className="status-pill status-runnable">{props.view.kindLabel}</span>
        {props.view.hasGoalDraft && <span className="status-pill status-runnable">Goal draft ready</span>}
        {props.view.sessionLabel && <span className="status-pill muted">{props.view.sessionLabel}</span>}
        {props.view.submittedGoalLabel && <span className="status-pill status-done">Accepted {props.view.submittedGoalLabel}</span>}
      </div>
      <div className="button-row">
        {props.view.hasGoalDraft && (
          <Button type="button" variant="outline" disabled={props.view.busy} onClick={props.onEditGoalDraft}>
            <Bell size={15} />
            Edit draft
          </Button>
        )}
        <Button type="button" variant="outline" disabled={props.view.busy || accepted} onClick={props.onDiscardGoalDraft}>
          <XCircle size={15} />
          Discard draft
        </Button>
        {props.view.hasGoalDraft && (
          <Button type="button" disabled={props.view.busy || accepted} onClick={props.onSubmitGoalDraft}>
            <ListChecks size={15} />
            {accepted ? "Accepted" : props.view.busy ? "Submitting" : "Accept draft"}
          </Button>
        )}
      </div>
      {props.view.errorMessage && <span className="error-text">{props.view.errorMessage}</span>}
    </section>
  );
}
