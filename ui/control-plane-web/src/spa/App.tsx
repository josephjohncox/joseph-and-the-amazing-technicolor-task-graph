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
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Background, Controls, MarkerType, MiniMap, ReactFlow, type Edge, type Node } from "@xyflow/react";
import clsx from "clsx";
import {
  Bell,
  Brain,
  CheckCircle2,
  CircleAlert,
  ClipboardList,
  GitBranch,
  ListChecks,
  MessageSquareText,
  Monitor,
  Moon,
  Network,
  RefreshCw,
  Route,
  Search,
  Send,
  Server,
  Settings2,
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
  draftFollowUpPlan,
  followUps,
  goalSnapshot,
  goals,
  isRecord,
  memoryContext,
  memorySearch,
  memoryWrite,
  overview,
  plans,
  rowsFrom,
  setAuthToken,
  steer,
  threads,
} from "./api";
import type { ChatMessage, ColorRef, GoalRow, GoalSnapshot, JsonRecord, Overview, TaskRow } from "./types";

type ViewKey = "dashboard" | "goals" | "graph" | "memory" | "plans" | "followups" | "human" | "runners";
type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";
type GraphFilter = "all" | "attention" | "active" | "completed";
type FollowUpItem = { plan: string; path: string; index: number; text: string };

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
  { key: "followups", label: "Follow-Ups", icon: ClipboardList },
  { key: "human", label: "Human Queue", icon: Bell },
  { key: "runners", label: "Runners", icon: Server },
];

const starterMessages: ChatMessage[] = [
  {
    role: "assistant",
    content:
      "Tell me the outcome you want. I can draft a goal, durable plan, steering directive, or memory search without changing durable state.",
  },
];

export function App() {
  const queryClient = useQueryClient();
  const [activeView, setActiveView] = useState<ViewKey>("dashboard");
  const [selectedGoalId, setSelectedGoalId] = useState("");
  const [token, setToken] = useState(authToken());
  const [messages, setMessages] = useState<ChatMessage[]>(starterMessages);
  const [chatInput, setChatInput] = useState("");
  const [chatMode, setChatMode] = useState("draft_plan");
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => initialThemePreference());
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() => resolveTheme(initialThemePreference()));

  const overviewQuery = useQuery({ queryKey: ["overview"], queryFn: overview });
  const goalsQuery = useQuery({ queryKey: ["goals"], queryFn: goals });
  const selectedGoalQuery = useQuery({
    queryKey: ["goal", selectedGoalId],
    queryFn: () => goalSnapshot(selectedGoalId),
    enabled: Boolean(selectedGoalId),
  });

  const sendChat = useMutation({
    mutationFn: async () => {
      const content = chatInput.trim();
      if (!content) {
        throw new Error("Write a request first.");
      }
      const nextMessages = [...messages, { role: "user" as const, content }];
      setMessages(nextMessages);
      setChatInput("");
      const response = await chat(chatMode, selectedGoalId, nextMessages);
      setMessages([...nextMessages, { role: "assistant", content: response.assistant ?? "No response." }]);
      return response;
    },
  });

  const refreshAll = () => {
    void queryClient.invalidateQueries();
  };

  const draftFollowUp = (item: FollowUpItem) => {
    setChatMode("draft_plan");
    sendChat.reset();
    void draftFollowUpPlan({ ...item }).then((draft) => {
      setChatMode(draft.mode ?? "draft_plan");
      setChatInput(String(draft.prompt ?? ""));
    }).catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      setChatInput(`Gateway failed to draft this follow-up prompt through /api/follow-ups/draft-plan: ${message}`);
    });
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
          <div className="brand-mark">JT</div>
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
            mode={chatMode}
            busy={sendChat.isPending}
            error={sendChat.error}
            latestDrafts={sendChat.data?.drafts}
            selectedGoalId={selectedGoalId}
            onModeChange={setChatMode}
            onInputChange={setChatInput}
            onSend={() => sendChat.mutate()}
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
          {activeView === "followups" && <FollowUpsView onDraftPlan={draftFollowUp} />}
          {activeView === "human" && <HumanQueueView selectedGoalId={selectedGoalId} />}
          {activeView === "runners" && <RunnersView overview={overviewData} />}
        </section>
      </main>
    </div>
  );
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
    followups: "Follow-Ups",
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
  mode: string;
  busy: boolean;
  error: Error | null;
  latestDrafts?: JsonRecord;
  selectedGoalId: string;
  onModeChange: (value: string) => void;
  onInputChange: (value: string) => void;
  onSend: () => void;
  onClear: () => void;
}) {
  return (
    <section className="command-panel">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Ask COAT</p>
          <h2>Plan, steer, search, or summarize</h2>
        </div>
        <select value={props.mode} onChange={(event) => props.onModeChange(event.target.value)} aria-label="Assistant mode">
          <option value="draft_plan">Draft durable plan</option>
          <option value="draft_goal">Draft goal</option>
          <option value="draft_steering">Draft steering</option>
          <option value="explain_state">Explain state</option>
          <option value="general">General</option>
        </select>
      </div>
      <div className="chat-row">
        <div className="chat-log">
          {props.messages.map((message, index) => (
            <div key={`${message.role}-${index}`} className={clsx("chat-bubble", message.role)}>
              {message.content}
            </div>
          ))}
        </div>
        <div className="draft-card">
          <Sparkles size={18} />
          <strong>Drafts are review-only</strong>
          <span>Goal ID context: {props.selectedGoalId || "none"}</span>
          {props.latestDrafts && Object.keys(props.latestDrafts).length > 0 && (
            <InspectButton title="Latest assistant draft" payload={props.latestDrafts} />
          )}
        </div>
      </div>
      <div className="composer">
        <textarea
          value={props.input}
          onChange={(event) => props.onInputChange(event.target.value)}
          placeholder="Example: Find the safest way to add ephemeral runner templates and update the goal plan before implementation."
        />
        <div className="composer-actions">
          {props.error && <span className="error-text">{props.error.message}</span>}
          <button type="button" className="secondary-button" onClick={props.onClear}>
            Clear
          </button>
          <button type="button" className="primary-button" onClick={props.onSend} disabled={props.busy}>
            <Send size={16} />
            {props.busy ? "Sending" : "Send"}
          </button>
        </div>
      </div>
    </section>
  );
}

