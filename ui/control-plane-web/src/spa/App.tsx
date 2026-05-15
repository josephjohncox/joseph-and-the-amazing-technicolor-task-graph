/**
 * User-facing COAT task graph manager SPA.
 *
 * Purpose: present goals, task graph state, shared memory, human feedback, and
 * runner capacity through product-facing workflows while keeping durable
 * authority in the Rust/Restate backend.
 *
 * Architecture reference: docs/design-docs/110-control-gateway-spa.md
 */
import * as Dialog from "@radix-ui/react-dialog";
import * as Popover from "@radix-ui/react-popover";
import {
  ChatContainer,
  MainContainer,
  Message,
  MessageInput,
  MessageList,
  TypingIndicator,
} from "@chatscope/chat-ui-kit-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Background, Controls, MarkerType, MiniMap, ReactFlow, type Edge, type Node } from "@xyflow/react";
import clsx from "clsx";
import { Command } from "cmdk";
import {
  Bell,
  Brain,
  CheckCircle2,
  ChevronDown,
  CircleAlert,
  FileJson,
  GitBranch,
  ListChecks,
  MessageSquareText,
  Monitor,
  Moon,
  Network,
  PauseCircle,
  RefreshCw,
  RotateCcw,
  Route,
  Search,
  Server,
  ShieldCheck,
  Split,
  Sparkles,
  Sun,
  ThumbsDown,
  ThumbsUp,
  Vote,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  approve,
  at,
  authToken,
  branchGoal,
  cancelGoal,
  chat,
  chatRun,
  chatSession,
  createThunk,
  isRecord,
  mechanismBallot,
  mechanismStart,
  memoryContext,
  memoryEdit,
  memoryEditPreview,
  memoryEvents,
  memorySearch,
  memoryWrite,
  operatorGoalDetail,
  operatorGoals,
  operatorWorkspace,
  operatorActions,
  plans,
  resolveOperatorAction,
  restartGoal,
  resumeThunk,
  rowsFrom,
  selectBranch,
  setAuthToken,
  steer,
  submitOperatorGoal,
  voteGoal,
} from "./api";
import type { ChatMessage, ChatResponse, ChatRunTrace, ColorRef, ComputeGraphNode, GoalRow, ComposedGoalSnapshot, JsonRecord, OperatorGoalDetail, OperatorWorkspaceSnapshot, ServiceHealth, TaskRow } from "./types";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "./components/ui/card";
import { ScrollArea } from "./components/ui/scroll-area";

type ViewKey = "dashboard" | "goals" | "graph" | "control" | "memory" | "plans" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";
type GraphFilter = "all" | "attention" | "active" | "completed";
type QueueFilter = "all" | "approvals" | "blocked" | "thunks" | "cancelled";
type QueueGroup = Exclude<QueueFilter, "all">;
type DraftKind = "plan" | "goal" | "search";
type GoalDraftEditField = "title" | "objective" | "acceptance_evidence" | "constraints";
type OperatorStateKey = "action-needed" | "running" | "waiting" | "reviewing" | "satisfied";
type OtherGoalAction = "review" | "research" | "priority" | "steer" | "restart_branch" | "wait" | "decision_round" | "ballot";
type ActionNeededKind = "approval" | "blocked-task" | "waiting-task" | "thunk" | "cancelled";
type ActionNeededItem = {
  actionId: string;
  key: string;
  kind: ActionNeededKind;
  label: string;
  status: string;
  detail: string;
  requestedInput: string;
  goalId: string;
  taskId: string;
  approvalId: string;
  thunkId: string;
  continuationId: string;
  risk: string;
  actionLabel: string;
};
type HumanPromptSpec = {
  title: string;
  question: string;
  detail: string;
  primaryLabel: string;
  contextLabel: string;
  inputLabel: string;
  placeholder: string;
  showInput: boolean;
  defaultResponseSummary: string;
  createPromptLabel?: string;
  cancelLabel?: string;
};
type ActionMutationInput = {
  item: ActionNeededItem;
  responseSummary?: string;
  intent?: ActionIntent;
};
type ActionIntent = "primary" | "context" | "create-human-prompt" | "cancel-goal";
type ActiveDraftState = {
  kind: DraftKind;
  mode: string;
  sessionId: string;
  selectedGoalId: string;
  savedAt: string;
  response: ChatResponse;
  goalDraft: JsonRecord | null;
  runId: string | null;
};
type DraftReviewSummary = {
  title: string;
  objective: string;
  summary: string;
  reference: string;
  source: string;
  evidenceCount: number;
  constraintCount: number;
};
type SubmittedGoalDraft = {
  draft: JsonRecord;
  submittedAt: number;
  projected: boolean;
};
type GoalSummary = {
  id: string;
  title: string;
  objective: string;
  status: string;
  progress: number;
  openTasks: number;
  blockedTasks: number;
  failedTasks: number;
  updatedAt: string;
};
type OperatorStateDefinition = {
  key: OperatorStateKey;
  label: string;
  detail: string;
  statuses: string[];
};
type ContinuationRow = {
  key: string;
  thunkId: string;
  continuationId: string;
  taskId: string;
  reason: string;
  requestedInput: string;
  status: string;
  waitKind: string;
  waitReference: string;
};

const defaultContinuationSummary = "Operator chose Continue.";
type GoalStreamState = {
  status: "idle" | "connecting" | "live" | "error";
  lastEventAt: string;
  error: string;
};

