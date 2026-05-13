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
  api,
  approvals,
  at,
  authToken,
  branchGoal,
  cancelGoal,
  chat,
  chatRun,
  chatSession,
  createThunk,
  goalSnapshot,
  goals,
  isRecord,
  mechanismBallot,
  mechanismStart,
  memoryContext,
  memoryEdit,
  memoryEditPreview,
  memoryEvents,
  memorySearch,
  memoryWrite,
  overview,
  plans,
  restartGoal,
  resumeThunk,
  rowsFrom,
  selectBranch,
  setAuthToken,
  steer,
  submitGoal,
  threads,
  voteGoal,
} from "./api";
import type { ChatMessage, ChatResponse, ChatRunTrace, ColorRef, ComputeGraphNode, GoalRow, GoalSnapshot, JsonRecord, Overview, TaskRow } from "./types";

type ViewKey = "dashboard" | "goals" | "graph" | "control" | "memory" | "plans" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";
type GraphFilter = "all" | "attention" | "active" | "completed";
type DraftKind = "plan" | "goal" | "search";
type GoalDraftEditField = "title" | "objective" | "acceptance_evidence" | "constraints";
type OperatorStateKey = "action-needed" | "running" | "waiting" | "reviewing" | "satisfied";
type ActionNeededKind = "approval" | "blocked-task" | "waiting-task" | "thunk";
type ActionNeededItem = {
  key: string;
  kind: ActionNeededKind;
  label: string;
  status: string;
  detail: string;
  goalId: string;
  taskId: string;
  approvalId: string;
  thunkId: string;
  risk: string;
  actionLabel: string;
};
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
  status: string;
  waitKind: string;
  waitReference: string;
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

const views: Array<{ key: ViewKey; label: string; icon: typeof Route }> = [
  { key: "dashboard", label: "Dashboard", icon: Route },
  { key: "goals", label: "Goals", icon: ListChecks },
  { key: "graph", label: "Work Graph", icon: Network },
  { key: "control", label: "Goal Controls", icon: ShieldCheck },
  { key: "memory", label: "Memory", icon: Brain },
  { key: "plans", label: "Plans", icon: GitBranch },
  { key: "human", label: "Human Queue", icon: Bell },
  { key: "runners", label: "Runners", icon: Server },
];

