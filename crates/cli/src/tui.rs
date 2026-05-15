//! Terminal operator UI for COAT.
//!
//! This module intentionally mirrors the TypeScript control gateway instead of
//! bypassing it. The TUI reads `/api/operator/workspace` and `/api/chat*` from
//! the gateway, so terminal chat remains an operator surface
//! over backend APIs rather than a second durable engine.

use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Wrap,
    },
};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use uuid::Uuid;

const CANCEL_CONFIRM_WINDOW: Duration = Duration::from_secs(8);

#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub control_gateway_url: String,
    pub token: Option<String>,
    pub session_id: String,
    pub goal_id: Option<String>,
    pub refresh: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatLine {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatMode {
    General,
    Goal,
    Plan,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiFocus {
    Dashboard,
    Chat,
    Input,
}

impl TuiFocus {
    fn next(self) -> Self {
        match self {
            Self::Dashboard => Self::Chat,
            Self::Chat => Self::Input,
            Self::Input => Self::Dashboard,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Dashboard => Self::Input,
            Self::Chat => Self::Dashboard,
            Self::Input => Self::Chat,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Chat => "chat",
            Self::Input => "input",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardView {
    Overview,
    Goals,
    Graph,
    Actions,
    Approvals,
    Events,
    Workers,
    Evidence,
    Adversarial,
    Debug,
}

impl DashboardView {
    const ALL: [Self; 10] = [
        Self::Overview,
        Self::Goals,
        Self::Graph,
        Self::Actions,
        Self::Approvals,
        Self::Events,
        Self::Workers,
        Self::Evidence,
        Self::Adversarial,
        Self::Debug,
    ];

    fn next(self) -> Self {
        let index = self.index();
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        let index = self.index();
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Goals => 1,
            Self::Graph => 2,
            Self::Actions => 3,
            Self::Approvals => 4,
            Self::Events => 5,
            Self::Workers => 6,
            Self::Evidence => 7,
            Self::Adversarial => 8,
            Self::Debug => 9,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Goals => "Goals",
            Self::Graph => "Graph",
            Self::Actions => "Actions",
            Self::Approvals => "Approvals",
            Self::Events => "Events",
            Self::Workers => "Workers",
            Self::Evidence => "Evidence",
            Self::Adversarial => "Adversarial",
            Self::Debug => "Debug",
        }
    }

    fn key_hint(self) -> &'static str {
        match self {
            Self::Overview => "1",
            Self::Goals => "2",
            Self::Graph => "3",
            Self::Actions => "4",
            Self::Approvals => "5",
            Self::Events => "6",
            Self::Workers => "7",
            Self::Evidence => "8",
            Self::Adversarial => "9",
            Self::Debug => "0",
        }
    }

    fn title(self) -> String {
        format!("{} ({})", self.label(), self.key_hint())
    }
}

fn dashboard_view_uses_action_queue(view: DashboardView) -> bool {
    matches!(view, DashboardView::Actions | DashboardView::Approvals)
}

#[derive(Debug, Clone, PartialEq)]
struct ActiveGoalDraft {
    goal_spec: Value,
    summary: GoalDraftSummary,
    session_id: String,
    selected_goal_id: Option<String>,
}

enum PendingRequestKind {
    Chat {
        payload: Value,
    },
    GoalSubmit {
        goal_spec: Value,
        draft_summary: Option<GoalDraftSummary>,
    },
    OperatorAction {
        label: String,
    },
}

struct PendingGatewayRequest {
    kind: PendingRequestKind,
    handle: JoinHandle<Result<Value, String>>,
}

impl ChatMode {
    fn next(self) -> Self {
        match self {
            Self::General => Self::Goal,
            Self::Goal => Self::Plan,
            Self::Plan => Self::Search,
            Self::Search => Self::General,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Goal => "draft_goal",
            Self::Plan => "draft_plan",
            Self::Search => "draft_search",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Goal => "goal",
            Self::Plan => "plan",
            Self::Search => "search",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DashboardSummary {
    services_ok: usize,
    services_total: usize,
    goals_count: usize,
    runners_count: usize,
    approvals_count: usize,
    events_count: usize,
    plans_count: usize,
    chat_backend: String,
    latest_goals: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct GoalSummary {
    id: String,
    title: String,
    status: String,
    progress: f64,
    open_tasks: usize,
    blocked_tasks: usize,
    current_blocker: Option<String>,
    current_action: Option<String>,
    latest_evidence: Option<String>,
    next_task: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoalDraftSummary {
    title: String,
    objective: String,
    initial_tasks: usize,
    done_criteria: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ApprovalSummary {
    id: String,
    status: String,
    action: String,
    risk: Option<String>,
    goal_id: Option<String>,
    task_id: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EventSummary {
    id: String,
    kind: String,
    message: String,
    goal_id: Option<String>,
    task_id: Option<String>,
    source: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EventSourceSummary {
    id: String,
    kind: String,
    status: String,
    route: Option<String>,
    approval: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GraphNodeSummary {
    id: String,
    kind: String,
    status: String,
    label: String,
    goal_id: Option<String>,
    task_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct WorkerRunSummary {
    id: String,
    runner: String,
    status: String,
    task: Option<String>,
    endpoint: Option<String>,
    node: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct EvidenceSummary {
    id: String,
    kind: String,
    summary: String,
    source: Option<String>,
    goal_id: Option<String>,
    task_id: Option<String>,
}

impl GoalDraftSummary {
    fn chat_preview(&self) -> String {
        format!(
            "Goal draft ready.\nTitle: {}\nObjective: {}\nInitial tasks: {}\nDone criteria: {}\nAccept this exact draft with F5 or Ctrl-G.",
            self.title, self.objective, self.initial_tasks, self.done_criteria
        )
    }

    fn submit_confirmation(&self, goal_id: &str) -> String {
        format!(
            "Accepted draft and submitted goal to the coordinator.\ngoal_id: {goal_id}\ntitle: {}\nobjective: {}\ninitial_tasks: {}\ndone_criteria: {}",
            self.title, self.objective, self.initial_tasks, self.done_criteria
        )
    }
}

struct App {
    config: TuiConfig,
    client: reqwest::Client,
    dashboard: DashboardSummary,
    goals: Vec<GoalSummary>,
    approvals: Vec<ApprovalSummary>,
    events: Vec<EventSummary>,
    event_sources: Vec<EventSourceSummary>,
    graph_nodes: Vec<GraphNodeSummary>,
    worker_runs: Vec<WorkerRunSummary>,
    evidence: Vec<EvidenceSummary>,
    selected_goal_approvals: Vec<ApprovalSummary>,
    selected_goal_events: Vec<EventSummary>,
    selected_goal_graph_nodes: Vec<GraphNodeSummary>,
    selected_goal_worker_runs: Vec<WorkerRunSummary>,
    selected_goal_evidence: Vec<EvidenceSummary>,
    selected_goal_id: Option<String>,
    selected_goal_snapshot: Option<Value>,
    selected_goal_outline: Vec<String>,
    messages: Vec<ChatLine>,
    last_chat_response: Option<Value>,
    active_goal_draft: Option<ActiveGoalDraft>,
    chat_scroll_from_bottom: u16,
    input: String,
    status: String,
    mode: ChatMode,
    focus: TuiFocus,
    dashboard_view: DashboardView,
    dashboard_scroll: u16,
    action_index: usize,
    cancel_goal_confirmation: Option<(String, Instant)>,
    last_refresh: Option<Instant>,
    busy: bool,
    pending_request: Option<PendingGatewayRequest>,
}

impl App {
    fn new(config: TuiConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("build TUI HTTP client")?;
        let selected_goal_id = config
            .goal_id
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Ok(Self {
            client,
            dashboard: DashboardSummary::default(),
            goals: Vec::new(),
            approvals: Vec::new(),
            events: Vec::new(),
            event_sources: Vec::new(),
            graph_nodes: Vec::new(),
            worker_runs: Vec::new(),
            evidence: Vec::new(),
            selected_goal_approvals: Vec::new(),
            selected_goal_events: Vec::new(),
            selected_goal_graph_nodes: Vec::new(),
            selected_goal_worker_runs: Vec::new(),
            selected_goal_evidence: Vec::new(),
            selected_goal_id,
            selected_goal_snapshot: None,
            selected_goal_outline: Vec::new(),
            messages: Vec::new(),
            last_chat_response: None,
            active_goal_draft: None,
            chat_scroll_from_bottom: 0,
            input: String::new(),
            status: "starting".to_string(),
            mode: ChatMode::General,
            focus: TuiFocus::Input,
            dashboard_view: DashboardView::Overview,
            dashboard_scroll: 0,
            action_index: 0,
            cancel_goal_confirmation: None,
            last_refresh: None,
            busy: false,
            pending_request: None,
            config,
        })
    }

    async fn load_initial_state(&mut self) {
        if let Err(error) = self.refresh_dashboard().await {
            self.status = format!("dashboard refresh failed: {error}");
        }
        if let Err(error) = self.load_chat_session().await {
            self.status = format!("chat session load failed: {error}");
        }
    }

    async fn refresh_dashboard(&mut self) -> anyhow::Result<()> {
        let operator_path = match self.selected_goal_id.as_deref() {
            Some(goal_id) => format!(
                "/api/operator/workspace?goal_id={}",
                percent_encode(goal_id)
            ),
            None => "/api/operator/workspace".to_string(),
        };
        let operator_workspace = self.get_json(&operator_path).await?;
        let goals = self.get_json("/api/operator/goals?limit=100").await?;
        let workspace_goals = goal_summaries_from_value(&operator_workspace);
        self.goals = if workspace_goals.is_empty() {
            goal_summaries_from_value(&goals)
        } else {
            workspace_goals
        };
        self.approvals = operator_action_summaries_from_value(&operator_workspace);
        self.events = event_summaries_from_value(&operator_workspace);
        self.event_sources = event_source_summaries_from_value(&operator_workspace);
        self.graph_nodes = graph_node_summaries_from_value(&operator_workspace);
        self.worker_runs = worker_run_summaries_from_value(&operator_workspace);
        self.evidence = evidence_summaries_from_value(&operator_workspace);
        let mut status = "dashboard refreshed".to_string();
        if let Some(selected_goal) = operator_workspace.get("selected_goal")
            && !selected_goal.is_null()
        {
            if let Some(snapshot) = selected_goal.get("snapshot").cloned() {
                self.selected_goal_outline = goal_outline_from_snapshot(&snapshot);
                self.selected_goal_approvals =
                    operator_action_summaries_from_value(&operator_workspace);
                self.selected_goal_events = event_summaries_from_value(&operator_workspace);
                self.selected_goal_graph_nodes = graph_node_summaries_from_value(&snapshot);
                self.selected_goal_worker_runs = worker_run_summaries_from_value(&snapshot);
                self.selected_goal_evidence = evidence_summaries_from_value(&snapshot);
                if let Some(goal_id) = self.selected_goal_id.clone()
                    && let Some(summary) = goal_summary_from_snapshot(&snapshot, &goal_id)
                {
                    upsert_goal_summary(&mut self.goals, summary);
                }
                self.selected_goal_snapshot = Some(snapshot);
            }
        } else if let Err(error) = self.refresh_selected_goal_summary().await {
            status = format!("dashboard refreshed; selected goal snapshot failed: {error}");
        }
        let action_count = self.current_action_items().len();
        if action_count == 0 {
            self.action_index = 0;
        } else {
            self.action_index = self.action_index.min(action_count - 1);
        }
        self.dashboard = dashboard_summary(&operator_workspace, &self.goals);
        self.last_refresh = Some(Instant::now());
        self.status = status;
        Ok(())
    }

    async fn refresh_selected_goal_summary(&mut self) -> anyhow::Result<()> {
        let Some(goal_id) = self.selected_goal_id.clone() else {
            return Ok(());
        };
        self.selected_goal_snapshot = None;
        let path = format!("/api/operator/goals/{}", percent_encode(&goal_id));
        let detail = self.get_json(&path).await?;
        let snapshot = detail
            .get("snapshot")
            .cloned()
            .unwrap_or_else(|| detail.clone());
        self.selected_goal_outline = goal_outline_from_snapshot(&snapshot);
        self.selected_goal_approvals = action_needed_summaries_from_value(&snapshot);
        self.selected_goal_events = event_summaries_from_value(&snapshot);
        self.selected_goal_graph_nodes = graph_node_summaries_from_value(&snapshot);
        self.selected_goal_worker_runs = worker_run_summaries_from_value(&snapshot);
        self.selected_goal_evidence = evidence_summaries_from_value(&snapshot);
        if let Some(summary) = goal_summary_from_snapshot(&snapshot, &goal_id) {
            upsert_goal_summary(&mut self.goals, summary);
        }
        self.selected_goal_snapshot = Some(snapshot);
        Ok(())
    }

    async fn load_chat_session(&mut self) -> anyhow::Result<()> {
        let session_id = self.current_session_id();
        let path = format!(
            "/api/chat/session?session_id={}",
            percent_encode(&session_id)
        );
        let value = self.get_json(&path).await?;
        self.messages = chat_lines_from_session(&value);
        self.chat_scroll_from_bottom = 0;
        if self.messages.is_empty() {
            self.messages.push(ChatLine {
                role: "assistant".to_string(),
                content: "COAT terminal chat is connected to the control gateway. Type a request and press Enter.".to_string(),
            });
        }
        Ok(())
    }

    fn begin_send_chat(&mut self) -> anyhow::Result<()> {
        if self.pending_request.is_some() {
            self.status =
                "gateway request is still running; input is kept for the next turn".to_string();
            return Ok(());
        }
        let content = self.input.trim().to_string();
        if content.is_empty() {
            return Ok(());
        }
        self.input.clear();
        self.messages.push(ChatLine {
            role: "user".to_string(),
            content: content.clone(),
        });
        self.chat_scroll_from_bottom = 0;
        self.status = "generating response via control gateway; input cleared for the next message"
            .to_string();
        self.busy = true;

        let durable_messages = durable_chat_lines(&self.messages);
        let payload = chat_request_payload(
            self.current_session_id(),
            self.selected_goal_id.clone(),
            self.mode,
            &durable_messages,
        );
        self.pending_request = Some(PendingGatewayRequest {
            kind: PendingRequestKind::Chat {
                payload: payload.clone(),
            },
            handle: self.spawn_post_json("/api/chat", payload),
        });
        Ok(())
    }

    async fn poll_pending_request(&mut self) -> anyhow::Result<()> {
        let Some(pending) = self.pending_request.as_ref() else {
            return Ok(());
        };
        if !pending.handle.is_finished() {
            return Ok(());
        }
        let pending = self
            .pending_request
            .take()
            .expect("pending request was checked above");
        let result = match pending.handle.await {
            Ok(result) => result,
            Err(error) => Err(format!("gateway task failed: {error}")),
        };
        self.busy = false;
        match pending.kind {
            PendingRequestKind::Chat { payload } => {
                let _session_id = payload.get("session_id").and_then(Value::as_str);
                self.finish_chat(result)
            }
            PendingRequestKind::GoalSubmit {
                goal_spec,
                draft_summary,
            } => {
                self.finish_goal_submit(result, goal_spec, draft_summary)
                    .await
            }
            PendingRequestKind::OperatorAction { label } => {
                self.finish_operator_action(label, result).await
            }
        }
    }

    fn finish_chat(&mut self, response: Result<Value, String>) -> anyhow::Result<()> {
        match response {
            Ok(value) => {
                let assistant = value
                    .get("assistant")
                    .and_then(Value::as_str)
                    .unwrap_or("The control gateway returned no assistant text.")
                    .trim()
                    .to_string();
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content: assistant,
                });
                self.last_chat_response = Some(value.clone());
                self.chat_scroll_from_bottom = 0;
                if let Some(draft) = active_goal_draft_from_response(&value) {
                    self.active_goal_draft = Some(ActiveGoalDraft {
                        session_id: self.current_session_id(),
                        selected_goal_id: self.selected_goal_id.clone(),
                        ..draft.clone()
                    });
                    self.messages.push(ChatLine {
                        role: "assistant".to_string(),
                        content: draft.summary.chat_preview(),
                    });
                    self.status = format!(
                        "{}; goal draft ready, review dashboard or chat, F5/Ctrl-G accept, Ctrl-D discard",
                        chat_status(&value)
                    );
                } else if self.active_goal_draft.is_some() {
                    self.status = format!(
                        "{}; active goal draft still available, F5/Ctrl-G accept, Ctrl-D discard",
                        chat_status(&value)
                    );
                } else {
                    self.status = chat_status(&value);
                }
                Ok(())
            }
            Err(error) => {
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content: format!("Chat request failed: {error}"),
                });
                self.last_chat_response = None;
                self.chat_scroll_from_bottom = 0;
                self.status = format!("chat failed: {error}");
                Ok(())
            }
        }
    }

    fn begin_submit_goal_draft(&mut self) -> anyhow::Result<()> {
        if self.pending_request.is_some() {
            self.status =
                "gateway request is still running; wait before accepting the draft".to_string();
            return Ok(());
        }
        let Some(draft) = self.active_goal_draft.clone() else {
            self.status =
                "no chat goal draft is available; switch to goal mode and send a prompt first"
                    .to_string();
            return Ok(());
        };
        if draft.session_id != self.current_session_id()
            || draft.selected_goal_id != self.selected_goal_id
        {
            self.status =
                "active goal draft belongs to another chat context; switch back or discard it"
                    .to_string();
            return Ok(());
        }

        self.status = "accepting active goal draft with coordinator".to_string();
        self.busy = true;
        self.pending_request = Some(PendingGatewayRequest {
            kind: PendingRequestKind::GoalSubmit {
                goal_spec: draft.goal_spec.clone(),
                draft_summary: Some(draft.summary),
            },
            handle: self.spawn_post_json("/api/operator/goals", draft.goal_spec),
        });
        Ok(())
    }

    async fn finish_goal_submit(
        &mut self,
        response: Result<Value, String>,
        goal_spec: Value,
        draft_summary: Option<GoalDraftSummary>,
    ) -> anyhow::Result<()> {
        match response {
            Ok(value) => {
                let goal_id = submitted_goal_id(&value)
                    .or_else(|| {
                        goal_spec
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "assigned by coordinator".to_string());
                let content = draft_summary
                    .as_ref()
                    .map(|summary| summary.submit_confirmation(&goal_id))
                    .unwrap_or_else(|| {
                        format!("Accepted draft and submitted goal to the coordinator.\ngoal_id: {goal_id}")
                    });
                if goal_id != "assigned by coordinator" {
                    self.selected_goal_id = Some(goal_id.clone());
                }
                self.active_goal_draft = None;
                let mut status = format!("goal submitted: {goal_id}");
                if let Err(error) = self.refresh_dashboard().await {
                    status = format!("goal submitted, dashboard refresh failed: {error}");
                } else if let Err(error) = self.load_chat_session().await {
                    status = format!("goal submitted, chat reload failed: {error}");
                }
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content,
                });
                self.chat_scroll_from_bottom = 0;
                self.status = status;
                Ok(())
            }
            Err(error) => {
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content: format!("Goal submit failed: {error}"),
                });
                self.chat_scroll_from_bottom = 0;
                self.status = format!("goal submit failed: {error}");
                Ok(())
            }
        }
    }

    fn discard_goal_draft(&mut self) {
        if self.active_goal_draft.take().is_some() {
            self.messages.push(ChatLine {
                role: "assistant".to_string(),
                content: "Active goal draft discarded.".to_string(),
            });
            self.chat_scroll_from_bottom = 0;
            self.status = "active goal draft discarded".to_string();
        } else {
            self.status = "no active goal draft to discard".to_string();
        }
    }

    fn scoped_action_items(&self) -> Vec<ApprovalSummary> {
        if self.selected_goal_id.is_some() && !self.selected_goal_approvals.is_empty() {
            self.selected_goal_approvals.clone()
        } else {
            self.approvals.clone()
        }
    }

    fn current_action_items(&self) -> Vec<ApprovalSummary> {
        let items = self.scoped_action_items();
        if self.dashboard_view == DashboardView::Approvals {
            items.into_iter().filter(is_approval_gate_action).collect()
        } else {
            items
        }
    }

    fn selected_action_item(&self) -> Option<ApprovalSummary> {
        let items = self.current_action_items();
        if items.is_empty() {
            return None;
        }
        items
            .get(self.action_index.min(items.len().saturating_sub(1)))
            .cloned()
    }

    fn move_action_selection(&mut self, step: isize) {
        let len = self.current_action_items().len();
        if len == 0 {
            self.action_index = 0;
            self.status = "operator action queue is clear".to_string();
            return;
        }
        let current = self.action_index.min(len.saturating_sub(1));
        self.action_index = if step < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(len.saturating_sub(1))
        };
        self.status = format!("selected queue item {}/{}", self.action_index + 1, len);
    }

    fn begin_apply_selected_action(&mut self) -> anyhow::Result<()> {
        if self.pending_request.is_some() {
            self.status = "gateway request is still running; queue action deferred".to_string();
            return Ok(());
        }
        let Some(action) = self.selected_action_item() else {
            self.status = "operator action queue is clear".to_string();
            return Ok(());
        };
        let Some(goal_id) = action
            .goal_id
            .clone()
            .or_else(|| self.selected_goal_id.clone())
            .filter(|goal_id| !goal_id.trim().is_empty())
        else {
            self.status = "selected queue item is missing a goal id".to_string();
            return Ok(());
        };
        let (path, payload, label) = operator_action_request(&goal_id, &action, self.input.trim())?;
        if action_requires_input(&action) {
            self.input.clear();
        }
        self.status = format!("running queue action: {label}");
        self.busy = true;
        self.pending_request = Some(PendingGatewayRequest {
            kind: PendingRequestKind::OperatorAction {
                label: label.clone(),
            },
            handle: self.spawn_post_json(&path, payload),
        });
        Ok(())
    }

    fn begin_cancel_selected_goal(&mut self) -> anyhow::Result<()> {
        if self.pending_request.is_some() {
            self.status = "gateway request is still running; cancel deferred".to_string();
            return Ok(());
        }
        let Some(goal_id) = self
            .selected_goal_id
            .clone()
            .filter(|goal_id| !goal_id.trim().is_empty())
        else {
            self.status = "select a goal before cancelling".to_string();
            return Ok(());
        };

        let now = Instant::now();
        let cancel_is_confirmed =
            self.cancel_goal_confirmation
                .as_ref()
                .is_some_and(|(armed_goal_id, armed_at)| {
                    armed_goal_id == &goal_id
                        && now.duration_since(*armed_at) <= CANCEL_CONFIRM_WINDOW
                });
        if !cancel_is_confirmed {
            self.cancel_goal_confirmation = Some((goal_id.clone(), now));
            self.status = format!(
                "cancel armed for {}; type a reason if needed, then press Ctrl-X again within {}s",
                short_id(&goal_id),
                CANCEL_CONFIRM_WINDOW.as_secs()
            );
            return Ok(());
        }

        let reason = if self.input.trim().is_empty() {
            "cancelled from COAT TUI by operator".to_string()
        } else {
            self.input.trim().to_string()
        };
        let (path, payload, label) = cancel_goal_request(&goal_id, &reason);
        self.input.clear();
        self.cancel_goal_confirmation = None;
        self.status = format!("cancelling selected goal: {}", short_id(&goal_id));
        self.busy = true;
        self.pending_request = Some(PendingGatewayRequest {
            kind: PendingRequestKind::OperatorAction {
                label: label.clone(),
            },
            handle: self.spawn_post_json(&path, payload),
        });
        Ok(())
    }

    fn clear_local_messages_and_result(&mut self) {
        let message_count = self.messages.len();
        let had_action_result = self.last_chat_response.take().is_some();
        self.messages.clear();
        self.active_goal_draft = None;
        self.chat_scroll_from_bottom = 0;
        self.cancel_goal_confirmation = None;
        self.status = format!(
            "cleared {message_count} local messages and {} action result; durable chat is unchanged",
            if had_action_result { "the last" } else { "no" }
        );
    }

    async fn finish_operator_action(
        &mut self,
        label: String,
        response: Result<Value, String>,
    ) -> anyhow::Result<()> {
        match response {
            Ok(value) => {
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content: format!("Action applied: {label}"),
                });
                self.last_chat_response = Some(value);
                self.chat_scroll_from_bottom = 0;
                self.action_index = 0;
                let mut status = format!("queue action complete: {label}");
                if let Err(error) = self.refresh_dashboard().await {
                    status = format!("queue action complete, dashboard refresh failed: {error}");
                }
                self.status = status;
                Ok(())
            }
            Err(error) => {
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content: format!("Action failed: {label}: {error}"),
                });
                self.chat_scroll_from_bottom = 0;
                self.status = format!("queue action failed: {error}");
                Ok(())
            }
        }
    }

    fn spawn_post_json(&self, path: &str, payload: Value) -> JoinHandle<Result<Value, String>> {
        let client = self.client.clone();
        let token = self.config.token.clone();
        let url = self.url(path);
        let endpoint = path.to_string();
        tokio::spawn(async move {
            let mut request = client.post(url).json(&payload);
            if let Some(token) = token.as_deref().filter(|token| !token.is_empty()) {
                request = request.bearer_auth(token);
            }
            let response = request
                .send()
                .await
                .map_err(|error| format!("POST {endpoint}: {error}"))?;
            parse_response(response)
                .await
                .map_err(|error| error.to_string())
        })
    }

    async fn get_json(&self, path: &str) -> anyhow::Result<Value> {
        let response = self
            .auth(self.client.get(self.url(path)))
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        parse_response(response).await
    }

    fn auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self
            .config
            .token
            .as_deref()
            .filter(|token| !token.is_empty())
        {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}{}",
            self.config.control_gateway_url.trim_end_matches('/'),
            path
        )
    }

    fn current_session_id(&self) -> String {
        if self
            .selected_goal_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return chat_session_id_for(self.selected_goal_id.as_deref());
        }
        self.config
            .session_id
            .trim()
            .is_empty()
            .then(|| "operator:default".to_string())
            .unwrap_or_else(|| self.config.session_id.trim().to_string())
    }

    fn display_session_id(&self) -> String {
        if let Some(goal_id) = self
            .selected_goal_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            return format!("goal:{}", short_id(goal_id));
        }
        self.current_session_id()
    }

    fn selected_goal(&self) -> Option<&GoalSummary> {
        let selected = self.selected_goal_id.as_deref()?;
        self.goals.iter().find(|goal| goal.id == selected)
    }

    async fn select_goal_by_step(&mut self, step: isize) -> anyhow::Result<()> {
        let Some(goal_id) = goal_id_after_step(&self.goals, self.selected_goal_id.as_deref(), step)
        else {
            self.status = "no goals available to select".to_string();
            return Ok(());
        };
        self.selected_goal_id = Some(goal_id.clone());
        self.active_goal_draft = None;
        self.cancel_goal_confirmation = None;
        self.dashboard_scroll = 0;
        self.status = format!("selected goal: {}", short_id(&goal_id));
        if let Err(error) = self.refresh_selected_goal_summary().await {
            self.status = format!(
                "selected goal: {}; snapshot pending: {error}",
                short_id(&goal_id)
            );
        }
        self.load_chat_session().await
    }

    async fn clear_goal_selection(&mut self) -> anyhow::Result<()> {
        self.selected_goal_id = None;
        self.selected_goal_snapshot = None;
        self.selected_goal_outline.clear();
        self.selected_goal_approvals.clear();
        self.selected_goal_events.clear();
        self.selected_goal_graph_nodes.clear();
        self.selected_goal_worker_runs.clear();
        self.selected_goal_evidence.clear();
        self.active_goal_draft = None;
        self.cancel_goal_confirmation = None;
        self.dashboard_scroll = 0;
        self.status = "goal selection cleared".to_string();
        self.load_chat_session().await
    }

    fn select_dashboard_view(&mut self, view: DashboardView) {
        self.dashboard_view = view;
        self.dashboard_scroll = 0;
        self.focus = TuiFocus::Dashboard;
        self.status = format!("view: {}", view.label());
    }

    fn cycle_dashboard_view(&mut self, step: isize) {
        self.dashboard_view = if step < 0 {
            self.dashboard_view.previous()
        } else {
            self.dashboard_view.next()
        };
        self.dashboard_scroll = 0;
        self.status = format!("view: {}", self.dashboard_view.label());
    }
}

