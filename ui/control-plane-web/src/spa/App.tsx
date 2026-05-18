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
  authToken,
  cancelGoal,
  chat,
  chatRun,
  chatSession,
  isRecord,
  operatorGoalDetail,
  operatorGoals,
  operatorWorkspace,
  rowsFrom,
  setAuthToken,
  submitOperatorGoal,
} from "./api";
import type { ChatMessage, ChatRunTrace, GoalRow } from "./types";
import { EmptyState, InspectButton } from "./components/operator-primitives";
import {
  ChatDraftPanel,
  draftKindLabel,
  draftReviewSummary,
  goalDraftFromChatResponse,
  goalIdFromSubmitResponse,
  modeForDraftKind,
  sessionDisplayLabel,
  updateGoalDraftField,
  type ActiveDraftState,
  type DraftKind,
  type GoalDraftEditField,
} from "./features/chat-draft-panel";
import { ActiveGoalRuntimeBar, DraftReviewDock, type ActiveRuntimeViewModel, type DraftDockViewModel } from "./features/operator-runtime";
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
import { Dashboard, HumanQueueView, PlansView, RunnersView, ServiceStrip } from "./features/operator-dashboard-routes";
import { CompilerControlView } from "./features/operator-control-panel";
import { useGoalStateStream } from "./features/operator-stream";
import { TaskGraphView } from "./features/task-graph-view";
import { MemoryView } from "./features/memory-view";
import {
  createRunId,
  friendlyRef,
  statusToken,
  stringValue,
} from "./features/workbench-format";

type ViewKey = "dashboard" | "goals" | "graph" | "control" | "memory" | "plans" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";

const themeStorageKey = "coat.theme";
const selectedGoalStorageKey = "coat.selectedGoalId";
const themeColors: Record<ResolvedTheme, string> = {
  light: "#f5f6f4",
  dark: "#080c0f",
};
const views: Array<{ key: ViewKey; label: string; icon: typeof Route }> = [
  { key: "dashboard", label: "Dashboard", icon: Route },
  { key: "goals", label: "Goals", icon: ListChecks },
  { key: "graph", label: "Work Graph", icon: Network },
  { key: "control", label: "Operator Actions", icon: ShieldCheck },
  { key: "memory", label: "Memory", icon: Brain },
  { key: "plans", label: "Plans", icon: GitBranch },
  { key: "human", label: "Action Queue", icon: Bell },
  { key: "runners", label: "Runners", icon: Server },
];

const starterMessages: ChatMessage[] = [
  {
    role: "assistant",
    content:
      "Ask about the workspace, or switch to Draft goal when you want to create coordinator-owned work.",
  },
];

