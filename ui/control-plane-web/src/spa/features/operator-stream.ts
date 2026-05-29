import { useQueryClient } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { at, isRecord } from "../api";
import type { JsonRecord, OperatorGoalDetail } from "../types";
import { stringValue } from "./workbench-format";

export type GoalStreamState = {
  status: "idle" | "connecting" | "live" | "error";
  lastEventAt: string;
  error: string;
};

export function useGoalStateStream(goalId: string, token: string, enabled: boolean): GoalStreamState {
  const queryClient = useQueryClient();
  const [state, setState] = useState<GoalStreamState>({ status: "idle", lastEventAt: "", error: "" });

  useEffect(() => {
    if (!enabled || !goalId) {
      setState({ status: "idle", lastEventAt: "", error: "" });
      return undefined;
    }

    const controller = new AbortController();
    setState((current) => ({ ...current, status: "connecting", error: "" }));

    const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const readStream = async () => {
      let buffer = "";
      const response = await fetch(`/api/operator/stream?goal_id=${encodeURIComponent(goalId)}`, {
        headers: token ? { authorization: `Bearer ${token}` } : undefined,
        signal: controller.signal,
      });
      if (!response.ok || !response.body) {
        throw new Error(`state stream failed with ${response.status}`);
      }
      setState((current) => ({ ...current, status: "live", error: "" }));
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      while (!controller.signal.aborted) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }
        buffer += decoder.decode(value, { stream: true });
        const blocks = buffer.split(/\n\n/);
        buffer = blocks.pop() ?? "";
        for (const block of blocks) {
          const event = sseEventFromBlock(block);
          if (operatorStreamCarriesWorkspace(event.name) && isRecord(event.data)) {
            applyOperatorWorkspaceToCache(queryClient, event.data, goalId);
            setState({ status: "live", lastEventAt: new Date().toISOString(), error: "" });
          } else if (event.name === "stream.heartbeat") {
            setState((current) => ({ ...current, status: "live", lastEventAt: current.lastEventAt || new Date().toISOString(), error: "" }));
          } else if (event.name === "stream.error" || event.name === "error") {
            setState({ status: "error", lastEventAt: new Date().toISOString(), error: stringValue(at(event.data, ["error"])) || "state stream error" });
          } else if (event.name === "stream.done" || event.name === "done") {
            return;
          }
        }
      }
    };

    const run = async () => {
      while (!controller.signal.aborted) {
        try {
          await readStream();
          if (!controller.signal.aborted) {
            setState((current) => ({ ...current, status: "connecting", error: "" }));
            await wait(1_500);
          }
        } catch (error) {
          if (controller.signal.aborted) {
            return;
          }
          setState({ status: "error", lastEventAt: new Date().toISOString(), error: error instanceof Error ? error.message : String(error) });
          await wait(3_000);
        }
      }
    };

    run().catch((error) => {
      if (!controller.signal.aborted) {
        setState({ status: "error", lastEventAt: new Date().toISOString(), error: error instanceof Error ? error.message : String(error) });
      }
    });

    return () => controller.abort();
  }, [enabled, goalId, queryClient, token]);

  return state;
}

function operatorStreamCarriesWorkspace(eventName: string): boolean {
  return [
    "message",
    "workspace.updated",
    "goal.updated",
    "task.updated",
    "worker.started",
    "worker.output",
    "worker.completed",
    "thunk.created",
    "approval.requested",
    "action.required",
    "evidence.added",
    "review.completed",
    "goal.satisfied",
    "goal.cancelled",
  ].includes(eventName);
}

function applyOperatorWorkspaceToCache(queryClient: QueryClient, workspace: JsonRecord, fallbackGoalId = ""): void {
  const goalId = stringValue(workspace.selected_goal_id) || fallbackGoalId;
  queryClient.setQueryData(["operator-workspace", goalId], workspace);

  if (Array.isArray(workspace.goals)) {
    queryClient.setQueryData(["goals"], (current: unknown) => ({
      ...(isRecord(current) ? current : {}),
      generated_at: stringValue(workspace.generated_at) || new Date().toISOString(),
      goals: workspace.goals,
      source: {
        stream: true,
        event: "operator_projection",
      },
    }));
  }

  const selectedGoal = at(workspace, ["selected_goal"]);
  if (goalId && isRecord(selectedGoal)) {
    queryClient.setQueryData(["operator-goal", goalId], selectedGoal as OperatorGoalDetail);
  }

  if (Array.isArray(workspace.actions)) {
    const actionsEnvelope = {
      generated_at: stringValue(workspace.generated_at) || new Date().toISOString(),
      actions: workspace.actions,
      source: {
        stream: true,
        event: "operator_projection",
      },
    };
    queryClient.setQueryData(["operator-actions", goalId], actionsEnvelope);
    queryClient.setQueryData(["operator-actions"], actionsEnvelope);
  }
}

function sseEventFromBlock(block: string): { name: string; data: unknown } {
  const lines = block.split(/\r?\n/);
  const name = lines.find((line) => line.startsWith("event:"))?.slice("event:".length).trim() || "message";
  const data = lines
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice("data:".length).trimStart())
    .join("\n");
  return { name, data: data ? safeJsonValue(data) : null };
}

function safeJsonValue(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}