const starterMessages: ChatMessage[] = [
  {
    role: "assistant",
    content:
      "Tell me the outcome you want. I can draft a plan, a goal, or a search request.",
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
  const [draftKind, setDraftKind] = useState<DraftKind>("plan");
  const [activeChatRunId, setActiveChatRunId] = useState<string | null>(null);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => initialThemePreference());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(initialThemePreference()));

  const overviewQuery = useQuery({ queryKey: ["overview"], queryFn: overview });
  const goalsQuery = useQuery({ queryKey: ["goals"], queryFn: goals });
  const chatSessionId = selectedGoalId ? `goal:${selectedGoalId}` : "operator:default";
  const activeDraftForSession = activeDraft?.sessionId === chatSessionId ? activeDraft : null;
  const chatSessionQuery = useQuery({
    queryKey: ["chat-session", chatSessionId],
    queryFn: () => chatSession(chatSessionId),
  });
  const messages = sessionMessages[chatSessionId] ?? starterMessages;
  const selectedGoalQuery = useQuery({
    queryKey: ["goal", selectedGoalId],
    queryFn: () => goalSnapshot(selectedGoalId),
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
  const selectGoalId = useCallback((goalId: string) => {
    const nextGoalId = goalId.trim();
    setSelectedGoalId(nextGoalId);
    persistSelectedGoalId(nextGoalId);
  }, []);

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
      const response = await chat(requestSessionId, requestMode, requestGoalId, nextMessages, runId);
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
  const latestResponse = activeDraftForSession?.response;
  const latestGoalDraft = activeDraftForSession?.goalDraft ?? null;
  const submitGoalDraft = useMutation({
    mutationFn: async () => {
      const draft = latestGoalDraft;
      if (!draft) {
        throw new Error("Generate a goal draft first.");
      }
      const response = await submitGoal(draft);
      assertGoalSubmitReachedCoordinator(response);
      return { response, draft };
    },
    onSuccess: (result) => {
      const goalId = goalIdFromSubmitResponse(result.response);
      if (goalId) {
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
        void queryClient.invalidateQueries({ queryKey: ["goal", goalId] });
        void queryClient.refetchQueries({ queryKey: ["goal", goalId] });
      }
      void queryClient.invalidateQueries({ queryKey: ["goals"] });
      void queryClient.invalidateQueries({ queryKey: ["overview"] });
      void queryClient.refetchQueries({ queryKey: ["goals"] });
      void queryClient.refetchQueries({ queryKey: ["overview"] });
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
    if (!selectedGoalId || !submittedGoalDrafts[selectedGoalId] || !goalSnapshotHasProjectedTasks(selectedGoalQuery.data)) {
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
    void queryClient.invalidateQueries({ queryKey: ["overview"] });
    void queryClient.refetchQueries({ queryKey: ["goals"] });
    void queryClient.refetchQueries({ queryKey: ["overview"] });
  }, [queryClient, selectedGoalId, selectedGoalQuery.data, submittedGoalDrafts]);

  const saveToken = (value: string) => {
    setToken(value);
    setAuthToken(value);
    refreshAll();
  };

  const overviewData = overviewQuery.data;
  const projectedGoalRows = useMemo(() => rowsFrom(at(goalsQuery.data, ["data"]) ?? goalsQuery.data) as GoalRow[], [goalsQuery.data]);
  const goalRows = useMemo(() => mergeSubmittedGoalRows(projectedGoalRows, submittedGoalDrafts), [projectedGoalRows, submittedGoalDrafts]);
  const currentGoal = selectedGoalQuery.data;
  const selectedSubmittedDraft = selectedGoalId ? submittedGoalDrafts[selectedGoalId]?.draft ?? null : null;
  const selectedGoal = useMemo(() => selectedGoalSummary(selectedGoalId, goalRows, currentGoal, selectedSubmittedDraft), [currentGoal, goalRows, selectedGoalId, selectedSubmittedDraft]);
  const selectableGoals = useMemo(() => goalRowsWithSelected(goalRows, selectedGoal), [goalRows, selectedGoal]);
  const serviceRows = overviewData?.services ?? [];

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
            onOpenChange={setGoalPickerOpen}
            onSelectGoal={selectGoalId}
            onRefreshGoals={() => {
              void queryClient.invalidateQueries({ queryKey: ["goals"] });
              void queryClient.invalidateQueries({ queryKey: ["overview"] });
            }}
            onOpenGraph={() => selectedGoalId && setActiveView("graph")}
          />
          <ServiceStrip services={serviceRows} />
        </header>

        <section className="content-grid">
          <CommandPanel
            messages={messages}
            input={chatInput}
            draftKind={draftKind}
            busy={sendChat.isPending}
            error={sendChat.error}
            activeDraft={activeDraftForSession}
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

          {activeView === "dashboard" && (
            <Dashboard
              overview={overviewData}
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
            <TaskGraphView goalId={selectedGoalId} snapshot={currentGoal} submittedDraft={selectedSubmittedDraft} loading={selectedGoalQuery.isFetching} onOpenGoalPicker={() => setGoalPickerOpen(true)} />
          )}
          {activeView === "control" && (
            <CompilerControlView goalId={selectedGoalId} snapshot={currentGoal} loading={selectedGoalQuery.isFetching} onOpenGoalPicker={() => setGoalPickerOpen(true)} />
          )}
          {activeView === "memory" && <MemoryView selectedGoalId={selectedGoalId} />}
          {activeView === "plans" && <PlansView />}
          {activeView === "human" && <HumanQueueView selectedGoalId={selectedGoalId} />}
          {activeView === "runners" && <RunnersView overview={overviewData} />}
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
    ["goal", "id"],
    ["goal", "spec", "id"],
    ["state", "goal", "id"],
    ["result", "goal_id"],
    ["result", "id"],
  ]) {
    const candidate = valueAt(response, path);
    if (typeof candidate === "string" && candidate.trim()) {
      return candidate.trim();
    }
  }
  const url = isRecord(response) && typeof response.url === "string" ? response.url : "";
  return url.match(/\/GoalWorkflow\/([0-9a-f-]{36})\/run$/i)?.[1] ?? "";
}

function assertGoalSubmitReachedCoordinator(response: unknown): void {
  if (!isRecord(response)) {
    return;
  }
  const proxyStatus = typeof response.status === "number" ? response.status : null;
  if (proxyStatus === null || (proxyStatus >= 200 && proxyStatus < 400)) {
    return;
  }
  const detail = typeof response.error === "string" ? response.error : typeof response.url === "string" ? response.url : "unknown upstream failure";
  throw new Error(`Goal submit returned an upstream failure: ${detail}`);
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

function selectedGoalSummary(goalId: string, rows: GoalRow[], snapshot?: GoalSnapshot, submittedDraft?: JsonRecord | null): GoalSummary | null {
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

function goalSnapshotHasProjectedTasks(snapshot?: GoalSnapshot): boolean {
  const agentRows = snapshot?.agent_activity ?? [];
  const taskRows = rowsFrom(at(snapshot, ["tasks", "data"]) ?? snapshot?.tasks);
  const computeNodes = rowsFrom(at(snapshot, ["workflow_compute_graph", "data", "nodes"]) ?? at(snapshot, ["workflow_compute_graph", "nodes"]));
  return Boolean(agentRows.length || taskRows.length || computeNodes.length);
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

function goalSubgoalsFromSnapshotOrDraft(snapshot?: GoalSnapshot, draft?: JsonRecord | null): JsonRecord[] {
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
    control: "Goal Controls",
    memory: "Shared Memory",
    plans: "Durable Plans",
    human: "Human Queue",
    runners: "Runner Fleet",
  }[view];
}

function ServiceStrip({ services }: { services: Overview["services"] }) {
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
  onOpenChange: (open: boolean) => void;
  onSelectGoal: (goalId: string) => void;
  onRefreshGoals: () => void;
  onOpenGraph: () => void;
}) {
  const done = Math.round((props.selectedGoal?.progress ?? 0) * 100);
  const selectedState = props.selectedGoal ? operatorStateForStatus(props.selectedGoal.status) : null;
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
                <small>Chat, graph, controls, memory, and queue use this context</small>
              )}
            </div>
            <span className={clsx("operator-state-pill", selectedState ? stateTone(selectedState.key) : "muted")}>
              {selectedState?.label ?? (props.loading ? "Loading" : "Select")}
            </span>
            <ChevronDown size={16} />
          </button>
        </Popover.Trigger>
        {props.selectedGoal && (
          <InspectButton
            title="Current goal details"
            payload={{
              goal_id: props.selectedGoalId,
              title: props.selectedGoal.title,
              status: props.selectedGoal.status,
              operator_state: selectedState,
              objective: props.selectedGoal.objective,
            }}
            buttonLabel="Details"
          />
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
                  onClick={() => {
                    props.onSelectGoal("");
                    props.onOpenChange(false);
                  }}
                >
                  <XCircle size={15} />
                  Clear goal
                </button>
                <button type="button" className="primary-button" disabled={!props.selectedGoalId} onClick={props.onOpenGraph}>
                  <Network size={15} />
                  Open graph
                </button>
              </div>
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
  const chatShellRef = useRef<HTMLDivElement>(null);
  const latestMessage = props.messages[props.messages.length - 1];

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
          <p className="eyebrow">Start work</p>
          <h2>{commandTitle(props.draftKind)}</h2>
        </div>
        <div className="mode-toggle" role="group" aria-label="Draft target">
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
      <div className="outcome-meta" aria-label="Chat scope">
        <span className="status-pill muted">Operator workspace</span>
        <span className={clsx("status-pill", props.selectedGoal ? statusTone(props.selectedGoal.status) : "muted")}>
          {props.selectedGoal ? `Selected goal: ${props.selectedGoal.title}` : "Selected goal: none"}
        </span>
        <span className={clsx("status-pill", props.activeDraft ? "status-runnable" : "muted")}>
          {props.activeDraft ? `Active draft: ${draftKindLabel(props.activeDraft.kind)} · ${sessionDisplayLabel(props.activeDraft.sessionId)}` : "Active draft: none"}
        </span>
        <span className={clsx("status-pill", props.busy ? "status-running" : "status-pending")}>
          {props.busy ? commandBusyLabel(props.draftKind) : `Session: ${props.mode} · ${sessionDisplayLabel(props.sessionId)}`}
        </span>
        {(props.busy || props.chatRun || props.latestResponse || draftKeys.length > 0) && (
          <InspectButton title="Chat activity" payload={activityPayload} buttonLabel={activityLabel} />
        )}
      </div>
      {props.activeDraft && (
        <div className="draft-action-bar">
          <span className="status-pill status-runnable">Saved {draftKindLabel(props.activeDraft.kind)}</span>
          {props.goalDraft && <span className="status-pill status-runnable">Goal draft ready</span>}
          {submittedGoalId && <span className="status-pill status-done">Submitted {friendlyRef(submittedGoalId)}</span>}
          <InspectButton title={props.goalDraft ? "GoalSpec draft" : "Active draft"} payload={props.goalDraft ?? props.activeDraft.response} buttonLabel="Review draft" />
          <button type="button" className="secondary-button" disabled={props.busy || props.goalSubmitBusy || Boolean(submittedGoalId)} onClick={props.onDiscardGoalDraft}>
            <XCircle size={15} />
            Discard draft
          </button>
          {props.goalDraft && (
            <button type="button" className="primary-button" disabled={props.busy || props.goalSubmitBusy || Boolean(submittedGoalId)} onClick={props.onSubmitGoalDraft}>
              <ListChecks size={15} />
              {submittedGoalId ? "Submitted" : props.goalSubmitBusy ? "Submitting" : "Submit goal"}
            </button>
          )}
        </div>
      )}
      {props.goalDraft && (
        <GoalDraftEditor
          draft={props.goalDraft}
          disabled={props.busy || props.goalSubmitBusy || Boolean(submittedGoalId)}
          onUpdate={props.onUpdateGoalDraftField}
        />
      )}
      <div className="quick-prompts" aria-label="Goal action prompts">
        {compilerPromptTemplates(props.selectedGoalId, props.selectedGoal?.title).map((template) => (
          <button key={template.label} type="button" className="secondary-button" disabled={props.busy} onClick={() => props.onSend(template.prompt)}>
            {template.icon === "graph" && <Network size={15} />}
            {template.icon === "control" && <ShieldCheck size={15} />}
            {template.icon === "research" && <Search size={15} />}
            {template.label}
          </button>
        ))}
      </div>
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

function GoalDraftEditor(props: { draft: JsonRecord; disabled: boolean; onUpdate: (field: GoalDraftEditField, value: string) => void }) {
  return (
    <details className="goal-draft-editor" open>
      <summary>
        <span>Edit draft</span>
        <small>Review the fields that will be submitted to the coordinator</small>
      </summary>
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
    </details>
  );
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
  if (kind === "goal") return "Goal draft";
  if (kind === "search") return "Search request";
  return "Plan first";
}

function draftKindLabel(kind: DraftKind): string {
  if (kind === "goal") return "Goal draft";
  if (kind === "search") return "Search request";
  return "Plan draft";
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
    drafts: props.latestResponse?.drafts ?? null,
    ready_goal_draft: props.goalDraft ?? null,
    goal_submit: props.goalSubmitResult ?? null,
  };
}

function Dashboard(props: { overview?: Overview; goals: GoalRow[]; selectedGoalId: string; onSelectGoal: (goalId: string) => void }) {
  const runnerRows = rowsFrom(at(props.overview, ["runner_status", "data"]) ?? props.overview?.runner_status);
  const approvalRows = rowsFrom(at(props.overview, ["approvals", "data"]) ?? props.overview?.approvals);
  const eventRows = rowsFrom(at(props.overview, ["recent_events", "data"]) ?? props.overview?.recent_events);
  const eventSourceRows = rowsFrom(at(props.overview, ["event_sources", "data"]) ?? props.overview?.event_sources);
  const attentionGoals = props.goals.filter((goal) => {
    const status = String(goal.status ?? "").toLowerCase();
    return status.includes("blocked") || status.includes("failed") || Number(goal.blocked_tasks ?? 0) > 0 || Number(goal.failed_tasks ?? 0) > 0;
  }).length;
  return (
    <div className="dashboard-grid">
      <MetricCard label="Active goals" value={String(props.goals.length)} detail="in progress" />
      <MetricCard label="Runners" value={String(runnerRows.length)} detail="available capacity" />
      <MetricCard label="Human queue" value={String(approvalRows.length)} detail="waiting decisions" />
      <MetricCard label="Events" value={String(eventRows.length)} detail="recent signals" />
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
          <OutcomeRow label="Approvals" value={approvalRows.length} tone={approvalRows.length ? "waiting-approval" : "done"} />
          <OutcomeRow label="Events" value={eventRows.length} tone={eventRows.length ? "runnable" : "done"} />
          <OutcomeRow label="Goal attention" value={attentionGoals} tone={attentionGoals ? "blocked" : "done"} />
          <OutcomeRow label="Runners" value={runnerRows.length} tone={runnerRows.length ? "running" : "pending"} />
        </ul>
      </section>
      <EventSourcesPanel rows={eventSourceRows} />
    </div>
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
              aria-label={`Inspect subgoal ${title}`}
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

function TaskGraphView(props: { goalId: string; snapshot?: GoalSnapshot; submittedDraft?: JsonRecord | null; loading: boolean; onOpenGoalPicker: () => void }) {
  const [graphFilter, setGraphFilter] = useState<GraphFilter>("all");
  const projectedTasks = useMemo(() => taskRowsFromSnapshot(props.snapshot), [props.snapshot]);
  const draftTasks = useMemo(() => taskRowsFromGoalDraft(props.goalId, props.submittedDraft), [props.goalId, props.submittedDraft]);
  const tasks = projectedTasks.length ? projectedTasks : draftTasks;
  const computeGraph = useMemo(() => props.snapshot ? workflowComputeGraph(props.snapshot) : undefined, [props.snapshot]);
  const computeNodes = useMemo(() => computeGraphNodes(computeGraph), [computeGraph]);
  const filteredComputeNodes = useMemo(() => computeNodes.filter((node) => computeNodeMatchesGraphFilter(node, graphFilter)), [computeNodes, graphFilter]);
  const filteredTasks = useMemo(() => tasks.filter((task) => taskMatchesGraphFilter(task, graphFilter)), [tasks, graphFilter]);
  const graph = useMemo(() => computeNodes.length ? graphFromComputeGraph(computeGraph, filteredComputeNodes) : graphFromTasks(filteredTasks), [computeGraph, computeNodes.length, filteredComputeNodes, filteredTasks]);
  const counts = useMemo(() => taskStatusCounts(tasks), [tasks]);
  const subgoals = useMemo(() => goalSubgoalsFromSnapshotOrDraft(props.snapshot, props.submittedDraft), [props.snapshot, props.submittedDraft]);
  const showingSubmittedDraft = Boolean(props.submittedDraft && !projectedTasks.length);
  const taskCount = tasks.length;
  const visibleCount = computeNodes.length ? filteredComputeNodes.length : filteredTasks.length;
  const totalCount = computeNodes.length ? computeNodes.length : taskCount;
  const graphUnit = computeNodes.length ? "compute nodes" : "tasks";
  const continuationCount = props.snapshot ? continuationRowsFromSnapshot(props.snapshot).length : 0;
  const actionNeeded = useMemo(() => actionNeededItemsFromSnapshot(props.snapshot, props.goalId), [props.snapshot, props.goalId]);
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
      {props.snapshot && <ActionNeededPanel items={actionNeeded} />}
      {props.snapshot && <EvidenceNextActionPanel snapshot={props.snapshot} counts={counts} taskCount={taskCount} />}
      {props.snapshot && <GraphStatusPanel counts={counts} taskCount={taskCount} />}
      {!props.goalId ? (
        <EmptyState title="Select a goal" detail="Use the top-bar goal switcher." actionLabel="Choose goal" onAction={props.onOpenGoalPicker} />
      ) : props.loading && !showingSubmittedDraft ? (
        <EmptyState title="Loading task graph" detail="Fetching goal snapshot and agent activity." />
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
          {continuationCount > 0 && <ContinuationQueue goalId={props.goalId} snapshot={props.snapshot} />}
          <CompilerControlPanel goalId={props.goalId} snapshot={props.snapshot} compact />
          <details className="advanced-details">
            <summary>
              <span>Advanced graph details</span>
              <small>Raw task counts and compute projection</small>
            </summary>
            <TaskSummary snapshot={props.snapshot} counts={counts} />
            <ComputeGraphDetails snapshot={props.snapshot} />
          </details>
        </>
      )}
    </section>
  );
}

function CompilerControlView(props: { goalId: string; snapshot?: GoalSnapshot; loading: boolean; onOpenGoalPicker: () => void }) {
  const tasks = taskRowsFromSnapshot(props.snapshot);
  const counts = taskStatusCounts(tasks);
  const continuationCount = props.snapshot ? continuationRowsFromSnapshot(props.snapshot).length : 0;
  const actionNeeded = useMemo(() => actionNeededItemsFromSnapshot(props.snapshot, props.goalId), [props.snapshot, props.goalId]);
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>Goal controls</h2>
          <span className="muted-small">Primary actions first; detailed flow tools are collapsed below</span>
        </div>
      </div>
      {!props.goalId ? (
        <EmptyState title="Select a goal" detail="Use the top-bar goal switcher." actionLabel="Choose goal" onAction={props.onOpenGoalPicker} />
      ) : props.loading ? (
        <EmptyState title="Loading controls" detail="Fetching workflow projection." />
      ) : (
        <>
          {props.snapshot && <ActionNeededPanel items={actionNeeded} />}
          {props.snapshot && <EvidenceNextActionPanel snapshot={props.snapshot} counts={counts} taskCount={tasks.length} />}
          {props.snapshot && <GraphStatusPanel counts={counts} taskCount={tasks.length} />}
          {props.snapshot && continuationCount > 0 && <ContinuationQueue goalId={props.goalId} snapshot={props.snapshot} />}
          <CompilerControlPanel goalId={props.goalId} snapshot={props.snapshot} />
          {props.snapshot && (
            <details className="advanced-details">
              <summary>
                <span>Advanced projection details</span>
                <small>Raw task counts and compute graph rows</small>
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

function ComputeGraphDetails({ snapshot }: { snapshot: GoalSnapshot }) {
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

function TaskSummary({ snapshot, counts }: { snapshot: GoalSnapshot; counts: Map<string, number> }) {
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
      <InspectButton title="Goal snapshot" payload={snapshot} />
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
      <div className="approval-list">
        {items.slice(0, 5).map((item) => (
          <div key={item.key} className="approval-card">
            <div>
              <strong>{item.label}</strong>
              <span>{statusLabel(item.status)} · {item.goalId ? friendlyRef(item.goalId) : "selected goal"}</span>
              {item.detail && <small>{item.detail}</small>}
            </div>
            <span className={clsx("operator-state-pill", stateTone(item.kind === "approval" || statusToken(item.status) === "waiting-approval" ? "action-needed" : "waiting"))}>
              {item.actionLabel}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}

function EvidenceNextActionPanel({ snapshot, counts, taskCount, compact = false }: { snapshot: GoalSnapshot; counts: Map<string, number>; taskCount: number; compact?: boolean }) {
  const highlights = evidenceHighlights(snapshot, counts, taskCount);
  const nextAction = nextActionSummary(counts, taskCount, snapshot);
  return (
    <div className={clsx("evidence-next-panel", compact && "compact")}>
      <section className="evidence-card">
        <div className="section-heading">
          <h3>Evidence</h3>
          <InspectButton
            title="Evidence detail"
            payload={{
              status_counts: Object.fromEntries(counts),
              workflow_progress: workflowProgress(snapshot) ?? null,
              compute_graph: workflowComputeGraph(snapshot) ?? null,
            }}
            buttonLabel="Inspect evidence"
          />
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

function evidenceHighlights(snapshot: GoalSnapshot, counts: Map<string, number>, taskCount: number): Array<{ label: string; value: string; state: OperatorStateKey }> {
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

function nextActionSummary(counts: Map<string, number>, taskCount: number, snapshot: GoalSnapshot): { title: string; detail: string; state: OperatorStateKey; stateLabel: string } {
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

function workflowComputeGraph(snapshot: GoalSnapshot): Record<string, unknown> | undefined {
  const direct = snapshot.workflow_compute_graph as Record<string, unknown> | undefined;
  if (direct && typeof direct === "object") {
    const data = (direct as { data?: unknown }).data;
    return data && typeof data === "object" ? data as Record<string, unknown> : direct;
  }
  return undefined;
}

function taskRowsFromSnapshot(snapshot?: GoalSnapshot | JsonRecord): TaskRow[] {
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

function actionNeededItemsFromSnapshot(snapshot?: GoalSnapshot | JsonRecord, selectedGoalId = ""): ActionNeededItem[] {
  if (!snapshot) {
    return [];
  }
  return mergeActionNeededItems([
    ...actionNeededItemsFromApprovals(rowsFrom(at(snapshot, ["approvals", "data", "approvals"]) ?? at(snapshot, ["approvals", "approvals"]) ?? at(snapshot, ["approvals", "data"]) ?? snapshot.approvals), selectedGoalId),
    ...actionNeededItemsFromTasks(taskRowsFromSnapshot(snapshot), selectedGoalId),
    ...actionNeededItemsFromThunks(snapshot as GoalSnapshot, selectedGoalId),
  ]);
}

function actionNeededItemsFromOverview(value?: unknown): ActionNeededItem[] {
  const source = isRecord(value) ? value : {};
  return mergeActionNeededItems([
    ...actionNeededItemsFromApprovals(rowsFrom(at(source, ["approvals", "data", "approvals"]) ?? at(source, ["approvals", "approvals"]) ?? at(source, ["approvals", "data"]) ?? source.approvals)),
    ...actionNeededItemsFromTasks(rowsFrom(at(source, ["agents", "data", "tasks"]) ?? at(source, ["agents", "tasks"]) ?? at(source, ["tasks", "data", "tasks"]) ?? source.agents)),
  ]);
}

function actionNeededItemsFromApprovals(rows: JsonRecord[], selectedGoalId = ""): ActionNeededItem[] {
  return rows.map((row, index) => {
    const approvalId = stringValue(row.approval_id) || stringValue(row.id) || stringValue(row.approval_ref);
    const goalId = stringValue(row.goal_id) || selectedGoalId;
    const risk = stringValue(row.risk) || stringValue(at(row, ["payload_json", "risk"]));
    const action = stringValue(row.requested_action) || stringValue(row.reason) || stringValue(at(row, ["payload_json", "requested_action"])) || stringValue(at(row, ["payload_json", "reason"])) || risk || "Review approval request";
    return {
      key: approvalId ? `approval:${approvalId}` : `approval:${goalId}:${index}`,
      kind: "approval" as const,
      label: action,
      status: normalizeStatus(row.status) || "pending",
      detail: risk ? `Risk: ${risk}` : "Human approval requested.",
      goalId,
      taskId: stringValue(row.task_id) || stringValue(at(row, ["payload_json", "task_id"])),
      approvalId,
      thunkId: "",
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
        key: id ? `task:${id}` : `task:${goalId}:${index}`,
        kind: status === "blocked" || status === "failed" ? "blocked-task" as const : "waiting-task" as const,
        label: title,
        status,
        detail: taskDetail(task, status),
        goalId,
        taskId: id,
        approvalId: "",
        thunkId: "",
        risk: "",
        actionLabel: status === "waiting-approval" ? "Approve" : status === "waiting-input" ? "Provide input" : "Review",
      };
    });
}

function actionNeededItemsFromThunks(snapshot: GoalSnapshot, selectedGoalId = ""): ActionNeededItem[] {
  return continuationRowsFromSnapshot(snapshot).map((row) => ({
    key: `thunk:${row.thunkId}`,
    kind: "thunk" as const,
    label: row.reason || "Continuation waiting for input",
    status: row.status || "waiting-input",
    detail: [row.waitKind ? `Wait: ${row.waitKind}` : "", row.waitReference ? `Ref ${friendlyRef(row.waitReference)}` : ""].filter(Boolean).join(" · ") || "Delayed compute continuation is paused.",
    goalId: selectedGoalId,
    taskId: row.taskId,
    approvalId: "",
    thunkId: row.thunkId,
    risk: "",
    actionLabel: "Resume",
  }));
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
  const reason = stringValue(task.reason) || stringValue(task.blocker) || stringValue(task.summary) || stringValue(at(task, ["payload_json", "reason"]));
  return [statusLabel(status), role, subgoal ? `subgoal ${subgoal}` : "", reason].filter(Boolean).join(" · ");
}

function continuationRowsFromSnapshot(snapshot?: GoalSnapshot): ContinuationRow[] {
  if (!snapshot) {
    return [];
  }
  const graph = workflowComputeGraph(snapshot);
  const nodes = Array.isArray(graph?.nodes) ? graph.nodes : [];
  return nodes
    .filter(isRecord)
    .filter((node) => normalizeStatus(node.kind) === "delayed-compute-thunk")
    .map((node) => continuationRowFromNode(node as ComputeGraphNode))
    .filter((row): row is ContinuationRow => Boolean(row))
    .filter((row) => !["resumed", "cancelled", "expired", "done"].includes(row.status));
}

function continuationRowFromNode(node: ComputeGraphNode): ContinuationRow | null {
  const thunkId = stringValue(node.thunk_id);
  if (!thunkId) {
    return null;
  }
  const waitRef = isRecord(node.wait_ref) ? node.wait_ref : {};
  return {
    key: thunkId,
    thunkId,
    continuationId: stringValue(node.continuation_id),
    taskId: stringValue(node.task_id),
    reason: stringValue(node.label) || "Waiting for input",
    status: normalizeStatus(node.status),
    waitKind: stringValue(waitRef.kind),
    waitReference: stringValue(waitRef.reference),
  };
}

function ContinuationQueue({ goalId, snapshot }: { goalId: string; snapshot?: GoalSnapshot }) {
  const queryClient = useQueryClient();
  const rows = continuationRowsFromSnapshot(snapshot);
  const [responses, setResponses] = useState<Record<string, string>>({});
  const resumeMutation = useMutation({
    mutationFn: ({ row, responseSummary }: { row: ContinuationRow; responseSummary: string }) => resumeThunk(goalId, {
      thunk_id: row.thunkId,
      responder: "operator",
      response_summary: responseSummary,
      artifact_refs: [],
    }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["goal", goalId] });
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
        const responseSummary = responses[row.thunkId] ?? "";
        return (
          <div key={row.key} className="continuation-card">
            <div className="continuation-copy">
              <strong>{row.reason}</strong>
              <span title={row.thunkId}>{statusLabel(row.status)} · wait {friendlyRef(row.thunkId)}</span>
              {row.taskId && <small title={row.taskId}>task {friendlyRef(row.taskId)}</small>}
              {row.continuationId && <small title={row.continuationId}>continuation {friendlyRef(row.continuationId)}</small>}
              {row.waitReference && <small title={row.waitReference}>{row.waitKind || "wait_ref"} · {friendlyRef(row.waitReference)}</small>}
            </div>
            <label>
              Response summary
              <textarea
                value={responseSummary}
                onChange={(event) => setResponses((current) => ({ ...current, [row.thunkId]: event.target.value }))}
                placeholder="What input or decision should resume this continuation?"
              />
            </label>
            <button
              type="button"
              className="secondary-button"
              disabled={!goalId || !responseSummary.trim() || resumeMutation.isPending}
              onClick={() => resumeMutation.mutate({ row, responseSummary: responseSummary.trim() })}
            >
              Resume
            </button>
          </div>
        );
      })}
    </div>
  );
}

function CompilerControlPanel({ goalId, snapshot, compact = false }: { goalId: string; snapshot?: GoalSnapshot; compact?: boolean }) {
  const queryClient = useQueryClient();
  const tasks = (snapshot?.agent_activity ?? []) as TaskRow[];
  const firstTaskId = taskId(tasks[0] ?? {});
  const [operator, setOperator] = useState("operator");
  const [result, setResult] = useState<unknown>(null);
  const [voteReason, setVoteReason] = useState("Promote or demote this goal based on current priority.");
  const [voteWeight, setVoteWeight] = useState(1);
  const [suggestedRole, setSuggestedRole] = useState("peer_goal");
  const [steerKind, setSteerKind] = useState("evaluate_goal_completion");
  const [steerTaskId, setSteerTaskId] = useState("");
  const [steerTopic, setSteerTopic] = useState("");
  const [steerReason, setSteerReason] = useState("Evaluate whether the durable evidence satisfies the current objective.");
  const [reviewCheck, setReviewCheck] = useState("behavioral_testing");
  const [flowMode, setFlowMode] = useState("restart");
  const [flowReason, setFlowReason] = useState("Operator requested control-plane action.");
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
      void queryClient.invalidateQueries({ queryKey: ["goal", goalId] });
      void queryClient.invalidateQueries({ queryKey: ["goals"] });
      void queryClient.invalidateQueries({ queryKey: ["overview"] });
    },
  });
  const disabled = !goalId || mutation.isPending;
  const run = (label: string, action: () => Promise<unknown>) => mutation.mutate({ label, run: action });
  const counts = taskStatusCounts(tasks);
  const nextAction = snapshot ? nextActionSummary(counts, tasks.length, snapshot) : null;
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = snapshot ? continuationRowsFromSnapshot(snapshot).length : 0;
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
          <h3>Goal controls</h3>
          <span className="muted-small">Primary actions first; branching, restart, waits, and decision rounds stay in advanced controls.</span>
        </div>
        {result ? <InspectButton title="Last control action" payload={result} buttonLabel="Inspect result" /> : <span className="status-pill muted">Action result pending</span>}
      </div>
      <div className="control-primary-grid" aria-label="Primary goal controls">
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
            <button
              type="button"
              className="secondary-button"
              disabled={disabled}
              data-testid="primary-request-review"
              onClick={() => runSteering("request review", "request_standard_review", "Run a focused standard review against the current task evidence.", "current task evidence")}
            >
              <CheckCircle2 size={16} />
              Request review
            </button>
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
            <span>Advanced controls</span>
            <small>Priority, steering fields, restart, branch, wait states, decision rounds, and ballots</small>
          </summary>
          <div className="control-grid nested">
        <section className="control-card">
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
        </section>

        <section className="control-card">
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
        </section>

        <section className="control-card">
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
        </section>

        <section className="control-card">
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
        </section>

        <section className="control-card span-2">
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
        </section>

        <section className="control-card">
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
        </section>
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

function workflowProgress(snapshot: GoalSnapshot): Record<string, unknown> | undefined {
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
            {Boolean(memoryEventsQuery.data) && <InspectButton title="Memory events" payload={memoryEventsQuery.data} />}
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
        <InspectButton title="Memory edit preview" payload={record} buttonLabel="Inspect preview" />
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
        <InspectButton title="Plan projection" payload={planQuery.data ?? {}} buttonLabel="Inspect plans" />
      </div>
    </section>
  );
}

function HumanQueueView({ selectedGoalId }: { selectedGoalId: string }) {
  const approvalQuery = useQuery({ queryKey: ["approvals"], queryFn: approvals });
  const overviewQuery = useQuery({ queryKey: ["overview"], queryFn: overview, enabled: !selectedGoalId });
  const selectedGoalQuery = useQuery({
    queryKey: ["goal", selectedGoalId],
    queryFn: () => goalSnapshot(selectedGoalId),
    enabled: Boolean(selectedGoalId),
  });
  const threadQuery = useQuery({ queryKey: ["threads"], queryFn: threads });
  const approvalRows = rowsFrom(at(approvalQuery.data, ["data"]) ?? approvalQuery.data);
  const actionItems = useMemo(() => {
    const approvalItems = actionNeededItemsFromApprovals(approvalRows, selectedGoalId);
    const selectedGoalItems = selectedGoalId ? actionNeededItemsFromSnapshot(selectedGoalQuery.data, selectedGoalId) : [];
    const overviewItems = selectedGoalId ? [] : actionNeededItemsFromOverview(overviewQuery.data);
    return mergeActionNeededItems([...approvalItems, ...selectedGoalItems, ...overviewItems]);
  }, [approvalRows, overviewQuery.data, selectedGoalId, selectedGoalQuery.data]);
  const threadRows = rowsFrom(at(threadQuery.data, ["data"]) ?? threadQuery.data);
  const approvalMutation = useMutation({
    mutationFn: ({ approvalId, goalId }: { approvalId: string; goalId: string }) => {
      if (!goalId) throw new Error("Approval row is missing a goal id.");
      return api(`/api/goals/${encodeURIComponent(goalId)}/approve`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ approval_id: approvalId, approved: true, comment: "Approved from Task Graph Manager" }),
      });
    },
  });
  return (
    <section className="dashboard-grid">
      <div className="panel span-2">
        <div className="section-heading">
          <h2>Action queue</h2>
          <span className="muted-small">Approvals, blockers, and waiting continuations</span>
        </div>
        <ApprovalList
          items={actionItems}
          busy={approvalMutation.isPending}
          onApprove={(approvalId, goalId) => approvalMutation.mutate({ approvalId, goalId })}
        />
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

function ApprovalList({
  items,
  busy,
  onApprove,
}: {
  items: ActionNeededItem[];
  busy: boolean;
  onApprove?: (approvalId: string, goalId: string) => void;
}) {
  return (
    <div className="approval-list">
      {items.length ? items.map((item) => {
        const canApprove = item.kind === "approval" && item.approvalId && item.goalId;
        return (
          <div key={item.key} className="approval-card">
            <div>
              <strong>{item.label}</strong>
              <span>{statusLabel(item.status)} · {item.goalId ? friendlyRef(item.goalId) : "no goal selected"}</span>
              {item.detail && <small>{item.detail}</small>}
            </div>
            {canApprove ? (
              <button
                type="button"
                className="secondary-button"
                disabled={busy || !onApprove}
                onClick={() => onApprove?.(item.approvalId, item.goalId)}
              >
                Approve
              </button>
            ) : (
              <span className={clsx("operator-state-pill", stateTone(item.kind === "blocked-task" ? "action-needed" : "waiting"))}>
                {item.actionLabel}
              </span>
            )}
          </div>
        );
      }) : <EmptyState title="Action queue clear" detail="Approvals, blocked tasks, and waiting continuations appear here." />}
    </div>
  );
}

function RunnersView({ overview }: { overview?: Overview }) {
  const rows = rowsFrom(at(overview, ["runner_status", "data"]) ?? overview?.runner_status);
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
    || [stringValue(labels.runtime), stringValue(labels.lane)].filter(Boolean).join(" / ")
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
  return <InspectButton title="Memory response" payload={value} buttonLabel="Inspect response" />;
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
