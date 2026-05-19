/**
 * User-facing COAT task graph manager SPA.
 *
 * Purpose: present goals, task graph state, shared memory, human feedback, and
 * runner capacity through product-facing workflows while keeping durable
 * authority in the Rust/Restate backend.
 *
 * Architecture reference: docs/design-docs/110-control-gateway-spa.md
 */
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import {
  Bell,
  Brain,
  GitBranch,
  ListChecks,
  Monitor,
  Moon,
  Network,
  RefreshCw,
  Route,
  Server,
  ShieldCheck,
  Sun,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  at,
  acceptPlanDraft,
  authToken,
  cancelGoal,
  chat,
  chatRun,
  chatSession,
  isRecord,
  operatorGoalDetail,
  operatorGoals,
  operatorWorkspace,
  planActions,
  plans,
  resolvePlanAction,
  rowsFrom,
  setAuthToken,
  submitOperatorGoal,
} from "./api";
import type { ChatMessage, ChatRunTrace, GoalRow, JsonRecord } from "./types";
import {
  ChatDraftPanel,
  goalDraftFromChatResponse,
  goalIdFromSubmitResponse,
  modeForDraftKind,
  updateGoalDraftField,
  type ActiveDraftState,
  type DraftKind,
  type GoalDraftEditField,
} from "./features/chat-draft-panel";
import {
  PlanContextBar,
  PlanPhaseRail,
  PlanRouteView,
  defaultWorkspacePlan,
  derivePlanPhase,
  draftIdFor,
  planDraftFromChatResponse,
  planSummariesFromRows,
  selectedPlanFromSummaries,
  workspacePlanId,
  type AcceptedPlanDraft,
  type PlanSummary,
  type StagedGoalDraft,
} from "./features/plan-workflow";
import { ActiveGoalRuntimeBar, type ActiveRuntimeViewModel } from "./features/operator-runtime";
import {
  GoalContextBar,
  GoalsView,
  composedSnapshotFromOperatorGoalDetail,
  composedSnapshotHasProjectedTasks,
  goalRowsWithSelected,
  mergeSubmittedGoalRows,
  selectedGoalSummary,
  taskRowsFromComposedSnapshot,
  taskStatusCounts,
  type GoalSummary,
  type SubmittedGoalDraft,
} from "./features/goal-graph-panel";
import {
  actionNeededItemsFromComposedSnapshot,
  applyActionEnvelopeToCache,
  continuationRowsFromComposedSnapshot,
  nextActionSummary,
} from "./features/operator-action-panels";
import { Dashboard, HumanQueueView, RunnersView, ServiceStrip } from "./features/operator-dashboard-routes";
import { CompilerControlView } from "./features/operator-control-panel";
import { useGoalStateStream } from "./features/operator-stream";
import { TaskGraphView } from "./features/task-graph-view";
import { MemoryView } from "./features/memory-view";
import {
  createRunId,
} from "./features/workbench-format";

type ViewKey = "dashboard" | "goals" | "graph" | "control" | "memory" | "plans" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";

const themeStorageKey = "coat.theme";
const selectedGoalStorageKey = "coat.selectedGoalId";
const selectedPlanStorageKey = "coat.selectedPlanId";
const themeColors: Record<ResolvedTheme, string> = {
  light: "#f5f6f4",
  dark: "#080c0f",
};
const views: Array<{ key: ViewKey; label: string; icon: typeof Route }> = [
  { key: "dashboard", label: "Dashboard", icon: Route },
  { key: "plans", label: "Plans", icon: GitBranch },
  { key: "goals", label: "Goals", icon: ListChecks },
  { key: "graph", label: "Work Graph", icon: Network },
  { key: "human", label: "Actions", icon: Bell },
  { key: "control", label: "Steer", icon: ShieldCheck },
  { key: "memory", label: "Memory", icon: Brain },
  { key: "runners", label: "Runners", icon: Server },
];

const starterMessages: ChatMessage[] = [
  {
    role: "assistant",
    content: "Ask about the current plan, draft the plan, or draft a goal to accept into it.",
  },
];

