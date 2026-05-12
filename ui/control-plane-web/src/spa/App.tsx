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
import {
  Bell,
  Brain,
  CheckCircle2,
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
import { useEffect, useMemo, useState } from "react";
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
  threads,
  voteGoal,
} from "./api";
import type { ChatMessage, ChatResponse, ChatRunTrace, ColorRef, ComputeGraphNode, GoalRow, GoalSnapshot, JsonRecord, Overview, TaskRow } from "./types";

type ViewKey = "dashboard" | "goals" | "graph" | "control" | "memory" | "plans" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";
type GraphFilter = "all" | "attention" | "active" | "completed";
type DraftKind = "plan" | "goal" | "search";
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
  "blocked",
  "failed",
  "cancelled",
]);
const statusLegend = [
  { token: "failed", label: "Failed", detail: "needs operator or retry" },
  { token: "blocked", label: "Blocked", detail: "waiting on dependency" },
  { token: "waiting-approval", label: "Approval", detail: "human gate open" },
  { token: "waiting-input", label: "Continuation", detail: "delayed compute thunk" },
  { token: "running", label: "Running", detail: "agent is active" },
  { token: "needs-validation", label: "Validate", detail: "evidence review" },
  { token: "runnable", label: "Runnable", detail: "ready frontier" },
  { token: "done", label: "Done", detail: "accepted work" },
  { token: "pending", label: "Pending", detail: "not yet runnable" },
  { token: "cancelled", label: "Cancelled", detail: "intentionally stopped" },
] as const;
const statusPriority = new Map<string, number>(statusLegend.map((item, index) => [item.token, index]));
const graphFilterOptions: Array<{ key: GraphFilter; label: string; detail: string }> = [
  { key: "all", label: "All", detail: "all projected tasks" },
  { key: "attention", label: "Attention", detail: "failed, blocked, approvals, continuations" },
  { key: "active", label: "Active", detail: "running, runnable, validation" },
  { key: "completed", label: "Completed", detail: "done or cancelled" },
];

const views: Array<{ key: ViewKey; label: string; icon: typeof Route }> = [
  { key: "dashboard", label: "Dashboard", icon: Route },
  { key: "goals", label: "Goals", icon: ListChecks },
  { key: "graph", label: "Task Graph", icon: Network },
  { key: "control", label: "Flow Control", icon: ShieldCheck },
  { key: "memory", label: "Memory", icon: Brain },
  { key: "plans", label: "Plans", icon: GitBranch },
  { key: "human", label: "Human Queue", icon: Bell },
  { key: "runners", label: "Runners", icon: Server },
];

const starterMessages: ChatMessage[] = [
  {
    role: "assistant",
    content:
      "Tell me the outcome you want. I can draft a durable plan, a GoalSpec, or a backend-routed search request without changing durable state.",
  },
];