function Dashboard(props: { overview?: Overview; goals: GoalRow[]; selectedGoalId: string; onSelectGoal: (goalId: string) => void }) {
  const runnerRows = rowsFrom(at(props.overview, ["runner_status", "data"]) ?? props.overview?.runner_status);
  const approvalRows = rowsFrom(at(props.overview, ["approvals", "data"]) ?? props.overview?.approvals);
  const followUps = rowsFrom(props.overview?.follow_ups);
  return (
    <div className="dashboard-grid">
      <MetricCard label="Active goals" value={String(props.goals.length)} detail="projected by goal-store" />
      <MetricCard label="Runner lanes" value={String(runnerRows.length)} detail="registered or recently seen" />
      <MetricCard label="Human queue" value={String(approvalRows.length)} detail="approvals and feedback" />
      <MetricCard label="Plan follow-ups" value={String(followUps.length || Number(at(props.overview?.follow_ups, ["follow_up_count"]) ?? 0))} detail="open planning work" />
      <section className="panel span-2">
        <div className="section-heading">
          <h2>Recent goals</h2>
          <span className="muted-small">Click a goal to open its task graph</span>
        </div>
        <GoalList goals={props.goals.slice(0, 6)} selectedGoalId={props.selectedGoalId} onSelect={props.onSelectGoal} />
      </section>
      <section className="panel">
        <div className="section-heading">
          <h2>Operating boundary</h2>
          <Settings2 size={18} />
        </div>
        <p className="body-copy">
          This manager submits signals and reads projections. Restate owns time, the coordinator owns task truth, and
          runners only provide bounded execution capacity.
        </p>
      </section>
    </div>
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
  const [query, setQuery] = useState("");
  const [note, setNote] = useState("");
  const [result, setResult] = useState<unknown>(null);
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
    onSuccess: setResult,
  });
  const busy = searchMutation.isPending || contextMutation.isPending || writeMutation.isPending;
  return (
    <section className="memory-layout">
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
          <h2>Memory results</h2>
          <span className="muted-small">Scoped by goal when selected</span>
        </div>
        <ResultList value={result} />
      </div>
    </section>
  );
}

function PlansView() {
  const planQuery = useQuery({ queryKey: ["plans"], queryFn: plans });
  const followUpQuery = useQuery({ queryKey: ["follow-ups"], queryFn: followUps });
  const rows = rowsFrom(at(planQuery.data, ["data"]) ?? planQuery.data);
  const followUpRows = followUpList(followUpQuery.data);
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
          <h2>Follow-ups</h2>
          <GitBranch size={18} />
        </div>
        <ul className="compact-list">
          {followUpRows.length ? followUpRows.slice(0, 12).map((item) => <li key={`${item.plan}-${item.text}`}><strong>{item.plan}</strong><span>{item.text}</span></li>) : <li>No active follow-ups.</li>}
        </ul>
      </div>
    </section>
  );
}