export function App() {
  const queryClient = useQueryClient();
  const [activeView, setActiveView] = useState<ViewKey>("dashboard");
  const [selectedGoalId, setSelectedGoalId] = useState(() => initialSelectedGoalId());
  const [selectedPlanId, setSelectedPlanId] = useState(() => initialSelectedPlanId());
  const [goalPickerOpen, setGoalPickerOpen] = useState(false);
  const [planPickerOpen, setPlanPickerOpen] = useState(false);
  const [submittedGoalDrafts, setSubmittedGoalDrafts] = useState<Record<string, SubmittedGoalDraft>>({});
  const [acceptedPlanDraft, setAcceptedPlanDraft] = useState<AcceptedPlanDraft | null>(null);
  const [stagedGoalDrafts, setStagedGoalDrafts] = useState<Record<string, StagedGoalDraft>>({});
  const [token, setToken] = useState(authToken());
  const [sessionMessages, setSessionMessages] = useState<Record<string, ChatMessage[]>>({});
  const [activeDraft, setActiveDraft] = useState<ActiveDraftState | null>(null);
  const [chatInput, setChatInput] = useState("");
  const [draftKind, setDraftKind] = useState<DraftKind>("ask");
  const [activeChatRunId, setActiveChatRunId] = useState<string | null>(null);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => initialThemePreference());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(initialThemePreference()));

  const goalsQuery = useQuery({ queryKey: ["goals"], queryFn: operatorGoals });
  const plansQuery = useQuery({ queryKey: ["plans"], queryFn: plans });
  const operatorWorkspaceQuery = useQuery({
    queryKey: ["operator-workspace", selectedGoalId],
    queryFn: () => operatorWorkspace(selectedGoalId || undefined),
    refetchInterval: 5_000,
  });
  const selectedGoalQuery = useQuery({
    queryKey: ["operator-goal", selectedGoalId],
    queryFn: () => operatorGoalDetail(selectedGoalId),
    enabled: Boolean(selectedGoalId),
    refetchInterval: () => {
      if (!selectedGoalId) {
        return false;
      }
      const pending = submittedGoalDrafts[selectedGoalId];
      if (!pending) {
        return 10_000;
      }
      if (pending.projected || Date.now() - pending.submittedAt > 30_000) {
        return 10_000;
      }
      return 1_000;
    },
  });
  const operatorWorkspaceData = operatorWorkspaceQuery.data;
  const projectedGoalRows = useMemo(() => {
    const operatorRows = rowsFrom(operatorWorkspaceData?.goals);
    return (operatorRows.length ? operatorRows : rowsFrom(at(goalsQuery.data, ["data"]) ?? goalsQuery.data)) as GoalRow[];
  }, [goalsQuery.data, operatorWorkspaceData?.goals]);
  const goalRows = useMemo(() => mergeSubmittedGoalRows(projectedGoalRows, submittedGoalDrafts), [projectedGoalRows, submittedGoalDrafts]);
  const currentGoalDetail = selectedGoalQuery.data;
  const currentGoal = composedSnapshotFromOperatorGoalDetail(currentGoalDetail);
  const selectedSubmittedDraft = selectedGoalId ? submittedGoalDrafts[selectedGoalId]?.draft ?? null : null;
  const selectedGoal = useMemo(() => selectedGoalSummary(selectedGoalId, goalRows, currentGoal, selectedSubmittedDraft), [currentGoal, goalRows, selectedGoalId, selectedSubmittedDraft]);
  const planRows = useMemo(() => rowsFrom(at(plansQuery.data, ["data"]) ?? plansQuery.data), [plansQuery.data]);
  const projectedPlans = useMemo(() => planSummariesFromRows(planRows), [planRows]);
  const fallbackPlan = useMemo(() => defaultWorkspacePlan(goalRows.length, selectedGoal?.title), [goalRows.length, selectedGoal?.title]);
  const selectedPlan = useMemo(() => selectedPlanFromSummaries(projectedPlans, selectedPlanId, fallbackPlan), [fallbackPlan, projectedPlans, selectedPlanId]);
  const selectablePlans = useMemo<PlanSummary[]>(() => {
    const rows = projectedPlans.length ? projectedPlans : [fallbackPlan];
    return rows.some((plan) => plan.id === selectedPlan.id) ? rows : [selectedPlan, ...rows];
  }, [fallbackPlan, projectedPlans, selectedPlan]);
  const chatSessionId = selectedPlan ? `plan:${selectedPlan.id}` : selectedGoalId ? `goal:${selectedGoalId}` : "operator:default";
  const activeDraftForSession = activeDraft?.sessionId === chatSessionId ? activeDraft : null;
  const visibleActiveDraft = activeDraftForSession ?? activeDraft;
  const chatSessionQuery = useQuery({
    queryKey: ["chat-session", chatSessionId],
    queryFn: () => chatSession(chatSessionId),
  });
  const messages = sessionMessages[chatSessionId] ?? starterMessages;
  const goalStream = useGoalStateStream(selectedGoalId, token, Boolean(selectedGoalId));
  const selectGoalId = useCallback((goalId: string) => {
    const nextGoalId = goalId.trim();
    setSelectedGoalId(nextGoalId);
    persistSelectedGoalId(nextGoalId);
  }, []);
  const selectedGoalCancel = useMutation({
    mutationFn: async (goalId: string) => cancelGoal(goalId, "Operator cancelled the selected goal from the current-goal control."),
    onSuccess: (response, goalId) => {
      applyActionEnvelopeToCache(queryClient, response, goalId);
      void queryClient.refetchQueries({ queryKey: ["operator-goal", goalId] });
      void queryClient.refetchQueries({ queryKey: ["goals"] });
    },
  });

  const sendChat = useMutation({
    mutationFn: async (overrideContent?: string) => {
      const content = (overrideContent ?? chatInput).trim();
      if (!content) {
        throw new Error("Write a request first.");
      }
      const requestSessionId = chatSessionId;
      const requestGoalId = selectedGoalId;
      const requestPlanId = selectedPlan.id === workspacePlanId ? "" : selectedPlan.id;
      const requestKind = draftKind;
      const requestMode = modeForDraftKind(requestKind);
      const currentMessages = sessionMessages[requestSessionId] ?? messages;
      const nextMessages = [...currentMessages, { role: "user" as const, content }];
      setSessionMessages((current) => ({ ...current, [requestSessionId]: nextMessages }));
      setChatInput("");
      const runId = createRunId();
      setActiveChatRunId(runId);
      const response = await chat(requestSessionId, requestMode, requestGoalId, content, runId, requestPlanId);
      const goalDraft = goalDraftFromChatResponse(response);
      const planDraft = planDraftFromChatResponse(response);
      setActiveChatRunId(response.run_id ?? runId);
      setSessionMessages((current) => ({
        ...current,
        [requestSessionId]: [...nextMessages, { role: "assistant" as const, content: response.assistant ?? "Response pending." }],
      }));
      if (requestKind === "ask" && !goalDraft && !planDraft) {
        setActiveDraft(null);
      } else {
        setActiveDraft({
          kind: requestKind,
          mode: requestMode,
          sessionId: requestSessionId,
          selectedPlanId: requestPlanId || selectedPlan.id,
          selectedGoalId: requestGoalId,
          savedAt: new Date().toISOString(),
          response,
          goalDraft,
          planDraft,
          runId: response.run_id ?? runId,
        });
      }
      void queryClient.invalidateQueries({ queryKey: ["chat-session", requestSessionId] });
      return response;
    },
  });
  const chatRunQuery = useQuery({
    queryKey: ["chat-run", activeChatRunId],
    queryFn: () => chatRun(activeChatRunId ?? ""),
    enabled: Boolean(activeChatRunId && sendChat.isPending),
    refetchInterval: sendChat.isPending ? 750 : false,
  });
  const latestResponse = visibleActiveDraft?.response;
  const latestGoalDraft = visibleActiveDraft?.goalDraft ?? null;
  const latestPlanDraft = visibleActiveDraft?.planDraft ?? null;
  const submitGoalDraft = useMutation({
    mutationFn: async (draftOverride?: JsonRecord) => {
      const draft = draftOverride ?? latestGoalDraft;
      if (!draft) {
        throw new Error("Generate a goal draft first.");
      }
      const response = await submitOperatorGoal(draft);
      assertGoalSubmitReachedCoordinator(response);
      return { response, draft };
    },
    onSuccess: (result) => {
      const goalId = goalIdFromSubmitResponse(result.response);
      if (goalId) {
        applyActionEnvelopeToCache(queryClient, result.response, goalId);
        setSubmittedGoalDrafts((current) => ({
          ...current,
          [goalId]: {
            draft: result.draft,
            submittedAt: Date.now(),
            projected: false,
          },
        }));
        selectGoalId(goalId);
        setActiveView("graph");
        setActiveDraft(null);
        setStagedGoalDrafts((current) => {
          const next = { ...current };
          for (const [draftId, staged] of Object.entries(next)) {
            if (staged.draft === result.draft) {
              delete next[draftId];
            }
          }
          return next;
        });
        void queryClient.invalidateQueries({ queryKey: ["operator-goal", goalId] });
        void queryClient.refetchQueries({ queryKey: ["operator-goal", goalId] });
      }
      void queryClient.invalidateQueries({ queryKey: ["goals"] });
      void queryClient.refetchQueries({ queryKey: ["goals"] });
    },
  });

  const acceptPlanDraftMutation = useMutation({
    mutationFn: async (draft: JsonRecord) => {
      const draftId = stringFrom(draft.draft_id) || draftIdFor("plan", draft);
      const isWorkspacePlan = selectedPlan.source === "workspace" || selectedPlan.id === workspacePlanId;
      const response = await acceptPlanDraft(draftId, {
        create_new: isWorkspacePlan,
        plan_id: isWorkspacePlan ? undefined : selectedPlan.id,
        status: "approved",
        operator: "operator",
        summary: "Operator accepted plan draft from the plan workspace.",
      });
      return { response, draft, draftId };
    },
    onSuccess: ({ response, draft, draftId }) => {
      const acceptedPlanId = planIdFromAcceptResponse(response) || selectedPlan.id;
      setAcceptedPlanDraft({
        draftId,
        draft,
        acceptedAt: new Date().toISOString(),
        sourceSessionId: chatSessionId,
      });
      if (acceptedPlanId && acceptedPlanId !== workspacePlanId) {
        setSelectedPlanId(acceptedPlanId);
        persistSelectedPlanId(acceptedPlanId);
      }
      setActiveDraft(null);
      setDraftKind("goal");
      setActiveView("plans");
      void queryClient.invalidateQueries({ queryKey: ["plans"] });
      void queryClient.invalidateQueries({ queryKey: ["plan-actions"] });
      void queryClient.invalidateQueries({ queryKey: ["operator-workspace"] });
      void queryClient.invalidateQueries({ queryKey: ["chat-session", chatSessionId] });
    },
  });

  const selectedPlanActionsQuery = useQuery({
    queryKey: ["plan-actions", selectedPlan.id],
    queryFn: () => planActions(selectedPlan.id),
    enabled: Boolean(selectedPlan.id && selectedPlan.id !== workspacePlanId),
    refetchInterval: 5_000,
  });
  const selectedPlanActions = useMemo(() => {
    if (!selectedPlanActionsQuery.data) {
      return [];
    }
    return rowsFrom(
      at(selectedPlanActionsQuery.data, ["actions"])
      ?? at(selectedPlanActionsQuery.data, ["data"])
      ?? selectedPlanActionsQuery.data,
    ) as JsonRecord[];
  }, [selectedPlanActionsQuery.data]);

  const resolveSelectedPlanAction = useMutation({
    mutationFn: async ({ actionId, resolution }: { actionId: string; resolution: string }) => {
      if (resolution === "draft_goal") {
        return { ok: true, local: true, resolution };
      }
      if (resolution === "accept_plan_draft" && latestPlanDraft) {
        return acceptPlanDraftMutation.mutateAsync(latestPlanDraft);
      }
      return resolvePlanAction(selectedPlan.id, actionId, {
        resolution,
        operator: "operator",
        response_summary: `Operator selected ${resolution}.`,
      });
    },
    onSuccess: (response, variables) => {
      if (variables.resolution === "draft_goal") {
        setDraftPhase("goal", "plans");
      }
      void queryClient.invalidateQueries({ queryKey: ["plans"] });
      void queryClient.invalidateQueries({ queryKey: ["plan-actions", selectedPlan.id] });
      void queryClient.invalidateQueries({ queryKey: ["operator-workspace"] });
      void queryClient.invalidateQueries({ queryKey: ["goals"] });
      const goalId = goalIdFromSubmitResponse(response);
      if (goalId) {
        selectGoalId(goalId);
        setActiveView("graph");
      }
    },
  });

  const refreshAll = () => {
    void queryClient.invalidateQueries();
  };
  const sendChatFromPanel = (content?: string) => {
    submitGoalDraft.reset();
    sendChat.mutate(content);
  };
  const discardActiveGoalDraft = () => {
    setActiveDraft(null);
    sendChat.reset();
    submitGoalDraft.reset();
  };
  const acceptActivePlanDraft = () => {
    if (!latestPlanDraft) {
      return;
    }
    acceptPlanDraftMutation.mutate(latestPlanDraft);
  };
  const acceptGoalDraftIntoPlan = () => {
    if (!latestGoalDraft) {
      return;
    }
    const draftId = draftIdFor("goal", latestGoalDraft);
    setStagedGoalDrafts((current) => ({
      ...current,
      [draftId]: {
        draftId,
        draft: latestGoalDraft,
        acceptedAt: new Date().toISOString(),
        sourcePlanId: selectedPlan.id,
        sourceGoalId: selectedGoalId || undefined,
      },
    }));
    setActiveDraft(null);
    setDraftKind("goal");
    setActiveView("plans");
  };
  const discardStagedGoalDraft = (draftId: string) => {
    setStagedGoalDrafts((current) => {
      const next = { ...current };
      delete next[draftId];
      return next;
    });
  };
  const updateActiveGoalDraftField = (field: GoalDraftEditField, value: string) => {
    setActiveDraft((current) => {
      if (!current?.goalDraft) {
        return current;
      }
      const goalDraft = updateGoalDraftField(current.goalDraft, field, value);
      return {
        ...current,
        savedAt: new Date().toISOString(),
        goalDraft,
        response: {
          ...current.response,
          drafts: {
            ...(isRecord(current.response.drafts) ? current.response.drafts : {}),
            goal_spec: goalDraft,
          },
        },
      };
    });
    submitGoalDraft.reset();
  };

  useEffect(() => {
    persistSelectedGoalId(selectedGoalId);
  }, [selectedGoalId]);

  useEffect(() => {
    persistSelectedPlanId(selectedPlanId);
  }, [selectedPlanId]);

  useEffect(() => {
    const handlePopState = () => {
      setSelectedGoalId(initialSelectedGoalId());
      setSelectedPlanId(initialSelectedPlanId());
    };
    window.addEventListener("popstate", handlePopState);
    return () => window.removeEventListener("popstate", handlePopState);
  }, []);

  useEffect(() => {
    const applyTheme = () => {
      const resolved = resolveTheme(themePreference);
      setResolvedTheme(resolved);
      document.documentElement.dataset.theme = resolved;
      document.documentElement.dataset.themePreference = themePreference;
      document.documentElement.style.colorScheme = resolved;
      document.querySelector('meta[name="theme-color"]')?.setAttribute("content", themeColors[resolved]);
      try {
        window.localStorage.setItem(themeStorageKey, themePreference);
      } catch {
        // Browsers may block storage in hardened contexts; the live document theme still applies.
      }
    };

    applyTheme();
    if (themePreference !== "system") {
      return undefined;
    }
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    query.addEventListener("change", applyTheme);
    return () => query.removeEventListener("change", applyTheme);
  }, [themePreference]);

  useEffect(() => {
    setSessionMessages((current) => current[chatSessionId] ? current : { ...current, [chatSessionId]: starterMessages });
  }, [chatSessionId]);

  useEffect(() => {
    const persistedMessages = chatSessionQuery.data?.messages ?? [];
    if (!persistedMessages.length) {
      return;
    }
    setSessionMessages((current) => {
      if (sameMessages(current[chatSessionId], persistedMessages)) {
        return current;
      }
      return { ...current, [chatSessionId]: persistedMessages };
    });
  }, [chatSessionId, chatSessionQuery.data, chatSessionQuery.dataUpdatedAt]);

  useEffect(() => {
    if (!selectedGoalId || !submittedGoalDrafts[selectedGoalId] || !composedSnapshotHasProjectedTasks(composedSnapshotFromOperatorGoalDetail(selectedGoalQuery.data))) {
      return;
    }
    setSubmittedGoalDrafts((current) => {
      const next = { ...current };
      const pending = next[selectedGoalId];
      if (pending) {
        next[selectedGoalId] = { ...pending, projected: true };
      }
      return next;
    });
    void queryClient.invalidateQueries({ queryKey: ["goals"] });
    void queryClient.refetchQueries({ queryKey: ["goals"] });
  }, [queryClient, selectedGoalId, selectedGoalQuery.data, submittedGoalDrafts]);

  const saveToken = (value: string) => {
    setToken(value);
    setAuthToken(value);
    refreshAll();
  };

  const selectableGoals = useMemo(() => goalRowsWithSelected(goalRows, selectedGoal), [goalRows, selectedGoal]);
  const serviceRows = operatorWorkspaceData?.services ?? [];
  const operatorActionCount = operatorWorkspaceData?.actions?.length ?? 0;
  const planActionCount = selectedPlanActions.length + operatorActionCount;
  const stagedGoalDraftList = useMemo(() => Object.values(stagedGoalDrafts), [stagedGoalDrafts]);
  const currentPlanPhase = derivePlanPhase({
    selectedPlan,
    selectedGoalStatus: selectedGoal?.status,
    activeDraftKind: visibleActiveDraft?.kind,
    hasPlanDraft: Boolean(latestPlanDraft),
    hasAcceptedPlanDraft: Boolean(acceptedPlanDraft),
    hasGoalDraft: Boolean(latestGoalDraft),
    stagedGoalCount: stagedGoalDraftList.length,
    actionCount: operatorActionCount,
  });
  const setDraftPhase = (kind: DraftKind, view: ViewKey = "plans") => {
    setDraftKind(kind);
    setActiveView(view);
  };
  const activeRuntimeView = useMemo<ActiveRuntimeViewModel | null>(() => {
    if (!selectedGoal) {
      return null;
    }
    const snapshot = currentGoal ?? { goal_id: selectedGoal.id };
    const tasks = taskRowsFromComposedSnapshot(snapshot);
    const counts = taskStatusCounts(tasks);
    const actions = actionNeededItemsFromComposedSnapshot(currentGoal, selectedGoal.id);
    const state = nextActionSummary(counts, tasks.length, snapshot);
    return {
      stateLabel: state.stateLabel,
      title: state.title,
      streamStatus: goalStream.status,
      streamUpdatedLabel: goalStream.lastEventAt ? `Updated ${timeLabel(goalStream.lastEventAt)}` : "Projection pending",
      streamError: goalStream.error,
      taskCount: tasks.length,
      actionCount: actions.length,
      actionBusy: submitGoalDraft.isPending,
    };
  }, [currentGoal, goalStream.error, goalStream.lastEventAt, goalStream.status, selectedGoal, submitGoalDraft.isPending]);
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <img
            className="brand-mark"
            src="/brand/coat-logo.png"
            width="50"
            height="50"
            alt="Joseph and the Amazing Technicolor Task Graph"
          />
          <div>
            <strong>Task Graph Manager</strong>
            <span>Joseph and the Amazing Technicolor Task Graph</span>
          </div>
        </div>
        <nav className="nav-list" aria-label="Primary">
          {views.map((view) => {
            const Icon = view.icon;
            return (
              <button
                key={view.key}
                type="button"
                className={clsx("nav-item", activeView === view.key && "active")}
                onClick={() => setActiveView(view.key)}
              >
                <Icon size={17} />
                {view.label}
              </button>
            );
          })}
        </nav>
        <div className="sidebar-footer">
          <label>
            Gateway token
            <input
              type="password"
              value={token}
              onChange={(event) => saveToken(event.target.value)}
              placeholder="optional bearer token"
            />
          </label>
          <button type="button" className="secondary-button" onClick={refreshAll}>
            <RefreshCw size={16} />
            Refresh
          </button>
          <ThemeSwitcher preference={themePreference} resolved={resolvedTheme} onChange={setThemePreference} />
        </div>
      </aside>

      <main className="workspace">
        <header className="topbar">
          <div className="topbar-title">
            <p className="eyebrow">Workspace</p>
            <h1>{titleFor(activeView)}</h1>
          </div>
          <div className="topbar-context-stack">
            <PlanContextBar
              plans={selectablePlans}
              selectedPlan={selectedPlan}
              selectedPlanId={selectedPlan.id}
              open={planPickerOpen}
              loading={plansQuery.isFetching}
              onOpenChange={setPlanPickerOpen}
              onSelectPlan={(planId) => {
                setSelectedPlanId(planId);
                persistSelectedPlanId(planId);
              }}
              onRefreshPlans={() => void queryClient.invalidateQueries({ queryKey: ["plans"] })}
              onOpenPlans={() => {
                setPlanPickerOpen(false);
                setActiveView("plans");
              }}
            />
            <GoalContextBar
              goals={selectableGoals}
              selectedGoal={selectedGoal}
              selectedGoalId={selectedGoalId}
              open={goalPickerOpen}
              loading={goalsQuery.isFetching || selectedGoalQuery.isFetching}
              cancelBusy={selectedGoalCancel.isPending}
              cancelError={selectedGoalCancel.error as Error | null}
              onOpenChange={setGoalPickerOpen}
              onSelectGoal={selectGoalId}
              onCancelGoal={() => selectedGoalId && selectedGoalCancel.mutate(selectedGoalId)}
              onRefreshGoals={() => {
                void queryClient.invalidateQueries({ queryKey: ["goals"] });
                void queryClient.invalidateQueries({ queryKey: ["operator-workspace"] });
              }}
              onOpenGraph={() => selectedGoalId && setActiveView("graph")}
            />
          </div>
          <ServiceStrip services={serviceRows} />
        </header>
        <PlanPhaseRail
          phase={currentPlanPhase}
          selectedPlan={selectedPlan}
          selectedGoalTitle={selectedGoal?.title}
          actionCount={planActionCount}
          stagedGoalCount={stagedGoalDraftList.length}
          hasPlanDraft={Boolean(latestPlanDraft)}
          hasGoalDraft={Boolean(latestGoalDraft)}
          onAsk={() => setDraftPhase("ask", "dashboard")}
          onDraftPlan={() => setDraftPhase("plan", "plans")}
          onDraftGoal={() => setDraftPhase("goal", "plans")}
          onReviewAccept={() => setActiveView("plans")}
          onOpenActions={() => setActiveView("human")}
        />
        <ActiveGoalRuntimeBar
          view={activeRuntimeView}
          onOpenGraph={() => setActiveView("graph")}
          onOpenQueue={() => setActiveView("human")}
          onOpenControls={() => setActiveView("control")}
        />
        <section className="content-grid">
          {activeView === "dashboard" && (
            <Dashboard
              workspace={operatorWorkspaceData}
              goals={goalRows}
              selectedGoalId={selectedGoalId}
              onSelectGoal={(goalId) => {
                selectGoalId(goalId);
                setActiveView("graph");
              }}
            />
          )}
          {activeView === "goals" && (
            <GoalsView
              goals={goalRows}
              selectedGoalId={selectedGoalId}
              onSelectGoal={(goalId) => {
                selectGoalId(goalId);
                setActiveView("graph");
              }}
            />
          )}
          {activeView === "graph" && (
            <TaskGraphView
              goalId={selectedGoalId}
              snapshot={currentGoal}
              submittedDraft={selectedSubmittedDraft}
              loading={selectedGoalQuery.isFetching}
              onOpenGoalPicker={() => setGoalPickerOpen(true)}
              onOpenControls={() => setActiveView("control")}
            />
          )}
          {activeView === "control" && (
            <CompilerControlView goalId={selectedGoalId} snapshot={currentGoal} loading={selectedGoalQuery.isFetching} onOpenGoalPicker={() => setGoalPickerOpen(true)} />
          )}
          {activeView === "memory" && <MemoryView selectedGoalId={selectedGoalId} />}
          {activeView === "plans" && (
            <PlanRouteView
              plans={selectablePlans}
              selectedPlan={selectedPlan}
              phase={currentPlanPhase}
              selectedGoalTitle={selectedGoal?.title}
              planDraft={latestPlanDraft}
              goalDraft={latestGoalDraft}
              acceptedPlanDraft={acceptedPlanDraft}
              stagedGoalDrafts={stagedGoalDraftList}
              planActions={selectedPlanActions}
              actionBusy={resolveSelectedPlanAction.isPending || acceptPlanDraftMutation.isPending}
              onSelectPlan={(planId) => {
                setSelectedPlanId(planId);
                persistSelectedPlanId(planId);
              }}
              onDraftPlan={() => setDraftPhase("plan", "plans")}
              onDraftGoal={() => setDraftPhase("goal", "plans")}
              onAcceptPlanDraft={acceptActivePlanDraft}
              onDiscardPlanDraft={discardActiveGoalDraft}
              onAcceptGoalIntoPlan={acceptGoalDraftIntoPlan}
              onSubmitGoalDraft={() => submitGoalDraft.mutate(latestGoalDraft ?? undefined)}
              onDiscardGoalDraft={discardActiveGoalDraft}
              onSubmitStagedGoal={(draftId) => {
                const staged = stagedGoalDrafts[draftId];
                if (staged) {
                  submitGoalDraft.mutate(staged.draft);
                }
              }}
              onDiscardStagedGoal={discardStagedGoalDraft}
              onResolvePlanAction={(actionId, resolution) => resolveSelectedPlanAction.mutate({ actionId, resolution })}
              onEditDraft={() => setActiveView("dashboard")}
            />
          )}
          {activeView === "human" && <HumanQueueView selectedGoalId={selectedGoalId} workspace={operatorWorkspaceData} />}
          {activeView === "runners" && <RunnersView workspace={operatorWorkspaceData} />}
          <ChatDraftPanel
            messages={messages}
            input={chatInput}
            draftKind={draftKind}
            busy={sendChat.isPending}
            error={sendChat.error}
            activeDraft={visibleActiveDraft}
            latestResponse={latestResponse}
            chatRun={(chatRunQuery.data ?? latestResponse?.chat_run) as ChatRunTrace | undefined}
            goalDraft={latestGoalDraft}
            planDraft={latestPlanDraft}
            goalSubmitBusy={submitGoalDraft.isPending}
            goalSubmitError={submitGoalDraft.error as Error | null}
            goalSubmitResult={submitGoalDraft.data?.response}
            selectedPlanId={selectedPlan.id}
            selectedPlan={{ title: selectedPlan.title, phase: currentPlanPhase }}
            selectedGoalId={selectedGoalId}
            selectedGoal={selectedGoal}
            sessionId={chatSessionId}
            mode={modeForDraftKind(draftKind)}
            onDraftKindChange={setDraftKind}
            onInputChange={setChatInput}
            onSend={sendChatFromPanel}
            onSubmitGoalDraft={() => submitGoalDraft.mutate(latestGoalDraft ?? undefined)}
            onAcceptGoalIntoPlan={acceptGoalDraftIntoPlan}
            onDiscardGoalDraft={discardActiveGoalDraft}
            onAcceptPlanDraft={acceptActivePlanDraft}
            onDiscardPlanDraft={discardActiveGoalDraft}
            onUpdateGoalDraftField={updateActiveGoalDraftField}
            onClear={() => {
              setSessionMessages((current) => ({ ...current, [chatSessionId]: starterMessages }));
              setChatInput("");
            }}
          />
        </section>
      </main>
    </div>
  );
}