export function App() {
  const queryClient = useQueryClient();
  const [activeView, setActiveView] = useState<ViewKey>("dashboard");
  const [selectedGoalId, setSelectedGoalId] = useState("");
  const [token, setToken] = useState(authToken());
  const [messages, setMessages] = useState<ChatMessage[]>(starterMessages);
  const [chatInput, setChatInput] = useState("");
  const [draftKind, setDraftKind] = useState<DraftKind>("plan");
  const [activeChatRunId, setActiveChatRunId] = useState<string | null>(null);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => initialThemePreference());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(initialThemePreference()));

  const overviewQuery = useQuery({ queryKey: ["overview"], queryFn: overview });
  const goalsQuery = useQuery({ queryKey: ["goals"], queryFn: goals });
  const chatSessionId = selectedGoalId ? `goal:${selectedGoalId}` : "operator:default";
  const chatSessionQuery = useQuery({
    queryKey: ["chat-session", chatSessionId],
    queryFn: () => chatSession(chatSessionId),
  });
  const selectedGoalQuery = useQuery({
    queryKey: ["goal", selectedGoalId],
    queryFn: () => goalSnapshot(selectedGoalId),
    enabled: Boolean(selectedGoalId),
  });

  const sendChat = useMutation({
    mutationFn: async (overrideContent?: string) => {
      const content = (overrideContent ?? chatInput).trim();
      if (!content) {
        throw new Error("Write a request first.");
      }
      const nextMessages = [...messages, { role: "user" as const, content }];
      setMessages(nextMessages);
      setChatInput("");
      const runId = createRunId();
      setActiveChatRunId(runId);
      const response = await chat(chatSessionId, modeForDraftKind(draftKind), selectedGoalId, nextMessages, runId);
      setActiveChatRunId(response.run_id ?? runId);
      setMessages([...nextMessages, { role: "assistant", content: response.assistant ?? "No response." }]);
      void queryClient.invalidateQueries({ queryKey: ["chat-session", chatSessionId] });
      return response;
    },
  });
  const chatRunQuery = useQuery({
    queryKey: ["chat-run", activeChatRunId],
    queryFn: () => chatRun(activeChatRunId ?? ""),
    enabled: Boolean(activeChatRunId && sendChat.isPending),
    refetchInterval: sendChat.isPending ? 750 : false,
  });

  const refreshAll = () => {
    void queryClient.invalidateQueries();
  };

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
    const persistedMessages = chatSessionQuery.data?.messages ?? [];
    setMessages(persistedMessages.length ? persistedMessages : starterMessages);
    setActiveChatRunId(null);
    sendChat.reset();
  }, [chatSessionId, chatSessionQuery.dataUpdatedAt]);

  const saveToken = (value: string) => {
    setToken(value);
    setAuthToken(value);
    refreshAll();
  };

  const overviewData = overviewQuery.data;
  const goalRows = useMemo(() => rowsFrom(at(goalsQuery.data, ["data"]) ?? goalsQuery.data) as GoalRow[], [goalsQuery.data]);
  const currentGoal = selectedGoalQuery.data;
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
          <div>
            <p className="eyebrow">User-facing manager</p>
            <h1>{titleFor(activeView)}</h1>
          </div>
          <ServiceStrip services={serviceRows} />
        </header>

        <section className="content-grid">
          <CommandPanel
            messages={messages}
            input={chatInput}
            draftKind={draftKind}
            busy={sendChat.isPending}
            error={sendChat.error}
            latestResponse={sendChat.data}
            chatRun={(chatRunQuery.data ?? sendChat.data?.chat_run) as ChatRunTrace | undefined}
            selectedGoalId={selectedGoalId}
            sessionId={chatSessionId}
            mode={modeForDraftKind(draftKind)}
            onDraftKindChange={setDraftKind}
            onInputChange={setChatInput}
            onSend={(content?: string) => sendChat.mutate(content)}
            onClear={() => {
              setMessages(starterMessages);
              setChatInput("");
              sendChat.reset();
            }}
          />

          {activeView === "dashboard" && (
            <Dashboard
              overview={overviewData}
              goals={goalRows}
              selectedGoalId={selectedGoalId}
              onSelectGoal={(goalId) => {
                setSelectedGoalId(goalId);
                setActiveView("graph");
              }}
            />
          )}
          {activeView === "goals" && (
            <GoalsView
              goals={goalRows}
              selectedGoalId={selectedGoalId}
              onSelectGoal={(goalId) => {
                setSelectedGoalId(goalId);
                setActiveView("graph");
              }}
            />
          )}
          {activeView === "graph" && (
            <TaskGraphView goalId={selectedGoalId} snapshot={currentGoal} loading={selectedGoalQuery.isFetching} onGoalIdChange={setSelectedGoalId} />
          )}
          {activeView === "control" && (
            <CompilerControlView goalId={selectedGoalId} snapshot={currentGoal} loading={selectedGoalQuery.isFetching} onGoalIdChange={setSelectedGoalId} />
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
    graph: "Technicolor Task Graph",
    control: "Compiler Controls",
    memory: "Shared Memory",
    plans: "Durable Plans",
    human: "Human Queue",
    runners: "Runner Fleet",
  }[view];
}