export function App() {
  const queryClient = useQueryClient();
  const [activeView, setActiveView] = useState<ViewKey>("dashboard");
  const [selectedGoalId, setSelectedGoalId] = useState(() => initialSelectedGoalId());
  const [goalPickerOpen, setGoalPickerOpen] = useState(false);
  const [submittedGoalDrafts, setSubmittedGoalDrafts] = useState<Record<string, SubmittedGoalDraft>>({});
  const [token, setToken] = useState(authToken());
  const [sessionMessages, setSessionMessages] = useState<Record<string, ChatMessage[]>>({});
  const [activeDraft, setActiveDraft] = useState<ActiveDraftState | null>(null);
  const [chatInput, setChatInput] = useState("");
  const [draftKind, setDraftKind] = useState<DraftKind>("ask");
  const [activeChatRunId, setActiveChatRunId] = useState<string | null>(null);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => initialThemePreference());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(initialThemePreference()));

  const goalsQuery = useQuery({ queryKey: ["goals"], queryFn: operatorGoals });
  const operatorWorkspaceQuery = useQuery({
    queryKey: ["operator-workspace", selectedGoalId],
    queryFn: () => operatorWorkspace(selectedGoalId || undefined),
    refetchInterval: 5_000,
  });
  const chatSessionId = selectedGoalId ? `goal:${selectedGoalId}` : "operator:default";
  const activeDraftForSession = activeDraft?.sessionId === chatSessionId ? activeDraft : null;
  const visibleActiveDraft = activeDraftForSession ?? activeDraft;
  const chatSessionQuery = useQuery({
    queryKey: ["chat-session", chatSessionId],
    queryFn: () => chatSession(chatSessionId),
  });
  const messages = sessionMessages[chatSessionId] ?? starterMessages;
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
      const requestKind = draftKind;
      const requestMode = modeForDraftKind(requestKind);
      const currentMessages = sessionMessages[requestSessionId] ?? messages;
      const nextMessages = [...currentMessages, { role: "user" as const, content }];
      setSessionMessages((current) => ({ ...current, [requestSessionId]: nextMessages }));
      setChatInput("");
      const runId = createRunId();
      setActiveChatRunId(runId);
      const response = await chat(requestSessionId, requestMode, requestGoalId, content, runId);
      const goalDraft = goalDraftFromChatResponse(response);
      setActiveChatRunId(response.run_id ?? runId);
      setSessionMessages((current) => ({
        ...current,
        [requestSessionId]: [...nextMessages, { role: "assistant" as const, content: response.assistant ?? "Response pending." }],
      }));
      if (requestKind === "ask" && !goalDraft) {
        setActiveDraft(null);
      } else {
        setActiveDraft({
          kind: requestKind,
          mode: requestMode,
          sessionId: requestSessionId,
          selectedGoalId: requestGoalId,
          savedAt: new Date().toISOString(),
          response,
          goalDraft,
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
  const submitGoalDraft = useMutation({
    mutationFn: async () => {
      const draft = latestGoalDraft;
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
        void queryClient.invalidateQueries({ queryKey: ["operator-goal", goalId] });
        void queryClient.refetchQueries({ queryKey: ["operator-goal", goalId] });
      }
      void queryClient.invalidateQueries({ queryKey: ["goals"] });
      void queryClient.refetchQueries({ queryKey: ["goals"] });
    },
  });

  const refreshAll = () => {
    void queryClient.invalidateQueries();
  };
  const sendChatFromPanel = (content?: string) => {
    submitGoalDraft.reset();
    sendChat.mutate(content);
  };
  const focusActiveDraftEditor = () => {
    setActiveView("dashboard");
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>(".goal-draft-editor input, .goal-draft-editor textarea")?.focus();
      document.querySelector<HTMLElement>(".goal-draft-editor")?.scrollIntoView({ block: "center", behavior: "smooth" });
    });
  };
  const discardActiveGoalDraft = () => {
    setActiveDraft(null);
    sendChat.reset();
    submitGoalDraft.reset();
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
    const handlePopState = () => setSelectedGoalId(initialSelectedGoalId());
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
  const selectableGoals = useMemo(() => goalRowsWithSelected(goalRows, selectedGoal), [goalRows, selectedGoal]);
  const serviceRows = operatorWorkspaceData?.services ?? [];
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
  const activeDraftView = useMemo<DraftDockViewModel | null>(() => {
    if (!visibleActiveDraft) {
      return null;
    }
    const summary = draftReviewSummary(visibleActiveDraft.response, latestGoalDraft);
    const submittedGoalId = goalIdFromSubmitResponse(submitGoalDraft.data?.response);
    return {
      title: summary.title,
      detail: summary.objective || summary.summary,
      kindLabel: draftKindLabel(visibleActiveDraft.kind),
      sessionLabel: visibleActiveDraft.sessionId ? sessionDisplayLabel(visibleActiveDraft.sessionId) : "",
      hasGoalDraft: Boolean(latestGoalDraft),
      submittedGoalLabel: submittedGoalId ? friendlyRef(submittedGoalId) : "",
      busy: submitGoalDraft.isPending,
      errorMessage: (submitGoalDraft.error as Error | null)?.message ?? "",
    };
  }, [latestGoalDraft, submitGoalDraft.data?.response, submitGoalDraft.error, submitGoalDraft.isPending, visibleActiveDraft]);

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
            <p className="eyebrow">User-facing manager</p>
            <h1>{titleFor(activeView)}</h1>
          </div>
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
          <ServiceStrip services={serviceRows} />
        </header>
        <ActiveGoalRuntimeBar
          view={activeRuntimeView}
          onOpenGraph={() => setActiveView("graph")}
          onOpenQueue={() => setActiveView("human")}
          onOpenControls={() => setActiveView("control")}
        />
        {activeDraftView && (
          <DraftReviewDock
            view={activeDraftView}
            onEditGoalDraft={focusActiveDraftEditor}
            onSubmitGoalDraft={() => submitGoalDraft.mutate()}
            onDiscardGoalDraft={discardActiveGoalDraft}
          />
        )}

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
          {activeView === "plans" && <PlansView />}
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
            goalSubmitBusy={submitGoalDraft.isPending}
            goalSubmitError={submitGoalDraft.error as Error | null}
            goalSubmitResult={submitGoalDraft.data?.response}
            selectedGoalId={selectedGoalId}
            selectedGoal={selectedGoal}
            sessionId={chatSessionId}
            mode={modeForDraftKind(draftKind)}
            onDraftKindChange={setDraftKind}
            onInputChange={setChatInput}
            onSend={sendChatFromPanel}
            onSubmitGoalDraft={() => submitGoalDraft.mutate()}
            onDiscardGoalDraft={discardActiveGoalDraft}
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
    control: "Actions",
    memory: "Shared Memory",
    plans: "Durable Plans",
    human: "Action Queue",
    runners: "Runner Fleet",
  }[view];
}