function timeLabel(iso: string): string {
  const elapsed = Date.now() - Date.parse(iso);
  if (!Number.isFinite(elapsed) || elapsed < 0) {
    return "now";
  }
  if (elapsed < 2_000) {
    return "now";
  }
  if (elapsed < 60_000) {
    return `${Math.round(elapsed / 1_000)}s ago`;
  }
  return `${Math.round(elapsed / 60_000)}m ago`;
}

function sameMessages(left: ChatMessage[] | undefined, right: ChatMessage[]): boolean {
  if (!left || left.length !== right.length) {
    return false;
  }
  return left.every((message, index) => message.role === right[index]?.role && message.content === right[index]?.content);
}

function selectedGoalIdFromLocation(search: string, storedGoalId?: string | null): string {
  const urlGoalId = new URLSearchParams(search).get("goal")?.trim();
  if (urlGoalId) {
    return urlGoalId;
  }
  return storedGoalId?.trim() ?? "";
}

function initialSelectedGoalId(): string {
  if (typeof window === "undefined") {
    return "";
  }
  let storedGoalId = "";
  try {
    storedGoalId = window.localStorage.getItem(selectedGoalStorageKey) ?? "";
  } catch {
    storedGoalId = "";
  }
  return selectedGoalIdFromLocation(window.location.search, storedGoalId);
}