function FollowUpsView({ onDraftPlan }: { onDraftPlan: (item: FollowUpItem) => void }) {
  const [query, setQuery] = useState("");
  const followUpQuery = useQuery({ queryKey: ["follow-ups"], queryFn: followUps });
  const followUpRows = followUpList(followUpQuery.data);
  const normalizedQuery = query.trim().toLowerCase();
  const filteredRows = normalizedQuery
    ? followUpRows.filter((item) => `${item.plan} ${item.path} ${item.text}`.toLowerCase().includes(normalizedQuery))
    : followUpRows;
  const planCount = Number(at(followUpQuery.data, ["plan_count"]) ?? 0);
  const checkedDirs = at(followUpQuery.data, ["checked_plan_dirs"]);
  const checkedDirText = Array.isArray(checkedDirs) ? checkedDirs.map(String).join(", ") : String(at(followUpQuery.data, ["plan_dir"]) ?? "");
  return (
    <section className="dashboard-grid">
      <MetricCard label="Follow-ups" value={String(followUpRows.length)} detail="open continuation items" />
      <MetricCard label="Plans with queue" value={String(planCount)} detail="active exec plans scanned" />
      <MetricCard label="Visible now" value={String(filteredRows.length)} detail={query ? "matching current filter" : "all active items"} />
      <div className="panel span-2">
        <div className="section-heading">
          <div>
            <h2>Continuation queue</h2>
            <span className="muted-small">Execution-plan follow-ups that should continue across sessions</span>
          </div>
          <InspectButton title="Follow-up projection" payload={followUpQuery.data} />
        </div>
        <label>
          Filter follow-ups
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search by plan, path, or follow-up text" />
        </label>
        <div className="followup-list">
          {filteredRows.length ? (
            filteredRows.map((item) => (
              <article key={`${item.path}-${item.index}-${item.text}`} className="followup-card">
                <div>
                  <strong>{item.plan}</strong>
                  <span className="status-pill status-runnable">open</span>
                </div>
                <p>{item.text}</p>
                <div className="followup-meta">
                  <small>{item.path}</small>
                  <button type="button" className="secondary-button" onClick={() => onDraftPlan(item)}>
                    <MessageSquareText size={15} />
                    Draft plan from follow-up
                  </button>
                </div>
              </article>
            ))
          ) : (
            <EmptyState title={followUpRows.length ? "No follow-ups match" : "No active follow-ups"} detail="Adjust the filter or add a follow-up under an active execution plan." />
          )}
        </div>
      </div>
      <div className="panel">
        <div className="section-heading">
          <h2>Source</h2>
          <GitBranch size={18} />
        </div>
        <p className="body-copy">
          Follow-ups are parsed from active execution plans. They are not durable workflow truth, but they are the
          operator continuity queue for carrying work across turns.
        </p>
        <div className="source-path">{checkedDirText || "No execution-plan directory projected."}</div>
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
    mutationFn: (approvalId: string) => {
      if (!selectedGoalId) throw new Error("Select a goal before approving.");
      return api(`/api/goals/${encodeURIComponent(selectedGoalId)}/approve`, {
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
        <div className="approval-list">
          {approvalRows.length ? approvalRows.map((row) => {
            const id = String(row.approval_id ?? row.id ?? "");
            return (
              <div key={id || JSON.stringify(row)} className="approval-card">
                <div>
                  <strong>{String(row.risk ?? row.title ?? "Approval")}</strong>
                  <span>{String(row.status ?? "pending")} · {String(row.goal_id ?? "")}</span>
                </div>
                <button type="button" className="secondary-button" disabled={!id || approvalMutation.isPending} onClick={() => approvalMutation.mutate(id)}>
                  Approve
                </button>
              </div>
            );
          }) : <EmptyState title="No approvals" detail="Blocked or risky work will appear here." />}
        </div>
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

function RunnersView({ overview }: { overview?: Overview }) {
  const rows = rowsFrom(at(overview, ["runner_status", "data"]) ?? overview?.runner_status);
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
        rows={rows.map((row) => [
          String(row.runner_id ?? row.id ?? ""),
          String(row.node_id ?? ""),
          String(row.status ?? (row.stale ? "stale" : "active")),
          String(row.capacity_remaining ?? row.running_tasks ?? ""),
          String(row.endpoint ?? ""),
        ])}
      />
    </section>
  );
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

function followUpList(value: unknown): FollowUpItem[] {
  const items = at(value, ["items"]);
  if (Array.isArray(items)) {
    return items.filter(isRecord).map((item, index) => ({
      plan: String(item.plan ?? item.title ?? item.source_plan ?? "Execution plan"),
      path: String(item.path ?? item.source_path ?? ""),
      index: Number.isFinite(Number(item.index)) ? Number(item.index) : index,
      text: String(item.text ?? item.follow_up ?? item.followup ?? ""),
    })).filter((item) => item.text.trim());
  }
  const plansValue = at(value, ["plans"]);
  const records = Array.isArray(plansValue) ? plansValue.filter(isRecord) : [];
  return records.flatMap((record) => {
    const title = String(record.title ?? record.path ?? "Plan");
    const path = String(record.path ?? "");
    const followUps = Array.isArray(record.follow_ups) ? record.follow_ups : [];
    return followUps.map((item, index) => ({ plan: title, path, index, text: String(item) }));
  });
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