const themeStorageKey = "coat.theme";
const selectedGoalStorageKey = "coat.selectedGoalId";
const themeColors: Record<ResolvedTheme, string> = {
  light: "#f5f6f4",
  dark: "#080c0f",
};
const knownStatusTones = new Set([
  "pending",
  "runnable",
  "running",
  "needs-validation",
  "waiting-approval",
  "waiting-input",
  "done",
  "submitted",
  "blocked",
  "failed",
  "cancelled",
]);
const operatorStateDefinitions: OperatorStateDefinition[] = [
  {
    key: "action-needed",
    label: "Action needed",
    detail: "failed, blocked, or approval work",
    statuses: ["failed", "blocked", "waiting-approval"],
  },
  {
    key: "running",
    label: "Running",
    detail: "active agents and runnable frontier",
    statuses: ["running", "runnable"],
  },
  {
    key: "waiting",
    label: "Waiting",
    detail: "queued, submitted, or paused continuations",
    statuses: ["waiting-input", "pending", "submitted", "unknown"],
  },
  {
    key: "reviewing",
    label: "Reviewing",
    detail: "validation and evidence checks",
    statuses: ["needs-validation"],
  },
  {
    key: "satisfied",
    label: "Satisfied",
    detail: "accepted task evidence",
    statuses: ["done"],
  },
];
const statusLegend = [
  { token: "failed", label: "Action needed", detail: "failed task" },
  { token: "blocked", label: "Action needed", detail: "blocked task" },
  { token: "waiting-approval", label: "Action needed", detail: "approval gate" },
  { token: "waiting-input", label: "Waiting", detail: "waiting continuation" },
  { token: "running", label: "Running", detail: "agent is active" },
  { token: "needs-validation", label: "Reviewing", detail: "evidence check" },
  { token: "runnable", label: "Running", detail: "ready frontier" },
  { token: "done", label: "Satisfied", detail: "accepted evidence" },
  { token: "submitted", label: "Waiting", detail: "projection sync" },
  { token: "pending", label: "Waiting", detail: "queued work" },
  { token: "cancelled", label: "Stopped", detail: "intentionally stopped" },
] as const;
const statusPriority = new Map<string, number>(statusLegend.map((item, index) => [item.token, index]));
const graphFilterOptions: Array<{ key: GraphFilter; label: string; detail: string }> = [
  { key: "all", label: "All", detail: "all projected tasks" },
  { key: "attention", label: "Action needed", detail: "failed, blocked, approvals, continuations" },
  { key: "active", label: "Active", detail: "running, runnable, validation" },
  { key: "completed", label: "Completed", detail: "done or cancelled" },
];
const queueFilterOptions: Array<{ key: QueueFilter; label: string; detail: string }> = [
  { key: "all", label: "All", detail: "all operator actions and stopped history" },
  { key: "approvals", label: "Approvals", detail: "approval gates that need a human decision" },
  { key: "blocked", label: "Recovery", detail: "blocked or failed work that can be retried, replanned, or turned into a prompt" },
  { key: "thunks", label: "Continuations", detail: "waiting prompts that can be resumed by an operator" },
  { key: "cancelled", label: "Stopped", detail: "stopped work kept as read-only history" },
];
const queueGroupLabels: Record<QueueGroup, string> = {
  approvals: "Approvals",
  blocked: "Recovery",
  thunks: "Continuations",
  cancelled: "Stopped history",
};
const queueGroupOrder: QueueGroup[] = ["approvals", "blocked", "thunks", "cancelled"];

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
      "Tell me the outcome you want. I will draft it first; durable state changes only when you accept a draft or use an action button.",
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
  const [draftKind, setDraftKind] = useState<DraftKind>("goal");
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
          selectedGoal={selectedGoal}
          snapshot={currentGoal}
          stream={goalStream}
          actionBusy={submitGoalDraft.isPending}
          onOpenGraph={() => setActiveView("graph")}
          onOpenQueue={() => setActiveView("human")}
          onOpenControls={() => setActiveView("control")}
        />
        {visibleActiveDraft && (
          <DraftReviewDock
            activeDraft={visibleActiveDraft}
            goalDraft={latestGoalDraft}
            goalSubmitBusy={submitGoalDraft.isPending}
            goalSubmitError={submitGoalDraft.error as Error | null}
            goalSubmitResult={submitGoalDraft.data?.response}
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
          <CommandPanel
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

function modeForDraftKind(kind: DraftKind): string {
  if (kind === "goal") return "draft_goal";
  if (kind === "search") return "draft_search";
  return "draft_plan";
}

function ActiveGoalRuntimeBar(props: {
  selectedGoal: GoalSummary | null;
  snapshot?: ComposedGoalSnapshot;
  stream: GoalStreamState;
  actionBusy: boolean;
  onOpenGraph: () => void;
  onOpenQueue: () => void;
  onOpenControls: () => void;
}) {
  if (!props.selectedGoal) {
    return null;
  }
  const snapshot = props.snapshot ?? { goal_id: props.selectedGoal.id };
  const tasks = taskRowsFromComposedSnapshot(snapshot);
  const counts = taskStatusCounts(tasks);
  const actions = actionNeededItemsFromComposedSnapshot(props.snapshot, props.selectedGoal.id);
  const state = nextActionSummary(counts, tasks.length, snapshot);
  const streamTone = props.stream.status === "live" ? "status-running" : props.stream.status === "error" ? "status-failed" : "status-pending";
  return (
    <section className="active-runtime-bar" aria-label="Selected goal active state">
      <div className="runtime-state">
        <span className="goal-context-kicker">Live state</span>
        <strong>{state.stateLabel}</strong>
        <small>{state.title}</small>
      </div>
      <div className="runtime-metrics">
        <span className={clsx("status-pill", streamTone)}>
          {props.stream.status === "live" ? "Streaming" : props.stream.status === "connecting" ? "Connecting" : props.stream.status === "error" ? "Stream error" : "Idle"}
        </span>
        <span className="status-pill muted">{tasks.length} tasks</span>
        <span className={clsx("status-pill", actions.length ? "status-waiting-approval" : "status-done")}>{actions.length} actions</span>
        <span className="status-pill muted">{props.stream.lastEventAt ? `Updated ${timeLabel(props.stream.lastEventAt)}` : "Projection pending"}</span>
        {props.actionBusy && <span className="status-pill status-running">Accepting draft</span>}
      </div>
      <div className="button-row">
        <button type="button" className="secondary-button" onClick={props.onOpenGraph}>
          <Network size={15} />
          Graph
        </button>
        <button type="button" className={actions.length ? "primary-button" : "secondary-button"} onClick={props.onOpenQueue}>
          <Bell size={15} />
          Actions
        </button>
        <button type="button" className="secondary-button" onClick={props.onOpenControls}>
          <ShieldCheck size={15} />
          Operator actions
        </button>
      </div>
  {props.stream.error && <span className="error-text">{props.stream.error}</span>}
    </section>
  );
}

function DraftReviewDock(props: {
  activeDraft: ActiveDraftState;
  goalDraft: JsonRecord | null;
  goalSubmitBusy: boolean;
  goalSubmitError: Error | null;
  goalSubmitResult?: unknown;
  onSubmitGoalDraft: () => void;
  onDiscardGoalDraft: () => void;
}) {
  const summary = draftReviewSummary(props.activeDraft.response, props.goalDraft);
  const submittedGoalId = goalIdFromSubmitResponse(props.goalSubmitResult);
  return (
    <section className="draft-review-dock" aria-label="Active draft">
      <div>
        <span className="goal-context-kicker">Active draft</span>
        <strong>{summary.title}</strong>
        <small>{summary.objective || summary.summary}</small>
      </div>
      <div className="draft-summary-meta">
        <span className="status-pill status-runnable">{draftKindLabel(props.activeDraft.kind)}</span>
        {props.goalDraft && <span className="status-pill status-runnable">Goal draft ready</span>}
        {props.activeDraft.sessionId && <span className="status-pill muted">{sessionDisplayLabel(props.activeDraft.sessionId)}</span>}
        {submittedGoalId && <span className="status-pill status-done">Accepted {friendlyRef(submittedGoalId)}</span>}
      </div>
      <div className="button-row">
        <Button type="button" variant="outline" disabled={props.goalSubmitBusy || Boolean(submittedGoalId)} onClick={props.onDiscardGoalDraft}>
          <XCircle size={15} />
          Discard
        </Button>
        {props.goalDraft && (
          <Button type="button" disabled={props.goalSubmitBusy || Boolean(submittedGoalId)} onClick={props.onSubmitGoalDraft}>
            <ListChecks size={15} />
            {submittedGoalId ? "Accepted" : props.goalSubmitBusy ? "Accepting" : "Accept draft"}
          </Button>
        )}
      </div>
      {props.goalSubmitError && <span className="error-text">{props.goalSubmitError.message}</span>}
    </section>
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

function createRunId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `chat-${Date.now()}-${Math.random().toString(16).slice(2)}`;
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

function goalDraftFromChatResponse(response?: ChatResponse): JsonRecord | null {
  const draft = response?.drafts?.goal_spec;
  if (!isRecord(draft)) {
    return null;
  }
  const title = typeof draft.title === "string" ? draft.title.trim() : "";
  const objective = typeof draft.objective === "string" ? draft.objective.trim() : "";
  return title && objective ? draft : null;
}

function draftReviewSummary(response?: ChatResponse, draft?: JsonRecord | null): DraftReviewSummary {
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
  return numberValue(value);
}

function listCount(value: unknown): number | null {
  if (Array.isArray(value)) {
    return value.length;
  }
  const lines = linesFromList(value);
  return lines ? lines.split("\n").length : null;
}

function updateGoalDraftField(draft: JsonRecord, field: GoalDraftEditField, value: string): JsonRecord {
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

function goalIdFromSubmitResponse(response: unknown): string {
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

function activeStateFromActionResponse(response: unknown): ComposedGoalSnapshot | null {
  if (!activeStateEnvelopeIsFresh(response)) {
    return null;
  }
  const activeState = at(response, ["active_state"]);
  if (isRecord(activeState)) {
    return activeState as ComposedGoalSnapshot;
  }
  const nestedActiveState = at(response, ["result", "data", "active_state"]);
  return isRecord(nestedActiveState) ? nestedActiveState as ComposedGoalSnapshot : null;
}

function activeStateEnvelopeIsFresh(response: unknown): boolean {
  if (!isRecord(response)) {
    return false;
  }
  if ("active_state_available" in response) {
    return response.active_state_available === true;
  }
  const status = stringValue(response.active_state_status) || stringValue(at(response, ["observability", "active_state_status"]));
  return status ? status === "fresh" : true;
}

function applyActionEnvelopeToCache(queryClient: ReturnType<typeof useQueryClient>, response: unknown, fallbackGoalId = ""): void {
  const snapshot = activeStateFromActionResponse(response);
  const goalId = stringValue(at(response, ["action", "goal_id"])) || stringValue(snapshot?.goal_id) || fallbackGoalId;
  if (goalId) {
    void queryClient.invalidateQueries({ queryKey: ["operator-goal", goalId] });
    void queryClient.invalidateQueries({ queryKey: ["operator-actions", goalId] });
  }
  void queryClient.invalidateQueries({ queryKey: ["operator-actions"] });
  void queryClient.invalidateQueries({ queryKey: ["operator-workspace"] });
  void queryClient.invalidateQueries({ queryKey: ["goals"] });
}

function actionEnvelopeSummary(response: unknown): { label: string; detail: string; status: string } | null {
  if (!isRecord(response)) {
    return null;
  }
  const handler = stringValue(at(response, ["action", "handler"])) || "action";
  const ok = response.ok !== false;
  const observability = isRecord(response.observability) ? response.observability : {};
  const attempts = numberValue(observability.active_state_attempts) ?? 0;
  const elapsed = numberValue(observability.active_state_after_ms) ?? 0;
  const stateStatus = stringValue(response.active_state_status) || stringValue(observability.active_state_status) || "fresh";
  const unavailableReads = (Array.isArray(response.active_state_unavailable_reads)
    ? response.active_state_unavailable_reads
    : Array.isArray(observability.active_state_unavailable_reads)
      ? observability.active_state_unavailable_reads
      : [])
    .map((value) => String(value))
    .filter(Boolean);
  const activeStateDetail = stateStatus === "fresh"
    ? attempts
      ? `Active state read after ${attempts} attempt${attempts === 1 ? "" : "s"} in ${elapsed}ms.`
      : "Active state read pending."
    : `Active state ${stateStatus}; ${unavailableReads.length ? `unavailable reads: ${unavailableReads.join(", ")}. ` : ""}Projection will refresh from the stream or next poll.`;
  return {
    label: ok ? `${humanActionName(handler)} accepted` : `${humanActionName(handler)} failed`,
    detail: activeStateDetail,
    status: ok ? "done" : "failed",
  };
}

function humanActionName(handler: string): string {
  return handler
    .replace(/_/g, " ")
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

function useGoalStateStream(goalId: string, token: string, enabled: boolean): GoalStreamState {
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
            queryClient.setQueryData(["operator-workspace", goalId], event.data);
            const selectedGoal = at(event.data, ["selected_goal"]);
            if (isRecord(selectedGoal)) {
              queryClient.setQueryData(["operator-goal", goalId], selectedGoal as OperatorGoalDetail);
            }
            setState({ status: "live", lastEventAt: new Date().toISOString(), error: "" });
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

function mergeSubmittedGoalRows(projectedRows: GoalRow[], submittedDrafts: Record<string, SubmittedGoalDraft | JsonRecord>): GoalRow[] {
  const projectedIds = new Set(projectedRows.map((goal) => String(goal.goal_id ?? goal.id ?? "")).filter(Boolean));
  const pendingRows = Object.entries(submittedDrafts)
    .filter(([goalId, pending]) => {
      const submitted = submittedGoalDraftState(pending);
      return !submitted.projected || !projectedIds.has(goalId);
    })
    .map(([goalId, pending]) => pendingGoalRow(goalId, submittedGoalDraftState(pending).draft));
  return [...pendingRows, ...projectedRows];
}

function submittedGoalDraftState(value: SubmittedGoalDraft | JsonRecord): SubmittedGoalDraft {
  if (isRecord(value.draft)) {
    return value as SubmittedGoalDraft;
  }
  return {
    draft: value,
    submittedAt: 0,
    projected: false,
  };
}

function goalSummaryFromRow(goal: GoalRow): GoalSummary | null {
  const id = String(goal.goal_id ?? goal.id ?? "").trim();
  if (!id) {
    return null;
  }
  return {
    id,
    title: stringValue(goal.title) || friendlyRef(id) || "Untitled goal",
    objective: stringValue(goal.objective),
    status: stringValue(goal.status) || "unknown",
    progress: numericProgress(goal.percent_done),
    openTasks: numberValue(goal.open_tasks) ?? 0,
    blockedTasks: numberValue(goal.blocked_tasks) ?? 0,
    failedTasks: numberValue(goal.failed_tasks) ?? 0,
    updatedAt: stringValue(goal.updated_at),
  };
}

function selectedGoalSummary(goalId: string, rows: GoalRow[], snapshot?: ComposedGoalSnapshot, submittedDraft?: JsonRecord | null): GoalSummary | null {
  if (!goalId) {
    return null;
  }
  const fromRows = rows.map(goalSummaryFromRow).find((goal): goal is GoalSummary => Boolean(goal && goal.id === goalId));
  if (fromRows) {
    return fromRows;
  }
  const payload = at(snapshot, ["goal_store_goal", "data", "goal", "payload_json"]);
  const source = isRecord(payload) ? payload : submittedDraft;
  return {
    id: goalId,
    title: stringValue(source?.title) || friendlyRef(goalId) || "Selected goal",
    objective: stringValue(source?.objective),
    status: stringValue(at(snapshot, ["goal_store_goal", "data", "goal", "status"])) || (submittedDraft ? "submitted" : "selected"),
    progress: numericProgress(at(snapshot, ["goal_store_goal", "data", "goal", "percent_done"])),
    openTasks: numberValue(at(snapshot, ["goal_store_goal", "data", "goal", "open_tasks"])) ?? rowsFrom(source?.initial_tasks).length,
    blockedTasks: numberValue(at(snapshot, ["goal_store_goal", "data", "goal", "blocked_tasks"])) ?? 0,
    failedTasks: numberValue(at(snapshot, ["goal_store_goal", "data", "goal", "failed_tasks"])) ?? 0,
    updatedAt: stringValue(at(snapshot, ["goal_store_goal", "data", "goal", "updated_at"])),
  };
}

function composedSnapshotFromOperatorGoalDetail(detail?: OperatorGoalDetail | null): ComposedGoalSnapshot | undefined {
  if (!detail) {
    return undefined;
  }
  if (isRecord(detail.snapshot)) {
    return detail.snapshot as ComposedGoalSnapshot;
  }
  const summary = isRecord(detail.summary) ? detail.summary : {};
  const goalId = stringValue(summary.goal_id) || stringValue(summary.id);
  const tasks = rowsFrom(detail.tasks);
  return {
    goal_id: goalId,
    goal_store_goal: {
      data: {
        goal: {
          goal_id: goalId,
          status: summary.status,
          percent_done: summary.percent_done,
          open_tasks: summary.open_tasks,
          blocked_tasks: summary.blocked_tasks,
          failed_tasks: summary.failed_tasks,
          updated_at: summary.updated_at,
          payload_json: {
            title: summary.title,
            objective: summary.objective,
          },
        },
      },
    },
    workflow_progress: { data: detail.progress ?? {} },
    workflow_compute_graph: { data: detail.graph ?? {} },
    tasks: { data: { tasks } },
    approvals: { data: { approvals: [] } },
    checkpoints: { data: { checkpoints: [] } },
    agent_activity: tasks as TaskRow[],
  };
}

function goalRowsWithSelected(rows: GoalRow[], selectedGoal: GoalSummary | null): GoalSummary[] {
  const summaries = rows.map(goalSummaryFromRow).filter((goal): goal is GoalSummary => Boolean(goal));
  if (selectedGoal && !summaries.some((goal) => goal.id === selectedGoal.id)) {
    return [selectedGoal, ...summaries];
  }
  return summaries;
}

function numericProgress(value: unknown): number {
  const numeric = numberValue(value) ?? 0;
  return numeric > 1 ? numeric / 100 : numeric;
}

function pendingGoalRow(goalId: string, draft: JsonRecord): GoalRow {
  const initialTasks = rowsFrom(draft.initial_tasks);
  const subgoals = goalSubgoalsFromDraft(draft);
  return {
    goal_id: goalId,
    id: goalId,
    title: stringValue(draft.title) || "Submitted goal",
    objective: stringValue(draft.objective) || "Waiting for the coordinator projection.",
    status: "submitted",
    percent_done: 0,
    open_tasks: Math.max(1, initialTasks.length || subgoals.length),
    blocked_tasks: 0,
    failed_tasks: 0,
    updated_at: new Date().toISOString(),
  };
}

function composedSnapshotHasProjectedTasks(snapshot?: ComposedGoalSnapshot): boolean {
  const projectedTasks = taskRowsFromComposedSnapshot(snapshot);
  const taskRows = rowsFrom(at(snapshot, ["tasks", "data"]) ?? snapshot?.tasks);
  const computeNodes = rowsFrom(at(snapshot, ["workflow_compute_graph", "data", "nodes"]) ?? at(snapshot, ["workflow_compute_graph", "nodes"]));
  return Boolean(projectedTasks.length || taskRows.length || computeNodes.length);
}

function taskRowsFromGoalDraft(goalId: string, draft?: JsonRecord | null): TaskRow[] {
  if (!draft) {
    return [];
  }
  const initialTasks = rowsFrom(draft.initial_tasks);
  if (initialTasks.length) {
    return initialTasks.map((task, index) => ({
      goal_id: goalId,
      task_id: stringValue(task.id) || stringValue(task.task_id) || `draft-task-${index + 1}`,
      parent_task_id: index === 0 ? null : "draft-root",
      subgoal_id: stringValue(task.subgoal_id) || null,
      title: stringValue(task.title) || stringValue(task.role) || `Draft task ${index + 1}`,
      role: stringValue(task.role) || "planner",
      purpose: "work",
      status: "submitted",
      current_prompt: stringValue(task.prompt) || stringValue(task.reason) || null,
      prompt: stringValue(task.prompt) || stringValue(task.reason) || null,
      raw_task: task,
    }));
  }
  return goalSubgoalsFromDraft(draft).map((subgoal, index) => ({
    goal_id: goalId,
    task_id: stringValue(subgoal.id) || stringValue(subgoal.subgoal_id) || `draft-subgoal-${index + 1}`,
    parent_task_id: "draft-root",
    subgoal_id: stringValue(subgoal.id) || stringValue(subgoal.subgoal_id) || null,
    title: stringValue(subgoal.title) || `Draft subgoal ${index + 1}`,
    role: "planner",
    purpose: "work",
    status: "submitted",
    current_prompt: stringValue(subgoal.objective) || stringValue(subgoal.summary) || null,
    prompt: stringValue(subgoal.objective) || stringValue(subgoal.summary) || null,
    raw_task: subgoal,
  }));
}

function goalSubgoalsFromDraft(draft?: JsonRecord | null): JsonRecord[] {
  return rowsFrom(at(draft, ["plan", "subgoals"]));
}

function goalSubgoalsFromComposedSnapshotOrDraft(snapshot?: ComposedGoalSnapshot, draft?: JsonRecord | null): JsonRecord[] {
  const projected = rowsFrom(at(snapshot, ["goal_store_goal", "data", "goal", "payload_json", "plan", "subgoals"]));
  return projected.length ? projected : goalSubgoalsFromDraft(draft);
}

function valueAt(value: unknown, path: string[]): unknown {
  let current = value;
  for (const key of path) {
    if (!isRecord(current)) {
      return null;
    }
    current = current[key];
  }
  return current;
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
    control: "Operator Actions",
    memory: "Shared Memory",
    plans: "Durable Plans",
    human: "Action Queue",
    runners: "Runner Fleet",
  }[view];
}

function ServiceStrip({ services }: { services?: ServiceHealth[] }) {
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

function GoalContextBar(props: {
  goals: GoalSummary[];
  selectedGoal: GoalSummary | null;
  selectedGoalId: string;
  open: boolean;
  loading: boolean;
  cancelBusy: boolean;
  cancelError: Error | null;
  onOpenChange: (open: boolean) => void;
  onSelectGoal: (goalId: string) => void;
  onCancelGoal: () => void;
  onRefreshGoals: () => void;
  onOpenGraph: () => void;
}) {
  const done = Math.round((props.selectedGoal?.progress ?? 0) * 100);
  const selectedState = props.selectedGoal ? operatorStateForStatus(props.selectedGoal.status) : null;
  const selectedStatus = statusToken(props.selectedGoal?.status);
  const canCancelSelectedGoal = Boolean(props.selectedGoalId && selectedStatus !== "cancelled" && selectedStatus !== "done");
  return (
    <Popover.Root open={props.open} onOpenChange={props.onOpenChange}>
      <div className="goal-context-bar">
        <Popover.Trigger asChild>
          <button
            type="button"
            className="goal-context-trigger"
            aria-expanded={props.open}
            aria-label={props.selectedGoal ? `Current goal: ${props.selectedGoal.title}, ${selectedState?.label ?? "state pending"}` : "Select current goal"}
            data-testid="goal-context-trigger"
          >
            <div>
              <span className="goal-context-kicker">Current goal</span>
              <strong>{props.selectedGoal?.title || "Select a goal"}</strong>
              {props.selectedGoal ? (
                <small>{done}% · {props.selectedGoal.openTasks} open · {props.selectedGoal.blockedTasks} blocked · {selectedState?.label}</small>
              ) : (
                <small>Chat, graph, actions, memory, and the action queue use this context</small>
              )}
            </div>
            <span className={clsx("operator-state-pill", selectedState ? stateTone(selectedState.key) : "muted")}>
              {selectedState?.label ?? (props.loading ? "Loading" : "Select")}
            </span>
            <ChevronDown size={16} />
          </button>
        </Popover.Trigger>
        {canCancelSelectedGoal && (
          <button
            type="button"
            className="danger-button goal-context-cancel"
            disabled={props.cancelBusy}
            title="Stop this durable goal through the coordinator."
            onClick={props.onCancelGoal}
          >
            <XCircle size={15} />
            Cancel goal
          </button>
        )}
        <Popover.Portal>
          <Popover.Content className="goal-picker" align="end" sideOffset={8}>
            <Command shouldFilter className="goal-command">
              <div className="goal-picker-actions">
                <label>
                  Search goals
                  <Command.Input placeholder="Title, status, or id" />
                </label>
                <button type="button" className="secondary-button" onClick={props.onRefreshGoals}>
                  <RefreshCw size={15} />
                  Refresh
                </button>
              </div>
              <Command.List className="goal-picker-list">
                <Command.Empty>
                  <EmptyState title="Goal match pending" detail="Refresh goals or submit one from chat." />
                </Command.Empty>
                <Command.Group>
                  {props.goals.map((goal) => (
                    <Command.Item
                      key={goal.id}
                      value={`${goal.title} ${goal.objective} ${goal.status} ${goal.id}`}
                      className={clsx("goal-picker-item", props.selectedGoalId === goal.id && "active")}
                      onSelect={() => {
                        props.onSelectGoal(goal.id);
                        props.onOpenChange(false);
                      }}
                    >
                      <span>
                        <strong>{goal.title || friendlyRef(goal.id) || "Untitled goal"}</strong>
                        <small>{goal.objective || friendlyRef(goal.id) || "Objective pending"}</small>
                      </span>
                      <span className={clsx("status-pill", statusTone(goal.status))}>{statusLabel(goal.status)}</span>
                    </Command.Item>
                  ))}
                </Command.Group>
              </Command.List>
              <div className="goal-picker-footer">
                <button
                  type="button"
                  className="secondary-button"
                  disabled={!props.selectedGoalId}
                  title="Clear the local goal selection. This does not cancel the goal."
                  onClick={() => {
                    props.onSelectGoal("");
                    props.onOpenChange(false);
                  }}
                >
                  <XCircle size={15} />
                  Clear selection
                </button>
                <button
                  type="button"
                  className="danger-button"
                  disabled={!canCancelSelectedGoal || props.cancelBusy}
                  title="Stop this durable goal through the coordinator."
                  onClick={props.onCancelGoal}
                >
                  <XCircle size={15} />
                  Cancel goal
                </button>
                <button type="button" className="primary-button" disabled={!props.selectedGoalId} onClick={props.onOpenGraph}>
                  <Network size={15} />
                  Open graph
                </button>
              </div>
              {props.cancelError && <span className="error-text">{props.cancelError.message}</span>}
            </Command>
          </Popover.Content>
        </Popover.Portal>
      </div>
    </Popover.Root>
  );
}

function CommandPanel(props: {
  messages: ChatMessage[];
  input: string;
  draftKind: DraftKind;
  busy: boolean;
  error: Error | null;
  activeDraft: ActiveDraftState | null;
  latestResponse?: ChatResponse;
  chatRun?: ChatRunTrace;
  goalDraft: JsonRecord | null;
  goalSubmitBusy: boolean;
  goalSubmitError: Error | null;
  goalSubmitResult?: unknown;
  selectedGoalId: string;
  selectedGoal: GoalSummary | null;
  sessionId: string;
  mode: string;
  onDraftKindChange: (value: DraftKind) => void;
  onInputChange: (value: string) => void;
  onSend: (content?: string) => void;
  onSubmitGoalDraft: () => void;
  onDiscardGoalDraft: () => void;
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
    <section className="command-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Assistant</p>
          <h2>Ask or draft</h2>
        </div>
        <div className="draft-mode-group">
          <span>Output</span>
          <div className="mode-toggle" role="group" aria-label="Draft type">
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
            Goal
          </button>
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
        </div>
      </div>
      <div className="draft-mode-hint">
        <strong>{draftModeHeadline(props.draftKind)}</strong>
        <span>{draftModeDetail(props.draftKind)}</span>
      </div>
      <div className="outcome-meta" aria-label="Chat scope">
        <span className="status-pill muted">Assistant</span>
        <span className={clsx("status-pill", props.selectedGoal ? statusTone(props.selectedGoal.status) : "muted")}>
          {props.selectedGoal ? `Context: ${props.selectedGoal.title}` : "Context: workspace"}
        </span>
        <span className={clsx("status-pill", props.activeDraft ? "status-runnable" : "muted")}>
          {props.activeDraft ? `Draft: ${draftKindLabel(props.activeDraft.kind)}` : "Draft: none"}
        </span>
        {draftFromOtherSession && <span className="status-pill status-waiting-input">Draft from {sessionDisplayLabel(props.activeDraft?.sessionId ?? "")}</span>}
        <span className={clsx("status-pill", props.busy ? "status-running" : "status-pending")}>
          {props.busy ? commandBusyLabel(props.draftKind) : `History: ${sessionDisplayLabel(props.sessionId)}`}
        </span>
        {!props.activeDraft && (props.busy || props.chatRun || props.latestResponse || draftKeys.length > 0) && (
          <AdvancedInspect summaryLabel={activityLabel} title="Chat activity" payload={activityPayload} buttonLabel="Inspect JSON" />
        )}
      </div>
      {props.goalDraft && (
        <GoalDraftEditor
          draft={props.goalDraft}
          summary={draftSummary}
          disabled={props.busy || props.goalSubmitBusy || Boolean(submittedGoalId)}
          onUpdate={props.onUpdateGoalDraftField}
        />
      )}
      {props.activeDraft && !props.goalDraft && (
        <DraftSummaryCard summary={draftSummary} />
      )}
      <details className="quick-prompts">
        <summary>
          <span>Prompt helpers</span>
          <small>Optional chat shortcuts</small>
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

function GoalDraftEditor(props: { draft: JsonRecord; summary: DraftReviewSummary; disabled: boolean; onUpdate: (field: GoalDraftEditField, value: string) => void }) {
  return (
    <section className="goal-draft-editor" aria-label="Goal draft review">
      <DraftSummaryCard summary={props.summary} />
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
    </section>
  );
}

function DraftSummaryCard({ summary }: { summary: DraftReviewSummary }) {
  return (
    <div className="draft-summary-card">
      <div>
        <span className="goal-context-kicker">Draft review</span>
        <strong>{summary.title}</strong>
        {summary.objective && <p>{summary.objective}</p>}
        {summary.summary && summary.summary !== summary.objective && <small>{summary.summary}</small>}
      </div>
      <div className="draft-summary-meta" aria-label="Draft summary">
        {summary.reference && <span className="status-pill muted">{summary.reference}</span>}
        <span className="status-pill muted">{summary.source}</span>
        <span className="status-pill muted">{countLabel(summary.evidenceCount, "evidence item")}</span>
        <span className="status-pill muted">{countLabel(summary.constraintCount, "constraint")}</span>
      </div>
    </div>
  );
}

function countLabel(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}

function compilerPromptTemplates(goalId: string, goalTitle?: string): Array<{ label: string; icon: "graph" | "control" | "research"; prompt: string }> {
  const goalClause = goalId ? ` for ${goalTitle ? `"${goalTitle}"` : "the selected goal"} (${goalId})` : "";
  return [
    {
      label: "Explain graph",
      icon: "graph",
      prompt: `Summarize the current compute graph${goalClause}: runnable work, waiting continuations, blocked tasks, and the next control action.`,
    },
    {
      label: "Draft steering",
      icon: "control",
      prompt: `Draft one structured steering directive${goalClause} that moves the objective forward through coordinator review.`,
    },
    {
      label: "Research gap",
      icon: "research",
      prompt: `Find the highest-risk missing information${goalClause} and draft a bounded research request with evidence requirements.`,
    },
  ];
}

function commandTitle(kind: DraftKind): string {
  if (kind === "goal") return "Goal";
  if (kind === "search") return "Search";
  return "Plan";
}

function draftKindLabel(kind: DraftKind): string {
  if (kind === "goal") return "Goal draft";
  if (kind === "search") return "Search request";
  return "Plan draft";
}

function draftModeHeadline(kind: DraftKind): string {
  if (kind === "goal") return "New durable goal";
  if (kind === "search") return "Research request";
  return "Planning draft";
}

function draftModeDetail(kind: DraftKind): string {
  if (kind === "goal") return "Create new work. Existing goals move through the graph and action queue above.";
  if (kind === "search") return "Prepare a sourced research request for backend tools or coordinator work.";
  return "Shape a plan before compiling it into a goal.";
}

function sessionDisplayLabel(sessionId: string): string {
  return sessionId;
}

function commandPlaceholder(kind: DraftKind): string {
  if (kind === "goal") return "Describe the goal, evidence, constraints, and stop conditions";
  if (kind === "search") return "Ask what to search across memory, references, docs, or web";
  return "Describe the outcome, constraints, and review gates";
}

function commandBusyLabel(kind: DraftKind): string {
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

function Dashboard(props: { workspace?: OperatorWorkspaceSnapshot; goals: GoalRow[]; selectedGoalId: string; onSelectGoal: (goalId: string) => void }) {
  const runnerRows = rowsFrom(at(props.workspace?.runners, ["data"]) ?? props.workspace?.runners);
  const eventSourceRows = rowsFrom(at(props.workspace?.event_sources, ["data"]) ?? props.workspace?.event_sources);
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
              <Badge variant="outline">{action.kind || "action"}</Badge>
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

function EventSourcesPanel({ rows }: { rows: JsonRecord[] }) {
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

function eventSourceTableRow(row: JsonRecord): string[] {
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

function GoalsView(props: { goals: GoalRow[]; selectedGoalId: string; onSelectGoal: (goalId: string) => void }) {
  return (
    <section className="panel">
      <div className="section-heading">
        <h2>Goal progress</h2>
        <span className="muted-small">Progress, blockers, and evidence</span>
      </div>
      <GoalList goals={props.goals} selectedGoalId={props.selectedGoalId} onSelect={props.onSelectGoal} />
    </section>
  );
}

function GoalList({ goals, selectedGoalId, onSelect }: { goals: GoalRow[]; selectedGoalId: string; onSelect: (goalId: string) => void }) {
  if (!goals.length) {
    return <EmptyState title="Start a goal" detail="Draft or submit work from chat." />;
  }
  return (
    <div className="goal-list">
      {goals.map((goal) => {
        const goalId = String(goal.goal_id ?? goal.id ?? "");
        const done = Math.round(numericProgress(goal.percent_done) * 100);
        const status = stringValue(goal.status) || "unknown";
        const state = goalOperatorState(goal);
        const nextAction = goalNextAction(goal);
        const openTasks = numberValue(goal.open_tasks) ?? 0;
        const blockedTasks = numberValue(goal.blocked_tasks) ?? 0;
        const failedTasks = numberValue(goal.failed_tasks) ?? 0;
        const title = stringValue(goal.title) || friendlyRef(goalId) || "Untitled goal";
        return (
          <button
            key={goalId || goal.title}
            type="button"
            className={clsx("goal-card", selectedGoalId === goalId && "active")}
            aria-label={`Open ${title} work graph. ${state.label}. ${nextAction}`}
            data-testid={`goal-card-${safeTestId(goalId || title)}`}
            onClick={() => goalId && onSelect(goalId)}
          >
            <div className="goal-card-header">
              <span>
                <strong>{title}</strong>
                {goalId && <small>{friendlyRef(goalId)}</small>}
              </span>
              <span className={clsx("operator-state-pill", stateTone(state.key))}>{state.label}</span>
            </div>
            <p>{goal.objective || "Objective pending."}</p>
            <div className="progress-line" aria-label={`${done}% complete`}>
              <span style={{ width: `${Math.max(0, Math.min(100, done))}%` }} />
            </div>
            <div className="goal-metrics" aria-label="Goal state summary">
              <span><strong>{done}%</strong><small>complete</small></span>
              <span><strong>{openTasks}</strong><small>open</small></span>
              <span className={blockedTasks || failedTasks ? "attention" : ""}><strong>{blockedTasks + failedTasks}</strong><small>blocked or failed</small></span>
              <span><strong>{statusLabel(status)}</strong><small>backend status</small></span>
            </div>
            <div className="goal-next-row">
              <span>{nextAction}</span>
              <strong>Open graph</strong>
            </div>
          </button>
        );
      })}
    </div>
  );
}

function SubgoalPlanPanel({ subgoals, source }: { subgoals: JsonRecord[]; source: string }) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const selected = subgoals[selectedIndex] ?? subgoals[0];
  return (
    <section className="subgoal-panel">
      <div className="section-heading">
        <div>
          <h3>Subgoals</h3>
          <span className="muted-small">{source}</span>
        </div>
      </div>
      <div className="subgoal-list">
        {subgoals.map((subgoal, index) => {
          const id = subgoalId(subgoal);
          const title = subgoalTitle(subgoal, index);
          const objective = subgoalObjective(subgoal);
          const status = stringValue(subgoal.status) || stringValue(subgoal.state) || "planned";
          return (
            <button
              key={id || `subgoal-${index}`}
              type="button"
              className={clsx("subgoal-card", selectedIndex === index && "active")}
              aria-pressed={selectedIndex === index}
              aria-label={`Open subgoal ${title}`}
              data-testid={`subgoal-card-${safeTestId(id || title)}`}
              onClick={() => setSelectedIndex(index)}
            >
              <span className={clsx("status-pill", statusTone(status))}>{statusLabel(status)}</span>
              <strong>{title}</strong>
              <p>{objective}</p>
              {id && <small>{friendlyRef(id)}</small>}
            </button>
          );
        })}
      </div>
      {selected && <SubgoalDetail subgoal={selected} index={selectedIndex} />}
    </section>
  );
}

function SubgoalDetail({ subgoal, index }: { subgoal: JsonRecord; index: number }) {
  const id = subgoalId(subgoal);
  const doneCriteria = textList(subgoal.done_criteria ?? subgoal.acceptance_criteria ?? subgoal.evidence);
  const constraints = textList(subgoal.constraints);
  const initialTasks = rowsFrom(subgoal.initial_tasks ?? subgoal.tasks);
  return (
    <article className="subgoal-detail" data-testid="subgoal-detail">
      <div>
        <span className="goal-context-kicker">Selected subgoal</span>
        <h4>{subgoalTitle(subgoal, index)}</h4>
        <p>{subgoalObjective(subgoal)}</p>
      </div>
      <div className="subgoal-detail-grid">
        <DetailList title="Evidence or done criteria" items={doneCriteria.length ? doneCriteria : ["Evidence requirements pending."]} />
        <DetailList title="Constraints" items={constraints.length ? constraints : ["No explicit constraints projected."]} />
        <DetailList
          title="Seed tasks"
          items={initialTasks.length ? initialTasks.map((task, taskIndex) => stringValue(task.title) || stringValue(task.role) || `Task ${taskIndex + 1}`) : ["No seed tasks projected."]}
        />
      </div>
      {id && <span className="status-pill muted">{friendlyRef(id)}</span>}
    </article>
  );
}

function DetailList({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="detail-list">
      <strong>{title}</strong>
      <ul>
        {items.slice(0, 4).map((item, index) => <li key={`${item}-${index}`}>{item}</li>)}
      </ul>
    </div>
  );
}

function TaskGraphView(props: { goalId: string; snapshot?: ComposedGoalSnapshot; submittedDraft?: JsonRecord | null; loading: boolean; onOpenGoalPicker: () => void; onOpenControls: () => void }) {
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
        <EmptyState title="Loading task graph" detail="Fetching operator goal detail and agent activity." />
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
      {showingSubmittedDraft && <EmptyState title="Submitted goal is syncing" detail="Draft tasks are visible while projection catches up." />}
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

function CompilerControlView(props: { goalId: string; snapshot?: ComposedGoalSnapshot; loading: boolean; onOpenGoalPicker: () => void }) {
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

function graphFromTasks(tasks: TaskRow[]): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = tasks.map((task, index) => {
    const id = taskId(task) || `task-${index}`;
    const color = colorRef(task.color ?? taskPayload(task).color);
    const status = String(task.status ?? "unknown");
    const state = operatorStateForStatus(status);
    return {
      id,
      className: clsx("task-node", statusTone(status)),
      position: { x: (Number(taskPayload(task).depth ?? index) % 4) * 280, y: Math.floor(index / 4) * 150 },
      data: {
        label: `${color?.label ? `${color.label}: ` : ""}${stringValue(task.title) || friendlyRef(id) || "Task"}\n${task.role ?? ""} · ${state.label} · ${statusLabel(status)}`,
      },
      style: {
        borderColor: safeHex(color?.hex),
        borderWidth: 2,
        borderRadius: 8,
        background: `linear-gradient(90deg, ${statusColorVar(status)} 0 5px, var(--node-bg) 5px)`,
        minWidth: 220,
        color: "var(--text)",
        whiteSpace: "pre-line",
        boxShadow: "var(--node-shadow)",
      },
    };
  });
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edges: Edge[] = [];
  for (const task of tasks) {
    const source = String(task.parent_task_id ?? taskPayload(task).parent_id ?? "");
    const target = taskId(task);
    if (source && target && nodeIds.has(source) && nodeIds.has(target)) {
      const edgeColor = statusColorVar(task.status);
      edges.push({
        id: `${source}-${target}`,
        source,
        target,
        animated: normalizeStatus(task.status) === "running",
        markerEnd: { type: MarkerType.ArrowClosed, color: edgeColor },
        style: { stroke: edgeColor, strokeWidth: 1.8 },
      });
    }
  }
  return { nodes, edges };
}

function graphFromComputeGraph(graph: Record<string, unknown> | undefined, nodesToShow: ComputeGraphNode[]): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = nodesToShow.map((node, index) => {
    const id = stringValue(node.id) || `compute-${index}`;
    const status = String(node.status ?? "unknown");
    const kind = String(node.kind ?? "node");
    const state = operatorStateForStatus(status);
    return {
      id,
      className: clsx("task-node", statusTone(status)),
      position: { x: (index % 4) * 285, y: Math.floor(index / 4) * 150 },
      data: {
        label: `${stringValue(node.label) || friendlyRef(id) || "Compute node"}\n${kind} · ${state.label} · ${statusLabel(status)}`,
      },
      style: {
        borderColor: statusColorVar(status),
        borderWidth: 2,
        borderRadius: 8,
        background: `linear-gradient(90deg, ${statusColorVar(status)} 0 5px, var(--node-bg) 5px)`,
        minWidth: 230,
        color: "var(--text)",
        whiteSpace: "pre-line",
        boxShadow: "var(--node-shadow)",
      },
    };
  });
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edgeRows = Array.isArray(graph?.edges) ? graph.edges.filter(isRecord) : [];
  const edges: Edge[] = edgeRows.flatMap((edge) => {
    const source = stringValue(edge.from);
    const target = stringValue(edge.to);
    if (!source || !target || !nodeIds.has(source) || !nodeIds.has(target)) {
      return [];
    }
    return [{
      id: `${source}-${target}-${stringValue(edge.kind) || "edge"}`,
      source,
      target,
      label: stringValue(edge.kind),
      animated: stringValue(edge.kind).includes("resume") || stringValue(edge.kind).includes("unblock"),
      markerEnd: { type: MarkerType.ArrowClosed, color: "var(--edge)" },
      style: { stroke: "var(--edge)", strokeWidth: 1.8 },
    }];
  });
  return { nodes, edges };
}

function computeGraphNodes(graph: Record<string, unknown> | undefined): ComputeGraphNode[] {
  const nodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  return nodes.filter(isRecord).map((node) => node as ComputeGraphNode);
}

function computeNodeMatchesGraphFilter(node: ComputeGraphNode, filter: GraphFilter): boolean {
  if (filter === "all") {
    return true;
  }
  const status = statusToken(node.status);
  if (filter === "attention") {
    return status === "failed" || status === "blocked" || status === "waiting-approval" || status === "waiting-input" || normalizeStatus(node.kind) === "delayed-compute-thunk";
  }
  if (filter === "active") {
    return status === "running" || status === "runnable" || status === "needs-validation" || status === "pending";
  }
  return status === "done" || status === "cancelled";
}

function ComputeGraphDetails({ snapshot }: { snapshot: ComposedGoalSnapshot }) {
  const graph = workflowComputeGraph(snapshot);
  const nodes = computeGraphNodes(graph);
  const openRows = nodes.filter((node) => computeNodeMatchesGraphFilter(node, "attention")).slice(0, 8);
  if (!nodes.length) {
    return <EmptyState title="Compute graph pending" detail="Waiting for workflow activity." />;
  }
  return (
    <div className="compute-details">
      <div className="section-heading">
        <h3>Compute graph</h3>
        <InspectButton title="Compute graph projection" payload={graph} buttonLabel="Inspect graph" />
      </div>
      <SimpleTable
        empty="Active nodes pending."
        headers={["Node", "Kind", "Status", "Wait / Continuation"]}
        rows={(openRows.length ? openRows : nodes.slice(0, 8)).map((node) => [
          stringValue(node.label) || friendlyRef(stringValue(node.id)) || "node",
          stringValue(node.kind) || "unknown",
          statusLabel(stringValue(node.status) || "unknown"),
          [stringValue(node.wait_ref?.kind), friendlyRef(stringValue(node.wait_ref?.reference)), friendlyRef(stringValue(node.continuation_id))].filter(Boolean).join(" · "),
        ])}
      />
    </div>
  );
}

function taskStatusCounts(tasks: TaskRow[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const task of tasks) {
    const status = String(task.status ?? "unknown");
    counts.set(status, (counts.get(status) ?? 0) + 1);
  }
  return counts;
}

function taskMatchesGraphFilter(task: TaskRow, filter: GraphFilter): boolean {
  if (filter === "all") {
    return true;
  }
  const status = statusToken(task.status);
  if (filter === "attention") {
    return status === "failed" || status === "blocked" || status === "waiting-approval" || status === "waiting-input";
  }
  if (filter === "active") {
    return status === "running" || status === "runnable" || status === "needs-validation";
  }
  return status === "done" || status === "cancelled";
}

function TaskSummary({ snapshot, counts }: { snapshot: ComposedGoalSnapshot; counts: Map<string, number> }) {
  const entries = sortedStatusEntries(counts);
  const computeGraph = workflowComputeGraph(snapshot);
  const progress = workflowProgress(snapshot);
  const ranking = progress?.ranking as Record<string, unknown> | undefined;
  const latestRanking = ranking?.latest_decision as Record<string, unknown> | undefined;
  const graphNodeCount = Array.isArray(computeGraph?.nodes) ? computeGraph.nodes.length : 0;
  const graphEdgeCount = Array.isArray(computeGraph?.edges) ? computeGraph.edges.length : 0;
  const openThunkCount = Number(computeGraph?.open_thunks ?? 0);
  const voteCount = Number(ranking?.vote_count ?? 0);
  const rankingScore = Number(ranking?.score ?? 0);
  const upvotes = Number(ranking?.upvotes ?? 0);
  const downvotes = Number(ranking?.downvotes ?? 0);
  const openMechanismRounds = Number(progress?.open_mechanism_rounds ?? 0);
  const ratificationRounds = Number(progress?.ratification_required_mechanism_rounds ?? 0);
  return (
    <div className="summary-row">
      {entries.map(([status, count]) => (
        <span key={status} className={clsx("status-pill", statusTone(status))}>{status}: {count}</span>
      ))}
      {voteCount > 0 && (
        <span className={clsx("status-pill", rankingScore >= 0 ? "status-runnable" : "status-blocked")}>
          ranking: {rankingScore >= 0 ? "+" : ""}{rankingScore} · {upvotes} up · {downvotes} down
          {latestRanking?.outcome ? ` · ${String(latestRanking.outcome)}` : ""}
        </span>
      )}
      {(openMechanismRounds > 0 || ratificationRounds > 0) && (
        <span className={clsx("status-pill", ratificationRounds > 0 ? "status-waiting-approval" : "status-running")}>
          mechanisms: {openMechanismRounds} open · {ratificationRounds} ratify
        </span>
      )}
      {(graphNodeCount > 0 || graphEdgeCount > 0 || openThunkCount > 0) && (
        <span className={clsx("status-pill", openThunkCount > 0 ? "status-waiting-input" : "status-done")}>
          compute graph: {graphNodeCount} nodes · {graphEdgeCount} edges · {openThunkCount} thunks
        </span>
      )}
      <InspectButton title="Operator goal detail" payload={snapshot} />
    </div>
  );
}

function ActionNeededPanel({ items }: { items: ActionNeededItem[] }) {
  if (!items.length) {
    return null;
  }
  return (
    <section className="evidence-card action-needed-panel" aria-label="Action needed">
      <div className="section-heading">
        <h3>Action needed</h3>
        <span className="muted-small">{items.length} waiting</span>
      </div>
      <OperatorActionList items={items.slice(0, 5)} compact />
    </section>
  );
}

function BlockerInsightPanel({ items, onOpenControls }: { items: ActionNeededItem[]; onOpenControls: () => void }) {
  const blockers = items.filter((item) => {
    const status = statusToken(item.status);
    return ["failed", "blocked", "waiting-approval", "waiting-input"].includes(status) || item.kind === "thunk" || item.kind === "approval";
  });
  if (!blockers.length) {
    return (
      <section className="blocker-panel stable" aria-label="Why blocked">
        <div>
          <span className="goal-context-kicker">Why blocked</span>
          <strong>No blockers projected</strong>
          <small>The coordinator has not projected failed, blocked, approval, or waiting-input work for this goal.</small>
        </div>
      </section>
    );
  }
  return (
    <section className="blocker-panel" aria-label="Why blocked">
      <div className="section-heading">
        <div>
          <span className="goal-context-kicker">Why blocked</span>
          <h3>{blockers.length} item{blockers.length === 1 ? "" : "s"} need attention</h3>
        </div>
        <button type="button" className="secondary-button" onClick={onOpenControls}>
          <ShieldCheck size={15} />
          Controls
        </button>
      </div>
      <div className="blocker-list">
        {blockers.slice(0, 4).map((item) => (
          <article key={`blocker-${item.key}`} className="blocker-card">
            <span className={clsx("status-pill", statusTone(item.status))}>{statusLabel(item.status)}</span>
            <strong>{item.label}</strong>
            <p>{blockerReason(item)}</p>
            <div className="blocker-meta">
              <span>{blockerActionText(item)}</span>
              {item.taskId && <small title={item.taskId}>task {friendlyRef(item.taskId)}</small>}
              {item.approvalId && <small title={item.approvalId}>approval {friendlyRef(item.approvalId)}</small>}
              {item.thunkId && <small title={item.thunkId}>continuation {friendlyRef(item.thunkId)}</small>}
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

function blockerReason(item: ActionNeededItem): string {
  if (item.detail) {
    return item.detail;
  }
  const status = statusToken(item.status);
  if (item.kind === "approval" || status === "waiting-approval") {
    return item.risk ? `Approval is waiting because risk was classified as ${item.risk}.` : "A human approval gate is waiting.";
  }
  if (isResumableThunkItem(item)) {
    return "A delayed compute continuation is waiting for operator input before the task graph can resume.";
  }
  if (status === "waiting-input") {
    return "The task is waiting for input, but no resumable thunk is projected yet.";
  }
  if (status === "failed") {
    return "The last task attempt failed and needs retry or replanning.";
  }
  return "The task is blocked and needs recovery, a dependency, or operator input.";
}

function blockerActionText(item: ActionNeededItem): string {
  const affordance = actionAffordanceForItem(item);
  if (affordance === "approve") return "Approve the gate in the Action queue.";
  if (affordance === "resume") return "Provide the missing input and resume the continuation.";
  const status = statusToken(item.status);
  if (status === "failed") return "Retry failed work or open operator actions for a different recovery path.";
  if (status === "blocked") return "Retry blocked work or open operator actions to replan.";
  if (status === "waiting-input") return "Create a human prompt or replan; no continuation can resume yet.";
  return "Review the item and choose a recovery action.";
}

function EvidenceNextActionPanel({ snapshot, counts, taskCount, compact = false }: { snapshot: ComposedGoalSnapshot; counts: Map<string, number>; taskCount: number; compact?: boolean }) {
  const highlights = evidenceHighlights(snapshot, counts, taskCount);
  const nextAction = nextActionSummary(counts, taskCount, snapshot);
  return (
    <div className={clsx("evidence-next-panel", compact && "compact")}>
      <section className="evidence-card">
        <div className="section-heading">
          <h3>Evidence</h3>
        </div>
        <ul className="evidence-list">
          {highlights.map((item) => (
            <li key={item.label}>
              <span>{item.label}</span>
              <strong className={clsx("operator-state-pill", stateTone(item.state))}>{item.value}</strong>
            </li>
          ))}
        </ul>
      </section>
      <section className="evidence-card next-action-card">
        <div className="section-heading">
          <h3>Next action</h3>
          <span className={clsx("operator-state-pill", stateTone(nextAction.state))}>{nextAction.stateLabel}</span>
        </div>
        <strong>{nextAction.title}</strong>
        <p>{nextAction.detail}</p>
      </section>
    </div>
  );
}

function evidenceHighlights(snapshot: ComposedGoalSnapshot, counts: Map<string, number>, taskCount: number): Array<{ label: string; value: string; state: OperatorStateKey }> {
  const computeGraph = workflowComputeGraph(snapshot);
  const progress = workflowProgress(snapshot);
  const nodeCount = Array.isArray(computeGraph?.nodes) ? computeGraph.nodes.length : 0;
  const openThunks = Number(computeGraph?.open_thunks ?? 0) || countForStatusToken(counts, "waiting-input");
  const reviewing = countForStatusToken(counts, "needs-validation");
  const satisfied = countForStatusToken(counts, "done");
  const running = countForStatusToken(counts, "running") + countForStatusToken(counts, "runnable");
  const rounds = Number(progress?.open_mechanism_rounds ?? 0);
  return [
    { label: "Task states", value: `${taskCount} visible`, state: running > 0 ? "running" : "waiting" },
    { label: "Satisfied evidence", value: `${satisfied} accepted`, state: "satisfied" },
    { label: "Review evidence", value: `${reviewing} checks`, state: reviewing > 0 ? "reviewing" : "satisfied" },
    { label: "Wait evidence", value: `${openThunks} continuations`, state: openThunks > 0 ? "waiting" : "satisfied" },
    { label: "Compute graph", value: `${nodeCount} nodes`, state: nodeCount > 0 ? "running" : "waiting" },
    { label: "Mechanisms", value: `${rounds} open`, state: rounds > 0 ? "action-needed" : "satisfied" },
  ];
}

function nextActionSummary(counts: Map<string, number>, taskCount: number, snapshot: ComposedGoalSnapshot): { title: string; detail: string; state: OperatorStateKey; stateLabel: string } {
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = countForStatusToken(counts, "waiting-input") || Number(workflowComputeGraph(snapshot)?.open_thunks ?? 0);
  const reviewing = countForStatusToken(counts, "needs-validation");
  const running = countForStatusToken(counts, "running");
  const runnable = countForStatusToken(counts, "runnable");
  const satisfied = countForStatusToken(counts, "done");
  if (failed + blocked + approvals > 0) {
    return {
      title: "Review action-needed work",
      detail: `${failed} failed · ${blocked} blocked · ${approvals} approvals`,
      state: "action-needed",
      stateLabel: "Action needed",
    };
  }
  if (continuations > 0) {
    return {
      title: "Resume waiting continuation",
      detail: `${continuations} waiting continuations can accept operator input.`,
      state: "waiting",
      stateLabel: "Waiting",
    };
  }
  if (reviewing > 0) {
    return {
      title: "Review evidence",
      detail: `${reviewing} validation checks are ready for satisfaction review.`,
      state: "reviewing",
      stateLabel: "Reviewing",
    };
  }
  if (running > 0) {
    return {
      title: "Monitor running work",
      detail: `${running} active tasks are producing evidence.`,
      state: "running",
      stateLabel: "Running",
    };
  }
  if (runnable > 0) {
    return {
      title: "Dispatch runnable frontier",
      detail: `${runnable} tasks are ready for an available runner.`,
      state: "running",
      stateLabel: "Running",
    };
  }
  if (taskCount > 0 && satisfied >= taskCount) {
    return {
      title: "Confirm satisfaction",
      detail: "Accepted evidence covers the visible task graph.",
      state: "satisfied",
      stateLabel: "Satisfied",
    };
  }
  return {
    title: "Refresh projection",
    detail: "Goal state is syncing from the coordinator.",
    state: "waiting",
    stateLabel: "Waiting",
  };
}

function GraphStatusPanel({ counts, taskCount }: { counts: Map<string, number>; taskCount: number }) {
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = countForStatusToken(counts, "waiting-input");
  const running = countForStatusToken(counts, "running");
  const done = countForStatusToken(counts, "done");
  const attention = failed + blocked + approvals + continuations;
  return (
    <div className="graph-status-panel" aria-label="Work graph status">
      <div className={clsx("graph-attention", attention > 0 ? "needs-attention" : "stable")}>
        <strong>{attention > 0 ? `${attention} need attention` : "All tasks steady"}</strong>
        <span>{taskCount} tasks · {running} running · {done} done</span>
      </div>
      <OperatorStateStrip counts={counts} />
      <details className="legend-details">
        <summary>Status legend</summary>
        <div className="status-legend" aria-label="Graph legend">
          {statusLegend.map((item) => (
            <span key={item.token} className="legend-item">
              <span className={clsx("legend-swatch", `status-${item.token}`)} />
              <span>
                <strong>{item.label}</strong>
                <small>{item.detail}</small>
              </span>
            </span>
          ))}
        </div>
      </details>
    </div>
  );
}

function OperatorStateStrip({ counts }: { counts: Map<string, number> }) {
  return (
    <div className="operator-state-row" aria-label="Operator state labels">
      {operatorStateDefinitions.map((definition) => {
        const count = definition.statuses.reduce((sum, status) => sum + countForStatusToken(counts, status), 0);
        return (
          <span key={definition.key} className={clsx("operator-state-chip", stateTone(definition.key), count > 0 && "active")}>
            <strong>{definition.label}</strong>
            <small>{count} · {definition.detail}</small>
          </span>
        );
      })}
    </div>
  );
}

function sortedStatusEntries(counts: Map<string, number>): Array<[string, number]> {
  return [...counts.entries()].sort(([left], [right]) => {
    const leftPriority = statusPriority.get(statusToken(left)) ?? 99;
    const rightPriority = statusPriority.get(statusToken(right)) ?? 99;
    return leftPriority - rightPriority || left.localeCompare(right);
  });
}

function countForStatusToken(counts: Map<string, number>, token: string): number {
  let total = 0;
  for (const [status, count] of counts) {
    if (statusToken(status) === token) {
      total += count;
    }
  }
  return total;
}

function workflowComputeGraph(snapshot: ComposedGoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_compute_graph as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

function taskRowsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot | JsonRecord): TaskRow[] {
  if (!snapshot) {
    return [];
  }
  const agentRows = Array.isArray(snapshot.agent_activity) ? snapshot.agent_activity.filter(isRecord) as TaskRow[] : [];
  if (agentRows.length) {
    return agentRows;
  }
  const taskRows = rowsFrom(at(snapshot, ["tasks", "data", "tasks"]) ?? at(snapshot, ["tasks", "tasks"]) ?? at(snapshot, ["tasks", "data"]) ?? snapshot.tasks);
  return taskRows as TaskRow[];
}

function actionNeededItemsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot | JsonRecord, selectedGoalId = ""): ActionNeededItem[] {
  if (!snapshot) {
    return [];
  }
  const continuationRows = continuationRowsFromComposedSnapshot(snapshot as ComposedGoalSnapshot);
  const continuationTaskIds = new Set(continuationRows.map((row) => row.taskId).filter(Boolean));
  const taskRows = taskRowsFromComposedSnapshot(snapshot).filter((task) => {
    const status = statusToken(task.status ?? at(task, ["payload_json", "status"]));
    const id = taskId(task as TaskRow) || stringValue(task.id);
    return !(status === "waiting-input" && id && continuationTaskIds.has(id));
  });
  return mergeActionNeededItems([
    ...actionNeededItemsFromApprovals(rowsFrom(at(snapshot, ["approvals", "data", "approvals"]) ?? at(snapshot, ["approvals", "approvals"]) ?? at(snapshot, ["approvals", "data"]) ?? snapshot.approvals), selectedGoalId),
    ...actionNeededItemsFromTasks(taskRows, selectedGoalId),
    ...continuationRows.map((row) => actionItemFromContinuation(selectedGoalId || stringValue(snapshot.goal_id), row)),
  ]);
}

function queueItemsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot | JsonRecord, selectedGoalId = ""): ActionNeededItem[] {
  if (!snapshot) {
    return [];
  }
  return mergeActionNeededItems([
    ...actionNeededItemsFromComposedSnapshot(snapshot, selectedGoalId),
    ...cancelledItemsFromComposedSnapshot(snapshot as ComposedGoalSnapshot, selectedGoalId),
  ]);
}

function actionNeededItemsFromOperatorActions(value?: unknown): ActionNeededItem[] {
  return rowsFrom(value).map((row, index) => {
    const actionId = stringValue(row.action_id) || `operator-action:${index}`;
    const goalId = stringValue(row.goal_id);
    const taskId = stringValue(row.task_id);
    const kind = stringValue(row.kind);
    const status = normalizeStatus(row.status) || "pending";
    const approval = isRecord(row.approval) ? row.approval : {};
    const thunk = isRecord(row.thunk) ? row.thunk : {};
    const approvalId = stringValue(approval.approval_id) || (kind === "resolve_approval" ? actionId.split(":").at(-1) ?? "" : "");
    const thunkId = stringValue(thunk.id) || stringValue(thunk.thunk_id) || (kind === "resume_thunk" ? actionId.split(":").at(-1) ?? "" : "");
    const derivedKind: ActionNeededKind = kind === "resolve_approval"
      ? "approval"
      : kind === "resume_thunk"
        ? "thunk"
        : status === "cancelled"
          ? "cancelled"
          : status === "waiting_input" || status === "waiting-input"
            ? "waiting-task"
            : "blocked-task";
    return {
      actionId,
      key: actionId,
      kind: derivedKind,
      label: stringValue(row.title) || stringValue(row.question) || "Action needed",
      status,
      detail: stringValue(row.question) || stringValue(at(row, ["payload_json", "reason"])) || "Operator action projected by the backend.",
      requestedInput: stringValue(row.question) || stringValue(thunk.requested_input) || "",
      goalId,
      taskId,
      approvalId,
      thunkId,
      continuationId: stringValue(at(thunk, ["continuation", "continuation_id"])),
      risk: stringValue(approval.risk),
      actionLabel: derivedKind === "approval" ? "Approve" : derivedKind === "thunk" ? "Continue" : "Review",
    };
  });
}

function actionNeededItemsFromApprovals(rows: JsonRecord[], selectedGoalId = ""): ActionNeededItem[] {
  return rows.filter((row) => {
    const status = normalizeStatus(row.status) || "pending";
    return status === "pending" || status === "waiting-approval";
  }).map((row, index) => {
    const approvalId = stringValue(row.approval_id) || stringValue(row.id) || stringValue(row.approval_ref);
    const goalId = stringValue(row.goal_id) || selectedGoalId;
    const risk = stringValue(row.risk) || stringValue(at(row, ["payload_json", "risk"]));
    const action = stringValue(row.requested_action) || stringValue(row.reason) || stringValue(at(row, ["payload_json", "requested_action"])) || stringValue(at(row, ["payload_json", "reason"])) || risk || "Review approval request";
    return {
      actionId: approvalId && goalId ? `approval:${goalId}:${approvalId}` : "",
      key: approvalId ? `approval:${approvalId}` : `approval:${goalId}:${index}`,
      kind: "approval" as const,
      label: action,
      status: normalizeStatus(row.status) || "pending",
      detail: risk ? `Risk: ${risk}` : "Human approval requested.",
      requestedInput: stringValue(row.requested_input) || stringValue(row.question) || stringValue(at(row, ["payload_json", "requested_input"])) || stringValue(at(row, ["payload_json", "question"])),
      goalId,
      taskId: stringValue(row.task_id) || stringValue(at(row, ["payload_json", "task_id"])),
      approvalId,
      thunkId: "",
      continuationId: "",
      risk,
      actionLabel: "Approve",
    };
  });
}

function actionNeededItemsFromTasks(rows: JsonRecord[], selectedGoalId = ""): ActionNeededItem[] {
  return rows
    .filter((task) => taskNeedsOperatorAttention(task))
    .map((task, index) => {
      const status = normalizeStatus(task.status ?? at(task, ["payload_json", "status"]));
      const id = taskId(task as TaskRow) || stringValue(task.id);
      const goalId = stringValue(task.goal_id) || stringValue(at(task, ["payload_json", "goal_id"])) || selectedGoalId;
      const title = taskTitle(task) || (id ? `Task ${friendlyRef(id)}` : "Task needs operator action");
      return {
        actionId: id && goalId ? `task:${goalId}:${id}` : "",
        key: id ? `task:${id}` : `task:${goalId}:${index}`,
        kind: status === "blocked" || status === "failed" ? "blocked-task" as const : "waiting-task" as const,
        label: title,
        status,
        detail: taskDetail(task, status),
        requestedInput: stringValue(task.requested_input) || stringValue(task.question) || stringValue(at(task, ["payload_json", "requested_input"])) || stringValue(at(task, ["payload_json", "question"])),
        goalId,
        taskId: id,
        approvalId: stringValue(task.approval_id) || stringValue(task.approval_ref) || stringValue(at(task, ["payload_json", "approval_id"])) || stringValue(at(task, ["payload_json", "approval_ref"])),
        thunkId: "",
        continuationId: "",
        risk: "",
        actionLabel: status === "waiting-approval" ? "Approve" : status === "waiting-input" ? "Create prompt" : "Review",
      };
    });
}

function actionNeededItemsFromThunks(snapshot: ComposedGoalSnapshot, selectedGoalId = ""): ActionNeededItem[] {
  return continuationRowsFromComposedSnapshot(snapshot).map((row) => actionItemFromContinuation(selectedGoalId || stringValue(snapshot.goal_id), row));
}

function cancelledItemsFromComposedSnapshot(snapshot: ComposedGoalSnapshot, selectedGoalId = ""): ActionNeededItem[] {
  const taskItems = taskRowsFromComposedSnapshot(snapshot)
    .filter((task) => statusToken(task.status ?? at(task, ["payload_json", "status"])) === "cancelled")
    .map((task, index) => {
      const id = taskId(task as TaskRow) || stringValue(task.id);
      const goalId = stringValue(at(task, ["goal_id"])) || stringValue(at(task, ["payload_json", "goal_id"])) || selectedGoalId || stringValue(snapshot.goal_id);
      return {
        actionId: id && goalId ? `task:${goalId}:${id}` : "",
        key: id ? `cancelled-task:${id}` : `cancelled-task:${goalId}:${index}`,
        kind: "cancelled" as const,
        label: taskTitle(task) || (id ? `Cancelled task ${friendlyRef(id)}` : "Cancelled task"),
        status: "cancelled",
        detail: taskDetail(task, "cancelled") || "Cancelled task retained for operator queue history.",
        requestedInput: "",
        goalId,
        taskId: id,
        approvalId: "",
        thunkId: "",
        continuationId: "",
        risk: "",
        actionLabel: "Cancelled",
      };
    });
  const thunkItems = continuationRowsFromComposedSnapshot(snapshot, true)
    .filter((row) => row.status === "cancelled")
    .map((row) => ({
      ...actionItemFromContinuation(selectedGoalId || stringValue(snapshot.goal_id), row),
      key: `cancelled-thunk:${row.thunkId}`,
      kind: "cancelled" as const,
      status: "cancelled",
      actionLabel: "Cancelled",
      detail: [row.waitKind ? `Wait: ${row.waitKind}` : "", row.waitReference ? `Ref ${friendlyRef(row.waitReference)}` : "", "Cancelled continuation retained for operator queue history."].filter(Boolean).join(" · "),
    }));
  return [...taskItems, ...thunkItems];
}

function mergeActionNeededItems(items: ActionNeededItem[]): ActionNeededItem[] {
  const seen = new Set<string>();
  const merged: ActionNeededItem[] = [];
  for (const item of items) {
    const key = item.key || `${item.kind}:${item.goalId}:${item.taskId}:${item.label}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push({ ...item, key });
  }
  return merged.sort((left, right) => actionNeededPriority(left) - actionNeededPriority(right) || left.label.localeCompare(right.label));
}

function actionNeededPriority(item: ActionNeededItem): number {
  const status = statusToken(item.status);
  if (item.kind === "cancelled" || status === "cancelled") return 4;
  if (item.kind === "approval" || status === "waiting-approval") return 0;
  if (status === "blocked" || status === "failed") return 1;
  if (item.kind === "thunk" || status === "waiting-input") return 2;
  return 3;
}

function taskNeedsOperatorAttention(task: JsonRecord): boolean {
  return ["blocked", "failed", "waiting-approval", "waiting-input"].includes(statusToken(task.status ?? at(task, ["payload_json", "status"])));
}

function taskTitle(task: JsonRecord): string {
  return stringValue(task.title) || stringValue(task.current_prompt) || stringValue(task.prompt) || stringValue(at(task, ["payload_json", "title"])) || stringValue(at(task, ["payload_json", "prompt"]));
}

function taskDetail(task: JsonRecord, status: string): string {
  const role = stringValue(task.role) || stringValue(at(task, ["payload_json", "role"]));
  const subgoal = stringValue(task.subgoal_id) || stringValue(at(task, ["payload_json", "subgoal_id"]));
  const reason = taskBlockerReason(task);
  return [statusLabel(status), role, subgoal ? `subgoal ${subgoal}` : "", reason].filter(Boolean).join(" · ");
}

function taskBlockerReason(task: JsonRecord): string {
  const payload = isRecord(task.payload_json) ? task.payload_json : {};
  const result = isRecord(task.result) ? task.result : isRecord(payload.result) ? payload.result : {};
  const candidates = [
    task.reason,
    task.blocker,
    task.blocked_reason,
    task.blocker_reason,
    task.error,
    task.last_error,
    task.summary,
    at(task, ["payload_json", "reason"]),
    at(task, ["payload_json", "blocker"]),
    at(task, ["payload_json", "blocked_reason"]),
    at(task, ["payload_json", "blocker_reason"]),
    at(task, ["payload_json", "error"]),
    at(task, ["payload_json", "last_error"]),
    at(task, ["payload_json", "summary"]),
    result.reason,
    result.error,
    result.summary,
    result.blocker,
    result.blocked_reason,
  ];
  for (const candidate of candidates) {
    const value = stringValue(candidate);
    if (value) {
      return value;
    }
  }
  return "";
}

function continuationRowsFromComposedSnapshot(snapshot?: ComposedGoalSnapshot, includeClosed = false): ContinuationRow[] {
  if (!snapshot) {
    return [];
  }
  const rowVisible = (row: ContinuationRow | null): row is ContinuationRow => {
    if (!row) {
      return false;
    }
    return includeClosed || isOpenContinuationRow(row);
  };
  const graph = workflowComputeGraph(snapshot);
  const nodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  const thunkRecords = delayedThunkRecordsByIdFromComposedSnapshot(snapshot);
  const graphRows = nodes
    .filter(isRecord)
    .filter((node) => normalizeStatus(node.kind) === "delayed-compute-thunk")
    .map((node) => continuationRowFromNode(node as ComputeGraphNode, thunkRecords.get(stringValue(node.thunk_id))))
    .filter(rowVisible);
  const graphThunkIds = new Set(graphRows.map((row) => row.thunkId));
  const recordRows = [...thunkRecords.values()]
    .filter((record) => !graphThunkIds.has(stringValue(record.id)))
    .map(continuationRowFromThunkRecord)
    .filter(rowVisible);
  return [...graphRows, ...recordRows];
}

function delayedThunkRecordsByIdFromComposedSnapshot(snapshot: ComposedGoalSnapshot): Map<string, JsonRecord> {
  const rows = rowsFrom(
    at(snapshot, ["workflow_status", "data", "delayed_compute_thunks"])
      ?? at(snapshot, ["workflow_status", "delayed_compute_thunks"])
      ?? at(snapshot, ["workflow_progress", "data", "delayed_compute_thunks"])
      ?? at(snapshot, ["workflow_progress", "delayed_compute_thunks"]),
  );
  return new Map(
    rows
      .map((row): [string, JsonRecord] => [stringValue(row.id), row])
      .filter(([id]) => Boolean(id)),
  );
}

function continuationRowFromNode(node: ComputeGraphNode, record?: JsonRecord): ContinuationRow | null {
  const thunkId = stringValue(node.thunk_id);
  if (!thunkId) {
    return null;
  }
  const waitRef = isRecord(record?.wait_ref) ? record.wait_ref : isRecord(node.wait_ref) ? node.wait_ref : {};
  const continuation = isRecord(record?.continuation) ? record.continuation : {};
  return {
    key: thunkId,
    thunkId,
    continuationId: stringValue(node.continuation_id) || stringValue(continuation.continuation_id),
    taskId: stringValue(node.task_id) || stringValue(record?.task_id),
    reason: stringValue(record?.reason) || stringValue(node.label) || "Waiting for input",
    requestedInput: stringValue(record?.requested_input) || stringValue(at(node, ["requested_input"])) || stringValue(at(node, ["payload_json", "requested_input"])) || stringValue(at(node, ["wait_ref", "requested_input"])),
    status: continuationStatus(record?.status ?? node.status),
    waitKind: stringValue(waitRef.kind),
    waitReference: stringValue(waitRef.reference),
  };
}

function continuationRowFromThunkRecord(record: JsonRecord): ContinuationRow | null {
  const thunkId = stringValue(record.id);
  if (!thunkId) {
    return null;
  }
  const waitRef = isRecord(record.wait_ref) ? record.wait_ref : {};
  const continuation = isRecord(record.continuation) ? record.continuation : {};
  return {
    key: thunkId,
    thunkId,
    continuationId: stringValue(continuation.continuation_id),
    taskId: stringValue(record.task_id),
    reason: stringValue(record.reason) || "Waiting for input",
    requestedInput: stringValue(record.requested_input),
    status: continuationStatus(record.status || "waiting-input"),
    waitKind: stringValue(waitRef.kind),
    waitReference: stringValue(waitRef.reference),
  };
}

function isOpenContinuationRow(row: ContinuationRow): boolean {
  return !["resumed", "cancelled", "expired", "done"].includes(row.status);
}

function continuationStatus(value: unknown): string {
  const status = normalizeStatus(value);
  return status === "pending" ? "waiting-input" : status;
}

function actionItemFromContinuation(goalId: string, row: ContinuationRow): ActionNeededItem {
  return {
    actionId: row.thunkId && goalId ? `thunk:${goalId}:${row.thunkId}` : "",
    key: `thunk:${row.thunkId}`,
    kind: "thunk",
    label: row.reason || "Human prompt",
    status: row.status || "waiting-input",
    detail: [row.waitKind ? `Wait: ${row.waitKind}` : "", row.waitReference ? `Ref ${friendlyRef(row.waitReference)}` : ""].filter(Boolean).join(" · ") || "Delayed compute continuation is paused.",
    requestedInput: row.requestedInput || row.reason || "Continue this work, or add the missing context.",
    goalId,
    taskId: row.taskId,
    approvalId: "",
    thunkId: row.thunkId,
    continuationId: row.continuationId,
    risk: "",
    actionLabel: "Continue",
  };
}

function ContinuationQueue({ goalId, snapshot }: { goalId: string; snapshot?: ComposedGoalSnapshot }) {
  const queryClient = useQueryClient();
  const rows = continuationRowsFromComposedSnapshot(snapshot);
  const [responses, setResponses] = useState<Record<string, string>>({});
  const actionMutation = useMutation({
    mutationFn: ({ item, responseSummary, intent }: ActionMutationInput) => runOperatorAction(item, responseSummary, intent),
    onSuccess: (value) => {
      applyActionEnvelopeToCache(queryClient, value, goalId);
    },
  });

  if (!rows.length) {
    return <EmptyState title="Continuations clear" detail="Waiting tasks will appear here." />;
  }

  return (
    <div className="continuation-list" aria-label="Continuations">
      <div className="section-heading">
        <h3>Continuations</h3>
        <span className="muted-small">{rows.length} waiting</span>
      </div>
      {rows.map((row) => {
        const item = actionItemFromContinuation(goalId, row);
        const responseSummary = responses[item.key] ?? "";
        return (
          <div key={row.key} className="continuation-card">
            <div className="continuation-copy">
              <strong>{row.reason}</strong>
              <span title={row.thunkId}>{statusLabel(row.status)} · wait {friendlyRef(row.thunkId)}</span>
              {row.taskId && <small title={row.taskId}>task {friendlyRef(row.taskId)}</small>}
              {row.continuationId && <small title={row.continuationId}>continuation {friendlyRef(row.continuationId)}</small>}
              {row.waitReference && <small title={row.waitReference}>{row.waitKind || "wait_ref"} · {friendlyRef(row.waitReference)}</small>}
            </div>
            <HumanPromptCard
              item={item}
              value={responseSummary}
              onChange={(value) => setResponses((current) => ({ ...current, [item.key]: value }))}
              onAction={(nextItem, nextSummary, intent) => actionMutation.mutate({ item: nextItem, responseSummary: nextSummary, intent })}
              pending={actionMutation.isPending}
              compact
            />
          </div>
        );
      })}
    </div>
  );
}

function CompilerControlPanel({ goalId, snapshot, compact = false }: { goalId: string; snapshot?: ComposedGoalSnapshot; compact?: boolean }) {
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
          <h3>Operator actions</h3>
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
          <button type="button" className="secondary-button" disabled={disabled || !thunkReason.trim()} onClick={() => run("create thunk", () => createThunk(goalId, thunkPayload({ goalId, taskId: firstTaskId, kind: thunkKind, reason: thunkReason, requestedInput: thunkInput, timeoutSeconds: thunkTimeoutSeconds })))}>
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

function blockedTaskReplanPayload(item: ActionNeededItem, operatorContext = ""): JsonRecord {
  const subject = item.taskId ? `task ${friendlyRef(item.taskId)}` : "the blocked work";
  const reason = `${statusLabel(item.status)} requires operator recovery: ${item.detail || item.label}`;
  return {
    id: createRunId(),
    goal_id: item.goalId,
    task_id: item.taskId || null,
    operator: "operator",
    message: `Request coordinator-owned recovery for ${subject}.`,
    kind: {
      kind: "inject_task",
      role: "planner",
      prompt: [
        `Re-plan or recover ${subject}.`,
        `Blocked item: ${item.label}.`,
        item.detail ? `Detail: ${item.detail}.` : "",
        operatorContext ? `Operator context: ${operatorContext}.` : "",
        "Return concrete next tasks, evidence requirements, and any human inputs still needed.",
      ].filter(Boolean).join(" "),
      reason,
    },
  };
}

function blockedTaskRecoveryPayload(item: ActionNeededItem): JsonRecord {
  const status = statusToken(item.status);
  const fallbackScope = status === "failed" ? "failed" : "blocked";
  const scope = item.taskId ? "task" : fallbackScope;
  const subject = item.taskId ? `task ${friendlyRef(item.taskId)}` : `${scope} work`;
  return {
    goal_id: item.goalId,
    scope,
    reason: "operator_requested",
    message: `Retry ${subject} from the action queue: ${item.detail || item.label}`,
    task_id: item.taskId || null,
    reset_attempts: false,
    preserve_artifacts: true,
    operator: "operator",
  };
}

function blockedTaskHumanPromptPayload(item: ActionNeededItem, operatorContext = ""): JsonRecord {
  const subject = item.taskId ? `task ${friendlyRef(item.taskId)}` : "blocked work";
  const requestedInput = operatorContext
    || item.requestedInput
    || item.detail
    || `What operator input is required to unblock ${subject}?`;
  return thunkPayload({
    goalId: item.goalId,
    taskId: item.taskId,
    kind: "human_input",
    reason: `Operator requested a human prompt for ${subject}: ${item.detail || item.label}`,
    requestedInput,
    timeoutSeconds: 0,
  });
}

function blockedTaskCancelReason(item: ActionNeededItem, operatorContext = ""): string {
  const subject = item.taskId ? `task ${friendlyRef(item.taskId)}` : "the selected goal";
  return [
    `Operator cancelled the goal from the action queue while reviewing ${subject}.`,
    item.detail ? `Blocked detail: ${item.detail}.` : "",
    operatorContext ? `Operator context: ${operatorContext}.` : "",
  ].filter(Boolean).join(" ");
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

function workflowProgress(snapshot: ComposedGoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_progress as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

function operatorStateForStatus(status: unknown): OperatorStateDefinition {
  const token = statusToken(status);
  return operatorStateDefinitions.find((definition) => definition.statuses.includes(token))
    ?? operatorStateDefinitions.find((definition) => definition.key === "waiting")
    ?? { key: "waiting", label: "Waiting", detail: "projection pending", statuses: ["unknown"] };
}

function stateTone(state: OperatorStateKey): string {
  return `state-${state}`;
}

function statusTone(status: unknown): string {
  const normalized = statusToken(status);
  return `status-${normalized}`;
}

function statusColorVar(status: unknown): string {
  return `var(--${statusTone(status)})`;
}

function statusToken(status: unknown): string {
  const normalized = normalizeStatus(status);
  return knownStatusTones.has(normalized) ? normalized : "unknown";
}

function normalizeStatus(status: unknown): string {
  return String(status ?? "unknown")
    .trim()
    .toLowerCase()
    .replace(/_/g, "-")
    .replace(/[^a-z0-9-]/g, "") || "unknown";
}

function statusLabel(status: unknown): string {
  return normalizeStatus(status).split("-").map((part) => part ? `${part[0]?.toUpperCase()}${part.slice(1)}` : part).join(" ");
}

function goalOperatorState(goal: GoalRow): OperatorStateDefinition {
  const failed = numberValue(goal.failed_tasks) ?? 0;
  const blocked = numberValue(goal.blocked_tasks) ?? 0;
  if (failed > 0 || blocked > 0) {
    return operatorStateDefinitions.find((definition) => definition.key === "action-needed") ?? operatorStateForStatus("blocked");
  }
  return operatorStateForStatus(goal.status);
}

function goalNextAction(goal: GoalRow): string {
  const failed = numberValue(goal.failed_tasks) ?? 0;
  const blocked = numberValue(goal.blocked_tasks) ?? 0;
  const openTasks = numberValue(goal.open_tasks) ?? 0;
  const state = goalOperatorState(goal);
  if (failed > 0 || blocked > 0) {
    return "Review blockers";
  }
  if (state.key === "reviewing") {
    return "Review evidence";
  }
  if (state.key === "running") {
    return "Monitor active work";
  }
  if (state.key === "satisfied") {
    return "Confirm satisfaction";
  }
  return openTasks > 0 ? "Dispatch or wait" : "Refresh projection";
}

function friendlyRef(value: unknown): string {
  const ref = shortRef(value);
  return ref ? `Ref ${ref}` : "";
}

function shortRef(value: unknown): string {
  const raw = stringValue(value);
  if (!raw) {
    return "";
  }
  const uuid = raw.match(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i)?.[0];
  const segments = raw.split(/[/:#?]+/).filter(Boolean);
  const candidate = uuid ?? segments[segments.length - 1] ?? raw;
  if (candidate.length <= 16) {
    return candidate;
  }
  return `${candidate.slice(0, 8)}...${candidate.slice(-4)}`;
}

function safeTestId(value: unknown): string {
  return (shortRef(value) || "item").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
}

function subgoalId(subgoal: JsonRecord): string {
  return stringValue(subgoal.id) || stringValue(subgoal.subgoal_id);
}

function subgoalTitle(subgoal: JsonRecord, index: number): string {
  return stringValue(subgoal.title) || stringValue(subgoal.name) || friendlyRef(subgoalId(subgoal)) || `Subgoal ${index + 1}`;
}

function subgoalObjective(subgoal: JsonRecord): string {
  return stringValue(subgoal.objective) || stringValue(subgoal.summary) || stringValue(subgoal.description) || "Objective pending.";
}

function textList(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.map((item) => {
      if (typeof item === "string") {
        return item.trim();
      }
      if (isRecord(item)) {
        return stringValue(item.title) || stringValue(item.description) || stringValue(item.summary);
      }
      return String(item ?? "").trim();
    }).filter(Boolean);
  }
  const text = stringValue(value);
  return text ? [text] : [];
}

function MemoryView({ selectedGoalId }: { selectedGoalId: string }) {
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const [note, setNote] = useState("");
  const [result, setResult] = useState<unknown>(null);
  const [replaceKeysText, setReplaceKeysText] = useState("");
  const [replacementKey, setReplacementKey] = useState("");
  const [replacementTitle, setReplacementTitle] = useState("");
  const [replacementContent, setReplacementContent] = useState("");
  const [replacementReason, setReplacementReason] = useState("");
  const [replacementTagsText, setReplacementTagsText] = useState("operator, reviewed");
  const [previewResult, setPreviewResult] = useState<unknown>(null);
  useEffect(() => {
    setPreviewResult(null);
  }, [selectedGoalId]);
  const memoryEventsQuery = useQuery({
    queryKey: ["memory-events", selectedGoalId],
    queryFn: () => memoryEvents(selectedGoalId),
    enabled: Boolean(selectedGoalId),
  });
  const editPayload = () => {
    if (!selectedGoalId) {
      throw new Error("Select a goal.");
    }
    return memoryEditPayload({
      goalId: selectedGoalId,
      replaceKeys: tokenList(replaceKeysText),
      replacementKey,
      replacementTitle,
      replacementContent,
      replacementReason,
      replacementTags: tokenList(replacementTagsText),
    });
  };
  const searchMutation = useMutation({
    mutationFn: () => memorySearch({ goal_id: selectedGoalId || undefined, query, limit: 8 }),
    onSuccess: setResult,
  });
  const contextMutation = useMutation({
    mutationFn: () => memoryContext({ goal_id: selectedGoalId || undefined, query, limit: 8 }),
    onSuccess: setResult,
  });
  const writeMutation = useMutation({
    mutationFn: () => memoryWrite({
      goal_id: selectedGoalId || undefined,
      scope: selectedGoalId ? "goal" : "global",
      kind: "operator_note",
      text: note,
      tags: ["operator", "dashboard"],
    }),
    onSuccess: (value) => {
      setResult(value);
      void queryClient.invalidateQueries({ queryKey: ["memory-events", selectedGoalId] });
    },
  });
  const previewMutation = useMutation({
    mutationFn: () => memoryEditPreview(editPayload()),
    onSuccess: setPreviewResult,
  });
  const editMutation = useMutation({
    mutationFn: () => memoryEdit({
      ...editPayload(),
      task_id: null,
      scope: "goal",
      store: null,
    }),
    onSuccess: (value) => {
      setResult(value);
      void queryClient.invalidateQueries({ queryKey: ["memory-events", selectedGoalId] });
    },
  });
  const busy = searchMutation.isPending || contextMutation.isPending || writeMutation.isPending || previewMutation.isPending || editMutation.isPending;
  const replacementReady = Boolean(
    selectedGoalId
      && tokenList(replaceKeysText).length
      && replacementTitle.trim()
      && replacementContent.trim()
      && replacementReason.trim(),
  );
  const editError = previewMutation.error ?? editMutation.error;
  return (
    <section className="memory-layout">
      <div className="panel-stack">
        <div className="panel">
          <div className="section-heading">
            <h2>Search shared memory</h2>
            <Search size={18} />
          </div>
          <label>
            Search or context request
            <textarea value={query} onChange={(event) => setQuery(event.target.value)} placeholder="What should the agents remember before continuing?" />
          </label>
          <div className="button-row">
            <button className="primary-button" type="button" disabled={busy} onClick={() => searchMutation.mutate()}>
              Search
            </button>
            <button className="secondary-button" type="button" disabled={busy} onClick={() => contextMutation.mutate()}>
              Build context
            </button>
          </div>
          <label>
            Durable operator note
            <textarea value={note} onChange={(event) => setNote(event.target.value)} placeholder="Write a reviewed fact, constraint, or decision." />
          </label>
          <button className="secondary-button" type="button" disabled={busy || !note.trim()} onClick={() => writeMutation.mutate()}>
            Save memory note
          </button>
        </div>

        <div className="panel">
          <div className="section-heading">
            <h2>Replace memory</h2>
            <span className={clsx("status-pill", selectedGoalId ? "status-running" : "status-pending")}>
              {selectedGoalId ? friendlyRef(selectedGoalId) : "Select goal"}
            </span>
          </div>
          <label>
            Replace keys
            <textarea
              value={replaceKeysText}
              onChange={(event) => {
                setReplaceKeysText(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="memory-key-1, memory-key-2"
            />
          </label>
          <label>
            Replacement key
            <input
              value={replacementKey}
              onChange={(event) => {
                setReplacementKey(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="optional stable key"
            />
          </label>
          <label>
            Replacement title
            <input
              value={replacementTitle}
              onChange={(event) => {
                setReplacementTitle(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="Reviewed decision"
            />
          </label>
          <label>
            Replacement content
            <textarea
              value={replacementContent}
              onChange={(event) => {
                setReplacementContent(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="Reviewed replacement memory."
            />
          </label>
          <label>
            Reason
            <input
              value={replacementReason}
              onChange={(event) => {
                setReplacementReason(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="why the replacement supersedes the old keys"
            />
          </label>
          <label>
            Tags
            <input
              value={replacementTagsText}
              onChange={(event) => {
                setReplacementTagsText(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="operator, reviewed"
            />
          </label>
          <div className="button-row">
            <button className="primary-button" type="button" disabled={busy || !replacementReady} onClick={() => previewMutation.mutate()}>
              Preview diff
            </button>
            <button className="secondary-button" type="button" disabled={busy || !replacementReady || !previewReady(previewResult)} onClick={() => editMutation.mutate()}>
              Apply edit
            </button>
          </div>
          {editError && <span className="error-text">{editError.message}</span>}
        </div>
      </div>
      <div className="panel-stack">
        <div className="panel">
          <div className="section-heading">
            <h2>Memory results</h2>
            <span className="muted-small">Scoped by goal when selected</span>
          </div>
          <ResultList value={result} />
        </div>
        <div className="panel">
          <div className="section-heading">
            <h2>Replacement diff</h2>
            <PreviewStatus value={previewResult} />
          </div>
          <MemoryDiffTable value={previewResult} />
        </div>
        <div className="panel">
          <div className="section-heading">
            <h2>Memory events</h2>
            {Boolean(memoryEventsQuery.data) && (
              <AdvancedInspect summaryLabel="Details" title="Memory events" payload={memoryEventsQuery.data} buttonLabel="Inspect JSON" />
            )}
          </div>
          <MemoryEventsTable selectedGoalId={selectedGoalId} value={memoryEventsQuery.data} loading={memoryEventsQuery.isFetching} />
        </div>
      </div>
    </section>
  );
}

function memoryEditPayload(input: {
  goalId: string;
  replaceKeys: string[];
  replacementKey: string;
  replacementTitle: string;
  replacementContent: string;
  replacementReason: string;
  replacementTags: string[];
}): JsonRecord {
  return {
    goal_id: input.goalId,
    replace_keys: input.replaceKeys,
    replacement_key: input.replacementKey.trim() || null,
    replacement_episode: {
      title: input.replacementTitle.trim(),
      content: input.replacementContent.trim(),
      source: {
        source_type: "human",
        uri: null,
        actor: "operator",
      },
      artifacts: [],
      tags: input.replacementTags,
    },
    reason: input.replacementReason.trim(),
  };
}

function tokenList(value: string): string[] {
  return value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function PreviewStatus({ value }: { value: unknown }) {
  if (!value) {
    return <span className="status-pill muted">Preview pending</span>;
  }
  return (
    <span className={clsx("status-pill", previewReady(value) ? "status-done" : "status-blocked")}>
      {previewReady(value) ? "Ready" : "Blocked"}
    </span>
  );
}

function previewReady(value: unknown): boolean {
  const record = previewRecord(value);
  return Boolean(record?.ready_to_edit);
}

function previewRecord(value: unknown): JsonRecord | null {
  const data = at(value, ["data"]);
  if (isRecord(data)) {
    return data;
  }
  return isRecord(value) ? value : null;
}

function MemoryDiffTable({ value }: { value: unknown }) {
  const record = previewRecord(value);
  if (!record) {
    return <EmptyState title="Preview memory edit" detail="Choose replacement details." />;
  }
  const diffs = rowsFrom(record.diffs);
  const missingKeys = arrayStrings(record.missing_keys);
  return (
    <>
      <div className="summary-row">
        <span className="status-pill">Replacement {String(record.replacement_key ?? "auto key")}</span>
        {missingKeys.length > 0 && <span className="status-pill status-blocked">Missing {missingKeys.join(", ")}</span>}
      </div>
      <SimpleTable
        empty="Diff rows pending."
        headers={["Key", "Before", "After"]}
        rows={diffs.map((row) => [
          String(row.key ?? ""),
          titledExcerpt(row.before_title, row.before_excerpt),
          titledExcerpt(row.after_title, row.after_excerpt),
        ])}
      />
      <div className="summary-row">
        <AdvancedInspect summaryLabel="Details" title="Memory edit preview" payload={record} buttonLabel="Inspect JSON" />
      </div>
    </>
  );
}

function MemoryEventsTable({ selectedGoalId, value, loading }: { selectedGoalId: string; value: unknown; loading: boolean }) {
  if (!selectedGoalId) {
    return <EmptyState title="Select a goal" detail="Memory events are scoped to the current goal." />;
  }
  if (loading && !value) {
    return <EmptyState title="Loading memory events" detail="Fetching memory event history." />;
  }
  const rows = rowsFrom(at(value, ["events"]) ?? value).slice(-10).reverse();
  return (
    <SimpleTable
      empty="Memory events pending."
      headers={["Action", "Key", "Scope", "Summary"]}
      rows={rows.map((row) => [
        String(row.action ?? ""),
        String(row.key ?? ""),
        String(row.scope ?? ""),
        excerpt(row.summary),
      ])}
    />
  );
}

function titledExcerpt(title: unknown, body: unknown): string {
  return [String(title ?? "").trim(), excerpt(body)].filter(Boolean).join(": ");
}

function excerpt(value: unknown): string {
  const text = String(value ?? "").replace(/\s+/g, " ").trim();
  return text.length > 180 ? `${text.slice(0, 177)}...` : text;
}

function arrayStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.map(String).filter(Boolean) : [];
}

function PlansView() {
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

function HumanQueueView({ selectedGoalId, workspace }: { selectedGoalId: string; workspace?: OperatorWorkspaceSnapshot }) {
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
  const actionItems = useMemo(() => {
    const operatorItems = actionNeededItemsFromOperatorActions(operatorActionsQuery.data ?? workspace?.actions);
    const selectedGoalItems = selectedGoalId ? queueItemsFromComposedSnapshot(composedSnapshotFromOperatorGoalDetail(selectedGoalQuery.data), selectedGoalId) : [];
    return mergeActionNeededItems([...operatorItems, ...selectedGoalItems]);
  }, [operatorActionsQuery.data, selectedGoalId, selectedGoalQuery.data, workspace?.actions]);
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

function OperatorActionList({ items, compact = false }: { items: ActionNeededItem[]; compact?: boolean }) {
  const queryClient = useQueryClient();
  const [queueFilter, setQueueFilter] = useState<QueueFilter>("all");
  const [responses, setResponses] = useState<Record<string, string>>({});
  const [lastAction, setLastAction] = useState<{ item: ActionNeededItem; response: unknown } | null>(null);
  const actionMutation = useMutation({
    mutationFn: ({ item, responseSummary, intent }: ActionMutationInput) => runOperatorAction(item, responseSummary, intent),
    onSuccess: (result, variables) => {
      setResponses((current) => {
        const next = { ...current };
        delete next[variables.item.key];
        return next;
      });
      setLastAction({ item: variables.item, response: result });
      applyActionEnvelopeToCache(queryClient, result, variables.item.goalId);
    },
  });
  const showQueueControls = !compact;
  const visibleItems = showQueueControls ? items.filter((item) => queueItemMatchesFilter(item, queueFilter)) : items;
  const groupedItems = showQueueControls ? queueGroupsForItems(visibleItems) : [];
  const renderItem = (item: ActionNeededItem) => {
    const responseSummary = responses[item.key] ?? "";
    if (queueGroupForItem(item) === "cancelled") {
      return <QueueHistoryCard key={item.key} item={item} />;
    }
    return (
      <HumanPromptCard
        key={item.key}
        item={item}
        value={responseSummary}
        onChange={(value) => setResponses((current) => ({ ...current, [item.key]: value }))}
        onAction={(nextItem, nextSummary, intent) => actionMutation.mutate({ item: nextItem, responseSummary: nextSummary, intent })}
        pending={actionMutation.isPending}
        compact={compact}
      />
    );
  };
  return (
    <div className="approval-list">
      {showQueueControls && <QueueFilterBar items={items} active={queueFilter} onChange={setQueueFilter} />}
      {lastAction && <ActionResultCard item={lastAction.item} response={lastAction.response} onClear={() => setLastAction(null)} />}
      {visibleItems.length ? (
        showQueueControls ? groupedItems.map((group) => (
          <section key={group.group} className="queue-group" aria-label={`${queueGroupLabels[group.group]} queue`}>
            <div className="queue-group-heading">
              <strong>{queueGroupLabels[group.group]}</strong>
              <span className="filter-count">{group.items.length} item{group.items.length === 1 ? "" : "s"}</span>
            </div>
            {group.items.map(renderItem)}
          </section>
        )) : visibleItems.map(renderItem)
      ) : (
        <EmptyState
          title={queueFilter === "all" ? "Action queue clear" : `${queueGroupLabels[queueFilter]} queue clear`}
          detail={queueFilter === "all" ? "Approvals, recovery work, continuations, and stopped history appear here." : `No ${queueGroupLabels[queueFilter].toLowerCase()} rows match this filter.`}
        />
      )}
      {actionMutation.error && <span className="error-text">{(actionMutation.error as Error).message}</span>}
    </div>
  );
}

function QueueFilterBar({ items, active, onChange }: { items: ActionNeededItem[]; active: QueueFilter; onChange: (value: QueueFilter) => void }) {
  return (
    <div className="queue-toolbar" aria-label="Action queue filters">
      <div className="graph-filter queue-filter" role="group" aria-label="Queue filter">
        {queueFilterOptions.map((option) => {
          const count = option.key === "all" ? items.length : items.filter((item) => queueItemMatchesFilter(item, option.key)).length;
          return (
            <button
              key={option.key}
              type="button"
              className={clsx("graph-filter-button", active === option.key && "active")}
              aria-pressed={active === option.key}
              title={option.detail}
              data-testid={`queue-filter-${option.key}`}
              onClick={() => onChange(option.key)}
            >
              {option.label}
              <span>{count}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ActionResultCard({ item, response, onClear }: { item: ActionNeededItem; response: unknown; onClear: () => void }) {
  const summary = actionEnvelopeSummary(response);
  if (!summary) {
    return null;
  }
  return (
    <div className="action-result-card">
      <div>
        <strong>{summary.label}</strong>
        <span>{item.label}</span>
        <small>{summary.detail}</small>
        <small>Clearing this notice only updates this browser view; it does not cancel or change the goal.</small>
      </div>
      <div className="action-result-controls">
        <AdvancedInspect summaryLabel="Details" title="Action result and active state" payload={response} buttonLabel="Inspect JSON" />
        <button type="button" className="secondary-button" title="Dismiss this local notice only." onClick={onClear}>
          Clear notice
        </button>
      </div>
    </div>
  );
}

function QueueHistoryCard({ item }: { item: ActionNeededItem }) {
  return (
    <article className="approval-card queue-history-card">
      <div>
        <span className="goal-context-kicker">Stopped history</span>
        <strong>{item.label}</strong>
        <span>{statusLabel(item.status)} · {item.goalId ? friendlyRef(item.goalId) : "goal unknown"}</span>
        <small>{item.detail || "This stopped row is kept for queue history."}</small>
        <small>History rows are read-only; use Cancel goal only on active work.</small>
      </div>
      <span className="status-pill status-cancelled">Read-only</span>
    </article>
  );
}

function queueGroupsForItems(items: ActionNeededItem[]): Array<{ group: QueueGroup; items: ActionNeededItem[] }> {
  return queueGroupOrder
    .map((group) => ({ group, items: items.filter((item) => queueGroupForItem(item) === group) }))
    .filter((group) => group.items.length > 0);
}

function queueItemMatchesFilter(item: ActionNeededItem, filter: QueueFilter): boolean {
  return filter === "all" || queueGroupForItem(item) === filter;
}

function queueGroupForItem(item: ActionNeededItem): QueueGroup {
  const status = statusToken(item.status);
  if (item.kind === "cancelled" || status === "cancelled") return "cancelled";
  if (item.kind === "approval" || status === "waiting-approval") return "approvals";
  if (isResumableThunkItem(item)) return "thunks";
  return "blocked";
}

function HumanPromptCard(props: {
  item: ActionNeededItem;
  value: string;
  onChange: (value: string) => void;
  onAction: (item: ActionNeededItem, responseSummary: string, intent: ActionIntent) => void;
  pending: boolean;
  compact?: boolean;
}) {
  const prompt = humanPromptForItem(props.item);
  const affordance = actionAffordanceForItem(props.item);
  const context = props.value.trim();
  const missingTarget = !props.item.goalId || (affordance === "approve" && !props.item.approvalId) || (affordance === "resume" && !props.item.thunkId);
  const primaryDisabled = props.pending || missingTarget;
  const contextDisabled = primaryDisabled || !context;
  const createPromptDisabled = props.pending || !props.item.goalId || !prompt.createPromptLabel;
  const cancelDisabled = props.pending || !props.item.goalId || !prompt.cancelLabel;
  return (
    <article className={clsx("approval-card", "human-prompt-card", props.compact && "compact")}>
      <div className="human-prompt-copy">
        <span className="goal-context-kicker">{prompt.title}</span>
        <strong>{prompt.question}</strong>
        <span>{statusLabel(props.item.status)} · {props.item.goalId ? friendlyRef(props.item.goalId) : "no goal selected"}</span>
        {prompt.detail && <small>{prompt.detail}</small>}
      </div>
      {prompt.showInput && (
        <label className="inline-action-input human-prompt-input">
          {prompt.inputLabel}
          <textarea
            value={props.value}
            onChange={(event) => props.onChange(event.target.value)}
            placeholder={prompt.placeholder}
            rows={props.compact ? 2 : 3}
          />
        </label>
      )}
      <div className="human-prompt-actions">
        <button
          type="button"
          className={affordance === "approve" || affordance === "resume" ? "primary-button" : "secondary-button"}
          disabled={primaryDisabled}
          onClick={() => props.onAction(props.item, prompt.defaultResponseSummary, "primary")}
        >
          {prompt.primaryLabel}
        </button>
        {prompt.showInput && (
          <button type="button" className="secondary-button" disabled={contextDisabled} onClick={() => props.onAction(props.item, context, "context")}>
            {prompt.contextLabel}
          </button>
        )}
        {prompt.createPromptLabel && (
          <button type="button" className="secondary-button" disabled={createPromptDisabled} onClick={() => props.onAction(props.item, context, "create-human-prompt")}>
            {prompt.createPromptLabel}
          </button>
        )}
        {prompt.cancelLabel && (
          <button
            type="button"
            className="danger-button"
            disabled={cancelDisabled}
            title="Stop the durable goal. This is not a local clear action."
            onClick={() => props.onAction(props.item, context, "cancel-goal")}
          >
            <XCircle size={15} />
            {prompt.cancelLabel}
          </button>
        )}
      </div>
      {prompt.cancelLabel && <small className="danger-note">Cancel stops the goal. Clear only dismisses local notices or selection.</small>}
    </article>
  );
}

function humanPromptForItem(item: ActionNeededItem): HumanPromptSpec {
  const status = statusToken(item.status);
  const question = item.requestedInput || item.label || "What should the coordinator do next?";
  if ((item.kind === "approval" || status === "waiting-approval") && item.approvalId) {
    const approvalQuestion = item.requestedInput || "Approve this gate and continue?";
    return {
      title: "Approval prompt",
      question: approvalQuestion,
      detail: item.detail || blockerReason(item),
      primaryLabel: "Approve and continue",
      contextLabel: "Approve with note",
      inputLabel: "Approval note",
      placeholder: "Add context for the approval record.",
      showInput: true,
      defaultResponseSummary: "Approved by operator.",
    };
  }
  if (isResumableThunkItem(item)) {
    return {
      title: "Human prompt",
      question,
      detail: item.detail || blockerReason(item),
      primaryLabel: "Continue",
      contextLabel: "Add context",
      inputLabel: "Context",
      placeholder: "Add context for the agent, or leave blank and press Continue.",
      showInput: true,
      defaultResponseSummary: defaultContinuationSummary,
    };
  }
  const failed = status === "failed";
  const waitingWithoutThunk = status === "waiting-input";
  return {
    title: failed ? "Failed work" : waitingWithoutThunk ? "Waiting task" : "Recovery",
    question: item.label || (failed ? "Retry failed work?" : waitingWithoutThunk ? "Create an operator prompt for this waiting task?" : "Recover blocked work?"),
    detail: item.detail || blockerReason(item),
    primaryLabel: waitingWithoutThunk ? "Create prompt" : "Retry work",
    contextLabel: "Replan",
    inputLabel: "Recovery context",
    placeholder: waitingWithoutThunk ? "Describe the operator input this task needs." : "Add what changed, what to avoid, or what the next attempt should consider.",
    showInput: true,
    defaultResponseSummary: "",
    createPromptLabel: waitingWithoutThunk ? undefined : "Ask for input",
    cancelLabel: "Cancel goal",
  };
}

function looksLikeQuestion(value: string): boolean {
  const trimmed = value.trim();
  return /\?\s*$/.test(trimmed) || /^(what|why|how|which|who|when|where|should|can|do|does|is|are)\b/i.test(trimmed);
}

function actionAffordanceForItem(item: ActionNeededItem): "approve" | "resume" | "replan" {
  const status = statusToken(item.status);
  if ((item.kind === "approval" || status === "waiting-approval") && item.approvalId) return "approve";
  if (isResumableThunkItem(item)) return "resume";
  return "replan";
}

function isResumableThunkItem(item: ActionNeededItem): boolean {
  return item.kind === "thunk" && Boolean(item.thunkId);
}

function runOperatorAction(item: ActionNeededItem, responseSummary = "", intent: ActionIntent = "primary"): Promise<unknown> {
  if (!item.goalId) {
    throw new Error("This action is missing a goal id.");
  }
  const actionId = item.actionId || actionIdForItem(item);
  const baseResolutionPayload = {
    goal_id: item.goalId,
    task_id: item.taskId || undefined,
    approval_id: item.approvalId || undefined,
    thunk_id: item.thunkId || undefined,
    operator: "operator",
    response_summary: responseSummary.trim(),
    answer: responseSummary.trim() || undefined,
    artifact_refs: [],
  };
  if (intent === "cancel-goal") {
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: "cancel_goal",
      response_summary: blockedTaskCancelReason(item, responseSummary.trim()),
    });
  }
  const affordance = actionAffordanceForItem(item);
  if (affordance === "approve") {
    if (!item.approvalId) throw new Error("This approval is missing an approval id.");
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: "approve",
      response_summary: responseSummary.trim() || "Approved from Task Graph Manager",
    });
  }
  if (affordance === "resume") {
    if (!item.thunkId) throw new Error("This continuation is missing a thunk id.");
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: intent === "context" ? "add_context" : "continue",
      response_summary: responseSummary.trim() || defaultContinuationSummary,
    });
  }
  if (intent === "create-human-prompt" || (statusToken(item.status) === "waiting-input" && !isResumableThunkItem(item) && intent === "primary")) {
    return createThunk(item.goalId, blockedTaskHumanPromptPayload(item, responseSummary.trim()));
  }
  const status = statusToken(item.status);
  if (status === "blocked" || status === "failed") {
    if (intent === "context" || responseSummary.trim()) {
      return resolveOperatorAction(actionId, {
        ...baseResolutionPayload,
        resolution: "replan",
        response_summary: responseSummary.trim() || "Operator requested replan.",
      });
    }
    return resolveOperatorAction(actionId, {
      ...baseResolutionPayload,
      resolution: "retry",
      response_summary: "Operator requested retry.",
    });
  }
  return resolveOperatorAction(actionId, {
    ...baseResolutionPayload,
    resolution: "replan",
    response_summary: responseSummary.trim() || "Operator requested replan.",
  });
}

function actionIdForItem(item: ActionNeededItem): string {
  if (item.kind === "approval" && item.approvalId) return `approval:${item.goalId}:${item.approvalId}`;
  if (item.kind === "thunk" && item.thunkId) return `thunk:${item.goalId}:${item.thunkId}`;
  if (item.taskId) return `task:${item.goalId}:${item.taskId}`;
  return `goal:${item.goalId}:action`;
}

function RunnersView({ workspace }: { workspace?: OperatorWorkspaceSnapshot }) {
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

function runnerTableRow(row: JsonRecord): string[] {
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

function stringValue(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function numberValue(value: unknown): number | null {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function ResultList({ value }: { value: unknown }) {
  const rows = rowsFrom(at(value, ["data"]) ?? value);
  if (!value) {
    return <EmptyState title="Memory results pending" detail="Search, build context, or save a note." />;
  }
  if (rows.length) {
    return (
      <ul className="result-list">
        {rows.slice(0, 20).map((row, index) => (
          <li key={String(row.key ?? row.id ?? index)}>
            <strong>{String(row.title ?? row.key ?? row.id ?? "Memory item")}</strong>
            <p>{String(row.content ?? row.text ?? row.summary ?? row.excerpt ?? "")}</p>
          </li>
        ))}
      </ul>
    );
  }
  return <AdvancedInspect summaryLabel="Details" title="Memory response" payload={value} buttonLabel="Inspect JSON" />;
}

function SimpleTable({ headers, rows, empty }: { headers: string[]; rows: string[][]; empty: string }) {
  if (!rows.length) {
    return <EmptyState title={empty} detail="Refresh or connect the backing service." />;
  }
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>{headers.map((header) => <th key={header}>{header}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={index}>{row.map((cell, cellIndex) => <td key={cellIndex}>{cell}</td>)}</tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function EmptyState({ title, detail, actionLabel, onAction }: { title: string; detail: string; actionLabel?: string; onAction?: () => void }) {
  return (
    <div className="empty-state">
      <CircleAlert size={18} />
      <strong>{title}</strong>
      <span>{detail}</span>
      {actionLabel && onAction && (
        <button type="button" className="secondary-button" onClick={onAction}>
          {actionLabel}
        </button>
      )}
    </div>
  );
}

function AdvancedInspect({
  title,
  payload,
  buttonLabel = "Inspect",
  summaryLabel = "Advanced details",
}: {
  title: string;
  payload: unknown;
  buttonLabel?: string;
  summaryLabel?: string;
}) {
  return (
    <details className="advanced-inline-details">
      <summary>{summaryLabel}</summary>
      <div className="button-row">
        <InspectButton title={title} payload={payload} buttonLabel={buttonLabel} />
      </div>
    </details>
  );
}

function InspectButton({ title, payload, buttonLabel = "Inspect" }: { title: string; payload: unknown; buttonLabel?: string }) {
  return (
    <Dialog.Root>
      <Dialog.Trigger asChild>
        <button type="button" className="secondary-button">{buttonLabel}</button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content">
          <Dialog.Title>{title}</Dialog.Title>
          <pre>{JSON.stringify(payload, null, 2)}</pre>
          <Dialog.Close asChild>
            <button type="button" className="primary-button">Close</button>
          </Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function taskId(task: TaskRow): string {
  return String(task.task_id ?? task.id ?? taskPayload(task).id ?? "");
}

function taskPayload(task: TaskRow): JsonRecord {
  if (isRecord(task.raw_task) && isRecord(task.raw_task.payload_json)) {
    return task.raw_task.payload_json;
  }
  if (isRecord(task.payload_json)) {
    return task.payload_json;
  }
  return {};
}

function colorRef(value: unknown): ColorRef | null {
  return isRecord(value) ? value as ColorRef : null;
}

function safeHex(value: unknown): string {
  const text = String(value ?? "");
  return /^#[0-9a-fA-F]{6}$/.test(text) ? text : "#7d8b94";
}