function selectedPlanIdFromLocation(search: string, storedPlanId?: string | null): string {
  const urlPlanId = new URLSearchParams(search).get("plan")?.trim();
  if (urlPlanId) {
    return urlPlanId;
  }
  return storedPlanId?.trim() || workspacePlanId;
}

function initialSelectedPlanId(): string {
  if (typeof window === "undefined") {
    return workspacePlanId;
  }
  let storedPlanId = workspacePlanId;
  try {
    storedPlanId = window.localStorage.getItem(selectedPlanStorageKey) ?? workspacePlanId;
  } catch {
    storedPlanId = workspacePlanId;
  }
  return selectedPlanIdFromLocation(window.location.search, storedPlanId);
}

function persistSelectedGoalId(goalId: string): void {
  if (typeof window === "undefined") {
    return;
  }
  const trimmed = goalId.trim();
  try {
    if (trimmed) {
      window.localStorage.setItem(selectedGoalStorageKey, trimmed);
    } else {
      window.localStorage.removeItem(selectedGoalStorageKey);
    }
  } catch {
    // Local persistence is a convenience; URL state remains the shareable selector.
  }
  const url = new URL(window.location.href);
  if (trimmed) {
    url.searchParams.set("goal", trimmed);
  } else {
    url.searchParams.delete("goal");
  }
  if (`${url.pathname}${url.search}${url.hash}` !== `${window.location.pathname}${window.location.search}${window.location.hash}`) {
    window.history.replaceState({}, "", url);
  }
}

