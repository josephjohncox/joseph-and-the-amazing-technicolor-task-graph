//! Terminal operator UI for COAT.
//!
//! This module intentionally mirrors the TypeScript control gateway instead of
//! bypassing it. The TUI reads `/api/overview`, `/api/config`, and
//! `/api/chat*` from the gateway, so terminal chat remains an operator surface
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
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde_json::{Value, json};
use uuid::Uuid;

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

impl GoalDraftSummary {
    fn chat_preview(&self) -> String {
        format!(
            "Goal draft ready for submission.\nTitle: {}\nObjective: {}\nInitial tasks: {}\nDone criteria: {}\nSubmit this exact draft with F5 or Ctrl-G.",
            self.title, self.objective, self.initial_tasks, self.done_criteria
        )
    }

    fn submit_confirmation(&self, goal_id: &str) -> String {
        format!(
            "Submitted drafted goal to the coordinator.\ngoal_id: {goal_id}\ntitle: {}\nobjective: {}\ninitial_tasks: {}\ndone_criteria: {}",
            self.title, self.objective, self.initial_tasks, self.done_criteria
        )
    }
}

struct App {
    config: TuiConfig,
    client: reqwest::Client,
    dashboard: DashboardSummary,
    goals: Vec<GoalSummary>,
    selected_goal_id: Option<String>,
    messages: Vec<ChatLine>,
    last_chat_response: Option<Value>,
    chat_scroll_from_bottom: u16,
    input: String,
    status: String,
    mode: ChatMode,
    last_refresh: Option<Instant>,
    busy: bool,
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
            selected_goal_id,
            messages: Vec::new(),
            last_chat_response: None,
            chat_scroll_from_bottom: 0,
            input: String::new(),
            status: "starting".to_string(),
            mode: ChatMode::General,
            last_refresh: None,
            busy: false,
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
        let config = self.get_json("/api/config").await?;
        let overview = self.get_json("/api/overview").await?;
        let goals = self.get_json("/api/goals?limit=100").await?;
        self.goals = goal_summaries_from_value(&goals);
        if self.goals.is_empty() {
            self.goals = goal_summaries_from_value(&overview);
        }
        let mut status = "dashboard refreshed".to_string();
        if let Err(error) = self.refresh_selected_goal_summary().await {
            status = format!("dashboard refreshed; selected goal snapshot failed: {error}");
        }
        self.dashboard = dashboard_summary(&config, &overview, &self.goals);
        self.last_refresh = Some(Instant::now());
        self.status = status;
        Ok(())
    }

    async fn refresh_selected_goal_summary(&mut self) -> anyhow::Result<()> {
        let Some(goal_id) = self.selected_goal_id.clone() else {
            return Ok(());
        };
        let path = format!("/api/goals/{}", percent_encode(&goal_id));
        let snapshot = self.get_json(&path).await?;
        if let Some(summary) = goal_summary_from_snapshot(&snapshot, &goal_id) {
            upsert_goal_summary(&mut self.goals, summary);
        }
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

    async fn send_chat(&mut self) -> anyhow::Result<()> {
        let content = self.input.trim().to_string();
        if content.is_empty() {
            return Ok(());
        }
        self.input.clear();
        self.messages.push(ChatLine {
            role: "user".to_string(),
            content: content.clone(),
        });
        self.status = "calling control gateway chat".to_string();
        self.busy = true;

        let durable_messages = durable_chat_lines(&self.messages);
        let payload = chat_request_payload(
            self.current_session_id(),
            self.selected_goal_id.clone(),
            self.mode,
            &durable_messages,
        );
        let response = self.post_json("/api/chat", &payload).await;
        self.busy = false;

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
                if let Some(summary) = goal_draft_summary_from_response(&value) {
                    self.messages.push(ChatLine {
                        role: "assistant".to_string(),
                        content: summary.chat_preview(),
                    });
                    self.status = format!(
                        "{}; goal draft ready, review dashboard or chat and press F5 to submit",
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

    async fn submit_last_goal_draft(&mut self) -> anyhow::Result<()> {
        let Some(response) = self.last_chat_response.as_ref() else {
            self.status =
                "no chat goal draft is available; switch to goal mode and send a prompt first"
                    .to_string();
            return Ok(());
        };
        let Some(goal_spec) = extract_goal_draft(response) else {
            self.status =
                "last chat response did not include drafts.goal_spec; switch to goal mode first"
                    .to_string();
            return Ok(());
        };
        let draft_summary = goal_draft_summary(&goal_spec);

        self.status = "submitting drafted goal to coordinator".to_string();
        self.busy = true;
        let response = self.post_json("/api/goals/submit", &goal_spec).await;
        self.busy = false;

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
                        format!("Submitted drafted goal to the coordinator.\ngoal_id: {goal_id}")
                    });
                self.messages.push(ChatLine {
                    role: "assistant".to_string(),
                    content,
                });
                self.chat_scroll_from_bottom = 0;
                self.status = format!("goal submitted: {goal_id}");
                if goal_id != "assigned by coordinator" {
                    self.selected_goal_id = Some(goal_id);
                }
                if let Err(error) = self.refresh_dashboard().await {
                    self.status = format!("goal submitted, dashboard refresh failed: {error}");
                } else if let Err(error) = self.load_chat_session().await {
                    self.status = format!("goal submitted, chat reload failed: {error}");
                }
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

    async fn get_json(&self, path: &str) -> anyhow::Result<Value> {
        let response = self
            .auth(self.client.get(self.url(path)))
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        parse_response(response).await
    }

    async fn post_json(&self, path: &str, payload: &Value) -> anyhow::Result<Value> {
        let response = self
            .auth(self.client.post(self.url(path)).json(payload))
            .send()
            .await
            .with_context(|| format!("POST {path}"))?;
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
        self.status = "goal selection cleared".to_string();
        self.load_chat_session().await
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
        if app
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
            app.mode = app.mode.next();
            app.status = format!("chat mode: {}", app.mode.label());
            Ok(false)
        }
        KeyCode::Enter => {
            app.send_chat().await?;
            Ok(false)
        }
        KeyCode::Backspace => {
            app.input.pop();
            Ok(false)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.clear();
            Ok(false)
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Err(error) = app.refresh_dashboard().await {
                app.status = format!("dashboard refresh failed: {error}");
            }
            Ok(false)
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.select_goal_by_step(1).await?;
            Ok(false)
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.select_goal_by_step(-1).await?;
            Ok(false)
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_goal_selection().await?;
            Ok(false)
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.submit_last_goal_draft().await?;
            Ok(false)
        }
        KeyCode::F(5) => {
            app.submit_last_goal_draft().await?;
            Ok(false)
        }
        KeyCode::Up => {
            app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_add(1);
            Ok(false)
        }
        KeyCode::Down => {
            app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_sub(1);
            Ok(false)
        }
        KeyCode::PageUp => {
            app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_add(10);
            Ok(false)
        }
        KeyCode::PageDown => {
            app.chat_scroll_from_bottom = app.chat_scroll_from_bottom.saturating_sub(10);
            Ok(false)
        }
        KeyCode::Home => {
            app.chat_scroll_from_bottom = u16::MAX;
            Ok(false)
        }
        KeyCode::End => {
            app.chat_scroll_from_bottom = 0;
            Ok(false)
        }
        KeyCode::Char(ch) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
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
        Line::from(format!("approvals  {}", dashboard.approvals_count)),
        Line::from(format!("events     {}", dashboard.events_count)),
        Line::from(format!("plans      {}", dashboard.plans_count)),
        Line::from(""),
        Line::from(vec![
            Span::raw("chat backend "),
            Span::styled(
                dashboard.chat_backend.as_str(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];
    lines.extend(current_goal_lines(
        app.selected_goal(),
        app.selected_goal_id.as_deref(),
        area.width,
    ));
    if let Some(response) = app.last_chat_response.as_ref()
        && let Some(summary) = goal_draft_summary_from_response(response)
    {
        lines.extend(goal_draft_dashboard_lines(&summary, area.width));
    }
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "latest goals",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ]);
    if dashboard.latest_goals.is_empty() {
        lines.push(Line::from("none projected"));
    } else {
        for goal in &dashboard.latest_goals {
            lines.push(Line::from(format!("• {goal}")));
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("Dashboard"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_chat(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut lines = Vec::new();
    for message in &app.messages {
        let color = if message.role == "user" {
            Color::Yellow
        } else {
            Color::Cyan
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}: ", message.role),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(message.content.as_str()),
        ]));
        lines.push(Line::from(""));
    }
    if app.busy {
        lines.push(Line::from(Span::styled(
            "calling gateway...",
            Style::default().fg(Color::Magenta),
        )));
    }
    let scroll_y = chat_scroll_y(
        rendered_row_count(&lines, area.width.saturating_sub(2)),
        area.height.saturating_sub(2),
        app.chat_scroll_from_bottom,
    );
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title("Gateway Chat"))
            .scroll((scroll_y, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_input(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = format!("Input [{}]", app.mode.label());
    frame.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().fg(Color::White)),
        area,
    );
    let cursor_x = area
        .x
        .saturating_add(1)
        .saturating_add(app.input.len().min(area.width.saturating_sub(2) as usize) as u16);
    frame.set_cursor_position(Position::new(cursor_x, area.y.saturating_add(1)));
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(
            "Enter send  Tab mode  Ctrl-N/P goals  Ctrl-O clear goal  ↑/↓ PgUp/PgDn scroll  F5/Ctrl-G submit  Ctrl-R refresh  Ctrl-U clear input  Esc/Ctrl-C/q quit",
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

fn dashboard_summary(config: &Value, overview: &Value, goals: &[GoalSummary]) -> DashboardSummary {
    let services = find_first_array(overview, &["services"]).unwrap_or(&[]);
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
        runners_count: find_first_array(overview, &["runner_status", "runners", "data"])
            .map_or(0, <[Value]>::len),
        approvals_count: find_first_array(overview, &["approvals"]).map_or(0, <[Value]>::len),
        events_count: find_first_array(overview, &["recent_events", "events"])
            .map_or(0, <[Value]>::len),
        plans_count: find_first_array(overview, &["plans"]).map_or(0, <[Value]>::len),
        chat_backend: chat_backend_label(config),
        latest_goals: goals.iter().take(5).map(goal_label).collect(),
    }
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
        }
        None if let Some(goal_id) = selected_goal_id => {
            lines.push(dashboard_value_line(
                "selected",
                &short_id(goal_id),
                value_width,
            ));
            lines.push(Line::from("status     loading projection"));
        }
        None => {
            lines.push(Line::from("select     Ctrl-N / Ctrl-P"));
            lines.push(Line::from("chat       operator workspace"));
        }
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
            !message
                .content
                .starts_with("Goal draft ready for submission.")
                && !message
                    .content
                    .starts_with("Submitted drafted goal to the coordinator.")
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

fn goal_draft_summary_from_response(value: &Value) -> Option<GoalDraftSummary> {
    let draft = extract_goal_draft(value)?;
    goal_draft_summary(&draft)
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
            "submit     F5 or Ctrl-G",
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

    #[test]
    fn dashboard_summary_reads_gateway_proxy_shapes() {
        let config = json!({
            "chat_backend": {
                "mode": "runner_registry",
                "provider": "openai_compatible",
                "model_configured": true
            }
        });
        let overview = json!({
            "services": [
                {"name": "goal-store", "ok": true},
                {"name": "runner-registry", "status": 503}
            ],
            "goals": {"data": {"goals": [
                {"goal_id": "018f8f2f-1fd8-7688-bb12-8bfb6b756601", "title": "Ship TUI", "status": "running"}
            ]}},
            "runner_status": {"data": {"runners": [{"runner_id": "r1"}]}},
            "approvals": {"data": [{"id": "a1"}, {"id": "a2"}]},
            "recent_events": {"data": {"events": [{"id": "e1"}]}},
            "plans": {"data": []}
        });

        let goals = goal_summaries_from_value(&overview);
        let summary = dashboard_summary(&config, &overview, &goals);

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
                        "blocked_tasks": 1
                    }
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
                content: "Goal draft ready for submission.\nTitle: X".to_string(),
            },
            ChatLine {
                role: "user".to_string(),
                content: "Submit it".to_string(),
            },
            ChatLine {
                role: "assistant".to_string(),
                content: "Submitted drafted goal to the coordinator.\ngoal_id: g1".to_string(),
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
}