pub async fn run(config: TuiConfig) -> anyhow::Result<()> {
    let mut app = App::new(config)?;
    app.load_initial_state().await;

    let mut terminal = TerminalSession::enter()?;
    let result = run_loop(&mut terminal.terminal, &mut app).await;
    terminal.restore()?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|frame| render(frame, app))?;
        app.poll_pending_request().await?;
        if !app.busy
            && app
                .last_refresh
                .is_none_or(|last_refresh| last_refresh.elapsed() >= app.config.refresh)
        {
            if let Err(error) = app.refresh_dashboard().await {
                app.status = format!("dashboard refresh failed: {error}");
            }
        }
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && handle_key(app, key).await?
        {
            return Ok(());
        }
    }
}

async fn handle_key(app: &mut App, key: KeyEvent) -> anyhow::Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }
    match key.code {
        KeyCode::Esc => Ok(true),
        KeyCode::Char('q') if app.input.is_empty() => Ok(true),
        KeyCode::Tab => {
            app.focus = app.focus.next();
            app.status = format!("focus: {}", app.focus.label());
            Ok(false)
        }
        KeyCode::BackTab => {
            app.focus = app.focus.previous();
            app.status = format!("focus: {}", app.focus.label());
            Ok(false)
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = app.mode.next();
            app.status = format!("chat mode: {}", app.mode.label());
            Ok(false)
        }
        KeyCode::Char('1') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Overview);
            Ok(false)
        }
        KeyCode::Char('2') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Goals);
            Ok(false)
        }
        KeyCode::Char('3') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Graph);
            Ok(false)
        }
        KeyCode::Char('4') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Actions);
            Ok(false)
        }
        KeyCode::Char('5') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Approvals);
            Ok(false)
        }
        KeyCode::Char('6') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Events);
            Ok(false)
        }
        KeyCode::Char('7') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Workers);
            Ok(false)
        }
        KeyCode::Char('8') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Evidence);
            Ok(false)
        }
        KeyCode::Char('9') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Adversarial);
            Ok(false)
        }
        KeyCode::Char('0') if app.focus == TuiFocus::Dashboard => {
            app.select_dashboard_view(DashboardView::Debug);
            Ok(false)
        }
        KeyCode::Left if app.focus == TuiFocus::Dashboard => {
            app.cycle_dashboard_view(-1);
            Ok(false)
        }
        KeyCode::Right if app.focus == TuiFocus::Dashboard => {
            app.cycle_dashboard_view(1);
            Ok(false)
        }
        KeyCode::Enter => {
            if app.focus == TuiFocus::Dashboard
                && dashboard_view_uses_action_queue(app.dashboard_view)
            {
                app.begin_apply_selected_action()?;
            } else if app.focus == TuiFocus::Input {
                app.begin_send_chat()?;
            } else {
                let previous_focus = app.focus.label();
                app.focus = TuiFocus::Input;
                app.status = format!(
                    "input focused from {}; press Enter again to send",
                    previous_focus
                );
            }
            Ok(false)
        }
        KeyCode::Backspace => {
            if app.focus == TuiFocus::Input {
                app.input.pop();
            }
            Ok(false)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.clear();
            Ok(false)
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_local_messages_and_result();
            Ok(false)
        }
        KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.begin_cancel_selected_goal()?;
            Ok(false)
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.busy {
                app.status = "gateway request is running; dashboard refresh deferred".to_string();
            } else if let Err(error) = app.refresh_dashboard().await {
                app.status = format!("dashboard refresh failed: {error}");
            }
            Ok(false)
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.busy {
                app.status = "gateway request is running; goal selection is unchanged".to_string();
            } else {
                app.select_goal_by_step(1).await?;
            }
            Ok(false)
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.busy {
                app.status = "gateway request is running; goal selection is unchanged".to_string();
            } else {
                app.select_goal_by_step(-1).await?;
            }
            Ok(false)
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.busy {
                app.status = "gateway request is running; goal selection is unchanged".to_string();
            } else {
                app.clear_goal_selection().await?;
            }
            Ok(false)
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.begin_submit_goal_draft()?;
            Ok(false)
        }
        KeyCode::F(5) => {
            app.begin_submit_goal_draft()?;
            Ok(false)
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.discard_goal_draft();
            Ok(false)
        }
        KeyCode::Char('a')
            if app.focus == TuiFocus::Dashboard
                && dashboard_view_uses_action_queue(app.dashboard_view) =>
        {
            app.begin_apply_selected_action()?;
            Ok(false)
        }
        KeyCode::Up => {
            if app.focus == TuiFocus::Dashboard
                && dashboard_view_uses_action_queue(app.dashboard_view)
            {
                app.move_action_selection(-1);
            } else if app.focus == TuiFocus::Dashboard && app.dashboard_view == DashboardView::Goals
            {
                if app.busy {
                    app.status =
                        "gateway request is running; goal selection is unchanged".to_string();
                } else {
                    app.select_goal_by_step(-1).await?;
                }
            } else if app.focus == TuiFocus::Dashboard {
                app.dashboard_scroll = app.dashboard_scroll.saturating_sub(1);
            } else if app.focus == TuiFocus::Chat {
                app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_add(1);
            }
            Ok(false)
        }
        KeyCode::Down => {
            if app.focus == TuiFocus::Dashboard
                && dashboard_view_uses_action_queue(app.dashboard_view)
            {
                app.move_action_selection(1);
            } else if app.focus == TuiFocus::Dashboard && app.dashboard_view == DashboardView::Goals
            {
                if app.busy {
                    app.status =
                        "gateway request is running; goal selection is unchanged".to_string();
                } else {
                    app.select_goal_by_step(1).await?;
                }
            } else if app.focus == TuiFocus::Dashboard {
                app.dashboard_scroll = app.dashboard_scroll.saturating_add(1);
            } else if app.focus == TuiFocus::Chat {
                app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_sub(1);
            }
            Ok(false)
        }
        KeyCode::PageUp => {
            if app.focus == TuiFocus::Chat {
                app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_add(10);
            } else if app.focus == TuiFocus::Dashboard {
                app.dashboard_scroll = app.dashboard_scroll.saturating_sub(10);
            }
            Ok(false)
        }
        KeyCode::PageDown => {
            if app.focus == TuiFocus::Chat {
                app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_sub(10);
            } else if app.focus == TuiFocus::Dashboard {
                app.dashboard_scroll = app.dashboard_scroll.saturating_add(10);
            }
            Ok(false)
        }
        KeyCode::Home => {
            if app.focus == TuiFocus::Chat {
                app.chat_scroll_from_bottom = u16::MAX;
            } else if app.focus == TuiFocus::Dashboard {
                app.dashboard_scroll = 0;
            }
            Ok(false)
        }
        KeyCode::End => {
            if app.focus == TuiFocus::Chat {
                app.chat_scroll_from_bottom = 0;
            } else if app.focus == TuiFocus::Dashboard {
                app.dashboard_scroll = u16::MAX;
            }
            Ok(false)
        }
        KeyCode::Char(ch)
            if app.focus == TuiFocus::Input
                && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
        {
            app.input.push(ch);
            Ok(false)
        }
        _ => Ok(false),
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    restored: bool,
}

impl TerminalSession {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode().context("enable terminal raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("create terminal backend")?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    fn restore(&mut self) -> anyhow::Result<()> {
        if self.restored {
            return Ok(());
        }
        disable_raw_mode().context("disable terminal raw mode")?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
            .context("leave alternate screen")?;
        self.terminal.show_cursor().context("show cursor")?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn render(frame: &mut Frame<'_>, app: &App) {
    let root = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(3),
            Constraint::Length(2),
        ])
        .split(root);