function persistSelectedPlanId(planId: string): void {
  if (typeof window === "undefined") {
    return;
  }
  const trimmed = planId.trim() || workspacePlanId;
  try {
    window.localStorage.setItem(selectedPlanStorageKey, trimmed);
  } catch {
    // URL state remains the shareable selector.
  }
  const url = new URL(window.location.href);
  if (trimmed && trimmed !== workspacePlanId) {
    url.searchParams.set("plan", trimmed);
  } else {
    url.searchParams.delete("plan");
  }
  if (`${url.pathname}${url.search}${url.hash}` !== `${window.location.pathname}${window.location.search}${window.location.hash}`) {
    window.history.replaceState({}, "", url);
  }
}

function stringFrom(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function planIdFromAcceptResponse(response: unknown): string {
  return stringFrom(at(response, ["plan_id"]))
    || stringFrom(at(response, ["data", "plan", "id"]))
    || stringFrom(at(response, ["data", "plan", "plan_id"]))
    || stringFrom(at(response, ["data", "plan_id"]));
}

function assertGoalSubmitReachedCoordinator(response: unknown): void {
  if (!isRecord(response)) {
    return;
  }
  const nested = isRecord(response.result) ? response.result : {};
  const proxyStatus = typeof response.status === "number"
    ? response.status
    : typeof nested.status === "number"
      ? nested.status
      : null;
  if (proxyStatus === null || (proxyStatus >= 200 && proxyStatus < 400)) {
    return;
  }
  const detail = typeof response.error === "string"
    ? response.error
    : typeof nested.error === "string"
      ? nested.error
      : typeof response.url === "string"
        ? response.url
        : typeof nested.url === "string"
          ? nested.url
          : "unknown upstream failure";
  throw new Error(`Goal submit returned an upstream failure: ${detail}`);
}

function initialThemePreference(): ThemePreference {
  if (typeof window === "undefined") {
    return "system";
  }
  try {
    const stored = window.localStorage.getItem(themeStorageKey);
    return stored === "light" || stored === "dark" || stored === "system" ? stored : "system";
  } catch {
    return "system";
  }
}

function resolveTheme(preference: ThemePreference): ResolvedTheme {
  if (preference === "light" || preference === "dark") {
    return preference;
  }
  if (typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches) {
    return "dark";
  }
  return "light";
}

function ThemeSwitcher(props: { preference: ThemePreference; resolved: ResolvedTheme; onChange: (value: ThemePreference) => void }) {
  const options: Array<{ value: ThemePreference; label: string; icon: typeof Sun }> = [
    { value: "light", label: "Light", icon: Sun },
    { value: "system", label: "Auto", icon: Monitor },
    { value: "dark", label: "Dark", icon: Moon },
  ];
  return (
    <div className="theme-control" aria-label={`Appearance, currently ${props.resolved}`}>
      {options.map((option) => {
        const Icon = option.icon;
        const active = props.preference === option.value;
        return (
          <button
            key={option.value}
            type="button"
            className={clsx("theme-option", active && "active")}
            aria-pressed={active}
            title={`${option.label} appearance`}
            onClick={() => props.onChange(option.value)}
          >
            <Icon size={15} />
            <span>{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}

function titleFor(view: ViewKey): string {
  return {
    dashboard: "Overview",
    goals: "Goals",
    graph: "Work Graph",
    control: "Steer",
    memory: "Shared Memory",
    plans: "Durable Plans",
    human: "Actions",
    runners: "Runner Fleet",
  }[view];
}