function ServiceStrip({ services }: { services: Overview["services"] }) {
  if (!services?.length) {
    return <span className="status-pill muted">No services projected</span>;
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

function CommandPanel(props: {
  messages: ChatMessage[];
  input: string;
  draftKind: DraftKind;
  busy: boolean;
  error: Error | null;
  latestResponse?: ChatResponse;
  chatRun?: ChatRunTrace;
  selectedGoalId: string;
  sessionId: string;
  mode: string;
  onDraftKindChange: (value: DraftKind) => void;
  onInputChange: (value: string) => void;
  onSend: (content?: string) => void;
  onClear: () => void;
}) {
  const draftKeys = Object.keys(props.latestResponse?.drafts ?? {});
  const activityPayload = chatActivityPayload(props);
  const activityLabel = props.busy ? "Activity" : "Run details";
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
      <div className="outcome-meta">
        <span className="status-pill">{props.selectedGoalId ? `Goal ${props.selectedGoalId.slice(0, 8)}` : "No goal selected"}</span>
        <span className={clsx("status-pill", props.busy ? "status-running" : "status-pending")}>
          {props.busy ? commandBusyLabel(props.draftKind) : `${props.mode} · ${props.sessionId}`}
        </span>
        {(props.busy || props.chatRun || props.latestResponse || draftKeys.length > 0) && (
          <InspectButton title="Chat activity" payload={activityPayload} buttonLabel={activityLabel} />
        )}
      </div>
      <div className="quick-prompts" aria-label="Compiler console prompts">
        {compilerPromptTemplates(props.selectedGoalId).map((template) => (
          <button key={template.label} type="button" className="secondary-button" disabled={props.busy} onClick={() => props.onSend(template.prompt)}>
            {template.icon === "graph" && <Network size={15} />}
            {template.icon === "control" && <ShieldCheck size={15} />}
            {template.icon === "research" && <Search size={15} />}
            {template.label}
          </button>
        ))}
      </div>
      <div className="chat-shell">
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
              disabled={props.busy}
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
        <button type="button" className="secondary-button" onClick={props.onClear}>
          Clear
        </button>
      </div>
    </section>
  );
}

function compilerPromptTemplates(goalId: string): Array<{ label: string; icon: "graph" | "control" | "research"; prompt: string }> {
  const goalClause = goalId ? ` for goal ${goalId}` : "";
  return [
    {
      label: "Explain graph",
      icon: "graph",
      prompt: `Summarize the current compute graph${goalClause}: runnable work, waiting thunks, blocked tasks, and the next control action.`,
    },
    {
      label: "Draft steering",
      icon: "control",
      prompt: `Draft one structured steering directive${goalClause} that would move the objective forward without bypassing coordinator review.`,
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
  latestResponse?: ChatResponse;
  chatRun?: ChatRunTrace;
}): JsonRecord {
  return {
    label: "operational trace",
    note: "This shows gateway and backend stages, not hidden model reasoning.",
    status: props.busy ? "running" : "idle",
    draft_kind: props.draftKind,
    mode: props.mode,
    selected_goal_id: props.selectedGoalId || null,
    session_id: props.sessionId,
    run: props.chatRun ?? props.latestResponse?.chat_run ?? null,
    backend: props.latestResponse?.chat_backend ?? props.chatRun?.backend ?? null,
    model_params: props.latestResponse?.model_params ?? props.chatRun?.model_params ?? null,
    chat_log: props.latestResponse?.chat_log ?? props.chatRun?.chat_log ?? null,
    drafts: props.latestResponse?.drafts ?? null,
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
      <MetricCard label="Runner lanes" value={String(runnerRows.length)} detail="available capacity" />
      <MetricCard label="Human queue" value={String(approvalRows.length)} detail="waiting decisions" />
      <MetricCard label="Events" value={String(eventRows.length)} detail="recent signals" />
      <MetricCard label="Event sources" value={String(eventSourceRows.length)} detail="registered ingress" />
      <section className="panel span-2">
        <div className="section-heading">
          <h2>Recent goals</h2>
          <span className="muted-small">Task graph</span>
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
          <OutcomeRow label="Runner lanes" value={runnerRows.length} tone={runnerRows.length ? "running" : "pending"} />
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
        empty="No event sources projected."
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
        <span className="muted-small">Clean progress cards, not workflow internals</span>
      </div>
      <GoalList goals={props.goals} selectedGoalId={props.selectedGoalId} onSelect={props.onSelectGoal} />
    </section>
  );
}

function GoalList({ goals, selectedGoalId, onSelect }: { goals: GoalRow[]; selectedGoalId: string; onSelect: (goalId: string) => void }) {
  if (!goals.length) {
    return <EmptyState title="No goals yet" detail="Submit or draft a goal to start a durable task graph." />;
  }
  return (
    <div className="goal-list">
      {goals.map((goal) => {
        const goalId = String(goal.goal_id ?? goal.id ?? "");
        const done = Math.round(Number(goal.percent_done ?? 0) * 100);
        return (
          <button key={goalId || goal.title} type="button" className={clsx("goal-card", selectedGoalId === goalId && "active")} onClick={() => goalId && onSelect(goalId)}>
            <div>
              <strong>{goal.title || goalId || "Untitled goal"}</strong>
              <span className={clsx("status-pill", statusTone(goal.status))}>{goal.status ?? "unknown"}</span>
            </div>
            <p>{goal.objective || "No projected objective."}</p>
            <div className="progress-line" aria-label={`${done}% complete`}>
              <span style={{ width: `${Math.max(0, Math.min(100, done))}%` }} />
            </div>
            <small>{done}% complete · {goal.open_tasks ?? 0} open · {goal.blocked_tasks ?? 0} blocked</small>
          </button>
        );
      })}
    </div>
  );
}

function TaskGraphView(props: { goalId: string; snapshot?: GoalSnapshot; loading: boolean; onGoalIdChange: (value: string) => void }) {
  const [graphFilter, setGraphFilter] = useState<GraphFilter>("all");
  const tasks = useMemo(() => (props.snapshot?.agent_activity ?? []) as TaskRow[], [props.snapshot]);
  const computeGraph = useMemo(() => props.snapshot ? workflowComputeGraph(props.snapshot) : undefined, [props.snapshot]);
  const computeNodes = useMemo(() => computeGraphNodes(computeGraph), [computeGraph]);
  const filteredComputeNodes = useMemo(() => computeNodes.filter((node) => computeNodeMatchesGraphFilter(node, graphFilter)), [computeNodes, graphFilter]);
  const filteredTasks = useMemo(() => tasks.filter((task) => taskMatchesGraphFilter(task, graphFilter)), [tasks, graphFilter]);
  const graph = useMemo(() => computeNodes.length ? graphFromComputeGraph(computeGraph, filteredComputeNodes) : graphFromTasks(filteredTasks), [computeGraph, computeNodes.length, filteredComputeNodes, filteredTasks]);
  const counts = useMemo(() => taskStatusCounts(tasks), [tasks]);
  const taskCount = tasks.length;
  const visibleCount = computeNodes.length ? filteredComputeNodes.length : filteredTasks.length;
  const totalCount = computeNodes.length ? computeNodes.length : taskCount;
  const graphUnit = computeNodes.length ? "compute nodes" : "tasks";
  return (
    <section className="panel graph-panel">
      <div className="section-heading">
        <div>
          <h2>Technicolor task graph</h2>
          <span className="muted-small">Color, role, status, and parent-child shape without prompt dumps</span>
        </div>
        <input
          className="goal-id-input"
          value={props.goalId}
          onChange={(event) => props.onGoalIdChange(event.target.value)}
          placeholder="Goal UUID"
          aria-label="Goal UUID"
        />
      </div>
      {props.snapshot && (
        <div className="graph-toolbar">
          <div className="graph-filter" aria-label="Task graph filter">
            {graphFilterOptions.map((option) => (
              <button
                key={option.key}
                type="button"
                className={clsx("graph-filter-button", graphFilter === option.key && "active")}
                aria-pressed={graphFilter === option.key}
                title={option.detail}
                onClick={() => setGraphFilter(option.key)}
              >
                {option.label}
              </button>
            ))}
          </div>
          <span className="filter-count">Showing {visibleCount} of {totalCount} {graphUnit}</span>
        </div>
      )}
      {!props.goalId ? (
        <EmptyState title="Choose a goal" detail="Open a goal from the dashboard or paste a goal UUID." />
      ) : props.loading ? (
        <EmptyState title="Loading task graph" detail="Fetching goal snapshot and agent activity." />
      ) : taskCount === 0 ? (
        <EmptyState title="No graph projection" detail="The goal exists, but no task activity is projected yet." />
      ) : graph.nodes.length ? (
        <div className="flow-canvas">
          <ReactFlow nodes={graph.nodes} edges={graph.edges} fitView>
            <MiniMap nodeColor={(node) => String(node.style?.borderColor ?? "var(--accent)")} maskColor="var(--flow-minimap-mask)" />
            <Controls />
            <Background color="var(--flow-dot)" gap={18} size={1.2} />
          </ReactFlow>
        </div>
      ) : (
        <EmptyState title="No tasks match this filter" detail="Change the graph filter to inspect other task states." />
      )}
      {props.snapshot && (
        <>
          <GraphStatusPanel counts={counts} taskCount={taskCount} />
          <TaskSummary snapshot={props.snapshot} counts={counts} />
          <ComputeGraphDetails snapshot={props.snapshot} />
          <ContinuationQueue goalId={props.goalId} snapshot={props.snapshot} />
          <CompilerControlPanel goalId={props.goalId} snapshot={props.snapshot} compact />
        </>
      )}
    </section>
  );
}

function CompilerControlView(props: { goalId: string; snapshot?: GoalSnapshot; loading: boolean; onGoalIdChange: (value: string) => void }) {
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>Flow control</h2>
          <span className="muted-small">Vote, steer, branch, restart, cancel, and resume durable work</span>
        </div>
        <input
          className="goal-id-input"
          value={props.goalId}
          onChange={(event) => props.onGoalIdChange(event.target.value)}
          placeholder="Goal UUID"
          aria-label="Goal UUID"
        />
      </div>
      {!props.goalId ? (
        <EmptyState title="Choose a goal" detail="Open a goal from the dashboard or paste a goal UUID." />
      ) : props.loading ? (
        <EmptyState title="Loading controls" detail="Fetching workflow projection." />
      ) : (
        <>
          {props.snapshot && <TaskSummary snapshot={props.snapshot} counts={taskStatusCounts((props.snapshot.agent_activity ?? []) as TaskRow[])} />}
          {props.snapshot && <ComputeGraphDetails snapshot={props.snapshot} />}
          <CompilerControlPanel goalId={props.goalId} snapshot={props.snapshot} />
          <ContinuationQueue goalId={props.goalId} snapshot={props.snapshot} />
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
    return {
      id,
      className: clsx("task-node", statusTone(status)),
      position: { x: (Number(taskPayload(task).depth ?? index) % 4) * 280, y: Math.floor(index / 4) * 150 },
      data: {
        label: `${color?.label ? `${color.label}: ` : ""}${task.title || id}\n${task.role ?? ""} · ${status}`,
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
    return {
      id,
      className: clsx("task-node", statusTone(status)),
      position: { x: (index % 4) * 285, y: Math.floor(index / 4) * 150 },
      data: {
        label: `${node.label || id}\n${kind} · ${status}`,
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
    return <EmptyState title="No compute graph nodes" detail="Workflow compute graph projections will appear here when available." />;
  }
  return (
    <div className="compute-details">
      <div className="section-heading">
        <h3>Compute graph</h3>
        <InspectButton title="Compute graph projection" payload={graph} buttonLabel="Inspect graph" />
      </div>
      <SimpleTable
        empty="No waiting or active compute nodes."
        headers={["Node", "Kind", "Status", "Wait / Continuation"]}
        rows={(openRows.length ? openRows : nodes.slice(0, 8)).map((node) => [
          stringValue(node.label) || stringValue(node.id) || "node",
          stringValue(node.kind) || "unknown",
          stringValue(node.status) || "unknown",
          [stringValue(node.wait_ref?.kind), stringValue(node.wait_ref?.reference), stringValue(node.continuation_id)].filter(Boolean).join(" · "),
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

function GraphStatusPanel({ counts, taskCount }: { counts: Map<string, number>; taskCount: number }) {
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const continuations = countForStatusToken(counts, "waiting-input");
  const running = countForStatusToken(counts, "running");
  const done = countForStatusToken(counts, "done");
  const attention = failed + blocked + approvals + continuations;
  return (
    <div className="graph-status-panel" aria-label="Task graph status legend">
      <div className={clsx("graph-attention", attention > 0 ? "needs-attention" : "stable")}>
        <strong>{attention > 0 ? `${attention} need attention` : "No urgent task states"}</strong>
        <span>{taskCount} tasks · {running} running · {done} done</span>
      </div>
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
    return <EmptyState title="No open continuations" detail="Delayed compute thunks that need operator input will appear here." />;
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
              <span>{row.status} · thunk {row.thunkId}</span>
              {row.taskId && <small>task {row.taskId}</small>}
              {row.continuationId && <small>continuation {row.continuationId}</small>}
              {row.waitReference && <small>{row.waitKind || "wait_ref"} · {row.waitReference}</small>}
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
  const [mechanismTitle, setMechanismTitle] = useState("Choose the next implementation lane");
  const [mechanismKind, setMechanismKind] = useState("approval_vote");
  const [mechanismTarget, setMechanismTarget] = useState("subgoal_selection");
  const [mechanismReason, setMechanismReason] = useState("Use a coordinator-owned round to choose the next task graph move.");
  const [mechanismProposals, setMechanismProposals] = useState("codex-fast | Fast Codex implementation lane | planner\nreview-deep | Deep reviewer-first lane | planner");
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

  return (
    <div className={clsx("compiler-control-panel", compact && "compact")}>
      <div className="section-heading">
        <div>
          <h3>Compiler controls</h3>
          <span className="muted-small">Goal ranking, steering, flow control, thunks, and mechanism rounds</span>
        </div>
        {result ? <InspectButton title="Last control action" payload={result} buttonLabel="Inspect result" /> : <span className="status-pill muted">No action yet</span>}
      </div>
      <div className="control-grid">
        <section className="control-card">
          <div className="section-heading">
            <h4>Vote</h4>
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
            <h4>Steer</h4>
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
              <input value={steerTaskId} onChange={(event) => setSteerTaskId(event.target.value)} placeholder={firstTaskId || "optional task id"} />
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
            <h4>Flow</h4>
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
              <input value={flowMode === "branch" ? branchTargetTaskId : restartTaskId} onChange={(event) => flowMode === "branch" ? setBranchTargetTaskId(event.target.value) : setRestartTaskId(event.target.value)} placeholder={firstTaskId || "optional task id"} />
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
              <input value={branchGroupId} onChange={(event) => setBranchGroupId(event.target.value)} disabled={flowMode !== "select_branch"} placeholder="branch group uuid" />
            </label>
            <label>
              Selected task
              <input value={selectedTaskId} onChange={(event) => setSelectedTaskId(event.target.value)} disabled={flowMode !== "select_branch"} placeholder="candidate task uuid" />
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
              Run flow action
            </button>
          </div>
        </section>

        <section className="control-card">
          <div className="section-heading">
            <h4>Thunk</h4>
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
            <h4>Mechanism round</h4>
            <Vote size={17} />
          </div>
          <div className="form-grid three">
            <label>
              Title
              <input value={mechanismTitle} onChange={(event) => setMechanismTitle(event.target.value)} />
            </label>
            <label>
              Mechanism
              <select value={mechanismKind} onChange={(event) => setMechanismKind(event.target.value)}>
                {mechanismKinds.map((kind) => <option key={kind} value={kind}>{kind}</option>)}
              </select>
            </label>
            <label>
              Target
              <select value={mechanismTarget} onChange={(event) => setMechanismTarget(event.target.value)}>
                {mechanismTargets.map((target) => <option key={target} value={target}>{target}</option>)}
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
            <InspectButton title="Mechanism proposal format" payload={{ format: "label | description | proposer", example: mechanismProposals }} buttonLabel="Format" />
          </div>
        </section>

        <section className="control-card">
          <div className="section-heading">
            <h4>Ballot</h4>
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
              {selectedGoalId ? selectedGoalId.slice(0, 8) : "Select goal"}
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
    return <span className="status-pill muted">No preview</span>;
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
    return <EmptyState title="No replacement preview" detail="Preview a memory edit." />;
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
        empty="No diff rows."
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
    return <EmptyState title="No goal selected" detail="Choose a goal to inspect memory events." />;
  }
  if (loading && !value) {
    return <EmptyState title="Loading memory events" detail="Fetching memory event history." />;
  }
  const rows = rowsFrom(at(value, ["events"]) ?? value).slice(-10).reverse();
  return (
    <SimpleTable
      empty="No memory events projected."
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
          empty="No plans projected."
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
          Continuation work belongs to durable plans, goals, events, or human queue items. Repo markdown follow-ups are kept for developer doc gardening, not as the product queue.
        </p>
        <InspectButton title="Plan projection" payload={planQuery.data ?? {}} buttonLabel="Inspect plans" />
      </div>
    </section>
  );
}

function HumanQueueView({ selectedGoalId }: { selectedGoalId: string }) {
  const approvalQuery = useQuery({ queryKey: ["approvals"], queryFn: approvals });
  const threadQuery = useQuery({ queryKey: ["threads"], queryFn: threads });
  const approvalRows = rowsFrom(at(approvalQuery.data, ["data"]) ?? approvalQuery.data);
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
          <h2>Approvals</h2>
          <span className="muted-small">Human gates remain explicit</span>
        </div>
        <ApprovalList
          rows={approvalRows}
          selectedGoalId={selectedGoalId}
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
          {threadRows.length ? threadRows.map((row) => <li key={String(row.thread_key ?? row.key ?? JSON.stringify(row))}><strong>{String(row.thread_key ?? row.key ?? "thread")}</strong><span>{String(row.entries ?? row.reports ?? "")} entries</span></li>) : <li>No local threads.</li>}
        </ul>
      </div>
    </section>
  );
}

function ApprovalList({
  rows,
  selectedGoalId,
  busy,
  onApprove,
}: {
  rows: JsonRecord[];
  selectedGoalId: string;
  busy: boolean;
  onApprove?: (approvalId: string, goalId: string) => void;
}) {
  return (
    <div className="approval-list">
      {rows.length ? rows.map((row) => {
        const id = String(row.approval_id ?? row.id ?? "");
        const goalId = String(row.goal_id ?? selectedGoalId ?? "");
        return (
          <div key={id || JSON.stringify(row)} className="approval-card">
            <div>
              <strong>{String(row.risk ?? row.title ?? "Approval")}</strong>
              <span>{String(row.status ?? "pending")} · {String(row.goal_id ?? "")}</span>
            </div>
            <button
              type="button"
              className="secondary-button"
              disabled={!id || !goalId || busy || !onApprove}
              onClick={() => onApprove?.(id, goalId)}
            >
              Approve
            </button>
          </div>
        );
      }) : <EmptyState title="No approvals" detail="Blocked or risky work will appear here." />}
    </div>
  );
}

function RunnersView({ overview }: { overview?: Overview }) {
  const rows = rowsFrom(at(overview, ["runner_status", "data"]) ?? overview?.runner_status);
  const tableRows = rows.map(runnerTableRow);
  return (
    <section className="panel">
      <div className="section-heading">
        <div>
          <h2>Runner fleet</h2>
          <span className="muted-small">Persistent and ephemeral capacity reported through the same registry</span>
        </div>
        <Server size={18} />
      </div>
      <SimpleTable
        empty="No runners registered."
        headers={["Runner", "Node", "Status", "Capacity", "Endpoint"]}
        rows={tableRows}
      />
    </section>
  );
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
  const status = stringValue(row.status) || (row.stale ? "stale" : row.full ? "full" : row.dispatchable === false ? "unavailable" : "active");
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
    displayName === runnerId ? runnerId : `${displayName} (${runnerId})`,
    nodeId,
    status,
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
    return <EmptyState title="No memory result yet" detail="Search, build context, or save a note." />;
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

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="empty-state">
      <CircleAlert size={18} />
      <strong>{title}</strong>
      <span>{detail}</span>
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
