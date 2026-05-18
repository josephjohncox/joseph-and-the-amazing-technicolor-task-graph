import clsx from "clsx";
import { Bell, Network, ShieldCheck } from "lucide-react";

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
