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
  GitBranch,
  ListChecks,
  MessageSquareText,
  Monitor,
  Moon,
  Network,
  RefreshCw,
  Route,
  Search,
  Server,
  Sparkles,
  Sun,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  api,
  approvals,
  at,
  authToken,
  chat,
  chatRun,
  chatSession,
  goalSnapshot,
  goals,
  isRecord,
  memoryContext,
  memoryEdit,
  memoryEditPreview,
  memoryEvents,
  memorySearch,
  memoryWrite,
  overview,
  plans,
  rowsFrom,
  setAuthToken,
  steer,
  threads,
} from "./api";
import type { ChatMessage, ChatResponse, ChatRunTrace, ColorRef, GoalRow, GoalSnapshot, JsonRecord, Overview, TaskRow } from "./types";

type ViewKey = "dashboard" | "goals" | "graph" | "memory" | "plans" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";
type GraphFilter = "all" | "attention" | "active" | "completed";
type DraftKind = "plan" | "goal" | "search";

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
  "done",
  "blocked",
  "failed",
  "cancelled",
]);
const statusLegend = [
  { token: "failed", label: "Failed", detail: "needs operator or retry" },
  { token: "blocked", label: "Blocked", detail: "waiting on dependency" },
  { token: "waiting-approval", label: "Approval", detail: "human gate open" },
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
  { key: "attention", label: "Attention", detail: "failed, blocked, approval" },
  { key: "active", label: "Active", detail: "running, runnable, validation" },
  { key: "completed", label: "Completed", detail: "done or cancelled" },
];

const views: Array<{ key: ViewKey; label: string; icon: typeof Route }> = [
  { key: "dashboard", label: "Dashboard", icon: Route },
  { key: "goals", label: "Goals", icon: ListChecks },
  { key: "graph", label: "Task Graph", icon: Network },
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
  const filteredTasks = useMemo(() => tasks.filter((task) => taskMatchesGraphFilter(task, graphFilter)), [tasks, graphFilter]);
  const graph = useMemo(() => graphFromTasks(filteredTasks), [filteredTasks]);
  const counts = useMemo(() => taskStatusCounts(tasks), [tasks]);
  const taskCount = tasks.length;
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
          <span className="filter-count">Showing {filteredTasks.length} of {taskCount} tasks</span>
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
    return status === "failed" || status === "blocked" || status === "waiting-approval";
  }
  if (filter === "active") {
    return status === "running" || status === "runnable" || status === "needs-validation";
  }
  return status === "done" || status === "cancelled";
}

function TaskSummary({ snapshot, counts }: { snapshot: GoalSnapshot; counts: Map<string, number> }) {
  const entries = sortedStatusEntries(counts);
  return (
    <div className="summary-row">
      {entries.map(([status, count]) => (
        <span key={status} className={clsx("status-pill", statusTone(status))}>{status}: {count}</span>
      ))}
      <InspectButton title="Goal snapshot" payload={snapshot} />
    </div>
  );
}

function GraphStatusPanel({ counts, taskCount }: { counts: Map<string, number>; taskCount: number }) {
  const failed = countForStatusToken(counts, "failed");
  const blocked = countForStatusToken(counts, "blocked");
  const approvals = countForStatusToken(counts, "waiting-approval");
  const running = countForStatusToken(counts, "running");
  const done = countForStatusToken(counts, "done");
  const attention = failed + blocked + approvals;
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