    render_header(frame, app, chunks[0]);
    render_body(frame, app, chunks[1]);
    render_input(frame, app, chunks[2]);
    render_help(frame, chunks[3]);
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = Line::from(vec![
        Span::styled(
            "COAT",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" terminal control plane  "),
        Span::styled(
            app.config.control_gateway_url.as_str(),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let status = Line::from(vec![
        Span::raw("mode "),
        Span::styled(app.mode.label(), Style::default().fg(Color::Yellow)),
        Span::raw("  focus "),
        Span::styled(app.focus.label(), Style::default().fg(Color::Magenta)),
        Span::raw("  session "),
        Span::styled(app.display_session_id(), Style::default().fg(Color::Green)),
        Span::raw("  status "),
        Span::raw(app.status.as_str()),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(vec![title, status]))
            .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_body(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(area);
    render_dashboard(frame, app, chunks[0]);
    render_chat(frame, app, chunks[1]);
}

fn render_dashboard(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);
    let titles = DashboardView::ALL
        .iter()
        .map(|view| view.title())
        .collect::<Vec<_>>();
    frame.render_widget(
        Tabs::new(titles)
            .select(app.dashboard_view.index())
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title(
                if app.focus == TuiFocus::Dashboard {
                    "Control *"
                } else {
                    "Control"
                },
            )),
        chunks[0],
    );

    let lines = match app.dashboard_view {
        DashboardView::Overview => overview_dashboard_lines(app, chunks[1].width),
        DashboardView::Goals => goals_dashboard_lines(app, chunks[1].width),
        DashboardView::Graph => graph_dashboard_lines(app, chunks[1].width),
        DashboardView::Actions => actions_dashboard_lines(app, chunks[1].width),
        DashboardView::Approvals => approval_gates_dashboard_lines(app, chunks[1].width),
        DashboardView::Events => events_dashboard_lines(app, chunks[1].width),
        DashboardView::Workers => workers_dashboard_lines(app, chunks[1].width),
        DashboardView::Evidence => evidence_dashboard_lines(app, chunks[1].width),
        DashboardView::Adversarial => adversarial_dashboard_lines(app, chunks[1].width),
        DashboardView::Debug => debug_dashboard_lines(chunks[1].width),
    };
    render_scrollable_lines(
        frame,
        chunks[1],
        app.dashboard_view.label(),
        lines,
        app.focus == TuiFocus::Dashboard,
        app.dashboard_scroll,
    );
}

fn overview_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let dashboard = &app.dashboard;
    let mut lines = vec![
        Line::from(vec![
            Span::raw("services "),
            Span::styled(
                format!("{}/{}", dashboard.services_ok, dashboard.services_total),
                Style::default().fg(if dashboard.services_ok == dashboard.services_total {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
        Line::from(format!("goals      {}", dashboard.goals_count)),
        Line::from(format!("runners    {}", dashboard.runners_count)),
        Line::from(format!("actions    {}", dashboard.approvals_count)),
        Line::from(format!("events     {}", dashboard.events_count)),
        Line::from(format!("plans      {}", dashboard.plans_count)),
        Line::from(""),
        Line::from(vec![
            Span::raw("chat backend "),
            Span::styled(
                dashboard.chat_backend.clone(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];
    lines.extend(current_goal_lines(
        app.selected_goal(),
        app.selected_goal_id.as_deref(),
        width,
    ));
    lines.extend(selected_goal_outline_lines(
        &app.selected_goal_outline,
        width,
    ));
    if let Some(draft) = app.active_goal_draft.as_ref() {
        lines.extend(goal_draft_dashboard_lines(&draft.summary, width));
    }
    lines
}

fn goals_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let value_width = width.saturating_sub(5).max(18) as usize;
    let mut lines = vec![
        Line::from(Span::styled(
            "goals",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "Ctrl-N/Ctrl-P or Up/Down selects. Ctrl-X cancels selected goal after confirmation.",
        ),
        Line::from(""),
    ];
    if app.goals.is_empty() {
        lines.push(Line::from("No projected goals yet."));
        return lines;
    }
    for goal in &app.goals {
        let selected = app
            .selected_goal_id
            .as_deref()
            .is_some_and(|selected| selected == goal.id);
        let marker = if selected { ">" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(
                marker,
                Style::default().fg(if selected {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
            Span::raw(" "),
            Span::styled(
                truncate_text(&goal.title, value_width),
                Style::default().add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ),
        ]));
        lines.push(Line::from(format!(
            "  {} {}% open:{} blocked:{} {}",
            goal.status,
            (goal.progress * 100.0).round() as u64,
            goal.open_tasks,
            goal.blocked_tasks,
            short_id(&goal.id)
        )));
        if let Some(action) = goal.current_action.as_deref() {
            lines.push(dashboard_value_line("  action", action, value_width));
        }
        if let Some(blocker) = goal.current_blocker.as_deref() {
            lines.push(dashboard_value_line("  blocker", blocker, value_width));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn graph_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let nodes = if app.selected_goal_id.is_some() && !app.selected_goal_graph_nodes.is_empty() {
        &app.selected_goal_graph_nodes
    } else {
        &app.graph_nodes
    };
    let value_width = width.saturating_sub(14).max(20) as usize;
    let scope = if app.selected_goal_id.is_some() && !app.selected_goal_graph_nodes.is_empty() {
        "selected goal"
    } else {
        "workspace"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "task graph",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("nodes {} scope {}", nodes.len(), scope)),
        Line::from("Ctrl-N/Ctrl-P changes current goal. Tab focuses chat or input."),
        Line::from(""),
    ];
    if nodes.is_empty() {
        lines.push(Line::from("No task graph nodes are projected yet."));
        return lines;
    }
    for node in nodes.iter().take(40) {
        lines.push(Line::from(vec![
            Span::styled(
                node.status.clone(),
                Style::default().fg(status_color(&node.status)),
            ),
            Span::raw(" "),
            Span::styled(node.kind.clone(), Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::raw(truncate_text(&node.label, value_width)),
        ]));
        lines.push(Line::from(format!(
            "  id:{} goal:{} task:{}",
            short_id(&node.id),
            node.goal_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            node.task_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string())
        )));
    }
    if nodes.len() > 40 {
        lines.push(Line::from(format!("+{} more nodes", nodes.len() - 40)));
    }
    lines
}

fn actions_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let approvals = app.scoped_action_items();
    let value_width = width.saturating_sub(15).max(20) as usize;
    let scope = if app.selected_goal_id.is_some() && !app.selected_goal_approvals.is_empty() {
        "selected goal"
    } else {
        "all goals"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "operator actions",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "queue {} ({}) scope {}",
            approvals.len(),
            action_queue_breakdown(&approvals),
            scope
        )),
        Line::from(
            "Up/Down selects. Enter/a runs the selected action. Ctrl-X cancels selected goal.",
        ),
        Line::from(
            "Human prompts: empty input = Continue; typed input = Add context/answer. Ctrl-L clears local results.",
        ),
        Line::from(""),
    ];
    if approvals.is_empty() {
        lines.push(Line::from("No operator actions are waiting."));
        return lines;
    }
    for (index, approval) in approvals.iter().enumerate() {
        let selected = index == app.action_index.min(approvals.len().saturating_sub(1));
        let marker = if selected { ">" } else { " " };
        let action_label = tui_action_label(approval);
        lines.push(Line::from(vec![
            Span::styled(
                marker,
                Style::default().fg(if selected {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::raw(" "),
            Span::styled(approval.status.clone(), action_status_style(approval)),
            Span::raw(" "),
            Span::styled(
                truncate_text(&approval.action, value_width),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(format!(
            "  kind:{} action:{} id:{} risk:{} goal:{} task:{}",
            action_kind_label(approval),
            action_label,
            short_id(&approval.id),
            action_risk_label(approval),
            approval
                .goal_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            approval
                .task_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string())
        )));
        if action_requires_input(approval) && selected {
            lines.push(Line::from(
                "  human prompt: press Enter to Continue, or type context/answer below first",
            ));
        } else if approval.id.starts_with("task:") && selected {
            lines.push(Line::from(
                "  recovery: retry task work, or request a replan with operator context",
            ));
        }
        if let Some(created_at) = approval.created_at.as_deref() {
            lines.push(dashboard_value_line("  created", created_at, value_width));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn approval_gates_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let approvals = current_approval_gate_items(app);
    let value_width = width.saturating_sub(15).max(20) as usize;
    let scope = if app.selected_goal_id.is_some() && !app.selected_goal_approvals.is_empty() {
        "selected goal"
    } else {
        "all goals"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "approval gates",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("pending {} scope {}", approvals.len(), scope)),
        Line::from("Enter/a approves the selected gate. Use Actions for prompts and recovery."),
        Line::from(""),
    ];
    if approvals.is_empty() {
        lines.push(Line::from("No approval gates are waiting."));
        return lines;
    }
    for (index, approval) in approvals.iter().enumerate() {
        let selected = action_id_is_selected(app, &approval.id, index);
        let marker = if selected { ">" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(
                marker,
                Style::default().fg(if selected {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::raw(" "),
            Span::styled(approval.status.clone(), action_status_style(approval)),
            Span::raw(" "),
            Span::styled(
                truncate_text(&approval.action, value_width),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(format!(
            "  id:{} risk:{} goal:{} task:{}",
            short_id(&approval.id),
            action_risk_label(approval),
            approval
                .goal_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            approval
                .task_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string())
        )));
        if let Some(created_at) = approval.created_at.as_deref() {
            lines.push(dashboard_value_line("  created", created_at, value_width));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn action_queue_breakdown(approvals: &[ApprovalSummary]) -> String {
    let approval_count = approvals
        .iter()
        .filter(|action| !action.id.starts_with("task:") && !action.id.starts_with("thunk:"))
        .count();
    let task_count = approvals
        .iter()
        .filter(|action| action.id.starts_with("task:"))
        .count();
    let thunk_count = approvals
        .iter()
        .filter(|action| action.id.starts_with("thunk:"))
        .count();
    format!("approval gates:{approval_count} recovery:{task_count} prompts:{thunk_count}")
}

fn current_approval_gate_items(app: &App) -> Vec<ApprovalSummary> {
    app.scoped_action_items()
        .into_iter()
        .filter(is_approval_gate_action)
        .collect()
}

fn is_approval_gate_action(action: &ApprovalSummary) -> bool {
    !action.id.starts_with("task:") && !action.id.starts_with("thunk:")
}

fn action_id_is_selected(app: &App, action_id: &str, fallback_index: usize) -> bool {
    app.selected_action_item()
        .as_ref()
        .map(|selected| selected.id == action_id)
        .unwrap_or_else(|| fallback_index == app.action_index)
}

fn action_kind_label(action: &ApprovalSummary) -> &'static str {
    if action.id.starts_with("thunk:") {
        "human prompt"
    } else if action.id.starts_with("task:") {
        "recovery"
    } else {
        "approval gate"
    }
}

fn action_risk_label(action: &ApprovalSummary) -> &str {
    match action.risk.as_deref() {
        Some("delayed compute thunk") => "human prompt",
        Some(risk) => risk,
        None => "unspecified",
    }
}

fn action_status_style(action: &ApprovalSummary) -> Style {
    match status_token(&action.status).as_str() {
        "pending" | "waiting-approval" => Style::default().fg(Color::Yellow),
        "blocked" | "failed" | "budget-exhausted" => Style::default().fg(Color::Red),
        "waiting-input" if action.id.starts_with("thunk:") => Style::default().fg(Color::Green),
        "waiting-input" => Style::default().fg(Color::Magenta),
        _ => Style::default().fg(Color::Cyan),
    }
}

fn events_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let events = if app.selected_goal_id.is_some() && !app.selected_goal_events.is_empty() {
        &app.selected_goal_events
    } else {
        &app.events
    };
    let value_width = width.saturating_sub(15).max(20) as usize;
    let scope = if app.selected_goal_id.is_some() && !app.selected_goal_events.is_empty() {
        "selected goal"
    } else {
        "all goals"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "events",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "recent {} sources {} scope {}",
            events.len(),
            app.event_sources.len(),
            scope
        )),
        Line::from("Ctrl-R refreshes projections. Ctrl-L clears local action/chat results."),
    ];
    for source in app.event_sources.iter().take(4) {
        lines.push(Line::from(format!(
            "  {} [{}] {} approval:{}",
            truncate_text(&source.id, value_width / 2),
            source.kind,
            source.status,
            source.approval.as_deref().unwrap_or("-")
        )));
        if let Some(route) = source.route.as_deref() {
            lines.push(dashboard_value_line("  route", route, value_width));
        }
    }
    lines.push(Line::from(""));
    if events.is_empty() {
        lines.push(Line::from("No recent events projected."));
        return lines;
    }
    for event in events.iter().rev() {
        lines.push(Line::from(vec![
            Span::styled(
                event.kind.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::raw(truncate_text(&event.message, value_width)),
        ]));
        lines.push(Line::from(format!(
            "  id:{} goal:{} task:{} source:{}",
            short_id(&event.id),
            event
                .goal_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            event
                .task_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            event.source.as_deref().unwrap_or("-")
        )));
        if let Some(created_at) = event.created_at.as_deref() {
            lines.push(dashboard_value_line("  at", created_at, value_width));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn workers_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let workers = if app.selected_goal_id.is_some() && !app.selected_goal_worker_runs.is_empty() {
        &app.selected_goal_worker_runs
    } else {
        &app.worker_runs
    };
    let value_width = width.saturating_sub(15).max(20) as usize;
    let scope = if app.selected_goal_id.is_some() && !app.selected_goal_worker_runs.is_empty() {
        "selected goal"
    } else {
        "workspace"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "workers",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("runs {} scope {}", workers.len(), scope)),
        Line::from("Registered runners and worker runs are read-only here."),
        Line::from(""),
    ];
    if workers.is_empty() {
        lines.push(Line::from("No worker or runner activity is projected yet."));
        return lines;
    }
    for worker in workers.iter().take(40) {
        lines.push(Line::from(vec![
            Span::styled(
                worker.status.clone(),
                Style::default().fg(status_color(&worker.status)),
            ),
            Span::raw(" "),
            Span::styled(
                truncate_text(&worker.runner, value_width),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(format!(
            "  id:{} task:{} endpoint:{} node:{}",
            short_id(&worker.id),
            worker
                .task
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            worker.endpoint.as_deref().unwrap_or("-"),
            worker.node.as_deref().unwrap_or("-")
        )));
        if let Some(updated_at) = worker.updated_at.as_deref() {
            lines.push(dashboard_value_line("  updated", updated_at, value_width));
        }
        lines.push(Line::from(""));
    }
    if workers.len() > 40 {
        lines.push(Line::from(format!("+{} more workers", workers.len() - 40)));
    }
    lines
}

fn evidence_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let evidence = if app.selected_goal_id.is_some() && !app.selected_goal_evidence.is_empty() {
        &app.selected_goal_evidence
    } else {
        &app.evidence
    };
    let value_width = width.saturating_sub(14).max(20) as usize;
    let scope = if app.selected_goal_id.is_some() && !app.selected_goal_evidence.is_empty() {
        "selected goal"
    } else {
        "workspace"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "evidence",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("items {} scope {}", evidence.len(), scope)),
        Line::from("Evidence explains why a task or goal can move forward."),
        Line::from(""),
    ];
    if evidence.is_empty() {
        lines.push(Line::from("No evidence artifacts are projected yet."));
        return lines;
    }
    for item in evidence.iter().take(40) {
        lines.push(Line::from(vec![
            Span::styled(item.kind.clone(), Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::raw(truncate_text(&item.summary, value_width)),
        ]));
        lines.push(Line::from(format!(
            "  id:{} source:{} goal:{} task:{}",
            short_id(&item.id),
            item.source.as_deref().unwrap_or("-"),
            item.goal_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string()),
            item.task_id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "-".to_string())
        )));
        lines.push(Line::from(""));
    }
    if evidence.len() > 40 {
        lines.push(Line::from(format!(
            "+{} more evidence items",
            evidence.len() - 40
        )));
    }
    lines
}

fn debug_dashboard_lines(width: u16) -> Vec<Line<'static>> {
    let value_width = width.saturating_sub(16).max(24) as usize;
    let mut lines = vec![
        Line::from(Span::styled(
            "debug",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Inspect advanced endpoints and explicit CLI escape hatches."),
        Line::from(""),
    ];
    for item in operator_debug_catalog() {
        lines.push(Line::from(vec![
            Span::styled(item.command, Style::default().fg(Color::Cyan)),
            Span::raw(" -> "),
            Span::styled(item.surface, Style::default().add_modifier(Modifier::BOLD)),
        ]));
        lines.push(dashboard_value_line("action", item.action, value_width));
        lines.push(dashboard_value_line("summary", item.summary, value_width));
        lines.push(Line::from(""));
    }
    lines
}

#[derive(Debug, Clone, Copy)]
struct OperatorDebugCatalogItem {
    command: &'static str,
    surface: &'static str,
    action: &'static str,
    summary: &'static str,
}

fn operator_debug_catalog() -> &'static [OperatorDebugCatalogItem] {
    &[
        OperatorDebugCatalogItem {
            command: "coat plan",
            surface: "Plans / Debug",
            action: "Open Plans in SPA, Debug in TUI",
            summary: "Draft, list, show, revise, compile, and continue durable plans.",
        },
        OperatorDebugCatalogItem {
            command: "coat goal",
            surface: "Goals / Graph / Actions / Adversarial",
            action: "Open Goals, Graph, Actions, Approvals, or Adversarial",
            summary: "Submit, inspect, steer, vote, branch, restart, and evaluate goals.",
        },
        OperatorDebugCatalogItem {
            command: "coat human",
            surface: "Actions / Human Prompts",
            action: "Enter or a runs the selected queue action",
            summary: "Approve gates, continue human prompts, and inspect feedback threads.",
        },
        OperatorDebugCatalogItem {
            command: "coat deploy",
            surface: "Debug",
            action: "Run explicit CLI deploy command",
            summary: "Local, cluster, chart, and Restate Cloud deployment remains operator-run.",
        },
        OperatorDebugCatalogItem {
            command: "coat runner",
            surface: "Workers / Debug",
            action: "Open runner status",
            summary: "Inspect runner registration, capacity, endpoints, and routing pressure.",
        },
        OperatorDebugCatalogItem {
            command: "coat tool",
            surface: "Debug",
            action: "Run CLI tool command",
            summary: "List tools, call MCP tools, and route web-search through backend tooling.",
        },
        OperatorDebugCatalogItem {
            command: "coat memory",
            surface: "Memory / Debug",
            action: "Open Memory",
            summary: "Search, write, context, join, edit, repair, and inspect memory events.",
        },
        OperatorDebugCatalogItem {
            command: "coat event",
            surface: "Events / Actions / Debug",
            action: "Open Events or Actions",
            summary: "Register, ingest, emit, poll, trigger, and route durable event sources.",
        },
        OperatorDebugCatalogItem {
            command: "coat store",
            surface: "Overview / Goals / Graph / Actions",
            action: "Open projection views",
            summary: "Read goal-store projections for goals, tasks, plans, events, artifacts, checkpoints, and approvals.",
        },
        OperatorDebugCatalogItem {
            command: "coat scenario",
            surface: "Debug",
            action: "Run explicit CLI scenario command",
            summary: "List, run, and report deterministic scenario evidence.",
        },
        OperatorDebugCatalogItem {
            command: "coat setup",
            surface: "Debug",
            action: "Run explicit CLI setup command",
            summary: "Login, SSO, model index, config, local auth, and chat-client setup.",
        },
        OperatorDebugCatalogItem {
            command: "coat tui",
            surface: "Terminal UI",
            action: "Already here",
            summary: "Terminal dashboard mirrors SPA goal, graph, action, worker, event, and evidence projections.",
        },
    ]
}

fn adversarial_dashboard_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let value_width = width.saturating_sub(17).max(22) as usize;
    let mut lines = vec![
        Line::from(Span::styled(
            "adversarial panel",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("Read-only grouped task view. Use Up/Down or PageUp/PageDown to scroll."),
    ];

    let Some(goal_id) = app.selected_goal_id.as_deref() else {
        lines.push(Line::from(""));
        lines.push(Line::from("Select a goal with Ctrl-N/Ctrl-P first."));
        return lines;
    };
    lines.push(Line::from(format!("goal {}", short_id(goal_id))));

    let Some(snapshot) = app.selected_goal_snapshot.as_ref() else {
        lines.push(Line::from(""));
        lines.push(Line::from("Selected goal snapshot is still loading."));
        return lines;
    };

    let summary = adversarial_summary_from_snapshot(snapshot);
    lines.extend(adversarial_satisfaction_lines(&summary, value_width));
    lines.extend(adversarial_task_group_lines(
        "actors and candidates",
        &summary.actors,
        value_width,
    ));
    lines.extend(adversarial_task_group_lines(
        "critics, testers, and formal methods",
        &summary.critics,
        value_width,
    ));
    lines.extend(adversarial_task_group_lines(
        "research tasks",
        &summary.research,
        value_width,
    ));
    lines.extend(adversarial_task_group_lines(
        "unification, votes, and mechanisms",
        &summary.mechanisms,
        value_width,
    ));
    lines.extend(adversarial_reference_lines(
        "agent context, chat, and thread refs",
        &summary.references,
        value_width,
    ));
    lines
}

fn adversarial_satisfaction_lines(
    summary: &AdversarialSummary,
    value_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "satisfaction",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    lines.push(Line::from(format!(
        "  score:{} satisfied:{}",
        summary.satisfaction_score.as_deref().unwrap_or("-"),
        summary.satisfied.as_deref().unwrap_or("-")
    )));
    if summary.satisfaction_reasons.is_empty() {
        lines.push(Line::from("  reasons: none projected"));
    } else {
        for reason in summary.satisfaction_reasons.iter().take(5) {
            lines.push(dashboard_value_line("  reason", reason, value_width));
        }
        if summary.satisfaction_reasons.len() > 5 {
            lines.push(Line::from(format!(
                "  +{} more reasons",
                summary.satisfaction_reasons.len() - 5
            )));
        }
    }
    lines
}

fn adversarial_task_group_lines(
    title: &str,
    rows: &[AdversarialTaskRow],
    value_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{title} ({})", rows.len()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    if rows.is_empty() {
        lines.push(Line::from("  none projected"));
        return lines;
    }
    for row in rows.iter().take(12) {
        lines.push(Line::from(format!(
            "  {} [{} {} {}] {}",
            row.id
                .as_deref()
                .map(short_id)
                .unwrap_or_else(|| "task".to_string()),
            row.status.as_deref().unwrap_or("-"),
            row.role.as_deref().unwrap_or("-"),
            row.purpose.as_deref().unwrap_or("-"),
            truncate_text(&row.title, value_width)
        )));
        if let Some(reference) = row.reference.as_deref() {
            lines.push(dashboard_value_line("    ref", reference, value_width));
        }
    }
    if rows.len() > 12 {
        lines.push(Line::from(format!("  +{} more", rows.len() - 12)));
    }
    lines
}

fn adversarial_reference_lines(
    title: &str,
    refs: &[String],
    value_width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{title} ({})", refs.len()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    if refs.is_empty() {
        lines.push(Line::from("  none projected"));
        return lines;
    }
    for reference in refs.iter().take(12) {
        lines.push(dashboard_value_line("  ref", reference, value_width));
    }
    if refs.len() > 12 {
        lines.push(Line::from(format!("  +{} more", refs.len() - 12)));
    }
    lines
}

fn render_chat(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut lines = Vec::new();
    for message in &app.messages {
        lines.extend(chat_message_lines(message));
    }
    if app.busy {
        lines.push(Line::from(Span::styled(
            "generating with control gateway...",
            Style::default().fg(Color::Magenta),
        )));
    }
    let total_rows = rendered_row_count(&lines, area.width.saturating_sub(2));
    let viewport_height = area.height.saturating_sub(2);
    let scroll_y = chat_scroll_y(total_rows, viewport_height, app.chat_scroll_from_bottom);
    let max_scroll = total_rows.saturating_sub(viewport_height as usize);
    let context = chat_context_label(app);
    let title = if app.focus == TuiFocus::Chat {
        format!("Chat {} * row {}/{}", context, scroll_y, max_scroll)
    } else {
        format!("Chat {} row {}/{}", context, scroll_y, max_scroll)
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(title))
            .scroll((scroll_y, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
    if max_scroll > 0 {
        let mut state = ScrollbarState::new(total_rows).position(scroll_y as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut state,
        );
    }
}

fn chat_message_lines(message: &ChatLine) -> Vec<Line<'static>> {
    let role_color = match message.role.as_str() {
        "user" => Color::Yellow,
        "assistant" => Color::Cyan,
        _ => Color::White,
    };
    let role_style = Style::default().fg(role_color).add_modifier(Modifier::BOLD);
    let text_style = Style::default().fg(Color::White);
    let code_style = Style::default().fg(Color::Green);
    let mut in_code_block = false;
    let mut lines = Vec::new();
    let content_lines: Vec<&str> = if message.content.is_empty() {
        vec![""]
    } else {
        message.content.lines().collect()
    };

    for (index, raw_line) in content_lines.into_iter().enumerate() {
        let is_fence = raw_line.trim_start().starts_with("```");
        let line_style = if in_code_block || is_fence {
            code_style
        } else {
            text_style
        };
        let mut spans = Vec::new();
        if index == 0 {
            spans.push(Span::styled(format!("{}: ", message.role), role_style));
        } else {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(raw_line.to_string(), line_style));
        lines.push(Line::from(spans));
        if is_fence {
            in_code_block = !in_code_block;
        }
    }
    lines.push(Line::from(""));
    lines
}

fn render_scrollable_lines(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    focused: bool,
    scroll: u16,
) {
    let total_rows = rendered_row_count(&lines, area.width.saturating_sub(2));
    let viewport_height = area.height.saturating_sub(2);
    let max_scroll = total_rows.saturating_sub(viewport_height as usize);
    let scroll_y = (scroll as usize).min(max_scroll).min(u16::MAX as usize) as u16;
    let title = if focused {
        format!("{title} * row {scroll_y}/{max_scroll}")
    } else {
        format!("{title} row {scroll_y}/{max_scroll}")
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(title))
            .scroll((scroll_y, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
    if max_scroll > 0 {
        let mut state = ScrollbarState::new(total_rows).position(scroll_y as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut state,
        );
    }
}

fn render_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let context = chat_context_label(app);
    let pending = if app.busy { " pending" } else { "" };
    let title = if app.focus == TuiFocus::Input {
        format!("Input [{} {}{}] *", app.mode.label(), context, pending)
    } else {
        format!("Input [{} {}{}]", app.mode.label(), context, pending)
    };
    frame.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().fg(Color::White)),
        area,
    );
    if app.focus == TuiFocus::Input {
        let cursor_x = area
            .x
            .saturating_add(1)
            .saturating_add(app.input.len().min(area.width.saturating_sub(2) as usize) as u16);
        frame.set_cursor_position(Position::new(cursor_x, area.y.saturating_add(1)));
    }
}

fn chat_context_label(app: &App) -> String {
    app.selected_goal()
        .map(|goal| format!("goal {}", truncate_text(&goal.title, 28)))
        .or_else(|| {
            app.selected_goal_id
                .as_deref()
                .map(|goal_id| format!("goal {}", short_id(goal_id)))
        })
        .unwrap_or_else(|| "workspace".to_string())
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(
            "Tab focus  1-0 views  ↑/↓ scroll/select  Enter/a run action  Ctrl-X cancel goal  Ctrl-L clear local  F5/Ctrl-G accept draft  Ctrl-R refresh  Esc quit",
        )
        .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

async fn parse_response(response: reqwest::Response) -> anyhow::Result<Value> {
    let status = response.status();
    let text = response.text().await.context("read response body")?;
    let value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()))
    };
    if !status.is_success() {
        bail!("gateway returned {status}: {value}");
    }
    Ok(value)
}

fn dashboard_summary(workspace: &Value, goals: &[GoalSummary]) -> DashboardSummary {
    let services = find_first_array(workspace, &["services"]).unwrap_or(&[]);
    let services_ok = services
        .iter()
        .filter(|service| {
            service.get("ok").and_then(Value::as_bool).unwrap_or(false)
                || service
                    .get("status")
                    .and_then(Value::as_u64)
                    .is_some_and(|status| (200..400).contains(&status))
        })
        .count();
    DashboardSummary {
        services_ok,
        services_total: services.len(),
        goals_count: goals.len(),
        runners_count: find_first_array(workspace, &["runners", "data"]).map_or(0, <[Value]>::len),
        approvals_count: operator_action_summaries_from_value(workspace).len(),
        events_count: find_first_array(workspace, &["events"]).map_or(0, <[Value]>::len),
        plans_count: find_first_array(workspace, &["plans"]).map_or(0, <[Value]>::len),
        chat_backend: chat_backend_label(workspace.get("config").unwrap_or(&Value::Null)),
        latest_goals: goals.iter().take(5).map(goal_label).collect(),
    }
}

fn approval_summaries_from_value(value: &Value) -> Vec<ApprovalSummary> {
    find_first_array(value, &["approvals"])
        .unwrap_or(&[])
        .iter()
        .filter_map(approval_summary_from_value)
        .collect()
}

fn action_needed_summaries_from_value(value: &Value) -> Vec<ApprovalSummary> {
    let operator_actions = operator_action_summaries_from_value(value);
    if !operator_actions.is_empty() {
        return operator_actions;
    }
    let thunks = thunk_action_needed_summaries_from_value(value);
    let thunk_task_ids = thunks
        .iter()
        .filter_map(|item| item.task_id.as_deref())
        .collect::<Vec<_>>();
    let tasks = task_action_needed_summaries_from_value(value)
        .into_iter()
        .filter(|item| {
            !(status_token(&item.status) == "waiting-input"
                && item
                    .task_id
                    .as_deref()
                    .is_some_and(|task_id| thunk_task_ids.contains(&task_id)))
        });
    let mut items = approval_summaries_from_value(value);
    items.extend(tasks);
    items.extend(thunks);
    dedupe_action_needed_summaries(items)
}

fn operator_action_summaries_from_value(value: &Value) -> Vec<ApprovalSummary> {
    first_array_at_paths(
        value,
        &[
            &["actions"],
            &["selected_goal", "actions"],
            &["data", "actions"],
            &["operator", "actions"],
        ],
    )
    .unwrap_or(&[])
    .iter()
    .filter_map(operator_action_summary_from_value)
    .collect()
}

fn operator_action_summary_from_value(value: &Value) -> Option<ApprovalSummary> {
    let id = compact_string_at_paths(value, &[&["action_id"], &["id"]])?;
    let status = compact_string_at_paths(value, &[&["status"]])
        .map(|status| status_token(&status))
        .unwrap_or_else(|| "pending".to_string());
    if matches!(status.as_str(), "done" | "resolved" | "cancelled") {
        return None;
    }
    Some(ApprovalSummary {
        id,
        status,
        action: compact_string_at_paths(value, &[&["question"], &["title"], &["detail"]])
            .unwrap_or_else(|| "operator action required".to_string()),
        risk: compact_string_at_paths(value, &[&["kind"], &["risk"]]),
        goal_id: compact_string_at_paths(value, &[&["goal_id"]]),
        task_id: compact_string_at_paths(value, &[&["task_id"]]),
        created_at: compact_string_at_paths(
            value,
            &[&["created_at"], &["payload_json", "created_at"]],
        ),
    })
}

fn dedupe_action_needed_summaries(items: Vec<ApprovalSummary>) -> Vec<ApprovalSummary> {
    let mut deduped = Vec::new();
    for item in items {
        let key = format!(
            "{}:{}:{}",
            item.id,
            item.goal_id.as_deref().unwrap_or_default(),
            item.task_id.as_deref().unwrap_or_default()
        );
        if deduped.iter().any(|existing: &ApprovalSummary| {
            format!(
                "{}:{}:{}",
                existing.id,
                existing.goal_id.as_deref().unwrap_or_default(),
                existing.task_id.as_deref().unwrap_or_default()
            ) == key
        }) {
            continue;
        }
        deduped.push(item);
    }
    deduped
}

fn approval_summary_from_value(value: &Value) -> Option<ApprovalSummary> {
    let id = compact_string_at_paths(
        value,
        &[
            &["approval_id"],
            &["id"],
            &["approval_ref"],
            &["payload_json", "approval_id"],
            &["payload_json", "id"],
        ],
    )
    .unwrap_or_else(|| "approval".to_string());
    let status = compact_string_at_paths(value, &[&["status"], &["payload_json", "status"]])
        .map(|status| status_token(&status))
        .unwrap_or_else(|| "pending".to_string());
    if status != "pending" && status != "waiting-approval" {
        return None;
    }
    let action = approval_action_text(value)
        .or_else(|| compact_string_at_paths(value, &[&["message"], &["summary"]]))
        .unwrap_or_else(|| "review requested".to_string());
    Some(ApprovalSummary {
        id,
        status,
        action,
        risk: compact_string_at_paths(value, &[&["risk"], &["payload_json", "risk"]]),
        goal_id: compact_string_at_paths(value, &[&["goal_id"], &["payload_json", "goal_id"]]),
        task_id: compact_string_at_paths(value, &[&["task_id"], &["payload_json", "task_id"]]),
        created_at: compact_string_at_paths(
            value,
            &[
                &["created_at"],
                &["requested_at"],
                &["payload_json", "created_at"],
                &["payload_json", "requested_at"],
            ],
        ),
    })
}

fn task_action_needed_summaries_from_value(value: &Value) -> Vec<ApprovalSummary> {
    first_array_at_paths(
        value,
        &[
            &["agent_activity"],
            &["agents", "data", "tasks"],
            &["agents", "tasks"],
            &["tasks", "data", "tasks"],
            &["tasks", "tasks"],
            &["data", "tasks"],
        ],
    )
    .unwrap_or(&[])
    .iter()
    .filter(|task| task_needs_operator_attention(task))
    .filter_map(task_action_needed_summary)
    .collect()
}

fn task_action_needed_summary(task: &Value) -> Option<ApprovalSummary> {
    let task_id = task_id(task);
    let status = task_status(task);
    let action = task_summary_line(task)?;
    Some(ApprovalSummary {
        id: task_id
            .as_ref()
            .map(|id| format!("task:{id}"))
            .unwrap_or_else(|| format!("task:{action}")),
        status: if status.is_empty() {
            "blocked".to_string()
        } else {
            status
        },
        action,
        risk: Some("task attention".to_string()),
        goal_id: compact_string_at_paths(
            task,
            &[
                &["goal_id"],
                &["payload_json", "goal_id"],
                &["raw_task", "goal_id"],
            ],
        ),
        task_id,
        created_at: compact_string_at_paths(
            task,
            &[
                &["updated_at"],
                &["created_at"],
                &["payload_json", "updated_at"],
                &["payload_json", "created_at"],
            ],
        ),
    })
}

fn thunk_action_needed_summaries_from_value(value: &Value) -> Vec<ApprovalSummary> {
    let records = delayed_thunk_records_from_value(value);
    let mut items = first_array_at_paths(
        value,
        &[
            &["workflow_compute_graph", "data", "nodes"],
            &["workflow_compute_graph", "nodes"],
            &["compute_graph", "nodes"],
        ],
    )
    .unwrap_or(&[])
    .iter()
    .filter(|node| {
        compact_string_at_paths(node, &[&["kind"]])
            .map(|kind| status_token(&kind) == "delayed-compute-thunk")
            .unwrap_or(false)
    })
    .filter(|node| {
        let status = compact_string_at_paths(node, &[&["status"]])
            .map(|status| status_token(&status))
            .unwrap_or_else(|| "waiting-input".to_string());
        !matches!(
            status.as_str(),
            "resumed" | "cancelled" | "expired" | "done"
        )
    })
    .filter_map(|node| {
        let thunk_id = compact_string_at_paths(node, &[&["thunk_id"], &["id"]])?;
        let record = records.iter().find(|record| {
            compact_string_at_paths(record, &[&["id"], &["thunk_id"]]).as_deref()
                == Some(thunk_id.as_str())
        });
        thunk_action_needed_summary(node, record)
    })
    .collect::<Vec<_>>();
    let graph_thunk_ids = items
        .iter()
        .filter_map(|item| item.id.strip_prefix("thunk:").map(str::to_string))
        .collect::<Vec<_>>();
    items.extend(
        records
            .iter()
            .filter(|record| {
                compact_string_at_paths(record, &[&["id"], &["thunk_id"]])
                    .is_some_and(|id| !graph_thunk_ids.contains(&id))
            })
            .filter_map(thunk_record_action_needed_summary),
    );
    dedupe_action_needed_summaries(items)
}

fn delayed_thunk_records_from_value(value: &Value) -> Vec<Value> {
    first_array_at_paths(
        value,
        &[
            &["workflow_status", "data", "delayed_compute_thunks"],
            &["workflow_status", "delayed_compute_thunks"],
            &["workflow_progress", "data", "delayed_compute_thunks"],
            &["workflow_progress", "delayed_compute_thunks"],
            &["delayed_compute_thunks"],
        ],
    )
    .unwrap_or(&[])
    .to_vec()
}

fn thunk_action_needed_summary(node: &Value, record: Option<&Value>) -> Option<ApprovalSummary> {
    let thunk_id = compact_string_at_paths(node, &[&["thunk_id"], &["id"]])?;
    let status = compact_string_at_paths(node, &[&["status"]])
        .or_else(|| record.and_then(|record| compact_string_at_paths(record, &[&["status"]])))
        .map(|status| status_token(&status))
        .unwrap_or_else(|| "waiting-input".to_string());
    let label = compact_string_at_paths(
        node,
        &[
            &["requested_input"],
            &["payload_json", "requested_input"],
            &["wait_ref", "requested_input"],
            &["label"],
            &["reason"],
        ],
    )
    .or_else(|| {
        record.and_then(|record| {
            compact_string_at_paths(
                record,
                &[
                    &["requested_input"],
                    &["reason"],
                    &["wait_ref", "requested_input"],
                    &["label"],
                ],
            )
        })
    })
    .unwrap_or_else(|| "Human prompt waiting for operator input".to_string());
    Some(ApprovalSummary {
        id: format!("thunk:{thunk_id}"),
        status,
        action: label,
        risk: Some("human prompt".to_string()),
        goal_id: compact_string_at_paths(node, &[&["goal_id"]])
            .or_else(|| record.and_then(|record| compact_string_at_paths(record, &[&["goal_id"]]))),
        task_id: compact_string_at_paths(node, &[&["task_id"]])
            .or_else(|| record.and_then(|record| compact_string_at_paths(record, &[&["task_id"]]))),
        created_at: record.and_then(|record| {
            compact_string_at_paths(record, &[&["created_at"], &["payload_json", "created_at"]])
        }),
    })
}

fn thunk_record_action_needed_summary(record: &Value) -> Option<ApprovalSummary> {
    let thunk_id = compact_string_at_paths(record, &[&["id"], &["thunk_id"]])?;
    let status = compact_string_at_paths(record, &[&["status"]])
        .map(|status| status_token(&status))
        .unwrap_or_else(|| "waiting-input".to_string());
    if matches!(
        status.as_str(),
        "resumed" | "cancelled" | "expired" | "done"
    ) {
        return None;
    }
    let label = compact_string_at_paths(
        record,
        &[
            &["requested_input"],
            &["reason"],
            &["wait_ref", "requested_input"],
            &["label"],
        ],
    )
    .unwrap_or_else(|| "Human prompt waiting for operator input".to_string());
    Some(ApprovalSummary {
        id: format!("thunk:{thunk_id}"),
        status,
        action: label,
        risk: Some("human prompt".to_string()),
        goal_id: compact_string_at_paths(record, &[&["goal_id"]]),
        task_id: compact_string_at_paths(record, &[&["task_id"]]),
        created_at: compact_string_at_paths(
            record,
            &[&["created_at"], &["payload_json", "created_at"]],
        ),
    })
}

fn action_requires_input(action: &ApprovalSummary) -> bool {
    action.id.starts_with("thunk:")
}

fn cancel_goal_request(goal_id: &str, reason: &str) -> (String, Value, String) {
    (
        format!("/api/operator/goals/{}/cancel", percent_encode(goal_id)),
        json!(reason),
        format!("cancel goal {}", short_id(goal_id)),
    )
}

fn operator_action_request(
    goal_id: &str,
    action: &ApprovalSummary,
    input: &str,
) -> anyhow::Result<(String, Value, String)> {
    let resolution = if action.id.starts_with("thunk:") {
        if input.trim().is_empty() {
            "continue"
        } else {
            "add_context"
        }
    } else if action.id.starts_with("task:") {
        let status = status_token(&action.status);
        if status == "blocked" || status == "failed" {
            "retry"
        } else {
            "replan"
        }
    } else {
        "approve"
    };
    let response_summary = if input.trim().is_empty() {
        match resolution {
            "continue" => "Continue".to_string(),
            "approve" => "Approved through COAT TUI action queue".to_string(),
            "retry" => format!("Retry from action queue: {}", action.action),
            "replan" => format!("Request replan from action queue: {}", action.action),
            _ => "Resolved through COAT TUI action queue".to_string(),
        }
    } else {
        input.trim().to_string()
    };
    let payload = json!({
        "goal_id": goal_id,
        "task_id": action.task_id,
        "approval_id": if action.id.starts_with("task:") || action.id.starts_with("thunk:") { Value::Null } else { Value::String(action.id.clone()) },
        "thunk_id": action.id.strip_prefix("thunk:"),
        "resolution": resolution,
        "operator": "operator",
        "response_summary": response_summary,
        "answer": input.trim(),
        "artifact_refs": []
    });
    Ok((
        format!(
            "/api/operator/actions/{}/resolve",
            percent_encode(&action.id)
        ),
        payload,
        format!("{} {}", resolution.replace('_', " "), short_id(&action.id)),
    ))
}

fn tui_action_label(action: &ApprovalSummary) -> &'static str {
    if action.id.starts_with("thunk:") {
        "Continue / Add context"
    } else if action.id.starts_with("task:") {
        match status_token(&action.status).as_str() {
            "blocked" | "failed" => "Retry task",
            _ => "Replan with context",
        }
    } else {
        "Approve and continue"
    }
}

fn event_summaries_from_value(value: &Value) -> Vec<EventSummary> {
    find_first_array(value, &["events", "recent_events"])
        .unwrap_or(&[])
        .iter()
        .filter_map(event_summary_from_value)
        .collect()
}

fn event_summary_from_value(value: &Value) -> Option<EventSummary> {
    let message = compact_string_at_paths(
        value,
        &[
            &["message"],
            &["subject"],
            &["summary"],
            &["payload_json", "message"],
            &["payload_json", "subject"],
            &["payload_json", "summary"],
        ],
    )?;
    let id = compact_string_at_paths(
        value,
        &[
            &["event_id"],
            &["id"],
            &["external_event_id"],
            &["payload_json", "event_id"],
            &["payload_json", "id"],
        ],
    )
    .unwrap_or_else(|| message.clone());
    let kind = compact_string_at_paths(
        value,
        &[
            &["kind"],
            &["event_type"],
            &["type"],
            &["payload_json", "kind"],
            &["payload_json", "event_type"],
        ],
    )
    .unwrap_or_else(|| "event".to_string());
    Some(EventSummary {
        id,
        kind,
        message,
        goal_id: compact_string_at_paths(value, &[&["goal_id"], &["payload_json", "goal_id"]]),
        task_id: compact_string_at_paths(value, &[&["task_id"], &["payload_json", "task_id"]]),
        source: compact_string_at_paths(
            value,
            &[
                &["source_id"],
                &["source"],
                &["payload_json", "source_id"],
                &["payload_json", "source"],
            ],
        ),
        created_at: compact_string_at_paths(
            value,
            &[
                &["created_at"],
                &["received_at"],
                &["occurred_at"],
                &["payload_json", "created_at"],
                &["payload_json", "received_at"],
                &["payload_json", "occurred_at"],
            ],
        ),
    })
}

fn event_source_summaries_from_value(value: &Value) -> Vec<EventSourceSummary> {
    find_first_array(value, &["event_sources", "sources"])
        .unwrap_or(&[])
        .iter()
        .filter_map(event_source_summary_from_value)
        .collect()
}

fn event_source_summary_from_value(value: &Value) -> Option<EventSourceSummary> {
    let id = compact_string_at_paths(
        value,
        &[
            &["source_id"],
            &["id"],
            &["name"],
            &["payload_json", "source_id"],
            &["payload_json", "id"],
        ],
    )?;
    let kind = compact_string_at_paths(
        value,
        &[
            &["kind"],
            &["source_kind"],
            &["payload_json", "kind"],
            &["payload_json", "source_kind"],
        ],
    )
    .unwrap_or_else(|| "source".to_string());
    let status = compact_string_at_paths(
        value,
        &[
            &["status"],
            &["enabled"],
            &["payload_json", "status"],
            &["payload_json", "enabled"],
        ],
    )
    .unwrap_or_else(|| "unknown".to_string());
    Some(EventSourceSummary {
        id,
        kind,
        status,
        route: compact_string_at_paths(
            value,
            &[
                &["route", "mode"],
                &["route_mode"],
                &["payload_json", "route", "mode"],
                &["payload_json", "route_mode"],
            ],
        ),
        approval: compact_string_at_paths(
            value,
            &[
                &["approval_ref"],
                &["approval_status"],
                &["payload_json", "approval_ref"],
                &["payload_json", "approval_status"],
            ],
        ),
    })
}

fn graph_node_summaries_from_value(value: &Value) -> Vec<GraphNodeSummary> {
    let mut nodes = Vec::new();
    for array in arrays_at_paths(
        value,
        &[
            &["workflow_compute_graph", "data", "nodes"],
            &["workflow_compute_graph", "nodes"],
            &["compute_graph", "nodes"],
        ],
    ) {
        nodes.extend(
            array
                .iter()
                .filter_map(graph_node_summary_from_compute_node),
        );
    }
    if nodes.is_empty() {
        for array in arrays_at_paths(
            value,
            &[
                &["agent_activity"],
                &["agents", "data", "tasks"],
                &["agents", "tasks"],
                &["tasks", "data", "tasks"],
                &["tasks", "tasks"],
                &["data", "tasks"],
            ],
        ) {
            nodes.extend(array.iter().filter_map(graph_node_summary_from_task));
        }
    }
    dedupe_graph_nodes(nodes)
}

fn graph_node_summary_from_compute_node(value: &Value) -> Option<GraphNodeSummary> {
    let id = compact_string_at_paths(value, &[&["id"], &["node_id"], &["thunk_id"], &["task_id"]])?;
    let kind = compact_string_at_paths(value, &[&["kind"], &["type"]])
        .map(|kind| status_token(&kind))
        .unwrap_or_else(|| "node".to_string());
    let status = compact_string_at_paths(value, &[&["status"]])
        .map(|status| status_token(&status))
        .unwrap_or_else(|| "unknown".to_string());
    let label = compact_string_at_paths(
        value,
        &[
            &["label"],
            &["title"],
            &["requested_input"],
            &["reason"],
            &["wait_ref", "requested_input"],
            &["task_id"],
            &["id"],
        ],
    )
    .unwrap_or_else(|| id.clone());
    Some(GraphNodeSummary {
        id,
        kind,
        status,
        label,
        goal_id: compact_string_at_paths(value, &[&["goal_id"]]),
        task_id: compact_string_at_paths(value, &[&["task_id"]]),
    })
}

fn graph_node_summary_from_task(value: &Value) -> Option<GraphNodeSummary> {
    let id = task_id(value)?;
    Some(GraphNodeSummary {
        kind: "task".to_string(),
        status: task_status(value),
        label: task_summary_line(value).unwrap_or_else(|| id.clone()),
        goal_id: compact_string_at_paths(
            value,
            &[
                &["goal_id"],
                &["payload_json", "goal_id"],
                &["raw_task", "goal_id"],
            ],
        ),
        task_id: Some(id.clone()),
        id,
    })
}

fn worker_run_summaries_from_value(value: &Value) -> Vec<WorkerRunSummary> {
    let mut workers = Vec::new();
    for array in arrays_at_paths(
        value,
        &[
            &["worker_runs"],
            &["runs"],
            &["workers"],
            &["runners", "data"],
            &["runners"],
            &["runner_registry", "data", "runners"],
        ],
    ) {
        workers.extend(array.iter().filter_map(worker_run_summary_from_value));
    }
    dedupe_worker_runs(workers)
}

fn worker_run_summary_from_value(value: &Value) -> Option<WorkerRunSummary> {
    let id = compact_string_at_paths(
        value,
        &[
            &["worker_run_id"],
            &["run_id"],
            &["runner_id"],
            &["id"],
            &["name"],
        ],
    )?;
    let runner = compact_string_at_paths(
        value,
        &[
            &["runner"],
            &["runner_id"],
            &["worker"],
            &["role"],
            &["kind"],
            &["name"],
        ],
    )
    .unwrap_or_else(|| id.clone());
    let status = compact_string_at_paths(value, &[&["status"], &["health"], &["mode"]])
        .map(|status| status_token(&status))
        .unwrap_or_else(|| "unknown".to_string());
    Some(WorkerRunSummary {
        id,
        runner,
        status,
        task: compact_string_at_paths(
            value,
            &[
                &["task_id"],
                &["current_task_id"],
                &["labels", "task_id"],
                &["labels", "jattg.dev/task-id"],
            ],
        ),
        endpoint: compact_string_at_paths(
            value,
            &[
                &["endpoint"],
                &["url"],
                &["base_url"],
                &["capabilities_url"],
                &["address"],
            ],
        ),
        node: compact_string_at_paths(
            value,
            &[
                &["node"],
                &["node_name"],
                &["hostname"],
                &["labels", "node"],
                &["labels", "kubernetes.io/hostname"],
            ],
        ),
        updated_at: compact_string_at_paths(
            value,
            &[&["updated_at"], &["last_seen_at"], &["heartbeat_at"]],
        ),
    })
}

fn evidence_summaries_from_value(value: &Value) -> Vec<EvidenceSummary> {
    let mut evidence = Vec::new();
    for array in arrays_at_paths(
        value,
        &[
            &["evidence"],
            &["evidence", "data"],
            &["artifacts", "data", "artifacts"],
            &["artifacts", "artifacts"],
            &["artifacts"],
            &["checkpoints", "data", "checkpoints"],
            &["checkpoints", "checkpoints"],
            &["checkpoints"],
        ],
    ) {
        evidence.extend(array.iter().filter_map(evidence_summary_from_value));
    }
    dedupe_evidence(evidence)
}

fn evidence_summary_from_value(value: &Value) -> Option<EvidenceSummary> {
    let summary = compact_string_at_paths(
        value,
        &[
            &["summary"],
            &["description"],
            &["label"],
            &["artifact", "description"],
            &["artifact", "summary"],
            &["artifact", "uri"],
            &["checkpoint", "summary"],
            &["checkpoint", "label"],
            &["object_artifact", "description"],
            &["git_result", "branch"],
            &["uri"],
            &["result_uri"],
        ],
    )?;
    let id = compact_string_at_paths(
        value,
        &[
            &["evidence_id"],
            &["artifact_id"],
            &["checkpoint_id"],
            &["id"],
            &["artifact", "id"],
            &["checkpoint", "id"],
            &["uri"],
            &["artifact", "uri"],
        ],
    )
    .unwrap_or_else(|| summary.clone());
    let kind = compact_string_at_paths(
        value,
        &[
            &["kind"],
            &["type"],
            &["artifact", "kind"],
            &["checkpoint", "kind"],
        ],
    )
    .unwrap_or_else(|| "evidence".to_string());
    Some(EvidenceSummary {
        id,
        kind,
        summary,
        source: compact_string_at_paths(
            value,
            &[
                &["source"],
                &["uri"],
                &["artifact", "uri"],
                &["checkpoint", "uri"],
                &["object_artifact", "uri"],
                &["git_result", "branch"],
            ],
        ),
        goal_id: compact_string_at_paths(value, &[&["goal_id"], &["payload_json", "goal_id"]]),
        task_id: compact_string_at_paths(value, &[&["task_id"], &["payload_json", "task_id"]]),
    })
}

fn arrays_at_paths<'a>(value: &'a Value, paths: &[&[&str]]) -> Vec<&'a [Value]> {
    paths
        .iter()
        .filter_map(|path| value_at_path(value, path).and_then(Value::as_array))
        .map(Vec::as_slice)
        .collect()
}

fn dedupe_graph_nodes(nodes: Vec<GraphNodeSummary>) -> Vec<GraphNodeSummary> {
    let mut deduped = Vec::new();
    for node in nodes {
        if deduped
            .iter()
            .any(|existing: &GraphNodeSummary| existing.id == node.id)
        {
            continue;
        }
        deduped.push(node);
    }
    deduped
}

fn dedupe_worker_runs(workers: Vec<WorkerRunSummary>) -> Vec<WorkerRunSummary> {
    let mut deduped = Vec::new();
    for worker in workers {
        if deduped
            .iter()
            .any(|existing: &WorkerRunSummary| existing.id == worker.id)
        {
            continue;
        }
        deduped.push(worker);
    }
    deduped
}

fn dedupe_evidence(evidence: Vec<EvidenceSummary>) -> Vec<EvidenceSummary> {
    let mut deduped = Vec::new();
    for item in evidence {
        if deduped
            .iter()
            .any(|existing: &EvidenceSummary| existing.id == item.id)
        {
            continue;
        }
        deduped.push(item);
    }
    deduped
}

fn chat_backend_label(config: &Value) -> String {
    let backend = config.get("chat_backend").unwrap_or(&Value::Null);
    let mode = backend
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let provider = backend
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("stub-or-config");
    let model = if backend
        .get("model_configured")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "model"
    } else {
        "no-model"
    };
    format!("{mode}/{provider}/{model}")
}

fn goal_label(value: &GoalSummary) -> String {
    format!("{} [{}] {}", value.title, value.status, short_id(&value.id))
}

fn current_goal_lines(
    selected: Option<&GoalSummary>,
    selected_goal_id: Option<&str>,
    width: u16,
) -> Vec<Line<'static>> {
    let value_width = width.saturating_sub(16).max(18) as usize;
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "current goal",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    match selected {
        Some(goal) => {
            lines.push(dashboard_value_line("title", &goal.title, value_width));
            lines.push(Line::from(format!(
                "status     {} {}% open:{} blocked:{}",
                goal.status,
                (goal.progress * 100.0).round() as u64,
                goal.open_tasks,
                goal.blocked_tasks
            )));
            lines.push(dashboard_value_line(
                "blocker",
                goal.current_blocker.as_deref().unwrap_or("none"),
                value_width,
            ));
            lines.push(dashboard_value_line(
                "action",
                goal.current_action
                    .as_deref()
                    .unwrap_or("wait for coordinator projection"),
                value_width,
            ));
            if let Some(evidence) = goal.latest_evidence.as_deref() {
                lines.push(dashboard_value_line("evidence", evidence, value_width));
            }
            if let Some(next_task) = goal.next_task.as_deref() {
                lines.push(dashboard_value_line("next", next_task, value_width));
            }
            lines.push(dashboard_value_line("id", &short_id(&goal.id), value_width));
            lines.push(Line::from(
                "controls   Ctrl-X arm/cancel, Ctrl-O clear selection",
            ));
        }
        None if let Some(goal_id) = selected_goal_id => {
            lines.push(dashboard_value_line(
                "selected",
                &short_id(goal_id),
                value_width,
            ));
            lines.push(Line::from("status     loading projection"));
            lines.push(Line::from(
                "controls   Ctrl-X arm/cancel, Ctrl-O clear selection",
            ));
        }
        None => {
            lines.push(Line::from("select     Ctrl-N / Ctrl-P"));
            lines.push(Line::from("chat       operator workspace"));
        }
    }
    lines
}

fn selected_goal_outline_lines(outline: &[String], width: u16) -> Vec<Line<'static>> {
    if outline.is_empty() {
        return Vec::new();
    }
    let value_width = width.saturating_sub(5).max(18) as usize;
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "goal outline",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    for item in outline.iter().take(8) {
        lines.push(Line::from(format!(
            "• {}",
            truncate_text(item, value_width)
        )));
    }
    if outline.len() > 8 {
        lines.push(Line::from(format!("  +{} more", outline.len() - 8)));
    }
    lines
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GoalRuntimeSummary {
    current_blocker: Option<String>,
    current_action: Option<String>,
    latest_evidence: Option<String>,
    next_task: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AdversarialTaskRow {
    id: Option<String>,
    title: String,
    status: Option<String>,
    role: Option<String>,
    purpose: Option<String>,
    reference: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AdversarialSummary {
    actors: Vec<AdversarialTaskRow>,
    critics: Vec<AdversarialTaskRow>,
    research: Vec<AdversarialTaskRow>,
    mechanisms: Vec<AdversarialTaskRow>,
    satisfaction_score: Option<String>,
    satisfied: Option<String>,
    satisfaction_reasons: Vec<String>,
    references: Vec<String>,
}

fn goal_outline_from_snapshot(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(subgoals) = first_array_at_paths(
        value,
        &[
            &[
                "goal_store_goal",
                "data",
                "goal",
                "payload_json",
                "plan",
                "subgoals",
            ],
            &[
                "goal_store_goal",
                "goal",
                "payload_json",
                "plan",
                "subgoals",
            ],
            &["goal", "payload_json", "plan", "subgoals"],
            &["plan", "subgoals"],
        ],
    ) {
        for subgoal in subgoals.iter().take(4) {
            if let Some(title) = compact_string_at_paths(
                subgoal,
                &[
                    &["title"],
                    &["name"],
                    &["objective"],
                    &["summary"],
                    &["id"],
                    &["subgoal_id"],
                ],
            ) {
                lines.push(format!("subgoal: {title}"));
            }
        }
    }

    if let Some(tasks) = first_array_at_paths(
        value,
        &[
            &["agent_activity"],
            &["agents", "data", "tasks"],
            &["agents", "tasks"],
            &["tasks", "data", "tasks"],
            &["tasks", "tasks"],
            &["data", "tasks"],
        ],
    ) {
        for task in tasks.iter().take(4).filter_map(task_summary_line) {
            lines.push(format!("task: {task}"));
        }
    }

    if let Some(nodes) = first_array_at_paths(
        value,
        &[
            &["workflow_compute_graph", "data", "nodes"],
            &["workflow_compute_graph", "nodes"],
            &["compute_graph", "nodes"],
        ],
    ) {
        for node in nodes.iter().take(4) {
            let kind = compact_string_at_paths(node, &[&["kind"]]).unwrap_or_else(|| "node".into());
            let status =
                compact_string_at_paths(node, &[&["status"]]).unwrap_or_else(|| "unknown".into());
            if let Some(label) = compact_string_at_paths(node, &[&["label"], &["id"]]) {
                lines.push(format!("compute: {label} [{kind} {status}]"));
            }
        }
    }

    lines
}

fn goal_summary_from_snapshot(value: &Value, fallback_goal_id: &str) -> Option<GoalSummary> {
    let mut summary = first_object_at_paths(
        value,
        &[
            &["goal_store_goal", "data", "goal"],
            &["goal_store_goal", "goal"],
            &["data", "goal"],
            &["goal"],
        ],
    )
    .and_then(goal_summary_from_value)
    .or_else(|| {
        first_object_at_paths(
            value,
            &[
                &["workflow_progress", "data"],
                &["workflow_progress"],
                &["progress", "data"],
                &["progress"],
            ],
        )
        .and_then(goal_summary_from_value)
    })
    .or_else(|| {
        let id = fallback_goal_id.trim();
        (!id.is_empty()).then(|| GoalSummary {
            id: id.to_string(),
            title: id.to_string(),
            status: "selected".to_string(),
            ..GoalSummary::default()
        })
    })?;

    if let Some(progress) = first_object_at_paths(
        value,
        &[
            &["workflow_progress", "data"],
            &["workflow_progress"],
            &["progress", "data"],
            &["progress"],
        ],
    ) {
        if progress
            .get("percent_done")
            .or_else(|| progress.get("progress"))
            .is_some()
        {
            summary.progress = progress_value(
                progress
                    .get("percent_done")
                    .or_else(|| progress.get("progress")),
            );
        }
        if let Some(status) = compact_string_at_paths(progress, &[&["status"]]) {
            summary.status = status;
        }
        if let Some(open_tasks) = usize_field_at_paths(progress, &[&["open_tasks"]]) {
            summary.open_tasks = open_tasks;
        }
        if let Some(blocked_tasks) = usize_field_at_paths(progress, &[&["blocked_tasks"]]) {
            summary.blocked_tasks = blocked_tasks;
        }
    }

    let runtime = goal_runtime_summary_from_snapshot(value);
    if summary.current_blocker.is_none() {
        summary.current_blocker = runtime.current_blocker;
    }
    if summary.current_action.is_none() {
        summary.current_action = runtime.current_action;
    }
    if summary.latest_evidence.is_none() {
        summary.latest_evidence = runtime.latest_evidence;
    }
    if summary.next_task.is_none() {
        summary.next_task = runtime.next_task;
    }
    Some(summary)
}

fn upsert_goal_summary(goals: &mut Vec<GoalSummary>, summary: GoalSummary) {
    if let Some(existing) = goals.iter_mut().find(|goal| goal.id == summary.id) {
        *existing = summary;
    } else {
        goals.insert(0, summary);
    }
}

fn goal_runtime_summary_from_snapshot(value: &Value) -> GoalRuntimeSummary {
    let tasks = first_array_at_paths(
        value,
        &[
            &["agent_activity"],
            &["tasks", "data", "tasks"],
            &["tasks", "tasks"],
            &["data", "tasks"],
        ],
    );
    let next_tasks = first_array_at_paths(
        value,
        &[
            &["workflow_progress", "data", "next_tasks"],
            &["workflow_progress", "next_tasks"],
            &["progress", "data", "next_tasks"],
            &["progress", "next_tasks"],
        ],
    );
    let events = first_array_at_paths(
        value,
        &[
            &["events", "data", "events"],
            &["events", "events"],
            &["data", "events"],
        ],
    );
    let artifacts = first_array_at_paths(
        value,
        &[
            &["artifacts", "data", "artifacts"],
            &["artifacts", "artifacts"],
            &["data", "artifacts"],
        ],
    );
    let checkpoints = first_array_at_paths(
        value,
        &[
            &["checkpoints", "data", "checkpoints"],
            &["checkpoints", "checkpoints"],
            &["data", "checkpoints"],
        ],
    );
    let approvals = first_array_at_paths(
        value,
        &[
            &["approvals", "data", "approvals"],
            &["approvals", "approvals"],
            &["data", "approvals"],
        ],
    );

    let next_task = first_task_matching(next_tasks, task_is_open)
        .or_else(|| first_task_matching(tasks, task_is_next_candidate));
    let current_blocker = current_blocker_from_approvals(approvals)
        .or_else(|| first_task_matching(tasks, task_needs_operator_attention))
        .or_else(|| latest_event_matching(events, event_is_blocker));
    let current_action = current_action_from_approvals(approvals)
        .or_else(|| first_task_matching(tasks, task_is_active))
        .or_else(|| next_task.as_ref().map(|task| format!("next: {task}")));
    let latest_evidence = latest_checkpoint_or_artifact(checkpoints)
        .or_else(|| latest_checkpoint_or_artifact(artifacts))
        .or_else(|| latest_task_evidence(tasks))
        .or_else(|| latest_event_matching(events, event_is_evidence));

    GoalRuntimeSummary {
        current_blocker,
        current_action,
        latest_evidence,
        next_task,
    }
}

fn adversarial_summary_from_snapshot(value: &Value) -> AdversarialSummary {
    let mut summary = AdversarialSummary::default();
    let task_rows = adversarial_task_rows_from_snapshot(value);
    for row in task_rows {
        if adversarial_row_is_mechanism(&row) {
            summary.mechanisms.push(row);
        } else if adversarial_row_is_research(&row) {
            summary.research.push(row);
        } else if adversarial_row_is_critic(&row) {
            summary.critics.push(row);
        } else if adversarial_row_is_actor(&row) {
            summary.actors.push(row);
        }
    }
    summary
        .mechanisms
        .extend(adversarial_mechanism_rows_from_snapshot(value));
    let (score, satisfied, reasons) = adversarial_satisfaction_from_snapshot(value);
    summary.satisfaction_score = score;
    summary.satisfied = satisfied;
    summary.satisfaction_reasons = reasons;
    summary.references = adversarial_references_from_snapshot(value);
    summary
}

fn adversarial_task_rows_from_snapshot(value: &Value) -> Vec<AdversarialTaskRow> {
    let mut rows = Vec::new();
    for tasks in [
        first_array_at_paths(
            value,
            &[
                &["agent_activity"],
                &["agents", "data", "tasks"],
                &["agents", "tasks"],
                &["tasks", "data", "tasks"],
                &["tasks", "tasks"],
                &["data", "tasks"],
            ],
        ),
        first_array_at_paths(
            value,
            &[
                &["workflow_progress", "data", "next_tasks"],
                &["workflow_progress", "next_tasks"],
                &["progress", "data", "next_tasks"],
                &["progress", "next_tasks"],
            ],
        ),
    ]
    .into_iter()
    .flatten()
    {
        rows.extend(tasks.iter().filter_map(adversarial_task_row_from_value));
    }
    dedupe_adversarial_rows(rows)
}

fn adversarial_task_row_from_value(value: &Value) -> Option<AdversarialTaskRow> {
    let title = compact_string_at_paths(
        value,
        &[
            &["title"],
            &["current_prompt"],
            &["prompt"],
            &["payload_json", "title"],
            &["payload_json", "prompt"],
            &["raw_task", "title"],
            &["raw_task", "prompt"],
        ],
    )
    .or_else(|| task_id(value).map(|id| format!("task {}", short_id(&id))))?;
    Some(AdversarialTaskRow {
        id: task_id(value),
        title,
        status: Some(task_status(value)).filter(|status| !status.is_empty()),
        role: adversarial_role(value),
        purpose: adversarial_purpose(value),
        reference: adversarial_task_reference(value),
    })
}

fn adversarial_role(value: &Value) -> Option<String> {
    compact_string_at_paths(
        value,
        &[
            &["role"],
            &["payload_json", "role"],
            &["raw_task", "role"],
            &["execution", "runner", "worker"],
            &["payload_json", "execution", "runner", "worker"],
        ],
    )
    .map(|role| status_token(&role))
}

fn adversarial_purpose(value: &Value) -> Option<String> {
    compact_string_at_paths(
        value,
        &[
            &["purpose_kind"],
            &["purpose", "kind"],
            &["payload_json", "purpose_kind"],
            &["payload_json", "purpose", "kind"],
            &["raw_task", "purpose_kind"],
            &["raw_task", "purpose", "kind"],
        ],
    )
    .map(|purpose| status_token(&purpose))
}

fn adversarial_task_reference(value: &Value) -> Option<String> {
    compact_string_at_paths(
        value,
        &[
            &["result_uri"],
            &["result", "uri"],
            &["payload_json", "result", "uri"],
            &["payload_json", "result_uri"],
            &["payload_json", "mcp_context_used", "context_id"],
            &["payload_json", "coordinator_trace_id"],
            &["raw_task", "execution", "mcp", "context_id"],
        ],
    )
}

fn adversarial_row_is_actor(row: &AdversarialTaskRow) -> bool {
    matches!(
        row.purpose.as_deref(),
        Some("work" | "actor-retry" | "candidate-branch")
    ) || matches!(
        row.role.as_deref(),
        Some(
            "planner"
                | "codex"
                | "claude-code"
                | "staff-engineer-claude"
                | "model-provider"
                | "rust-tool"
        )
    )
}

fn adversarial_row_is_critic(row: &AdversarialTaskRow) -> bool {
    matches!(row.purpose.as_deref(), Some("review"))
        || matches!(
            row.role.as_deref(),
            Some("reviewer" | "tester" | "formal-methods" | "validator")
        )
}

fn adversarial_row_is_research(row: &AdversarialTaskRow) -> bool {
    matches!(row.purpose.as_deref(), Some("research"))
        || matches!(row.role.as_deref(), Some("research"))
}

fn adversarial_row_is_mechanism(row: &AdversarialTaskRow) -> bool {
    matches!(
        row.purpose.as_deref(),
        Some("unification" | "branch-vote" | "branch-unification")
    ) || matches!(row.role.as_deref(), Some("patch-merger"))
}

fn adversarial_mechanism_rows_from_snapshot(value: &Value) -> Vec<AdversarialTaskRow> {
    let mut rows = Vec::new();
    for votes in [
        first_array_at_paths(
            value,
            &[
                &["branch_votes"],
                &["branch_votes", "data", "branch_votes"],
                &["branch_votes", "data"],
            ],
        ),
        first_array_at_paths(
            value,
            &[
                &[
                    "goal_store_goal",
                    "data",
                    "goal",
                    "payload_json",
                    "branch_votes",
                ],
                &["goal", "payload_json", "branch_votes"],
            ],
        ),
    ]
    .into_iter()
    .flatten()
    {
        rows.extend(votes.iter().filter_map(|vote| {
            let id = compact_string_at_paths(vote, &[&["selected_task_id"], &["task_id"], &["id"]]);
            let title = compact_string_at_paths(vote, &[&["rationale"], &["reason"]])
                .unwrap_or_else(|| "branch vote".to_string());
            Some(AdversarialTaskRow {
                id,
                title,
                status: compact_string_at_paths(vote, &[&["decision"], &["status"]]),
                role: Some("vote".to_string()),
                purpose: Some("branch-vote".to_string()),
                reference: compact_string_at_paths(vote, &[&["group_id"], &["branch_group_id"]]),
            })
        }));
    }
    for rounds in [
        first_array_at_paths(
            value,
            &[
                &["review_rounds"],
                &["review_rounds", "data", "review_rounds"],
                &["review_rounds", "data"],
            ],
        ),
        first_array_at_paths(
            value,
            &[
                &[
                    "goal_store_goal",
                    "data",
                    "goal",
                    "payload_json",
                    "review_rounds",
                ],
                &["goal", "payload_json", "review_rounds"],
            ],
        ),
    ]
    .into_iter()
    .flatten()
    {
        rows.extend(rounds.iter().enumerate().map(|(index, round)| {
            let round_id = compact_string_at_paths(round, &[&["round"], &["id"]])
                .unwrap_or_else(|| (index + 1).to_string());
            AdversarialTaskRow {
                id: compact_string_at_paths(round, &[&["unification_task_id"]]),
                title: format!("review round {round_id}"),
                status: compact_string_at_paths(round, &[&["status"]])
                    .map(|status| status_token(&status)),
                role: Some("review-round".to_string()),
                purpose: Some("unification".to_string()),
                reference: compact_string_at_paths(
                    round,
                    &[&["subject_task_ids"], &["reviewer_task_ids"]],
                ),
            }
        }));
    }
    dedupe_adversarial_rows(rows)
}

fn adversarial_satisfaction_from_snapshot(
    value: &Value,
) -> (Option<String>, Option<String>, Vec<String>) {
    let Some(satisfaction) = first_object_at_paths(
        value,
        &[
            &["workflow_progress", "data", "satisfaction"],
            &["workflow_progress", "satisfaction"],
            &["progress", "data", "satisfaction"],
            &["progress", "satisfaction"],
            &[
                "goal_store_goal",
                "data",
                "goal",
                "payload_json",
                "satisfaction",
            ],
            &["goal", "payload_json", "satisfaction"],
            &["satisfaction"],
        ],
    ) else {
        return (None, None, Vec::new());
    };
    let score = compact_string_at_paths(
        satisfaction,
        &[
            &["score"],
            &["satisfaction_score"],
            &["min_satisfaction_score"],
        ],
    );
    let satisfied = compact_string_at_paths(satisfaction, &[&["satisfied"], &["is_satisfied"]]);
    let reasons = value_at_path(satisfaction, &["reasons"])
        .and_then(Value::as_array)
        .map(|reasons| reasons.iter().filter_map(compact_value_string).collect())
        .unwrap_or_default();
    (score, satisfied, reasons)
}

fn adversarial_references_from_snapshot(value: &Value) -> Vec<String> {
    let mut refs = Vec::new();
    collect_adversarial_references(value, &mut refs);
    refs.truncate(40);
    refs
}

fn collect_adversarial_references(value: &Value, refs: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if adversarial_reference_key(key)
                    && let Some(text) = compact_value_string(child)
                {
                    push_unique_string(refs, format!("{key}={text}"));
                }
                collect_adversarial_references(child, refs);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_adversarial_references(child, refs);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn adversarial_reference_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "agent_context_id"
            | "context_id"
            | "mcp_context_id"
            | "thread_id"
            | "session_id"
            | "chat_session_id"
            | "conversation_id"
            | "coordinator_trace_id"
            | "trace_id"
            | "run_id"
            | "checkpoint_id"
            | "result_uri"
            | "artifact_uri"
            | "pull_request_url"
            | "branch"
            | "uri"
    )
}

fn dedupe_adversarial_rows(rows: Vec<AdversarialTaskRow>) -> Vec<AdversarialTaskRow> {
    let mut deduped = Vec::new();
    for row in rows {
        let key = format!(
            "{}:{}:{}",
            row.id.as_deref().unwrap_or_default(),
            row.title,
            row.purpose.as_deref().unwrap_or_default()
        );
        if deduped.iter().any(|existing: &AdversarialTaskRow| {
            format!(
                "{}:{}:{}",
                existing.id.as_deref().unwrap_or_default(),
                existing.title,
                existing.purpose.as_deref().unwrap_or_default()
            ) == key
        }) {
            continue;
        }
        deduped.push(row);
    }
    deduped
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn goal_summaries_from_value(value: &Value) -> Vec<GoalSummary> {
    find_first_array(value, &["goals"])
        .unwrap_or(&[])
        .iter()
        .filter_map(goal_summary_from_value)
        .collect()
}

fn goal_summary_from_value(value: &Value) -> Option<GoalSummary> {
    let id = value
        .get("goal_id")
        .or_else(|| value.get("id"))
        .or_else(|| value.pointer("/goal/id"))
        .or_else(|| value.pointer("/spec/id"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if id.is_empty() {
        return None;
    }
    let title = value
        .get("title")
        .or_else(|| value.get("objective"))
        .or_else(|| value.pointer("/goal/title"))
        .or_else(|| value.pointer("/goal/objective"))
        .or_else(|| value.pointer("/spec/title"))
        .or_else(|| value.pointer("/spec/objective"))
        .and_then(Value::as_str)
        .map(compact_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| id.clone());
    let status = value
        .get("status")
        .or_else(|| value.pointer("/state/status"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    Some(GoalSummary {
        id,
        title,
        status,
        progress: progress_value(value.get("percent_done").or_else(|| value.get("progress"))),
        open_tasks: usize_value(value.get("open_tasks")),
        blocked_tasks: usize_value(value.get("blocked_tasks")),
        current_blocker: compact_string_at_paths(
            value,
            &[
                &["current_blocker"],
                &["blocker"],
                &["payload_json", "current_blocker"],
                &["payload_json", "blocker"],
            ],
        ),
        current_action: compact_string_at_paths(
            value,
            &[
                &["current_action"],
                &["action"],
                &["payload_json", "current_action"],
                &["payload_json", "action"],
            ],
        ),
        latest_evidence: compact_string_at_paths(
            value,
            &[
                &["latest_evidence"],
                &["evidence"],
                &["payload_json", "latest_evidence"],
                &["payload_json", "evidence"],
            ],
        ),
        next_task: compact_string_at_paths(
            value,
            &[
                &["next_task"],
                &["payload_json", "next_task"],
                &["payload_json", "next_action"],
            ],
        ),
    })
}

fn progress_value(value: Option<&Value>) -> f64 {
    let number = value.and_then(Value::as_f64).unwrap_or(0.0);
    if number > 1.0 {
        number / 100.0
    } else {
        number.max(0.0)
    }
}

fn usize_value(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(usize::MAX as u64) as usize
}

fn first_object_at_paths<'a>(value: &'a Value, paths: &[&[&str]]) -> Option<&'a Value> {
    paths
        .iter()
        .find_map(|path| value_at_path(value, path).filter(|value| value.is_object()))
}

fn first_array_at_paths<'a>(value: &'a Value, paths: &[&[&str]]) -> Option<&'a [Value]> {
    paths
        .iter()
        .find_map(|path| value_at_path(value, path).and_then(Value::as_array))
        .map(Vec::as_slice)
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn compact_string_at_paths(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value_at_path(value, path).and_then(compact_value_string))
}

fn compact_value_string(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(values) => values.iter().find_map(compact_value_string)?,
        Value::Null | Value::Object(_) => return None,
    };
    let text = compact_text(&text);
    (!text.is_empty()).then_some(text)
}

fn usize_field_at_paths(value: &Value, paths: &[&[&str]]) -> Option<usize> {
    paths.iter().find_map(|path| {
        let value = value_at_path(value, path)?;
        if let Some(number) = value.as_u64() {
            return Some(number.min(usize::MAX as u64) as usize);
        }
        value
            .as_str()
            .and_then(|text| text.trim().parse::<usize>().ok())
    })
}

fn first_task_matching(tasks: Option<&[Value]>, predicate: fn(&Value) -> bool) -> Option<String> {
    tasks?
        .iter()
        .find(|task| predicate(task))
        .and_then(task_summary_line)
}

fn task_summary_line(task: &Value) -> Option<String> {
    let title = compact_string_at_paths(
        task,
        &[
            &["title"],
            &["current_prompt"],
            &["prompt"],
            &["payload_json", "title"],
            &["payload_json", "prompt"],
            &["raw_task", "title"],
        ],
    )
    .or_else(|| task_id(task).map(|id| format!("task {}", short_id(&id))))?;
    let mut meta = Vec::new();
    let status = task_status(task);
    if !status.is_empty() {
        meta.push(status);
    }
    if let Some(role) = compact_string_at_paths(task, &[&["role"], &["payload_json", "role"]]) {
        meta.push(role);
    }
    if let Some(id) = task_id(task) {
        meta.push(short_id(&id));
    }
    if meta.is_empty() {
        Some(title)
    } else {
        Some(format!("{title} [{}]", meta.join(" ")))
    }
}

fn task_id(task: &Value) -> Option<String> {
    compact_string_at_paths(
        task,
        &[
            &["task_id"],
            &["id"],
            &["payload_json", "id"],
            &["raw_task", "task_id"],
        ],
    )
}

fn task_status(task: &Value) -> String {
    compact_string_at_paths(
        task,
        &[
            &["status"],
            &["progress", "status"],
            &["payload_json", "status"],
            &["raw_task", "status"],
        ],
    )
    .map(|status| status_token(&status))
    .unwrap_or_default()
}

fn status_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn status_color(status: &str) -> Color {
    match status_token(status).as_str() {
        "done" | "satisfied" | "healthy" | "ready" | "completed" => Color::Green,
        "running" | "runnable" | "in-progress" | "active" => Color::Cyan,
        "blocked" | "failed" | "error" | "unhealthy" => Color::Red,
        "waiting-input" | "waiting-approval" | "pending" | "queued" => Color::Yellow,
        "cancelled" | "canceled" => Color::DarkGray,
        _ => Color::White,
    }
}

fn task_is_open(task: &Value) -> bool {
    !matches!(task_status(task).as_str(), "done" | "failed" | "cancelled")
}

fn task_is_next_candidate(task: &Value) -> bool {
    if task_runnable(task) {
        return true;
    }
    matches!(
        task_status(task).as_str(),
        "pending" | "runnable" | "running" | "needs-validation"
    )
}

fn task_is_active(task: &Value) -> bool {
    task_runnable(task)
        || matches!(
            task_status(task).as_str(),
            "running" | "runnable" | "needs-validation"
        )
}

fn task_needs_operator_attention(task: &Value) -> bool {
    matches!(
        task_status(task).as_str(),
        "blocked" | "failed" | "waiting-approval" | "waiting-input"
    )
}

fn task_runnable(task: &Value) -> bool {
    value_at_path(task, &["runnable"])
        .or_else(|| value_at_path(task, &["progress", "runnable"]))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn current_blocker_from_approvals(approvals: Option<&[Value]>) -> Option<String> {
    approvals?
        .iter()
        .rev()
        .find(|approval| approval_is_pending(approval))
        .and_then(|approval| {
            approval_action_text(approval)
                .map(|text| format!("approval: {text}"))
                .or_else(|| Some("approval pending".to_string()))
        })
}

fn current_action_from_approvals(approvals: Option<&[Value]>) -> Option<String> {
    approvals?
        .iter()
        .rev()
        .find(|approval| approval_is_pending(approval))
        .and_then(|approval| {
            approval_action_text(approval).map(|text| format!("review approval: {text}"))
        })
}

fn approval_is_pending(approval: &Value) -> bool {
    let status = compact_string_at_paths(approval, &[&["status"], &["payload_json", "status"]])
        .map(|status| status_token(&status))
        .unwrap_or_default();
    status.is_empty() || matches!(status.as_str(), "pending" | "requested" | "waiting")
}

fn approval_action_text(approval: &Value) -> Option<String> {
    compact_string_at_paths(
        approval,
        &[
            &["requested_action"],
            &["reason"],
            &["payload_json", "requested_action"],
            &["payload_json", "reason"],
        ],
    )
}

fn latest_checkpoint_or_artifact(items: Option<&[Value]>) -> Option<String> {
    items?
        .iter()
        .rev()
        .find_map(checkpoint_or_artifact_summary_line)
}

fn checkpoint_or_artifact_summary_line(value: &Value) -> Option<String> {
    let label = compact_string_at_paths(value, &[&["label"], &["checkpoint", "label"]]);
    let summary = compact_string_at_paths(
        value,
        &[
            &["summary"],
            &["checkpoint", "summary"],
            &["artifact", "description"],
            &["description"],
            &["object_artifact", "description"],
            &["git_result", "branch"],
            &["artifact", "uri"],
            &["uri"],
        ],
    )?;
    Some(match label {
        Some(label) if !summary.contains(&label) => format!("{label}: {summary}"),
        _ => summary,
    })
}

fn latest_task_evidence(tasks: Option<&[Value]>) -> Option<String> {
    tasks?.iter().rev().find_map(|task| {
        let evidence = compact_string_at_paths(
            task,
            &[
                &["result", "summary"],
                &["result", "description"],
                &["result", "uri"],
                &["result_uri"],
                &["payload_json", "result", "summary"],
                &["payload_json", "result", "description"],
                &["payload_json", "result", "uri"],
            ],
        )?;
        task_summary_line(task)
            .map(|task| format!("{task}: {evidence}"))
            .or(Some(evidence))
    })
}

fn latest_event_matching(
    events: Option<&[Value]>,
    predicate: fn(&Value) -> bool,
) -> Option<String> {
    events?
        .iter()
        .rev()
        .find(|event| predicate(event))
        .and_then(event_summary_line)
}

fn event_is_blocker(event: &Value) -> bool {
    let kind = compact_string_at_paths(event, &[&["kind"], &["payload_json", "kind"]])
        .map(|kind| status_token(&kind))
        .unwrap_or_default();
    let message = compact_string_at_paths(event, &[&["message"], &["payload_json", "message"]])
        .map(|message| message.to_ascii_lowercase())
        .unwrap_or_default();
    kind.contains("blocked")
        || kind.contains("approval-requested")
        || message.starts_with("task_blocked")
        || message.contains("blocked")
}

fn event_is_evidence(event: &Value) -> bool {
    let kind = compact_string_at_paths(event, &[&["kind"], &["payload_json", "kind"]])
        .map(|kind| status_token(&kind))
        .unwrap_or_default();
    let message = compact_string_at_paths(event, &[&["message"], &["payload_json", "message"]])
        .map(|message| message.to_ascii_lowercase())
        .unwrap_or_default();
    kind.contains("completed")
        || kind.contains("validation")
        || kind.contains("artifact")
        || message.starts_with("task_completed")
        || message.starts_with("validation_")
}

fn event_summary_line(event: &Value) -> Option<String> {
    let message = compact_string_at_paths(event, &[&["message"], &["payload_json", "message"]])?;
    let task = compact_string_at_paths(event, &[&["task_id"], &["payload_json", "task_id"]])
        .map(|id| format!(" task {}", short_id(&id)))
        .unwrap_or_default();
    Some(format!("{message}{task}"))
}

fn chat_session_id_for(goal_id: Option<&str>) -> String {
    goal_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|goal_id| format!("goal:{goal_id}"))
        .unwrap_or_else(|| "operator:default".to_string())
}

fn goal_id_after_step(
    goals: &[GoalSummary],
    selected_goal_id: Option<&str>,
    step: isize,
) -> Option<String> {
    if goals.is_empty() {
        return None;
    }
    let current = selected_goal_id
        .and_then(|selected| goals.iter().position(|goal| goal.id == selected))
        .unwrap_or(0);
    let len = goals.len() as isize;
    let next = (current as isize + step).rem_euclid(len) as usize;
    Some(goals[next].id.clone())
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

fn find_first_array<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a [Value]> {
    if let Some(array) = value.as_array() {
        return Some(array.as_slice());
    }
    let object = value.as_object()?;
    for key in keys {
        if let Some(found) = object
            .get(*key)
            .and_then(|child| find_first_array(child, keys))
        {
            return Some(found);
        }
    }
    for key in ["data", "items", "records", "rows", "result"] {
        if let Some(found) = object
            .get(key)
            .and_then(|child| find_first_array(child, keys))
        {
            return Some(found);
        }
    }
    None
}

fn chat_lines_from_session(value: &Value) -> Vec<ChatLine> {
    value
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let role = item.get("role").and_then(Value::as_str)?;
            let content = item.get("content").and_then(Value::as_str)?.trim();
            if content.is_empty() {
                return None;
            }
            Some(ChatLine {
                role: role.to_string(),
                content: content.to_string(),
            })
        })
        .collect()
}

fn chat_request_payload(
    session_id: String,
    goal_id: Option<String>,
    mode: ChatMode,
    messages: &[ChatLine],
) -> Value {
    let mut payload = json!({
        "session_id": session_id,
        "mode": mode.as_str(),
        "run_id": Uuid::new_v4().to_string(),
        "messages": messages.iter().map(|message| {
            json!({
                "role": message.role,
                "content": message.content,
            })
        }).collect::<Vec<_>>(),
    });
    if let Some(goal_id) = goal_id.filter(|value| !value.trim().is_empty()) {
        payload["goal_id"] = Value::String(goal_id);
    }
    payload
}

fn durable_chat_lines(messages: &[ChatLine]) -> Vec<ChatLine> {
    messages
        .iter()
        .filter(|message| {
            !message.content.starts_with("Goal draft ready.")
                && !message
                    .content
                    .starts_with("Accepted draft and submitted goal to the coordinator.")
                && !message.content.starts_with("Goal submit failed:")
        })
        .cloned()
        .collect()
}

fn chat_status(value: &Value) -> String {
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("stub");
    let model = value.get("model").and_then(Value::as_str).unwrap_or("none");
    let durable = value
        .pointer("/chat_log/durable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!("chat done provider={provider} model={model} durable_log={durable}")
}

fn extract_goal_draft(value: &Value) -> Option<Value> {
    for path in [
        "/drafts/goal_spec",
        "/drafts/goal",
        "/drafts/goal_payload",
        "/goal_spec",
        "/goal",
    ] {
        let Some(draft) = value.pointer(path) else {
            continue;
        };
        if draft.is_object()
            && draft
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|title| !title.trim().is_empty())
            && draft
                .get("objective")
                .and_then(Value::as_str)
                .is_some_and(|objective| !objective.trim().is_empty())
        {
            return Some(draft.clone());
        }
    }
    None
}

#[cfg(test)]
fn goal_draft_summary_from_response(value: &Value) -> Option<GoalDraftSummary> {
    let draft = extract_goal_draft(value)?;
    goal_draft_summary(&draft)
}

fn active_goal_draft_from_response(value: &Value) -> Option<ActiveGoalDraft> {
    let goal_spec = extract_goal_draft(value)?;
    let summary = goal_draft_summary(&goal_spec)?;
    Some(ActiveGoalDraft {
        goal_spec,
        summary,
        session_id: String::new(),
        selected_goal_id: None,
    })
}

fn goal_draft_summary(goal_spec: &Value) -> Option<GoalDraftSummary> {
    let title = compact_text(goal_spec.get("title")?.as_str()?);
    let objective = compact_text(goal_spec.get("objective")?.as_str()?);
    if title.is_empty() || objective.is_empty() {
        return None;
    }
    Some(GoalDraftSummary {
        title,
        objective,
        initial_tasks: goal_spec
            .get("initial_tasks")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        done_criteria: done_criteria_summary(goal_spec.get("done_criteria")),
    })
}

fn done_criteria_summary(done_criteria: Option<&Value>) -> String {
    let Some(done_criteria) = done_criteria else {
        return "not specified".to_string();
    };
    let mut parts = Vec::new();
    match done_criteria.get("tests_pass").and_then(Value::as_bool) {
        Some(true) => parts.push("tests_pass".to_string()),
        Some(false) => parts.push("tests optional".to_string()),
        None => {}
    }
    match done_criteria
        .get("artifact_exists")
        .and_then(Value::as_bool)
    {
        Some(true) => parts.push("artifact_exists".to_string()),
        Some(false) => parts.push("artifact optional".to_string()),
        None => {}
    }
    if let Some(score) = done_criteria
        .get("validator_score_min")
        .and_then(Value::as_f64)
    {
        parts.push(format!("validator >= {score:.2}"));
    } else if let Some(score) = done_criteria
        .get("validator_score_min")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|score| !score.is_empty())
    {
        parts.push(format!("validator >= {score}"));
    }

    if parts.is_empty() {
        "not specified".to_string()
    } else {
        parts.join(", ")
    }
}

fn goal_draft_dashboard_lines(summary: &GoalDraftSummary, width: u16) -> Vec<Line<'static>> {
    let value_width = width.saturating_sub(15).max(20) as usize;
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "active goal draft",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        dashboard_value_line("title", &summary.title, value_width),
        dashboard_value_line("objective", &summary.objective, value_width),
        Line::from(format!("tasks      {}", summary.initial_tasks)),
        dashboard_value_line("criteria", &summary.done_criteria, value_width),
        Line::from(Span::styled(
            "accept     F5 or Ctrl-G",
            Style::default().fg(Color::Green),
        )),
    ]
}

fn dashboard_value_line(label: &str, value: &str, value_width: usize) -> Line<'static> {
    Line::from(format!(
        "{label:<10} {}",
        truncate_text(value, value_width.max(8))
    ))
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let value = compact_text(value);
    if value.chars().count() <= max_chars {
        return value;
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    format!(
        "{}...",
        value
            .chars()
            .take(max_chars.saturating_sub(3))
            .collect::<String>()
    )
}

fn submitted_goal_id(value: &Value) -> Option<String> {
    for path in [
        "/goal_id",
        "/id",
        "/goal/id",
        "/goal/spec/id",
        "/state/goal/id",
        "/result/goal_id",
        "/result/id",
    ] {
        if let Some(id) = value.pointer(path).and_then(Value::as_str) {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn rendered_row_count(lines: &[Line<'_>], width: u16) -> usize {
    let width = width.max(1) as usize;
    lines
        .iter()
        .map(|line| {
            let cells = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>();
            cells.max(1).div_ceil(width)
        })
        .sum()
}

fn chat_scroll_y(total_rows: usize, viewport_height: u16, from_bottom: u16) -> u16 {
    let visible_rows = viewport_height as usize;
    let max_scroll = total_rows.saturating_sub(visible_rows);
    max_scroll
        .saturating_sub(from_bottom as usize)
        .min(u16::MAX as usize) as u16
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        App::new(TuiConfig {
            control_gateway_url: "http://127.0.0.1:9".to_string(),
            token: None,
            session_id: "operator:default".to_string(),
            goal_id: None,
            refresh: Duration::from_secs(30),
        })
        .expect("app")
    }

    #[test]
    fn dashboard_summary_reads_gateway_proxy_shapes() {
        let workspace = json!({
            "config": {
                "chat_backend": {
                    "mode": "runner_registry",
                    "provider": "openai_compatible",
                    "model_configured": true
                }
            },
            "services": [
                {"name": "goal-store", "ok": true},
                {"name": "runner-registry", "status": 503}
            ],
            "goals": [
                {"goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601", "title": "Ship TUI", "status": "running"}
            ],
            "runners": {"data": [{"runner_id": "r1"}]},
            "actions": [{"action_id": "a1"}, {"action_id": "a2"}],
            "events": [{"id": "e1", "message": "event"}],
            "plans": {"data": []}
        });

        let goals = goal_summaries_from_value(&workspace);
        let summary = dashboard_summary(&workspace, &goals);

        assert_eq!(summary.services_ok, 1);
        assert_eq!(summary.services_total, 2);
        assert_eq!(summary.goals_count, 1);
        assert_eq!(summary.runners_count, 1);
        assert_eq!(summary.approvals_count, 2);
        assert_eq!(summary.events_count, 1);
        assert_eq!(
            summary.chat_backend,
            "runner_registry/openai_compatible/model"
        );
        assert_eq!(summary.latest_goals, vec!["Ship TUI [running] 018f8f2f"]);
    }

    #[test]
    fn approval_event_and_source_rows_parse_gateway_proxy_shapes() {
        let value = json!({
            "approvals": {
                "data": {
                    "approvals": [
                        {
                            "approval_id": "a-12345678",
                            "status": "pending",
                            "requested_action": "approve Kubernetes executor capacity",
                            "risk": "critical",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "118f8f2f-1fd8-7688-bb12-8bfb6b756602",
                            "created_at": "2026-05-12T12:00:00Z"
                        }
                    ]
                }
            },
            "recent_events": {
                "data": {
                    "events": [
                        {
                            "event_id": "e-12345678",
                            "kind": "ci_check_failed",
                            "message": "Runner target check failed",
                            "source_id": "github-actions",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "118f8f2f-1fd8-7688-bb12-8bfb6b756602",
                            "received_at": "2026-05-12T12:01:00Z"
                        }
                    ]
                }
            },
            "event_sources": {
                "data": {
                    "sources": [
                        {
                            "source_id": "github-actions",
                            "kind": "github_actions_checks",
                            "status": "enabled",
                            "route": {"mode": "trigger_goal"},
                            "approval_ref": "approved-by-ops"
                        }
                    ]
                }
            }
        });

        let approvals = approval_summaries_from_value(&value);
        let events = event_summaries_from_value(&value);
        let sources = event_source_summaries_from_value(&value);

        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].status, "pending");
        assert_eq!(approvals[0].action, "approve Kubernetes executor capacity");
        assert_eq!(approvals[0].risk.as_deref(), Some("critical"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "ci_check_failed");
        assert_eq!(events[0].message, "Runner target check failed");
        assert_eq!(events[0].source.as_deref(), Some("github-actions"));
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "github-actions");
        assert_eq!(sources[0].kind, "github_actions_checks");
        assert_eq!(sources[0].route.as_deref(), Some("trigger_goal"));
    }

    #[test]
    fn action_needed_summaries_include_blocked_tasks_and_thunks() {
        let value = json!({
            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
            "tasks": {
                "data": {
                    "tasks": [
                        {
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "task-blocked-1",
                            "title": "Review blocked executor task",
                            "status": "blocked",
                            "role": "reviewer"
                        },
                        {
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "task-waiting-1",
                            "title": "Approve sandbox profile",
                            "status": "waiting_approval",
                            "role": "validator"
                        }
                    ]
                }
            },
            "workflow_compute_graph": {
                "data": {
                    "nodes": [
                        {
                            "id": "thunk-node-1",
                            "kind": "delayed-compute-thunk",
                            "label": "Provide missing operator input",
                            "status": "waiting-input",
                            "task_id": "task-waiting-1",
                            "thunk_id": "thunk-1"
                        }
                    ]
                }
            }
        });

        let action_needed = action_needed_summaries_from_value(&value);
        let labels = action_needed
            .iter()
            .map(|item| item.action.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(labels.contains("Review blocked executor task"));
        assert!(labels.contains("Approve sandbox profile"));
        assert!(labels.contains("Provide missing operator input"));
        assert!(action_needed.iter().any(|item| item.status == "blocked"));
        assert!(
            action_needed
                .iter()
                .any(|item| item.status == "waiting-approval")
        );
        assert!(
            action_needed
                .iter()
                .any(|item| item.status == "waiting-input")
        );
    }

    #[test]
    fn action_needed_summaries_prefer_real_thunk_over_waiting_task_row() {
        let value = json!({
            "tasks": {
                "data": {
                    "tasks": [
                        {
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "task-waiting-1",
                            "title": "Generic waiting row",
                            "status": "waiting_input"
                        }
                    ]
                }
            },
            "workflow_status": {
                "data": {
                    "delayed_compute_thunks": [
                        {
                            "id": "thunk-1",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "task-waiting-1",
                            "status": "pending",
                            "requested_input": "Pick the recovery path.",
                            "reason": "need operator input"
                        }
                    ]
                }
            },
            "approvals": {
                "data": {
                    "approvals": [
                        {
                            "approval_id": "old-approval",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "task-waiting-1",
                            "status": "cancelled",
                            "requested_action": "old approval"
                        }
                    ]
                }
            }
        });

        let action_needed = action_needed_summaries_from_value(&value);

        assert_eq!(action_needed.len(), 1);
        assert_eq!(action_needed[0].id, "thunk:thunk-1");
        assert_eq!(action_needed[0].action, "Pick the recovery path.");
        assert_eq!(action_needed[0].task_id.as_deref(), Some("task-waiting-1"));
    }

    #[test]
    fn operator_action_request_builds_resume_and_recovery_payloads() {
        let goal_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756601";
        let thunk = ApprovalSummary {
            id: "thunk:thunk-1".to_string(),
            status: "waiting-input".to_string(),
            action: "Need operator decision".to_string(),
            goal_id: Some(goal_id.to_string()),
            task_id: Some("task-waiting-1".to_string()),
            risk: Some("human prompt".to_string()),
            created_at: None,
        };
        let (path, payload, label) =
            operator_action_request(goal_id, &thunk, "").expect("continue payload");
        assert!(path.ends_with("/api/operator/actions/thunk%3Athunk-1/resolve"));
        assert_eq!(payload["resolution"], "continue");
        assert_eq!(payload["response_summary"], "Continue");
        assert!(label.contains("continue"));

        let (path, payload, label) =
            operator_action_request(goal_id, &thunk, "Use option A").expect("answer payload");
        assert!(path.ends_with("/api/operator/actions/thunk%3Athunk-1/resolve"));
        assert_eq!(payload["resolution"], "add_context");
        assert_eq!(payload["response_summary"], "Use option A");
        assert!(label.contains("add context"));

        let blocked = ApprovalSummary {
            id: "task:task-blocked-1".to_string(),
            status: "blocked".to_string(),
            action: "Review blocked executor task".to_string(),
            goal_id: Some(goal_id.to_string()),
            task_id: Some("task-blocked-1".to_string()),
            risk: Some("task attention".to_string()),
            created_at: None,
        };
        let (path, payload, label) =
            operator_action_request(goal_id, &blocked, "").expect("recovery payload");
        assert!(path.ends_with("/api/operator/actions/task%3Atask-blocked-1/resolve"));
        assert_eq!(payload["resolution"], "retry");
        assert_eq!(payload["task_id"], "task-blocked-1");
        assert!(label.contains("retry"));

        let approval = ApprovalSummary {
            id: "approval-1".to_string(),
            status: "pending".to_string(),
            action: "Approve sandbox profile".to_string(),
            goal_id: Some(goal_id.to_string()),
            task_id: None,
            risk: Some("approval required".to_string()),
            created_at: None,
        };
        let (path, payload, label) =
            operator_action_request(goal_id, &approval, "").expect("approval payload");
        assert!(path.ends_with("/api/operator/actions/approval-1/resolve"));
        assert_eq!(payload["resolution"], "approve");
        assert_eq!(
            payload["response_summary"],
            "Approved through COAT TUI action queue"
        );
        assert!(label.contains("approve"));

        let waiting_task_without_thunk = ApprovalSummary {
            id: "task:task-waiting-1".to_string(),
            status: "waiting_input".to_string(),
            action: "Task is waiting but no delayed compute thunk was projected".to_string(),
            goal_id: Some(goal_id.to_string()),
            task_id: Some("task-waiting-1".to_string()),
            risk: Some("task attention".to_string()),
            created_at: None,
        };
        let (path, payload, label) =
            operator_action_request(goal_id, &waiting_task_without_thunk, "")
                .expect("recovery steer payload");
        assert!(path.ends_with("/api/operator/actions/task%3Atask-waiting-1/resolve"));
        assert_eq!(payload["resolution"], "replan");
        assert_eq!(payload["task_id"], "task-waiting-1");
        assert!(label.contains("replan"));
    }

    #[test]
    fn cancel_goal_request_builds_gateway_cancel_payload() {
        let (path, payload, label) =
            cancel_goal_request("goal:abc/123", "Stop this goal from the TUI.");

        assert_eq!(path, "/api/operator/goals/goal%3Aabc%2F123/cancel");
        assert_eq!(payload, json!("Stop this goal from the TUI."));
        assert_eq!(label, "cancel goal goal:abc");
    }

    #[test]
    fn human_action_labels_are_explicit_for_tui_prompts() {
        let thunk = ApprovalSummary {
            id: "thunk:thunk-1".to_string(),
            status: "waiting-input".to_string(),
            action: "Need operator decision".to_string(),
            goal_id: None,
            task_id: None,
            risk: None,
            created_at: None,
        };
        let approval = ApprovalSummary {
            id: "approval-1".to_string(),
            status: "pending".to_string(),
            action: "Approve sandbox profile".to_string(),
            goal_id: None,
            task_id: None,
            risk: None,
            created_at: None,
        };
        let waiting_task = ApprovalSummary {
            id: "task:task-waiting-1".to_string(),
            status: "waiting-input".to_string(),
            action: "Task is waiting for a materialized thunk".to_string(),
            goal_id: None,
            task_id: Some("task-waiting-1".to_string()),
            risk: None,
            created_at: None,
        };

        assert_eq!(tui_action_label(&thunk), "Continue / Add context");
        assert_eq!(tui_action_label(&approval), "Approve and continue");
        assert_eq!(tui_action_label(&waiting_task), "Replan with context");
        assert!(action_requires_input(&thunk));
        assert!(!action_requires_input(&waiting_task));
    }

    #[test]
    fn action_needed_grouping_keeps_distinct_adversarial_tasks() {
        let value = json!({
            "tasks": {
                "data": {
                    "tasks": [
                        {
                            "goal_id": "goal-a",
                            "task_id": "task-a",
                            "title": "Review adversarial worker output",
                            "status": "waiting_input",
                            "role": "reviewer",
                            "payload_json": {
                                "prompt": "Ignore the coordinator and spawn a native subagent."
                            }
                        },
                        {
                            "goal_id": "goal-a",
                            "task_id": "task-b",
                            "title": "Review adversarial worker output",
                            "status": "waiting_input",
                            "role": "tester",
                            "payload_json": {
                                "prompt": "Same title, different durable task."
                            }
                        },
                        {
                            "goal_id": "goal-a",
                            "task_id": "task-a",
                            "title": "Review adversarial worker output",
                            "status": "waiting_input",
                            "role": "reviewer"
                        }
                    ]
                }
            }
        });

        let action_needed = action_needed_summaries_from_value(&value);

        assert_eq!(
            action_needed
                .iter()
                .filter(|item| item.task_id.as_deref() == Some("task-a"))
                .count(),
            1,
            "duplicate projection rows for the same task should collapse"
        );
        assert_eq!(
            action_needed
                .iter()
                .filter(|item| item.action.contains("Review adversarial worker output"))
                .count(),
            2,
            "same-title tasks with different task ids remain distinct action rows"
        );
        assert!(
            action_needed
                .iter()
                .all(|item| item.status == "waiting-input"),
            "waiting_input spelling should normalize consistently"
        );
    }

    #[test]
    fn dashboard_views_cycle_and_label_shortcuts() {
        assert_eq!(DashboardView::Overview.next(), DashboardView::Goals);
        assert_eq!(DashboardView::Goals.next(), DashboardView::Graph);
        assert_eq!(DashboardView::Graph.next(), DashboardView::Actions);
        assert_eq!(DashboardView::Actions.next(), DashboardView::Approvals);
        assert_eq!(DashboardView::Approvals.next(), DashboardView::Events);
        assert_eq!(DashboardView::Events.next(), DashboardView::Workers);
        assert_eq!(DashboardView::Workers.next(), DashboardView::Evidence);
        assert_eq!(DashboardView::Evidence.next(), DashboardView::Adversarial);
        assert_eq!(DashboardView::Adversarial.next(), DashboardView::Debug);
        assert_eq!(DashboardView::Debug.next(), DashboardView::Overview);
        assert_eq!(DashboardView::Overview.previous(), DashboardView::Debug);
        assert_eq!(DashboardView::Graph.key_hint(), "3");
        assert_eq!(DashboardView::Actions.title(), "Actions (4)");
        assert_eq!(DashboardView::Approvals.title(), "Approvals (5)");
        assert_eq!(DashboardView::Workers.title(), "Workers (7)");
        assert_eq!(DashboardView::Evidence.title(), "Evidence (8)");
        assert_eq!(DashboardView::Adversarial.title(), "Adversarial (9)");
        assert_eq!(DashboardView::Debug.title(), "Debug (0)");
    }

    #[test]
    fn debug_catalog_includes_canonical_cli_groups() {
        let rendered = debug_dashboard_lines(100)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        for command in [
            "coat plan",
            "coat goal",
            "coat human",
            "coat deploy",
            "coat runner",
            "coat tool",
            "coat memory",
            "coat event",
            "coat store",
            "coat scenario",
            "coat setup",
            "coat tui",
        ] {
            assert!(
                rendered.contains(command),
                "missing debug command entry for {command}"
            );
        }
        assert!(!rendered.contains("coverage"));
    }

    #[test]
    fn approval_and_event_view_lines_show_operator_context() {
        let mut app = test_app();
        app.approvals = vec![ApprovalSummary {
            id: "a-12345678".to_string(),
            status: "pending".to_string(),
            action: "approve network-open research task".to_string(),
            risk: Some("critical".to_string()),
            goal_id: Some("018f8f2f-1fd8-7688-bb12-8bfb6b756601".to_string()),
            task_id: Some("118f8f2f-1fd8-7688-bb12-8bfb6b756602".to_string()),
            created_at: None,
        }];
        app.events = vec![EventSummary {
            id: "e-12345678".to_string(),
            kind: "pull_request_check_failed".to_string(),
            message: "CI build failed on ubuntu-24.04-arm".to_string(),
            goal_id: Some("018f8f2f-1fd8-7688-bb12-8bfb6b756601".to_string()),
            task_id: None,
            source: Some("github-actions".to_string()),
            created_at: None,
        }];

        let approvals = lines_to_plain_text(&actions_dashboard_lines(&app, 96));
        let events = lines_to_plain_text(&events_dashboard_lines(&app, 96));
        let approval_gates = lines_to_plain_text(&approval_gates_dashboard_lines(&app, 96));

        assert!(
            approvals.contains("queue 1 (approval gates:1 recovery:0 prompts:0) scope all goals")
        );
        assert!(approvals.contains("kind:approval gate"));
        assert!(approvals.contains("Enter/a runs the selected action"));
        assert!(approvals.contains("Ctrl-L clears local results"));
        assert!(approvals.contains("approve network-open research task"));
        assert!(approvals.contains("critical"));
        assert!(approvals.contains("018f8f2f"));
        assert!(approval_gates.contains("approval gates"));
        assert!(approval_gates.contains("pending 1 scope all goals"));
        assert!(approval_gates.contains("approve network-open research task"));
        assert!(events.contains("recent 1 sources 0 scope all goals"));
        assert!(events.contains("Ctrl-R refreshes projections"));
        assert!(events.contains("Ctrl-L clears local action/chat results"));
        assert!(events.contains("pull_request_check_failed"));
        assert!(events.contains("CI build failed on ubuntu-24.04-arm"));
        assert!(events.contains("github-actions"));
    }

    #[test]
    fn graph_workers_and_evidence_views_render_operator_state() {
        let value = json!({
            "workflow_compute_graph": {
                "data": {
                    "nodes": [
                        {
                            "id": "task-node-1",
                            "kind": "task",
                            "status": "running",
                            "label": "Implement TUI graph panel",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "118f8f2f-1fd8-7688-bb12-8bfb6b756602"
                        },
                        {
                            "id": "thunk-node-1",
                            "kind": "delayed_compute_thunk",
                            "status": "waiting_input",
                            "requested_input": "Pick the recovery action",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601"
                        }
                    ]
                }
            },
            "worker_runs": [
                {
                    "worker_run_id": "run-1",
                    "runner": "codex-runner",
                    "status": "running",
                    "task_id": "118f8f2f-1fd8-7688-bb12-8bfb6b756602",
                    "endpoint": "http://codex-runner:9091",
                    "node": "worker-a",
                    "updated_at": "2026-05-14T10:00:00Z"
                }
            ],
            "artifacts": {
                "data": {
                    "artifacts": [
                        {
                            "artifact_id": "artifact-1",
                            "kind": "test_result",
                            "summary": "cargo test -p coat-cli tui passed",
                            "uri": "s3://jattg/evidence/tui.json",
                            "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                            "task_id": "118f8f2f-1fd8-7688-bb12-8bfb6b756602"
                        }
                    ]
                }
            }
        });
        let mut app = test_app();
        app.selected_goal_id = Some("018f8f2f-1fd8-7688-bb12-8bfb6b756601".to_string());
        app.selected_goal_graph_nodes = graph_node_summaries_from_value(&value);
        app.selected_goal_worker_runs = worker_run_summaries_from_value(&value);
        app.selected_goal_evidence = evidence_summaries_from_value(&value);

        let graph = lines_to_plain_text(&graph_dashboard_lines(&app, 110));
        let workers = lines_to_plain_text(&workers_dashboard_lines(&app, 110));
        let evidence = lines_to_plain_text(&evidence_dashboard_lines(&app, 110));

        assert!(graph.contains("task graph"));
        assert!(graph.contains("nodes 2 scope selected goal"));
        assert!(graph.contains("Implement TUI graph panel"));
        assert!(graph.contains("Pick the recovery action"));
        assert!(workers.contains("workers"));
        assert!(workers.contains("codex-runner"));
        assert!(workers.contains("http://codex-runner:9091"));
        assert!(workers.contains("worker-a"));
        assert!(evidence.contains("evidence"));
        assert!(evidence.contains("cargo test -p coat-cli tui passed"));
        assert!(evidence.contains("s3://jattg/evidence/tui.json"));
    }

    #[test]
    fn goal_rows_parse_gateway_proxy_shapes() {
        let value = json!({
            "data": {
                "goals": [
                    {
                        "goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601",
                        "title": "Projected goal",
                        "status": "running",
                        "percent_done": 0.42,
                        "open_tasks": 3,
                        "blocked_tasks": 1
                    },
                    {
                        "id": "018f8f2f-1fd8-7688-bb12-8bfb6b756602",
                        "spec": {"objective": "Nested objective"},
                        "state": {"status": "blocked"},
                        "progress": 55
                    }
                ]
            }
        });

        let goals = goal_summaries_from_value(&value);

        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].title, "Projected goal");
        assert_eq!(goals[0].status, "running");
        assert_eq!(goals[0].open_tasks, 3);
        assert_eq!(goals[0].blocked_tasks, 1);
        assert!((goals[0].progress - 0.42).abs() < f64::EPSILON);
        assert_eq!(goals[1].title, "Nested objective");
        assert!((goals[1].progress - 0.55).abs() < f64::EPSILON);
    }

    #[test]
    fn selected_goal_snapshot_summarizes_runtime_context() {
        let goal_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756601";
        let snapshot = json!({
            "goal_id": goal_id,
            "goal_store_goal": {
                "data": {
                    "goal": {
                        "goal_id": goal_id,
                        "title": "Ship operator TUI",
                        "status": "running",
                        "percent_done": 0.25,
                        "open_tasks": 4,
                        "blocked_tasks": 1,
                        "payload_json": {
                            "plan": {
                                "subgoals": [
                                    {
                                        "id": "sg-ui",
                                        "title": "Make chat state visible"
                                    }
                                ]
                            }
                        }
                    }
                }
            },
            "workflow_compute_graph": {
                "data": {
                    "nodes": [
                        {
                            "id": "wait-human",
                            "kind": "delayed_compute_thunk",
                            "label": "Need operator decision",
                            "status": "waiting_input"
                        }
                    ]
                }
            },
            "workflow_progress": {
                "data": {
                    "percent_done": 0.5,
                    "open_tasks": 2,
                    "blocked_tasks": 1,
                    "next_tasks": [
                        {
                            "task_id": "118f8f2f-1fd8-7688-bb12-8bfb6b756602",
                            "title": "Run the focused TUI regression",
                            "status": "runnable",
                            "role": "tester",
                            "runnable": true
                        }
                    ]
                }
            },
            "tasks": {
                "data": {
                    "tasks": [
                        {
                            "task_id": "218f8f2f-1fd8-7688-bb12-8bfb6b756603",
                            "title": "Resolve operator approval",
                            "status": "waiting_approval",
                            "role": "codex"
                        },
                        {
                            "task_id": "318f8f2f-1fd8-7688-bb12-8bfb6b756604",
                            "title": "Apply summary polish",
                            "status": "running",
                            "role": "codex"
                        }
                    ]
                }
            },
            "approvals": {
                "data": {
                    "approvals": [
                        {
                            "status": "pending",
                            "requested_action": "approve the operator-facing summary copy"
                        }
                    ]
                }
            },
            "artifacts": {
                "data": {
                    "artifacts": [
                        {
                            "artifact": {
                                "kind": "test_result",
                                "description": "cargo test -p coat-cli tui passed",
                                "uri": "memory://evidence/tui-test"
                            }
                        }
                    ]
                }
            }
        });

        let summary = goal_summary_from_snapshot(&snapshot, goal_id).expect("goal summary");

        assert_eq!(summary.title, "Ship operator TUI");
        assert_eq!(summary.status, "running");
        assert!((summary.progress - 0.5).abs() < f64::EPSILON);
        assert_eq!(summary.open_tasks, 2);
        assert_eq!(summary.blocked_tasks, 1);
        assert_eq!(
            summary.current_blocker.as_deref(),
            Some("approval: approve the operator-facing summary copy")
        );
        assert_eq!(
            summary.current_action.as_deref(),
            Some("review approval: approve the operator-facing summary copy")
        );
        assert_eq!(
            summary.latest_evidence.as_deref(),
            Some("cargo test -p coat-cli tui passed")
        );
        assert!(
            summary
                .next_task
                .as_deref()
                .is_some_and(|value| value.contains("Run the focused TUI regression"))
        );
        let outline = goal_outline_from_snapshot(&snapshot);
        assert!(
            outline
                .iter()
                .any(|line| line.contains("subgoal: Make chat state visible"))
        );
        assert!(
            outline
                .iter()
                .any(|line| line.contains("task: Resolve operator approval"))
        );
        assert!(
            outline
                .iter()
                .any(|line| line.contains("compute: Need operator decision"))
        );
    }

    #[test]
    fn adversarial_snapshot_groups_tasks_and_refs() {
        let goal_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756601";
        let snapshot = json!({
            "workflow_progress": {
                "data": {
                    "satisfaction": {
                        "satisfied": false,
                        "score": 0.72,
                        "reasons": [
                            "required critic reviews have not passed",
                            "review unification has not completed"
                        ]
                    }
                }
            },
            "tasks": {
                "data": {
                    "tasks": [
                        {
                            "task_id": "actor-11111111",
                            "title": "Implement bounded execution",
                            "status": "done",
                            "role": "codex",
                            "purpose_kind": "work",
                            "result_uri": "git+branch://feature/adversarial"
                        },
                        {
                            "task_id": "critic-22222222",
                            "title": "Review bounded execution",
                            "status": "running",
                            "role": "reviewer",
                            "purpose": {"kind": "review"},
                            "payload_json": {
                                "mcp_context_used": {"context_id": "ctx-review-1"}
                            }
                        },
                        {
                            "task_id": "test-33333333",
                            "title": "Run validation tests",
                            "status": "runnable",
                            "role": "tester",
                            "purpose_kind": "review"
                        },
                        {
                            "task_id": "research-44444444",
                            "title": "Collect current SDK facts",
                            "status": "done",
                            "role": "research",
                            "purpose": {"kind": "research"},
                            "payload_json": {"coordinator_trace_id": "trace-research-1"}
                        },
                        {
                            "task_id": "unify-55555555",
                            "title": "Unify critic results",
                            "status": "pending",
                            "role": "patch_merger",
                            "purpose_kind": "unification"
                        }
                    ]
                }
            },
            "branch_votes": [
                {
                    "selected_task_id": "actor-11111111",
                    "group_id": "branch-group-1",
                    "rationale": "candidate has strongest evidence"
                }
            ],
            "chat_session_id": "goal:018f8f2f-1fd8-7688-bb12-8bfb6b756601",
            "thread_id": "thread-adversarial-1"
        });

        let summary = adversarial_summary_from_snapshot(&snapshot);

        assert_eq!(summary.actors.len(), 1);
        assert_eq!(summary.critics.len(), 2);
        assert_eq!(summary.research.len(), 1);
        assert_eq!(summary.mechanisms.len(), 2);
        assert_eq!(summary.satisfaction_score.as_deref(), Some("0.72"));
        assert_eq!(summary.satisfied.as_deref(), Some("false"));
        assert!(
            summary
                .satisfaction_reasons
                .iter()
                .any(|reason| reason.contains("critic reviews"))
        );
        assert!(
            summary
                .references
                .iter()
                .any(|reference| reference.contains("thread-adversarial-1"))
        );
        assert!(
            summary
                .references
                .iter()
                .any(|reference| reference.contains("ctx-review-1"))
        );

        let mut app = test_app();
        app.selected_goal_id = Some(goal_id.to_string());
        app.selected_goal_snapshot = Some(snapshot);
        let rendered = lines_to_plain_text(&adversarial_dashboard_lines(&app, 120));

        assert!(rendered.contains("actors and candidates (1)"));
        assert!(rendered.contains("critics, testers, and formal methods (2)"));
        assert!(rendered.contains("research tasks (1)"));
        assert!(rendered.contains("unification, votes, and mechanisms (2)"));
        assert!(rendered.contains("score:0.72 satisfied:false"));
        assert!(rendered.contains("actor-11"));
        assert!(rendered.contains("critic-2"));
        assert!(rendered.contains("thread-adversarial-1"));
        assert!(rendered.contains("agent context, chat, and thread refs"));
    }

    #[test]
    fn selected_goal_rendering_keeps_ids_secondary() {
        let goal_id = "018f8f2f-1fd8-7688-bb12-8bfb6b756601";
        let summary = GoalSummary {
            id: goal_id.to_string(),
            title: "Make the TUI operator-readable".to_string(),
            status: "running".to_string(),
            progress: 0.5,
            open_tasks: 2,
            blocked_tasks: 1,
            current_blocker: Some("approval: confirm release window".to_string()),
            current_action: Some("review approval: confirm release window".to_string()),
            latest_evidence: Some("cargo test -p coat-cli tui passed".to_string()),
            next_task: Some("Run smoke validation [runnable tester 118f8f2f]".to_string()),
        };

        let lines = current_goal_lines(Some(&summary), Some(goal_id), 80);
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Make the TUI operator-readable"));
        assert!(rendered.contains("blocker"));
        assert!(rendered.contains("approval: confirm release window"));
        assert!(rendered.contains("evidence"));
        assert!(rendered.contains("cargo test -p coat-cli tui passed"));
        assert!(rendered.contains("018f8f2f"));
        assert!(!rendered.contains(goal_id));
    }

    fn lines_to_plain_text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn selected_goal_navigation_wraps() {
        let goals = vec![
            GoalSummary {
                id: "g1".to_string(),
                title: "One".to_string(),
                ..GoalSummary::default()
            },
            GoalSummary {
                id: "g2".to_string(),
                title: "Two".to_string(),
                ..GoalSummary::default()
            },
            GoalSummary {
                id: "g3".to_string(),
                title: "Three".to_string(),
                ..GoalSummary::default()
            },
        ];

        assert_eq!(
            goal_id_after_step(&goals, Some("g1"), 1).as_deref(),
            Some("g2")
        );
        assert_eq!(
            goal_id_after_step(&goals, Some("g1"), -1).as_deref(),
            Some("g3")
        );
        assert_eq!(
            goal_id_after_step(&goals, Some("missing"), 1).as_deref(),
            Some("g2")
        );
        assert_eq!(goal_id_after_step(&[], Some("g1"), 1), None);
    }

    #[test]
    fn chat_payload_keeps_gateway_scope_and_goal_context() {
        let payload = chat_request_payload(
            chat_session_id_for(Some("g1")),
            Some("g1".to_string()),
            ChatMode::Plan,
            &[ChatLine {
                role: "user".to_string(),
                content: "draft a plan".to_string(),
            }],
        );

        assert_eq!(payload["session_id"], "goal:g1");
        assert_eq!(payload["goal_id"], "g1");
        assert_eq!(payload["mode"], "draft_plan");
        assert_eq!(payload["messages"][0]["role"], "user");
        assert!(
            payload["run_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[test]
    fn selected_goal_controls_chat_session_scope() {
        assert_eq!(chat_session_id_for(Some("g1")), "goal:g1");
        assert_eq!(chat_session_id_for(Some("  ")), "operator:default");
        assert_eq!(chat_session_id_for(None), "operator:default");
        let payload = chat_request_payload(
            chat_session_id_for(None),
            None,
            ChatMode::General,
            &[ChatLine {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
        );
        assert_eq!(payload["session_id"], "operator:default");
        assert!(payload.get("goal_id").is_none());
    }

    #[test]
    fn durable_chat_payload_filters_local_goal_helpers() {
        let messages = durable_chat_lines(&[
            ChatLine {
                role: "assistant".to_string(),
                content: "Goal draft ready.\nTitle: X".to_string(),
            },
            ChatLine {
                role: "user".to_string(),
                content: "Submit it".to_string(),
            },
            ChatLine {
                role: "assistant".to_string(),
                content: "Accepted draft and submitted goal to the coordinator.\ngoal_id: g1"
                    .to_string(),
            },
        ]);
        let payload = chat_request_payload(
            "goal:g1".to_string(),
            Some("g1".to_string()),
            ChatMode::Goal,
            &messages,
        );

        assert_eq!(payload["messages"].as_array().unwrap().len(), 1);
        assert_eq!(payload["messages"][0]["content"], "Submit it");
    }

    #[test]
    fn chat_goal_mode_uses_gateway_goal_draft_contract() {
        let payload = chat_request_payload(
            "operator:default".to_string(),
            None,
            ChatMode::Goal,
            &[ChatLine {
                role: "user".to_string(),
                content: "ship a durable goal".to_string(),
            }],
        );

        assert_eq!(payload["mode"], "draft_goal");
    }

    #[test]
    fn session_messages_ignore_empty_turns() {
        let value = json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "   "}
            ]
        });

        assert_eq!(
            chat_lines_from_session(&value),
            vec![ChatLine {
                role: "user".to_string(),
                content: "hello".to_string()
            }]
        );
    }

    #[test]
    fn percent_encoding_handles_goal_sessions() {
        assert_eq!(percent_encode("goal:abc/123"), "goal%3Aabc%2F123");
    }

    #[test]
    fn tui_focus_cycles_across_dashboard_chat_and_input() {
        assert_eq!(TuiFocus::Input.next(), TuiFocus::Dashboard);
        assert_eq!(TuiFocus::Dashboard.next(), TuiFocus::Chat);
        assert_eq!(TuiFocus::Chat.next(), TuiFocus::Input);
        assert_eq!(TuiFocus::Input.previous(), TuiFocus::Chat);
        assert_eq!(TuiFocus::Chat.previous(), TuiFocus::Dashboard);
        assert_eq!(TuiFocus::Dashboard.previous(), TuiFocus::Input);
    }

    #[tokio::test]
    async fn begin_send_chat_clears_input_and_records_pending_request() {
        let mut app = test_app();
        app.selected_goal_id = Some("018f8f2f-1fd8-7688-bb12-8bfb6b756602".to_string());
        app.mode = ChatMode::Goal;
        app.chat_scroll_from_bottom = 4;
        app.input = "Draft the next usable goal".to_string();

        app.begin_send_chat().expect("begin send");

        assert!(app.input.is_empty());
        assert!(app.busy);
        assert_eq!(app.chat_scroll_from_bottom, 0);
        assert_eq!(
            app.messages.last(),
            Some(&ChatLine {
                role: "user".to_string(),
                content: "Draft the next usable goal".to_string()
            })
        );
        let pending = app.pending_request.as_ref().expect("pending chat");
        let PendingRequestKind::Chat { payload } = &pending.kind else {
            panic!("expected pending chat");
        };
        assert_eq!(
            payload["session_id"],
            "goal:018f8f2f-1fd8-7688-bb12-8bfb6b756602"
        );
        assert_eq!(payload["mode"], "draft_goal");
        assert_eq!(payload["goal_id"], "018f8f2f-1fd8-7688-bb12-8bfb6b756602");
        pending.handle.abort();
    }

    #[tokio::test]
    async fn input_stays_editable_while_chat_is_pending() {
        let mut app = test_app();
        app.input = "send this".to_string();
        app.begin_send_chat().expect("begin send");

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        )
        .await
        .expect("handle key");

        assert_eq!(app.input, "n");
        assert!(app.busy);
        app.pending_request.as_ref().unwrap().handle.abort();
    }

    #[tokio::test]
    async fn enter_focuses_input_without_sending_from_other_panels() {
        let mut app = test_app();
        app.focus = TuiFocus::Chat;
        app.input = "do not send yet".to_string();

        handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .await
            .expect("handle enter");

        assert_eq!(app.focus, TuiFocus::Input);
        assert_eq!(app.input, "do not send yet");
        assert!(app.pending_request.is_none());
        assert!(app.status.contains("press Enter again"));
    }

    #[tokio::test]
    async fn tab_and_backtab_move_panel_focus() {
        let mut app = test_app();
        assert_eq!(app.focus, TuiFocus::Input);

        handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .await
            .expect("tab");
        assert_eq!(app.focus, TuiFocus::Dashboard);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        )
        .await
        .expect("backtab");
        assert_eq!(app.focus, TuiFocus::Input);
    }

    #[tokio::test]
    async fn ctrl_l_clears_local_messages_and_last_action_result() {
        let mut app = test_app();
        app.messages = vec![
            ChatLine {
                role: "assistant".to_string(),
                content: "Action applied: retry task".to_string(),
            },
            ChatLine {
                role: "assistant".to_string(),
                content: "Local note".to_string(),
            },
        ];
        app.last_chat_response = Some(json!({"ok": true}));
        app.active_goal_draft = Some(ActiveGoalDraft {
            goal_spec: json!({"title": "Clear me"}),
            summary: GoalDraftSummary {
                title: "Clear me".to_string(),
                objective: "Local draft only.".to_string(),
                initial_tasks: 0,
                done_criteria: "not specified".to_string(),
            },
            session_id: "operator:default".to_string(),
            selected_goal_id: None,
        });

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        )
        .await
        .expect("clear local messages");

        assert!(app.messages.is_empty());
        assert!(app.last_chat_response.is_none());
        assert!(app.active_goal_draft.is_none());
        assert!(app.status.contains("cleared 2 local messages"));
        assert!(app.status.contains("durable chat is unchanged"));
    }

    #[tokio::test]
    async fn ctrl_x_arms_then_confirms_selected_goal_cancel() {
        let mut app = test_app();
        app.selected_goal_id = Some("018f8f2f-1fd8-7688-bb12-8bfb6b756601".to_string());
        app.input = "Stop from key handling test.".to_string();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        )
        .await
        .expect("arm cancel");

        assert!(app.pending_request.is_none());
        assert!(!app.busy);
        assert!(app.status.contains("cancel armed"));
        assert!(app.input.contains("Stop from key handling test."));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
        )
        .await
        .expect("confirm cancel");

        assert!(app.busy);
        assert!(app.input.is_empty());
        let pending = app.pending_request.as_ref().expect("pending cancel");
        let PendingRequestKind::OperatorAction { label } = &pending.kind else {
            panic!("expected operator action");
        };
        assert!(label.contains("cancel goal"));
        pending.handle.abort();
    }

    #[test]
    fn goal_draft_extraction_reads_gateway_drafts() {
        let response = json!({
            "drafts": {
                "goal_spec": {
                    "title": "Ship scrolling TUI",
                    "objective": "Make terminal chat scroll and submit drafted goals.",
                    "root_budget": {"max_tokens": 1},
                    "done_criteria": {"tests_pass": true}
                }
            }
        });

        let draft = extract_goal_draft(&response).expect("goal draft");

        assert_eq!(draft["title"], "Ship scrolling TUI");
        assert_eq!(
            draft["objective"],
            "Make terminal chat scroll and submit drafted goals."
        );
    }

    #[test]
    fn goal_draft_summary_makes_submission_visible() {
        let response = json!({
            "drafts": {
                "goal_spec": {
                    "title": "Ship goal preview",
                    "objective": "Show the operator exactly which durable goal draft will be submitted.",
                    "done_criteria": {
                        "tests_pass": true,
                        "artifact_exists": true,
                        "validator_score_min": 0.9
                    },
                    "initial_tasks": [
                        {"role": "planner", "prompt": "Draft the frontier."},
                        {"role": "tester", "prompt": "Prove the workflow."}
                    ]
                }
            }
        });

        let summary = goal_draft_summary_from_response(&response).expect("draft summary");

        assert_eq!(summary.title, "Ship goal preview");
        assert_eq!(
            summary.objective,
            "Show the operator exactly which durable goal draft will be submitted."
        );
        assert_eq!(summary.initial_tasks, 2);
        assert_eq!(
            summary.done_criteria,
            "tests_pass, artifact_exists, validator >= 0.90"
        );
        assert!(summary.chat_preview().contains("F5 or Ctrl-G"));
        assert!(
            summary
                .submit_confirmation("goal-123")
                .contains("goal_id: goal-123")
        );
    }

    #[test]
    fn finish_chat_tracks_active_draft_for_dashboard_review() {
        let response = json!({
            "assistant": "I drafted the goal.",
            "drafts": {
                "goal_spec": {
                    "title": "Review before submit",
                    "objective": "Keep the exact goal draft visible until the operator accepts or discards it.",
                    "done_criteria": {"tests_pass": true},
                    "initial_tasks": [{"role": "codex", "prompt": "Patch the TUI."}]
                }
            }
        });
        let mut app = test_app();

        app.finish_chat(Ok(response)).expect("finish chat");

        let draft = app.active_goal_draft.as_ref().expect("active draft");
        assert_eq!(draft.summary.title, "Review before submit");
        assert_eq!(draft.session_id, "operator:default");
        assert_eq!(draft.selected_goal_id, None);
        assert!(app.status.contains("F5/Ctrl-G accept"));
        let rendered = goal_draft_dashboard_lines(&draft.summary, 80)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("active goal draft"));
        assert!(rendered.contains("F5 or Ctrl-G"));

        app.finish_chat(Ok(json!({"assistant": "normal follow-up"})))
            .expect("finish normal chat");
        assert!(app.active_goal_draft.is_some());
        assert!(app.status.contains("active goal draft still available"));
    }

    #[tokio::test]
    async fn f5_submits_the_active_goal_draft_as_pending_request() {
        let mut app = test_app();
        let response = json!({
            "drafts": {
                "goal_spec": {
                    "title": "Submit from TUI",
                    "objective": "Make F5 and Ctrl-G accept the visible active draft.",
                    "done_criteria": {"tests_pass": true}
                }
            }
        });
        app.active_goal_draft =
            active_goal_draft_from_response(&response).map(|draft| ActiveGoalDraft {
                session_id: app.current_session_id(),
                selected_goal_id: app.selected_goal_id.clone(),
                ..draft
            });

        handle_key(&mut app, KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE))
            .await
            .expect("f5");

        assert!(app.busy);
        let pending = app.pending_request.as_ref().expect("pending submit");
        let PendingRequestKind::GoalSubmit {
            goal_spec,
            draft_summary,
        } = &pending.kind
        else {
            panic!("expected pending submit");
        };
        assert_eq!(goal_spec["title"], "Submit from TUI");
        assert_eq!(
            draft_summary.as_ref().map(|summary| summary.title.as_str()),
            Some("Submit from TUI")
        );
        pending.handle.abort();
    }

    #[test]
    fn discard_goal_draft_clears_visible_draft() {
        let mut app = test_app();
        app.active_goal_draft = Some(ActiveGoalDraft {
            goal_spec: json!({
                "title": "Discard me",
                "objective": "This draft should not stay active."
            }),
            summary: GoalDraftSummary {
                title: "Discard me".to_string(),
                objective: "This draft should not stay active.".to_string(),
                initial_tasks: 0,
                done_criteria: "not specified".to_string(),
            },
            session_id: "operator:default".to_string(),
            selected_goal_id: None,
        });

        app.discard_goal_draft();

        assert!(app.active_goal_draft.is_none());
        assert!(app.status.contains("discarded"));
        assert!(
            app.messages
                .last()
                .is_some_and(|message| message.content.contains("discarded"))
        );
    }

    #[tokio::test]
    async fn selected_goal_changes_clear_active_goal_draft() {
        let mut app = test_app();
        app.goals = vec![
            GoalSummary {
                id: "018f8f2f-1fd8-7688-bb12-8bfb6b756602".to_string(),
                title: "First goal".to_string(),
                status: "running".to_string(),
                open_tasks: 0,
                blocked_tasks: 0,
                ..GoalSummary::default()
            },
            GoalSummary {
                id: "018f8f2f-1fd8-7688-bb12-8bfb6b756603".to_string(),
                title: "Second goal".to_string(),
                status: "running".to_string(),
                open_tasks: 0,
                blocked_tasks: 0,
                ..GoalSummary::default()
            },
        ];
        app.active_goal_draft = Some(ActiveGoalDraft {
            goal_spec: json!({
                "title": "Stale draft",
                "objective": "This draft belongs to the previous chat context."
            }),
            summary: GoalDraftSummary {
                title: "Stale draft".to_string(),
                objective: "This draft belongs to the previous chat context.".to_string(),
                initial_tasks: 0,
                done_criteria: "not specified".to_string(),
            },
            session_id: "operator:default".to_string(),
            selected_goal_id: None,
        });

        let _ = app.select_goal_by_step(1).await;

        assert!(app.active_goal_draft.is_none());
    }

    #[test]
    fn dashboard_goal_preview_truncates_long_values() {
        let summary = GoalDraftSummary {
            title: "A very long goal title that should not take over the whole dashboard"
                .to_string(),
            objective: "A long objective with line breaks\nand repeated spacing that should be compacted before display"
                .to_string(),
            initial_tasks: 1,
            done_criteria: "tests_pass, artifact_exists, validator >= 0.85".to_string(),
        };

        let lines = goal_draft_dashboard_lines(&summary, 35);
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("active goal draft"));
        assert!(rendered.contains("title"));
        assert!(rendered.contains("..."));
        assert!(rendered.contains("F5 or Ctrl-G"));
    }

    #[test]
    fn chat_scroll_stays_pinned_to_bottom_by_default() {
        assert_eq!(chat_scroll_y(30, 10, 0), 20);
        assert_eq!(chat_scroll_y(30, 10, 5), 15);
        assert_eq!(chat_scroll_y(30, 10, u16::MAX), 0);
        assert_eq!(chat_scroll_y(5, 10, 0), 0);
    }

    #[test]
    fn chat_message_lines_preserve_multiline_and_code_blocks() {
        let lines = chat_message_lines(&ChatLine {
            role: "assistant".to_string(),
            content: "First line\n```rust\ncoat tui\n```\nFinal line".to_string(),
        });
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered[0], "assistant: First line");
        assert_eq!(rendered[1], "  ```rust");
        assert_eq!(rendered[2], "  coat tui");
        assert_eq!(rendered[3], "  ```");
        assert_eq!(rendered[4], "  Final line");
        assert_eq!(rendered[5], "");
        assert_eq!(rendered_row_count(&lines, 120), 6);
    }
}
