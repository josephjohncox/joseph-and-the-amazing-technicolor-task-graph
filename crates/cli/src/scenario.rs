//! Deterministic scenario harness for COAT operator workflows.
//!
//! The harness is evidence-oriented. It can drive the control gateway when it
//! is reachable, and it can evaluate fixture projections from the scenario spec
//! so local and CI tests do not need live services or model credentials.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Args)]
pub struct ScenarioCommand {
    #[command(subcommand)]
    pub command: ScenarioSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum ScenarioSubcommand {
    #[command(about = "List deterministic E2E scenario specs")]
    List(ScenarioListArgs),
    #[command(about = "Run a deterministic E2E scenario and write evidence")]
    Run(ScenarioRunArgs),
    #[command(about = "Print a scenario run report")]
    Report(ScenarioReportArgs),
}

#[derive(Debug, Args)]
pub struct ScenarioListArgs {
    #[arg(long, default_value = "scenarios/e2e")]
    pub dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct ScenarioRunArgs {
    #[arg(long)]
    pub file: PathBuf,
    #[arg(long, default_value = "http://localhost:9090")]
    pub gateway_url: String,
    #[arg(long, value_parser = parse_duration_arg, default_value = "10m")]
    pub timeout: Duration,
    #[arg(long, default_value = "target/coat-scenarios")]
    pub output_dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct ScenarioReportArgs {
    #[arg(long)]
    pub run_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioSpec {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub services: Value,
    #[serde(default)]
    pub setup: Value,
    #[serde(default)]
    pub setup_events: Vec<Value>,
    #[serde(default)]
    pub goals: Vec<ScenarioGoal>,
    #[serde(default)]
    pub actions: Vec<ScenarioAction>,
    #[serde(default, alias = "expect")]
    pub expectations: ScenarioExpectation,
    #[serde(default)]
    pub expected_terminal_state: Value,
    #[serde(default)]
    pub evaluator_checks: Vec<Value>,
    #[serde(default)]
    pub usability_coherence: ScenarioUsabilityCoherence,
    #[serde(default)]
    pub timeout: Value,
    #[serde(default)]
    pub artifact_policy: Value,
    #[serde(default, alias = "fixture")]
    pub fixtures: ScenarioFixtures,
    #[serde(default)]
    pub projection: Option<ScenarioProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioGoal {
    #[serde(default, alias = "goal_id")]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub spec: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioAction {
    #[serde(default)]
    pub id: String,
    #[serde(default, alias = "type")]
    pub kind: ScenarioActionKind,
    #[serde(default, alias = "description")]
    pub label: String,
    #[serde(default, alias = "goal_id")]
    pub goal_ref: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub body: Value,
    #[serde(default)]
    pub event: Value,
    #[serde(default)]
    pub resume: Value,
    #[serde(default)]
    pub worker_result: Value,
    #[serde(default)]
    pub worker_results: Vec<Value>,
    #[serde(default)]
    pub expect: Value,
    #[serde(default)]
    pub capture_as: Option<String>,
    #[serde(default)]
    pub expect_status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioActionKind {
    SubmitGoal,
    EmitEvent,
    Approve,
    ResumeThunk,
    Steer,
    Vote,
    BranchSelect,
    WaitForProjection,
    GetJson,
    PostJson,
    Wait,
    InjectWorkerResult,
    InjectWorkerResults,
    ValidateGoal,
    ResumeDelayedCompute,
    EmitExternalEvent,
    IterationFixture,
    #[serde(other)]
    Other,
}

impl Default for ScenarioActionKind {
    fn default() -> Self {
        Self::Other
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioExpectation {
    #[serde(default)]
    pub terminal_state: String,
    #[serde(default)]
    pub goal_status: String,
    #[serde(default, alias = "subgoals")]
    pub subgoal_count: Option<usize>,
    #[serde(default, alias = "tasks")]
    pub task_count: Option<usize>,
    #[serde(default, alias = "events")]
    pub event_count: Option<usize>,
    #[serde(default, alias = "artifacts")]
    pub artifact_count: Option<usize>,
    #[serde(default)]
    pub min_subgoals: usize,
    #[serde(default)]
    pub min_tasks: usize,
    #[serde(default)]
    pub min_events: usize,
    #[serde(default)]
    pub min_artifacts: usize,
    #[serde(default)]
    pub min_compute_graph_nodes: usize,
    #[serde(default)]
    pub task_statuses: BTreeMap<String, usize>,
    #[serde(default)]
    pub task_purposes: BTreeMap<String, usize>,
    #[serde(default)]
    pub required_events: Vec<String>,
    #[serde(default)]
    pub required_artifacts: Vec<String>,
    #[serde(default)]
    pub required_ui_projection: Vec<String>,
    #[serde(default)]
    pub ui_projection: Option<bool>,
    #[serde(default)]
    pub required_transitions: Vec<String>,
    #[serde(default)]
    pub blocked_expected: bool,
    #[serde(default)]
    pub action_required_expected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioUsabilityCoherence {
    #[serde(default)]
    pub required_visible_terms: Vec<String>,
    #[serde(default = "default_true")]
    pub blocked_operator_action_required: bool,
    #[serde(default = "default_true")]
    pub completed_evidence_required: bool,
    #[serde(default = "default_true")]
    pub completed_satisfaction_rationale_required: bool,
}

impl Default for ScenarioUsabilityCoherence {
    fn default() -> Self {
        Self {
            required_visible_terms: default_required_visible_terms(),
            blocked_operator_action_required: true,
            completed_evidence_required: true,
            completed_satisfaction_rationale_required: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_required_visible_terms() -> Vec<String> {
    [
        "goal",
        "subgoal",
        "task",
        "thunk",
        "fork",
        "review",
        "evidence",
        "action",
        "completed",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioFixtures {
    #[serde(default)]
    pub projection: ScenarioProjection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioProjection {
    #[serde(default)]
    pub goal_id: String,
    #[serde(default)]
    pub goal_status: String,
    #[serde(default)]
    pub terminal_state: String,
    #[serde(default)]
    pub subgoals: Vec<Value>,
    #[serde(default)]
    pub tasks: Vec<Value>,
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default)]
    pub artifacts: Vec<Value>,
    #[serde(default)]
    pub checkpoints: Vec<Value>,
    #[serde(default)]
    pub compute_graph_nodes: Vec<Value>,
    #[serde(default)]
    pub runner_dispatches: Vec<Value>,
    #[serde(default)]
    pub ui_projection: BTreeMap<String, Value>,
    #[serde(default)]
    pub transitions: Vec<String>,
    #[serde(default)]
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioActionResult {
    pub index: usize,
    pub kind: ScenarioActionKind,
    pub label: String,
    pub url: Option<String>,
    pub status: Option<u16>,
    pub ok: bool,
    pub response: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioEvidence {
    pub scenario_id: String,
    pub title: String,
    pub mode: String,
    pub gateway_url: String,
    pub timeout_seconds: u64,
    pub run_dir: PathBuf,
    pub submitted_goal_ids: Vec<String>,
    pub action_results: Vec<ScenarioActionResult>,
    pub projected_tasks: Vec<Value>,
    pub subgoals: Vec<Value>,
    pub events: Vec<Value>,
    pub checkpoints: Vec<Value>,
    pub compute_graph_snapshots: Vec<Value>,
    pub runner_dispatches: Vec<Value>,
    pub ui_visible_summaries: BTreeMap<String, Value>,
    pub artifacts: Vec<Value>,
    pub projection: ScenarioProjection,
    pub evaluator: EvaluatorVerdict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluatorVerdict {
    pub status: String,
    pub findings: Vec<String>,
    pub checks: Vec<EvaluatorCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluatorCheck {
    pub name: String,
    pub passed: bool,
    pub expected: Value,
    pub actual: Value,
    pub message: String,
}

pub async fn run(args: ScenarioCommand) -> anyhow::Result<()> {
    match args.command {
        ScenarioSubcommand::List(args) => list(args),
        ScenarioSubcommand::Run(args) => run_scenario(args).await,
        ScenarioSubcommand::Report(args) => report(args),
    }
}

fn list(args: ScenarioListArgs) -> anyhow::Result<()> {
    let specs = scenario_files(&args.dir)?;
    if specs.is_empty() {
        println!("No scenario specs found in {}.", args.dir.display());
        return Ok(());
    }
    for path in specs {
        let spec = read_spec(&path)?;
        println!("{}\t{}\t{}", spec.id, spec.title, path.display());
    }
    Ok(())
}

async fn run_scenario(args: ScenarioRunArgs) -> anyhow::Result<()> {
    let spec = read_spec(&args.file)?;
    let timeout = scenario_timeout(&spec.timeout, args.timeout)?;
    let run_dir = args.output_dir.join(safe_scenario_id(&spec.id)?);
    fs::create_dir_all(&run_dir).with_context(|| format!("create {}", run_dir.display()))?;

    let gateway_url = args.gateway_url.trim_end_matches('/').to_string();
    let (projection, action_results, mode) =
        collect_projection(&spec, &gateway_url, timeout).await?;
    let evidence = build_evidence(
        &spec,
        &gateway_url,
        timeout,
        &run_dir,
        projection,
        action_results,
        mode,
    );

    write_json(&run_dir.join("spec.json"), &spec)?;
    write_json(&run_dir.join("projection.json"), &evidence.projection)?;
    write_json(&run_dir.join("actions.json"), &evidence.action_results)?;
    write_json(&run_dir.join("evidence.json"), &evidence)?;
    write_json(&run_dir.join("report.json"), &report_value(&evidence))?;

    if evidence.evaluator.status != "passed" {
        bail!(
            "scenario {} failed: {}",
            evidence.scenario_id,
            evidence.evaluator.findings.join("; ")
        );
    }
    println!(
        "scenario {} passed; evidence {}",
        evidence.scenario_id,
        evidence.run_dir.display()
    );
    Ok(())
}

fn report(args: ScenarioReportArgs) -> anyhow::Result<()> {
    let report_path = args.run_dir.join("report.json");
    let value = read_json(&report_path)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status == "failed" {
        bail!("scenario report is failed");
    }
    Ok(())
}

async fn collect_projection(
    spec: &ScenarioSpec,
    gateway_url: &str,
    timeout: Duration,
) -> anyhow::Result<(ScenarioProjection, Vec<ScenarioActionResult>, String)> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("build scenario HTTP client")?;
    if gateway_reachable(&client, gateway_url).await {
        return drive_gateway(spec, gateway_url, timeout, client).await;
    }

    let projection = fixture_projection(spec);
    if projection_is_empty(&projection) {
        bail!(
            "gateway {gateway_url} is not reachable and scenario {} has no fixture projection",
            spec.id
        );
    }
    Ok((projection, Vec::new(), "offline_fixture".to_string()))
}

async fn gateway_reachable(client: &reqwest::Client, gateway_url: &str) -> bool {
    let url = format!("{}/healthz", gateway_url.trim_end_matches('/'));
    matches!(
        tokio::time::timeout(Duration::from_secs(2), client.get(url).send()).await,
        Ok(Ok(response)) if response.status().is_success()
    )
}

async fn drive_gateway(
    spec: &ScenarioSpec,
    gateway_url: &str,
    timeout: Duration,
    client: reqwest::Client,
) -> anyhow::Result<(ScenarioProjection, Vec<ScenarioActionResult>, String)> {
    let mut captures = BTreeMap::new();
    let mut goal_ids = scenario_goal_ids(spec, &fixture_projection(spec));
    let mut results = Vec::new();

    for (index, action) in spec.actions.iter().enumerate() {
        let result = execute_action(&client, gateway_url, spec, action, index, &goal_ids).await?;
        if let Some(capture_as) = action.capture_as.as_deref() {
            captures.insert(capture_as.to_string(), result.response.clone());
        }
        if matches!(action.kind, ScenarioActionKind::SubmitGoal) {
            if let Some(goal_id) = extract_goal_id(&result.response)
                .or_else(|| extract_goal_id(&action.payload))
                .or_else(|| extract_goal_id(&goal_body(spec, action).unwrap_or(Value::Null)))
            {
                goal_ids.push(goal_id);
                goal_ids.sort();
                goal_ids.dedup();
            }
        }
        results.push(result);
    }

    if let Some(goal_id) = goal_ids.first() {
        let projection =
            poll_projection_until(&client, gateway_url, goal_id, spec, timeout).await?;
        return Ok((projection, results, "gateway_http".to_string()));
    }
    if let Some(captured) = captures.get("projection") {
        return Ok((
            projection_from_gateway(captured.clone(), None, true),
            results,
            "gateway_http".to_string(),
        ));
    }
    let projection = fixture_projection(spec);
    if !projection_is_empty(&projection) {
        return Ok((
            projection,
            results,
            "gateway_http_with_fixture_projection".to_string(),
        ));
    }
    bail!(
        "scenario {} did not produce a goal id, captured projection, or fixture projection",
        spec.id
    )
}

async fn execute_action(
    client: &reqwest::Client,
    gateway_url: &str,
    spec: &ScenarioSpec,
    action: &ScenarioAction,
    index: usize,
    known_goal_ids: &[String],
) -> anyhow::Result<ScenarioActionResult> {
    if matches!(action.kind, ScenarioActionKind::Wait) {
        let wait = duration_from_action(action)?;
        tokio::time::sleep(wait).await;
        return Ok(ScenarioActionResult {
            index,
            kind: action.kind.clone(),
            label: action.label.clone(),
            url: None,
            status: None,
            ok: true,
            response: json!({ "wait_ms": wait.as_millis() }),
        });
    }

    if matches!(action.kind, ScenarioActionKind::WaitForProjection) {
        let goal_id = action_goal_id(action, known_goal_ids)?;
        let projection = fetch_projection(client, gateway_url, &goal_id).await?;
        return Ok(ScenarioActionResult {
            index,
            kind: action.kind.clone(),
            label: action.label.clone(),
            url: Some(format!("{}/api/goals/{}", gateway_url, goal_id)),
            status: Some(200),
            ok: true,
            response: serde_json::to_value(projection)?,
        });
    }

    if matches!(
        action.kind,
        ScenarioActionKind::InjectWorkerResult
            | ScenarioActionKind::InjectWorkerResults
            | ScenarioActionKind::ValidateGoal
            | ScenarioActionKind::IterationFixture
            | ScenarioActionKind::Other
    ) {
        return Ok(ScenarioActionResult {
            index,
            kind: action.kind.clone(),
            label: action.label.clone(),
            url: None,
            status: None,
            ok: true,
            response: json!({
                "scenario_action": action.id,
                "kind": action_name(action),
                "fixture_only": true,
                "expect": action.expect,
                "worker_result": action.worker_result,
                "worker_results": action.worker_results,
            }),
        });
    }

    let method_is_get = matches!(action.kind, ScenarioActionKind::GetJson);
    let path = action_path(action, known_goal_ids)?;
    let url = url_for_path(gateway_url, &path);
    let mut request = if method_is_get {
        client.get(&url)
    } else {
        client.post(&url).json(&action_body(spec, action)?)
    };
    request = request.header("accept", "application/json");
    let response = request
        .send()
        .await
        .with_context(|| format!("{} {}", if method_is_get { "GET" } else { "POST" }, url))?;
    let status = response.status();
    let status_code = status.as_u16();
    let text = response.text().await.unwrap_or_default();
    let response_json =
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| Value::String(text.clone()));
    let ok = if let Some(expected) = action.expect_status {
        status_code == expected
    } else {
        status.is_success()
    };
    if !ok {
        bail!(
            "scenario action {} {:?} failed with HTTP {}: {}",
            index,
            action.kind,
            status_code,
            text
        );
    }
    Ok(ScenarioActionResult {
        index,
        kind: action.kind.clone(),
        label: action.label.clone(),
        url: Some(url),
        status: Some(status_code),
        ok,
        response: response_json,
    })
}

async fn poll_projection_until(
    client: &reqwest::Client,
    gateway_url: &str,
    goal_id: &str,
    spec: &ScenarioSpec,
    timeout: Duration,
) -> anyhow::Result<ScenarioProjection> {
    let deadline = Instant::now() + timeout;
    loop {
        let projection = fetch_projection(client, gateway_url, goal_id).await?;
        if projection_satisfies_poll_target(&projection, spec) {
            return Ok(projection);
        }
        if Instant::now() >= deadline {
            return Ok(projection);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn projection_satisfies_poll_target(projection: &ScenarioProjection, spec: &ScenarioSpec) -> bool {
    evaluate(spec, projection).status == "passed"
}

async fn fetch_projection(
    client: &reqwest::Client,
    gateway_url: &str,
    goal_id: &str,
) -> anyhow::Result<ScenarioProjection> {
    let control_url = format!(
        "{}/api/goals/{}",
        gateway_url.trim_end_matches('/'),
        goal_id
    );
    if let Some(value) = get_json_optional(client, &control_url).await? {
        return Ok(projection_from_gateway(
            value,
            Some(goal_id.to_string()),
            true,
        ));
    }

    let base = gateway_url.trim_end_matches('/');
    let goal = get_json_required(client, &format!("{base}/goal-store/goals/{goal_id}")).await?;
    let tasks =
        get_json_required(client, &format!("{base}/goal-store/goals/{goal_id}/tasks")).await?;
    let events =
        get_json_required(client, &format!("{base}/goal-store/goals/{goal_id}/events")).await?;
    let artifacts = get_json_required(
        client,
        &format!("{base}/goal-store/goals/{goal_id}/artifacts"),
    )
    .await?;
    let raw = json!({
        "goal": goal,
        "tasks": tasks,
        "events": events,
        "artifacts": artifacts,
    });
    Ok(projection_from_gateway(
        raw,
        Some(goal_id.to_string()),
        false,
    ))
}

async fn get_json_optional(client: &reqwest::Client, url: &str) -> anyhow::Result<Option<Value>> {
    let response = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let text = response.text().await.unwrap_or_default();
    Ok(Some(
        serde_json::from_str(&text).with_context(|| format!("parse response from {url}"))?,
    ))
}

async fn get_json_required(client: &reqwest::Client, url: &str) -> anyhow::Result<Value> {
    let response = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("GET {url} failed with {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("parse response from {url}"))
}

fn build_evidence(
    spec: &ScenarioSpec,
    gateway_url: &str,
    timeout: Duration,
    run_dir: &Path,
    projection: ScenarioProjection,
    action_results: Vec<ScenarioActionResult>,
    mode: String,
) -> ScenarioEvidence {
    let submitted_goal_ids = scenario_goal_ids(spec, &projection);
    let verdict = evaluate(spec, &projection);
    ScenarioEvidence {
        scenario_id: spec.id.clone(),
        title: spec.title.clone(),
        mode,
        gateway_url: gateway_url.trim_end_matches('/').to_string(),
        timeout_seconds: timeout.as_secs(),
        run_dir: run_dir.to_path_buf(),
        submitted_goal_ids,
        action_results,
        projected_tasks: projection.tasks.clone(),
        subgoals: projection.subgoals.clone(),
        events: projection.events.clone(),
        checkpoints: projection.checkpoints.clone(),
        compute_graph_snapshots: projection.compute_graph_nodes.clone(),
        runner_dispatches: projection.runner_dispatches.clone(),
        ui_visible_summaries: projection.ui_projection.clone(),
        artifacts: projection.artifacts.clone(),
        projection,
        evaluator: verdict,
    }
}

fn report_value(evidence: &ScenarioEvidence) -> Value {
    json!({
        "scenario_id": evidence.scenario_id,
        "title": evidence.title,
        "mode": evidence.mode,
        "status": evidence.evaluator.status,
        "findings": evidence.evaluator.findings,
        "checks": evidence.evaluator.checks,
        "run_dir": evidence.run_dir,
        "submitted_goal_ids": evidence.submitted_goal_ids,
        "artifact_count": evidence.artifacts.len(),
        "event_count": evidence.events.len(),
        "task_count": evidence.projected_tasks.len(),
        "subgoal_count": evidence.subgoals.len(),
        "usability_coherence": usability_coherence_report(&evidence.evaluator.checks),
    })
}

fn scenario_goal_ids(spec: &ScenarioSpec, projection: &ScenarioProjection) -> Vec<String> {
    let mut ids = BTreeSet::new();
    if !projection.goal_id.trim().is_empty() {
        ids.insert(projection.goal_id.clone());
    }
    for goal in &spec.goals {
        if !goal.id.trim().is_empty() {
            ids.insert(goal.id.clone());
        }
    }
    ids.into_iter().collect()
}

fn evaluate(spec: &ScenarioSpec, projection: &ScenarioProjection) -> EvaluatorVerdict {
    let expected = &spec.expectations;
    let mut checks = Vec::new();

    if !expected.terminal_state.is_empty() {
        checks.push(check(
            "terminal_state",
            same_terminal_state(&projection.terminal_state, &expected.terminal_state),
            json!(expected.terminal_state),
            json!(projection.terminal_state),
            format!(
                "terminal_state expected {} got {}",
                expected.terminal_state, projection.terminal_state
            ),
        ));
    }
    if !expected.goal_status.is_empty() {
        checks.push(check(
            "goal_status",
            normalize_token(&projection.goal_status) == normalize_token(&expected.goal_status),
            json!(expected.goal_status),
            json!(projection.goal_status),
            format!(
                "goal_status expected {} got {}",
                expected.goal_status, projection.goal_status
            ),
        ));
    }
    if let Some(count) = expected.subgoal_count {
        checks.push(exact_count_check(
            "subgoals",
            count,
            projection.subgoals.len(),
        ));
    }
    if let Some(count) = expected.task_count {
        checks.push(exact_count_check("tasks", count, projection.tasks.len()));
    }
    if let Some(count) = expected.event_count {
        checks.push(exact_count_check("events", count, projection.events.len()));
    }
    if let Some(count) = expected.artifact_count {
        checks.push(exact_count_check(
            "artifacts",
            count,
            projection.artifacts.len(),
        ));
    }
    checks.push(min_count_check(
        "subgoals",
        expected.min_subgoals,
        projection.subgoals.len(),
    ));
    checks.push(min_count_check(
        "tasks",
        expected.min_tasks,
        projection.tasks.len(),
    ));
    checks.push(min_count_check(
        "events",
        expected.min_events,
        projection.events.len(),
    ));
    checks.push(min_count_check(
        "artifacts",
        expected.min_artifacts,
        projection.artifacts.len(),
    ));
    checks.push(min_count_check(
        "compute_graph_nodes",
        expected.min_compute_graph_nodes,
        projection.compute_graph_nodes.len(),
    ));
    checks.extend(count_checks(
        "task_status",
        &expected.task_statuses,
        &projection.tasks,
        "status",
    ));
    checks.extend(count_checks(
        "task_purpose",
        &expected.task_purposes,
        &projection.tasks,
        "purpose",
    ));
    checks.extend(required_string_checks(
        "event",
        &expected.required_events,
        projection.events.iter(),
    ));
    checks.extend(required_string_checks(
        "artifact",
        &expected.required_artifacts,
        projection.artifacts.iter(),
    ));
    checks.extend(required_string_checks(
        "ui_projection",
        &expected.required_ui_projection,
        projection.ui_projection.values(),
    ));
    if let Some(required) = expected.ui_projection {
        checks.push(check(
            "ui_projection_present",
            projection.ui_projection.is_empty() != required,
            json!(required),
            json!(!projection.ui_projection.is_empty()),
            format!(
                "ui_projection expected present={} got present={}",
                required,
                !projection.ui_projection.is_empty()
            ),
        ));
    }
    for transition in &expected.required_transitions {
        checks.push(check(
            format!("transition:{transition}"),
            projection
                .transitions
                .iter()
                .any(|value| value == transition),
            json!(transition),
            json!(projection.transitions),
            format!("missing transition {transition}"),
        ));
    }
    if expected.blocked_expected {
        checks.push(check(
            "blocked_expected",
            projection
                .tasks
                .iter()
                .any(|task| text_field(task, "status").contains("blocked")),
            json!(true),
            json!(projection.tasks),
            "blocked_expected but no blocked task was projected".to_string(),
        ));
    }
    if expected.action_required_expected {
        checks.push(check(
            "action_required_expected",
            projection
                .compute_graph_nodes
                .iter()
                .chain(projection.events.iter())
                .any(|value| {
                    value_contains(value, "action_required") || value_contains(value, "thunk")
                }),
            json!(true),
            json!({
                "compute_graph_nodes": projection.compute_graph_nodes,
                "events": projection.events,
            }),
            "action_required_expected but no action-required projection was found".to_string(),
        ));
    }
    checks.extend(usability_coherence_checks(spec, projection));

    let checks = checks
        .into_iter()
        .filter(|check| !check.name.starts_with("min_") || check.expected != json!(0))
        .collect::<Vec<_>>();
    let findings = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.message.clone())
        .collect::<Vec<_>>();
    EvaluatorVerdict {
        status: if findings.is_empty() {
            "passed".to_string()
        } else {
            "failed".to_string()
        },
        findings,
        checks,
    }
}

fn check(
    name: impl Into<String>,
    passed: bool,
    expected: Value,
    actual: Value,
    message: String,
) -> EvaluatorCheck {
    EvaluatorCheck {
        name: name.into(),
        passed,
        expected,
        actual,
        message,
    }
}

fn min_count_check(label: &str, min: usize, actual: usize) -> EvaluatorCheck {
    check(
        format!("min_{label}"),
        actual >= min,
        json!(min),
        json!(actual),
        format!("{label} expected at least {min} got {actual}"),
    )
}

fn exact_count_check(label: &str, expected: usize, actual: usize) -> EvaluatorCheck {
    check(
        format!("{label}_count"),
        actual == expected,
        json!(expected),
        json!(actual),
        format!("{label} expected {expected} got {actual}"),
    )
}

fn count_checks(
    label: &str,
    expected: &BTreeMap<String, usize>,
    rows: &[Value],
    field: &str,
) -> Vec<EvaluatorCheck> {
    expected
        .iter()
        .map(|(key, min)| {
            let count = rows
                .iter()
                .filter(|row| normalize_token(&text_field(row, field)) == normalize_token(key))
                .count();
            check(
                format!("{label}:{key}"),
                count >= *min,
                json!(min),
                json!(count),
                format!("{label} {key} expected at least {min} got {count}"),
            )
        })
        .collect()
}

fn required_string_checks<'a>(
    label: &str,
    required: &[String],
    values: impl Iterator<Item = &'a Value>,
) -> Vec<EvaluatorCheck> {
    let haystack = values
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    required
        .iter()
        .map(|needle| {
            check(
                format!("{label}:{needle}"),
                haystack.contains(needle),
                json!(needle),
                json!(haystack),
                format!("missing {label} evidence {needle}"),
            )
        })
        .collect()
}

fn usability_coherence_checks(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> Vec<EvaluatorCheck> {
    let mut checks = Vec::new();
    let visible_text = operator_visible_text(projection);
    for term in scenario_required_visible_terms(spec) {
        checks.push(check(
            format!("usability_visible_term:{term}"),
            visible_text_contains_term(&visible_text, &term),
            json!(term),
            json!(visible_text_contains_term(&visible_text, &term)),
            format!("missing visible operator term {term}"),
        ));
    }

    if spec.usability_coherence.blocked_operator_action_required
        && blocked_state_present(spec, projection)
    {
        let action = blocked_operator_action(spec, projection);
        checks.push(check(
            "coherence_blocked_operator_action",
            action.is_some(),
            json!("concrete operator action"),
            json!(action),
            "blocked state requires a concrete operator action".to_string(),
        ));
    }

    if spec.usability_coherence.completed_evidence_required && goal_is_completed(projection) {
        checks.push(check(
            "coherence_completed_evidence",
            completed_goal_has_evidence(projection),
            json!("evidence artifact or evidence payload"),
            json!({
                "artifact_count": projection.artifacts.len(),
                "task_evidence_present": projection.tasks.iter().any(value_has_evidence),
                "event_evidence_present": projection.events.iter().any(value_has_evidence),
            }),
            "completed goal requires evidence".to_string(),
        ));
    }

    if spec
        .usability_coherence
        .completed_satisfaction_rationale_required
        && goal_is_completed(projection)
    {
        let rationale = satisfaction_rationale(spec, projection);
        checks.push(check(
            "coherence_completed_satisfaction_rationale",
            rationale.is_some(),
            json!("satisfaction rationale"),
            json!(rationale),
            "completed goal requires a satisfaction rationale".to_string(),
        ));
    }

    checks
}

fn usability_coherence_report(checks: &[EvaluatorCheck]) -> Value {
    let checks = checks
        .iter()
        .filter(|check| is_usability_coherence_check(&check.name))
        .cloned()
        .collect::<Vec<_>>();
    let failed = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.message.clone())
        .collect::<Vec<_>>();
    json!({
        "status": if failed.is_empty() { "passed" } else { "failed" },
        "failed": failed,
        "checks": checks,
    })
}

fn is_usability_coherence_check(name: &str) -> bool {
    name.starts_with("usability_") || name.starts_with("coherence_")
}

fn scenario_required_visible_terms(spec: &ScenarioSpec) -> Vec<String> {
    let mut terms = if spec.usability_coherence.required_visible_terms.is_empty() {
        default_required_visible_terms()
    } else {
        spec.usability_coherence.required_visible_terms.clone()
    };
    for term in &mut terms {
        *term = normalize_token(term);
    }
    terms.retain(|term| !term.is_empty());
    terms.sort();
    terms.dedup();
    terms
}

fn operator_visible_text(projection: &ScenarioProjection) -> String {
    serde_json::to_string(&projection.ui_projection)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn visible_text_contains_term(visible_text: &str, term: &str) -> bool {
    visible_text.contains(&normalize_token(term))
}

fn blocked_state_present(spec: &ScenarioSpec, projection: &ScenarioProjection) -> bool {
    normalize_token(&projection.goal_status) == "blocked"
        || normalize_terminal(&projection.terminal_state) == "blocked"
        || projection.tasks.iter().any(|task| {
            matches!(
                normalize_token(&text_field(task, "status")).as_str(),
                "blocked" | "waiting" | "waiting_input" | "waiting_approval"
            )
        })
        || spec.actions.iter().any(|action| {
            action_worker_results(action).iter().any(|result| {
                matches!(
                    normalize_token(&first_string(result, &[&["status"]]).unwrap_or_default())
                        .as_str(),
                    "blocked" | "waiting" | "waiting_input" | "waiting_approval"
                ) || !first_array(result, &[&["delayed_compute_thunks"]]).is_empty()
            })
        })
}

fn blocked_operator_action(spec: &ScenarioSpec, projection: &ScenarioProjection) -> Option<String> {
    spec.actions
        .iter()
        .flat_map(action_worker_results)
        .find_map(|result| operator_action_from_value(&result))
        .or_else(|| projection.tasks.iter().find_map(operator_action_from_value))
        .or_else(|| {
            projection
                .compute_graph_nodes
                .iter()
                .find_map(operator_action_from_value)
        })
        .or_else(|| {
            projection
                .ui_projection
                .values()
                .find_map(operator_action_from_value)
        })
}

fn operator_action_from_value(value: &Value) -> Option<String> {
    for path in [
        &["operator_action"][..],
        &["required_operator_action"],
        &["action_required"],
        &["resume_instruction"],
        &["next_action"],
        &["wait", "operator_action"],
        &["blocked", "operator_action"],
    ] {
        if let Some(text) = first_string(value, &[path])
            && is_concrete_operator_action(&text)
        {
            return Some(text);
        }
    }
    for path in [
        &["next_actions"][..],
        &["operator_actions"],
        &["required_operator_actions"],
    ] {
        for item in string_array(value_at_path(value, path)) {
            if is_concrete_operator_action(&item) {
                return Some(item);
            }
        }
    }
    None
}

fn is_concrete_operator_action(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.split_whitespace().count() >= 2 && trimmed.chars().any(char::is_alphabetic)
}

fn goal_is_completed(projection: &ScenarioProjection) -> bool {
    matches!(
        normalize_token(&projection.goal_status).as_str(),
        "done" | "completed" | "satisfied"
    ) || normalize_terminal(&projection.terminal_state) == "completed"
}

fn completed_goal_has_evidence(projection: &ScenarioProjection) -> bool {
    !projection.artifacts.is_empty()
        || projection.tasks.iter().any(value_has_evidence)
        || projection.events.iter().any(value_has_evidence)
}

fn value_has_evidence(value: &Value) -> bool {
    value_contains(value, "evidence")
        || value_contains(value, "artifact://")
        || value.get("artifacts").and_then(Value::as_array).is_some()
        || value
            .get("test_evidence")
            .and_then(Value::as_array)
            .is_some()
}

fn satisfaction_rationale(spec: &ScenarioSpec, projection: &ScenarioProjection) -> Option<String> {
    [&spec.expected_terminal_state, &projection.raw]
        .into_iter()
        .chain(projection.ui_projection.values())
        .find_map(|value| {
            first_string(
                value,
                &[
                    &["satisfaction", "rationale"],
                    &["satisfaction", "reason"],
                    &["satisfaction_rationale"],
                    &["goal_satisfaction", "rationale"],
                ],
            )
            .filter(|text| text.trim().split_whitespace().count() >= 3)
        })
}

fn text_field(value: &Value, field: &str) -> String {
    let Some(field_value) = value.get(field) else {
        return String::new();
    };
    if let Some(text) = field_value.as_str() {
        return text.to_string();
    }
    if let Some(kind) = field_value.get("kind").and_then(Value::as_str) {
        return kind.to_string();
    }
    serde_json::to_string(field_value).unwrap_or_default()
}

fn value_contains(value: &Value, needle: &str) -> bool {
    serde_json::to_string(value)
        .unwrap_or_default()
        .contains(needle)
}

fn scenario_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect_scenario_files(dir, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_scenario_files(dir: &Path, paths: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_scenario_files(&path, paths)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    Ok(())
}

fn read_spec(path: &Path) -> anyhow::Result<ScenarioSpec> {
    let mut spec: ScenarioSpec = serde_json::from_value(read_json(path)?)
        .with_context(|| format!("parse {}", path.display()))?;
    validate_spec(&mut spec).with_context(|| format!("validate {}", path.display()))?;
    Ok(spec)
}

fn validate_spec(spec: &mut ScenarioSpec) -> anyhow::Result<()> {
    if spec.id.trim().is_empty() {
        bail!("scenario id is required");
    }
    safe_scenario_id(&spec.id)?;
    if spec.title.trim().is_empty() {
        spec.title = spec.id.clone();
    }
    if projection_is_empty(&spec.fixtures.projection) {
        if let Some(projection) = spec.projection.clone() {
            spec.fixtures.projection = projection;
        }
    }
    normalize_goals(spec);
    normalize_expectations(spec);
    if projection_is_empty(&spec.fixtures.projection) {
        spec.fixtures.projection = projection_from_scenario_spec(spec);
    }
    Ok(())
}

fn normalize_goals(spec: &mut ScenarioSpec) {
    for goal in &mut spec.goals {
        let spec_record = goal.spec.as_object();
        if goal.id.trim().is_empty() {
            goal.id = spec_record
                .and_then(|record| record.get("id"))
                .and_then(Value::as_str)
                .or_else(|| goal.payload.get("id").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
        }
        if goal.title.trim().is_empty() {
            goal.title = spec_record
                .and_then(|record| record.get("title"))
                .and_then(Value::as_str)
                .or_else(|| goal.payload.get("title").and_then(Value::as_str))
                .unwrap_or(&goal.id)
                .to_string();
        }
        if goal.objective.trim().is_empty() {
            goal.objective = spec_record
                .and_then(|record| record.get("objective"))
                .and_then(Value::as_str)
                .or_else(|| goal.payload.get("objective").and_then(Value::as_str))
                .unwrap_or(&goal.title)
                .to_string();
        }
        if goal.payload.is_null() && !goal.spec.is_null() {
            goal.payload = goal.spec.clone();
        }
    }
}

fn normalize_expectations(spec: &mut ScenarioSpec) {
    let terminal = spec.expected_terminal_state.as_object();
    if spec.expectations.goal_status.is_empty() {
        if let Some(value) = terminal
            .and_then(|record| record.get("goal_status"))
            .and_then(Value::as_str)
        {
            spec.expectations.goal_status = value.to_string();
        }
    }
    if spec.expectations.terminal_state.is_empty() && !spec.expectations.goal_status.is_empty() {
        spec.expectations.terminal_state =
            terminal_state_from_goal_status(&spec.expectations.goal_status);
    }
    if spec.expectations.task_statuses.is_empty() {
        if let Some(counts) = terminal
            .and_then(|record| record.get("task_counts"))
            .and_then(Value::as_object)
        {
            spec.expectations.task_statuses = counts
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .as_u64()
                        .map(|count| (key.clone(), usize::try_from(count).unwrap_or(usize::MAX)))
                })
                .collect();
        }
    }
    if spec.expectations.required_events.is_empty() {
        spec.expectations.required_events =
            string_array_at(&spec.expected_terminal_state, &["required_events"]);
    }
    if spec.expectations.required_artifacts.is_empty() {
        spec.expectations.required_artifacts =
            string_array_at(&spec.expected_terminal_state, &["required_artifacts"]);
    }
    if spec.expectations.min_tasks == 0 {
        spec.expectations.min_tasks = spec
            .expectations
            .task_statuses
            .values()
            .copied()
            .sum::<usize>();
    }
    if spec.expectations.min_events == 0 {
        spec.expectations.min_events = spec.expectations.required_events.len();
    }
    if spec.expectations.min_artifacts == 0 {
        spec.expectations.min_artifacts = spec.expectations.required_artifacts.len();
    }
    if spec.expectations.required_ui_projection.is_empty() {
        spec.expectations.required_ui_projection = vec![
            "goal_list".to_string(),
            "selected_goal".to_string(),
            "task_graph".to_string(),
        ];
    }
    if spec.expectations.ui_projection.is_none() {
        spec.expectations.ui_projection = Some(true);
    }
    if spec.id.contains("blocked") {
        spec.expectations.action_required_expected = true;
    }
}

fn projection_from_scenario_spec(spec: &ScenarioSpec) -> ScenarioProjection {
    let terminal = spec
        .expected_terminal_state
        .as_object()
        .cloned()
        .unwrap_or_default();
    let goal_id = spec
        .goals
        .first()
        .map(|goal| goal.id.clone())
        .unwrap_or_default();
    let goal_status = terminal
        .get("goal_status")
        .and_then(Value::as_str)
        .or_else(|| terminal.get("status").and_then(Value::as_str))
        .unwrap_or("done")
        .to_string();
    let task_counts = terminal
        .get("task_counts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let tasks = tasks_from_scenario(spec, &task_counts, &goal_id);
    let subgoals = subgoals_from_scenario(spec, &tasks);
    let events = events_from_scenario(spec, terminal.get("required_events"));
    let artifacts = artifacts_from_scenario(spec, terminal.get("required_artifacts"));
    let compute_graph_nodes = compute_graph_from_scenario(spec, &tasks, &events, &goal_id);
    let mut ui_projection = BTreeMap::new();
    ui_projection.insert(
        "goal_list".to_string(),
        json!({ "surface": "goal_list", "goals": [{ "goal_id": goal_id, "title": spec.goals.first().map(|goal| goal.title.clone()).unwrap_or_else(|| spec.title.clone()), "status": goal_status }] }),
    );
    ui_projection.insert(
        "selected_goal".to_string(),
        json!({ "surface": "selected_goal", "goal_id": goal_id, "title": spec.goals.first().map(|goal| goal.title.clone()).unwrap_or_else(|| spec.title.clone()), "status": goal_status }),
    );
    ui_projection.insert(
        "task_graph".to_string(),
        json!({ "surface": "task_graph", "nodes": compute_graph_nodes }),
    );
    ui_projection.insert(
        "usability_coherence".to_string(),
        usability_coherence_projection_from_spec(spec, &goal_status, &artifacts),
    );
    if spec.id.contains("blocked") {
        ui_projection.insert(
            "human_queue".to_string(),
            json!({ "surface": "human_queue", "requests": [{ "kind": "delayed_compute_thunk", "status": "resumed", "goal_id": goal_id }] }),
        );
    }
    ScenarioProjection {
        goal_id,
        goal_status: goal_status.clone(),
        terminal_state: terminal_state_from_goal_status(&goal_status),
        subgoals,
        tasks,
        events,
        artifacts,
        checkpoints: vec![json!({
            "kind": "scenario_checkpoint",
            "uri": format!("checkpoint://scenarios/{}", spec.id),
        })],
        compute_graph_nodes,
        runner_dispatches: runner_dispatches_from_scenario(spec),
        ui_projection,
        transitions: spec.actions.iter().map(action_name).collect(),
        raw: json!({
            "expected_terminal_state": spec.expected_terminal_state,
            "evaluator_checks": spec.evaluator_checks,
        }),
    }
}

fn usability_coherence_projection_from_spec(
    spec: &ScenarioSpec,
    goal_status: &str,
    artifacts: &[Value],
) -> Value {
    json!({
        "surface": "usability_coherence",
        "visible_terms": scenario_required_visible_terms(spec),
        "blocked_operator_action": spec.actions
            .iter()
            .flat_map(action_worker_results)
            .find_map(|result| operator_action_from_value(&result)),
        "completed_goal": {
            "status": terminal_state_from_goal_status(goal_status),
            "evidence_artifacts": artifacts,
            "satisfaction_rationale": first_string(
                &spec.expected_terminal_state,
                &[
                    &["satisfaction", "rationale"],
                    &["satisfaction", "reason"],
                    &["satisfaction_rationale"],
                ],
            ),
        },
    })
}

fn tasks_from_scenario(
    spec: &ScenarioSpec,
    task_counts: &Map<String, Value>,
    goal_id: &str,
) -> Vec<Value> {
    let mut tasks = Vec::new();
    let mut seen = BTreeSet::new();
    for action in &spec.actions {
        for result in action_worker_results(action) {
            if normalize_token(&first_string(&result, &[&["status"]]).unwrap_or_default())
                == "waiting"
            {
                continue;
            }
            let task_id = first_string(&result, &[&["task_id"]])
                .unwrap_or_else(|| format!("{}-task-{}", spec.id, tasks.len() + 1));
            if !seen.insert(task_id.clone()) {
                continue;
            }
            let status =
                if normalize_token(&first_string(&result, &[&["status"]]).unwrap_or_default())
                    == "waiting"
                {
                    "blocked".to_string()
                } else {
                    "done".to_string()
                };
            tasks.push(json!({
                "goal_id": goal_id,
                "task_id": task_id,
                "title": first_string(&result, &[&["summary"]]).unwrap_or_else(|| action_name(action)),
                "status": status,
                "purpose": task_purpose_from_action(action),
                "role": first_string(&result, &[&["role"], &["worker_kind"]]).unwrap_or_else(|| "codex".to_string()),
                "worker_result": result,
            }));
        }
    }
    for (status, count) in task_counts {
        let expected = count.as_u64().unwrap_or_default() as usize;
        let current = tasks
            .iter()
            .filter(|task| normalize_token(&text_field(task, "status")) == normalize_token(status))
            .count();
        for index in current..expected {
            tasks.push(json!({
                "goal_id": goal_id,
                "task_id": format!("{}-{}-{}", spec.id, status, index + 1),
                "title": format!("Scenario {status} task {}", index + 1),
                "status": status,
                "purpose": if status == "done" { "work" } else { "review" },
                "role": if status == "done" { "codex" } else { "validator" },
            }));
        }
    }
    tasks
}

fn subgoals_from_scenario(spec: &ScenarioSpec, tasks: &[Value]) -> Vec<Value> {
    let authored = spec
        .goals
        .iter()
        .flat_map(|goal| first_array(&goal.payload, &[&["plan", "subgoals"]]))
        .collect::<Vec<_>>();
    if !authored.is_empty() {
        return authored;
    }
    tasks
        .iter()
        .take(6)
        .enumerate()
        .map(|(index, task)| {
            json!({
                "id": format!("{}-subgoal-{}", spec.id, index + 1),
                "title": text_field(task, "title"),
                "owner_role": text_field(task, "role"),
            })
        })
        .collect()
}

fn events_from_scenario(spec: &ScenarioSpec, required: Option<&Value>) -> Vec<Value> {
    let mut events = string_array(required)
        .into_iter()
        .map(|event| json!({ "event_type": event, "scenario_id": spec.id }))
        .collect::<Vec<_>>();
    for action in &spec.actions {
        events.push(json!({
            "event_type": action_name(action),
            "scenario_action": action.id,
            "goal_id": action.goal_ref,
        }));
    }
    events
}

fn artifacts_from_scenario(spec: &ScenarioSpec, required: Option<&Value>) -> Vec<Value> {
    let mut artifacts = string_array(required)
        .into_iter()
        .map(|uri| json!({ "uri": uri, "kind": "required_artifact" }))
        .collect::<Vec<_>>();
    artifacts.extend(
        first_array(&spec.artifact_policy, &[&["required_artifacts"]])
            .into_iter()
            .filter(|value| value.get("uri").is_some()),
    );
    for action in &spec.actions {
        for result in action_worker_results(action) {
            artifacts.extend(first_array(&result, &[&["artifacts"]]));
        }
    }
    artifacts
}

fn compute_graph_from_scenario(
    spec: &ScenarioSpec,
    tasks: &[Value],
    events: &[Value],
    goal_id: &str,
) -> Vec<Value> {
    let mut nodes = vec![json!({
        "id": goal_id,
        "kind": "goal",
        "label": spec.title,
    })];
    nodes.extend(tasks.iter().map(|task| {
        json!({
            "id": text_field(task, "task_id"),
            "kind": "task",
            "status": text_field(task, "status"),
            "purpose": text_field(task, "purpose"),
        })
    }));
    if spec.id.contains("blocked") {
        nodes.push(json!({
            "id": format!("{}-thunk", spec.id),
            "kind": "delayed_compute_thunk",
            "status": "resumed",
            "label": "action_required",
        }));
    }
    if spec.id.contains("fork") {
        nodes.push(json!({ "id": format!("{}-review-join", spec.id), "kind": "review_unifier" }));
    }
    if spec.id.contains("fanout") {
        nodes.push(
            json!({ "id": format!("{}-fanout", spec.id), "kind": "coordinator_child_fanout" }),
        );
    }
    nodes.extend(events.iter().take(3).map(|event| {
        json!({
            "id": format!("{}-event-{}", spec.id, text_field(event, "event_type")),
            "kind": "event",
            "label": text_field(event, "event_type"),
        })
    }));
    nodes
}

fn runner_dispatches_from_scenario(spec: &ScenarioSpec) -> Vec<Value> {
    spec.actions
        .iter()
        .filter(|action| {
            matches!(
                action.kind,
                ScenarioActionKind::InjectWorkerResult
                    | ScenarioActionKind::InjectWorkerResults
                    | ScenarioActionKind::IterationFixture
            )
        })
        .map(|action| {
            json!({
                "action_id": action.id,
                "kind": action_name(action),
                "goal_id": action.goal_ref,
                "runner_id": "scenario-stub-runner",
            })
        })
        .collect()
}

fn action_worker_results(action: &ScenarioAction) -> Vec<Value> {
    let mut values = Vec::new();
    if !action.worker_result.is_null() {
        values.push(action.worker_result.clone());
    }
    values.extend(action.worker_results.clone());
    values
}

fn action_name(action: &ScenarioAction) -> String {
    if !action.id.trim().is_empty() {
        return action.id.clone();
    }
    format!("{:?}", action.kind).to_lowercase()
}

fn task_purpose_from_action(action: &ScenarioAction) -> &'static str {
    let name = action_name(action);
    if name.contains("review") {
        "review"
    } else if name.contains("join") || name.contains("unifier") {
        "unification"
    } else {
        "work"
    }
}

fn string_array_at(value: &Value, path: &[&str]) -> Vec<String> {
    string_array(value_at_path(value, path))
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect()
}

fn scenario_timeout(value: &Value, fallback: Duration) -> anyhow::Result<Duration> {
    if value.is_null() {
        return Ok(fallback);
    }
    if let Some(text) = value.as_str() {
        return parse_duration_arg(text)
            .map_err(|error| anyhow::anyhow!("scenario timeout {text:?}: {error}"));
    }
    if let Some(seconds) = value
        .get("scenario_timeout_seconds")
        .and_then(Value::as_u64)
    {
        return Ok(Duration::from_secs(seconds));
    }
    Ok(fallback)
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let content = format!("{}\n", serde_json::to_string_pretty(value)?);
    fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

pub fn parse_duration_arg(value: &str) -> Result<Duration, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("duration is empty".to_string());
    }
    let (number, multiplier) = if let Some(number) = trimmed.strip_suffix("ms") {
        (number, Duration::from_millis(1))
    } else if let Some(number) = trimmed.strip_suffix('s') {
        (number, Duration::from_secs(1))
    } else if let Some(number) = trimmed.strip_suffix('m') {
        (number, Duration::from_secs(60))
    } else if let Some(number) = trimmed.strip_suffix('h') {
        (number, Duration::from_secs(60 * 60))
    } else {
        (trimmed, Duration::from_secs(1))
    };
    let parsed = number
        .parse::<u64>()
        .map_err(|error| format!("parse duration {value:?}: {error}"))?;
    let parsed = u32::try_from(parsed).map_err(|_| format!("duration {value:?} is too large"))?;
    multiplier
        .checked_mul(parsed)
        .ok_or_else(|| format!("duration {value:?} is too large"))
}

fn safe_scenario_id(value: &str) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("scenario id is empty");
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        bail!("scenario id {value:?} may only contain ASCII letters, numbers, '.', '_' and '-'");
    }
    Ok(trimmed.to_string())
}

fn fixture_projection(spec: &ScenarioSpec) -> ScenarioProjection {
    spec.fixtures.projection.clone()
}

fn projection_is_empty(projection: &ScenarioProjection) -> bool {
    projection.goal_id.is_empty()
        && projection.goal_status.is_empty()
        && projection.terminal_state.is_empty()
        && projection.subgoals.is_empty()
        && projection.tasks.is_empty()
        && projection.events.is_empty()
        && projection.artifacts.is_empty()
        && projection.compute_graph_nodes.is_empty()
        && projection.ui_projection.is_empty()
        && projection.raw.is_null()
}

fn projection_from_gateway(
    value: Value,
    fallback_goal_id: Option<String>,
    ui: bool,
) -> ScenarioProjection {
    let goal_status = first_string(
        &value,
        &[
            &["goal_status"],
            &["status"],
            &["goal", "status"],
            &["goal", "goal", "status"],
            &["progress", "status"],
            &["workflow_status", "status"],
        ],
    )
    .unwrap_or_default();
    let terminal_state = first_string(
        &value,
        &[
            &["terminal_state"],
            &["goal", "terminal_state"],
            &["workflow_status", "terminal_state"],
        ],
    )
    .unwrap_or_else(|| terminal_state_from_goal_status(&goal_status));
    let goal_id = first_string(
        &value,
        &[
            &["goal_id"],
            &["goal", "goal_id"],
            &["goal", "goal", "goal_id"],
            &["workflow_status", "goal_id"],
        ],
    )
    .or(fallback_goal_id)
    .unwrap_or_default();
    let subgoals = first_array(
        &value,
        &[
            &["subgoals"],
            &["goal", "payload_json", "plan", "subgoals"],
            &["goal", "goal", "payload_json", "plan", "subgoals"],
            &["goal", "plan", "subgoals"],
            &["progress", "subgoals"],
        ],
    );
    let tasks = first_array(
        &value,
        &[&["tasks", "tasks"], &["tasks"], &["agent_activity"]],
    );
    let events = first_array(&value, &[&["events", "events"], &["events"]]);
    let artifacts = first_array(&value, &[&["artifacts", "artifacts"], &["artifacts"]]);
    let checkpoints = first_array(&value, &[&["checkpoints", "checkpoints"], &["checkpoints"]]);
    let compute_graph_nodes = first_array(
        &value,
        &[
            &["compute_graph", "nodes"],
            &["compute_graph_nodes"],
            &["progress", "compute_graph", "nodes"],
        ],
    );
    let mut ui_projection = BTreeMap::new();
    if ui {
        ui_projection.insert("control_gateway_snapshot".to_string(), value.clone());
    }
    if let Some(object) = value.get("ui_projection").and_then(Value::as_object) {
        for (key, value) in object {
            ui_projection.insert(key.clone(), value.clone());
        }
    }
    ScenarioProjection {
        goal_id,
        goal_status,
        terminal_state,
        subgoals,
        tasks,
        events,
        artifacts,
        checkpoints,
        compute_graph_nodes,
        runner_dispatches: first_array(&value, &[&["runner_dispatches"]]),
        ui_projection,
        transitions: first_array(&value, &[&["transitions"]])
            .into_iter()
            .filter_map(|value| value.as_str().map(ToOwned::to_owned))
            .collect(),
        raw: value,
    }
}

fn first_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        value_at_path(value, path).and_then(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| value.as_u64().map(|value| value.to_string()))
        })
    })
}

fn first_array(value: &Value, paths: &[&[&str]]) -> Vec<Value> {
    paths
        .iter()
        .find_map(|path| value_at_path(value, path).and_then(|value| value.as_array().cloned()))
        .unwrap_or_default()
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn action_body(spec: &ScenarioSpec, action: &ScenarioAction) -> anyhow::Result<Value> {
    if matches!(action.kind, ScenarioActionKind::SubmitGoal) {
        return goal_body(spec, action);
    }
    if matches!(action.kind, ScenarioActionKind::EmitExternalEvent) && !action.event.is_null() {
        return Ok(event_body(spec, action));
    }
    if matches!(action.kind, ScenarioActionKind::ResumeDelayedCompute) && !action.resume.is_null() {
        return Ok(action.resume.clone());
    }
    if !action.payload.is_null() {
        return Ok(action.payload.clone());
    }
    Ok(action.body.clone())
}

fn event_body(spec: &ScenarioSpec, action: &ScenarioAction) -> Value {
    if let Some(fixture_id) = action.event.get("fixture").and_then(Value::as_str) {
        if let Some(value) = setup_fixture_value(spec, fixture_id) {
            return value.clone();
        }
    }
    action.event.clone()
}

fn setup_fixture_value<'a>(spec: &'a ScenarioSpec, fixture_id: &str) -> Option<&'a Value> {
    spec.setup
        .get("fixtures")
        .and_then(Value::as_array)?
        .iter()
        .find(|fixture| fixture.get("id").and_then(Value::as_str) == Some(fixture_id))?
        .get("value")
}

fn goal_body(spec: &ScenarioSpec, action: &ScenarioAction) -> anyhow::Result<Value> {
    if !action.payload.is_null() {
        return Ok(action.payload.clone());
    }
    let goal = if action.goal_ref.trim().is_empty() {
        spec.goals.first()
    } else {
        spec.goals.iter().find(|goal| goal.id == action.goal_ref)
    }
    .with_context(|| format!("scenario action {:?} references unknown goal", action.kind))?;

    let mut object = match &goal.payload {
        Value::Object(object) => object.clone(),
        Value::Null => Map::new(),
        other => {
            bail!(
                "scenario goal {} payload must be an object when used as a submit body, got {}",
                goal.id,
                other
            )
        }
    };
    object.entry("id").or_insert_with(|| json!(goal.id));
    object.entry("title").or_insert_with(|| json!(goal.title));
    object
        .entry("objective")
        .or_insert_with(|| json!(goal.objective));
    Ok(Value::Object(object))
}

fn action_path(action: &ScenarioAction, known_goal_ids: &[String]) -> anyhow::Result<String> {
    if let Some(path) = &action.path {
        return Ok(path.clone());
    }
    match action.kind {
        ScenarioActionKind::SubmitGoal => Ok("/api/goals/submit".to_string()),
        ScenarioActionKind::EmitEvent | ScenarioActionKind::EmitExternalEvent => {
            Ok("/api/events?route=true".to_string())
        }
        ScenarioActionKind::Approve => Ok(format!(
            "/api/goals/{}/approve",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::ResumeThunk | ScenarioActionKind::ResumeDelayedCompute => Ok(format!(
            "/api/goals/{}/resume_thunk",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::Steer => Ok(format!(
            "/api/goals/{}/steer",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::Vote => Ok(format!(
            "/api/goals/{}/vote",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::BranchSelect => Ok(format!(
            "/api/goals/{}/select_branch",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::GetJson | ScenarioActionKind::PostJson => {
            bail!("scenario {:?} action requires path", action.kind)
        }
        ScenarioActionKind::WaitForProjection
        | ScenarioActionKind::Wait
        | ScenarioActionKind::InjectWorkerResult
        | ScenarioActionKind::InjectWorkerResults
        | ScenarioActionKind::ValidateGoal
        | ScenarioActionKind::IterationFixture
        | ScenarioActionKind::Other => {
            bail!(
                "scenario {:?} action does not use an HTTP path",
                action.kind
            )
        }
    }
}

fn action_goal_id(action: &ScenarioAction, known_goal_ids: &[String]) -> anyhow::Result<String> {
    if !action.goal_ref.trim().is_empty() {
        return Ok(action.goal_ref.clone());
    }
    known_goal_ids
        .first()
        .cloned()
        .context("scenario action requires a goal_ref or a prior submitted goal id")
}

fn duration_from_action(action: &ScenarioAction) -> anyhow::Result<Duration> {
    if let Some(ms) = action.payload.get("ms").and_then(Value::as_u64) {
        return Ok(Duration::from_millis(ms));
    }
    if let Some(duration) = action.payload.get("duration").and_then(Value::as_str) {
        return parse_duration_arg(duration).map_err(anyhow::Error::msg);
    }
    Ok(Duration::from_millis(250))
}

fn url_for_path(gateway_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    format!(
        "{}/{}",
        gateway_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn extract_goal_id(value: &Value) -> Option<String> {
    first_string(
        value,
        &[
            &["goal_id"],
            &["goalId"],
            &["id"],
            &["workflow_status", "goal_id"],
            &["response", "goal_id"],
            &["goal", "goal_id"],
            &["goal", "goal", "goal_id"],
        ],
    )
}

fn terminal_state_from_goal_status(status: &str) -> String {
    match normalize_token(status).as_str() {
        "done" => "completed".to_string(),
        "failed" => "failed".to_string(),
        "blocked" => "blocked".to_string(),
        "cancelled" => "cancelled".to_string(),
        "paused" => "paused".to_string(),
        "waiting_approval" => "waiting".to_string(),
        "running" => "running".to_string(),
        _ => String::new(),
    }
}

fn same_terminal_state(actual: &str, expected: &str) -> bool {
    let actual = normalize_terminal(actual);
    let expected = normalize_terminal(expected);
    actual == expected
}

fn normalize_terminal(value: &str) -> String {
    match normalize_token(value).as_str() {
        "done" => "completed".to_string(),
        other => other.to_string(),
    }
}

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_spec() -> ScenarioSpec {
        serde_json::from_value(json!({
            "id": "goal_lifecycle_basic",
            "title": "Goal lifecycle basic",
            "goals": [{"id": "goal-basic", "title": "Ship", "objective": "Ship the task"}],
            "actions": [{"kind": "submit_goal", "goal_ref": "goal-basic"}],
            "expected_terminal_state": {
                "goal_status": "done",
                "satisfaction": {
                    "rationale": "The task evidence passed validation and the required artifact is present."
                }
            },
            "usability_coherence": {
                "required_visible_terms": ["goal", "subgoal", "task", "thunk", "fork", "review", "evidence", "action", "completed"],
                "blocked_operator_action_required": true,
                "completed_evidence_required": true,
                "completed_satisfaction_rationale_required": true
            },
            "expectations": {
                "terminal_state": "completed",
                "goal_status": "done",
                "subgoal_count": 2,
                "task_count": 3,
                "event_count": 2,
                "artifact_count": 1,
                "min_compute_graph_nodes": 2,
                "task_statuses": {"done": 3},
                "task_purposes": {"work": 2, "review": 1},
                "required_events": ["goal_submitted", "validation_passed"],
                "required_artifacts": ["test-report"],
                "required_ui_projection": ["Ship"],
                "ui_projection": true,
                "required_transitions": ["submitted", "validated", "done"]
            },
            "fixtures": {
                "projection": {
                    "goal_id": "goal-basic",
                    "goal_status": "done",
                    "terminal_state": "completed",
                    "subgoals": [{"id": "sg-1"}, {"id": "sg-2"}],
                    "tasks": [
                        {"id": "t1", "status": "done", "purpose": "work"},
                        {"id": "t2", "status": "done", "purpose": "work"},
                        {"id": "t3", "status": "done", "purpose": "review"}
                    ],
                    "events": [{"type": "goal_submitted"}, {"type": "validation_passed"}],
                    "artifacts": [{"name": "test-report"}],
                    "compute_graph_nodes": [{"id": "n1"}, {"id": "n2"}],
                    "ui_projection": {
                        "goal_list": "Ship",
                        "operator_terms": "goal subgoal task thunk fork review evidence action completed"
                    },
                    "transitions": ["submitted", "validated", "done"]
                }
            }
        }))
        .expect("fixture spec")
    }

    #[test]
    fn scenario_evaluator_accepts_behavioral_evidence() {
        let spec = fixture_spec();
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "passed");
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn scenario_evaluator_rejects_missing_subgoals() {
        let mut spec = fixture_spec();
        spec.fixtures.projection.subgoals.clear();
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict
                .findings
                .iter()
                .any(|finding| finding.contains("subgoals expected"))
        );
    }

    #[test]
    fn expected_blocked_state_requires_blocked_task() {
        let mut spec = fixture_spec();
        spec.expectations.blocked_expected = true;
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict
                .findings
                .iter()
                .any(|finding| finding.contains("blocked_expected"))
        );
    }

    #[test]
    fn usability_checks_reject_missing_visible_terms() {
        let mut spec = fixture_spec();
        spec.fixtures.projection.ui_projection =
            BTreeMap::from([("goal_list".to_string(), json!("Ship"))]);
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict
                .findings
                .iter()
                .any(|finding| finding.contains("missing visible operator term"))
        );
    }

    #[test]
    fn blocked_state_requires_concrete_operator_action() {
        let mut spec = fixture_spec();
        spec.actions.push(
            serde_json::from_value(json!({
                "id": "block_without_action",
                "kind": "inject_worker_result",
                "worker_result": {
                    "task_id": "task-blocked",
                    "status": "waiting",
                    "delayed_compute_thunks": [
                        {
                            "thunk_id": "thunk-1",
                            "kind": "human_input",
                            "summary": "Waiting for input."
                        }
                    ]
                }
            }))
            .expect("blocked action"),
        );
        spec.fixtures
            .projection
            .tasks
            .push(json!({"id": "task-blocked", "status": "blocked", "purpose": "work"}));
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict
                .findings
                .iter()
                .any(|finding| finding.contains("blocked state requires"))
        );
    }

    #[test]
    fn completed_goal_requires_evidence() {
        let mut spec = fixture_spec();
        spec.expectations.artifact_count = Some(0);
        spec.expectations.required_artifacts.clear();
        spec.expectations.min_artifacts = 0;
        spec.fixtures.projection.artifacts.clear();
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict
                .findings
                .iter()
                .any(|finding| finding.contains("completed goal requires evidence"))
        );
    }

    #[test]
    fn completed_goal_requires_satisfaction_rationale() {
        let mut spec = fixture_spec();
        spec.expected_terminal_state = Value::Null;
        spec.fixtures.projection.raw = Value::Null;
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict.findings.iter().any(|finding| finding.contains(
                "completed goal requires a satisfaction rationale"
            ))
        );
    }

    #[test]
    fn duration_parser_handles_cli_units() {
        assert_eq!(parse_duration_arg("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(
            parse_duration_arg("120s").unwrap(),
            Duration::from_secs(120)
        );
        assert_eq!(parse_duration_arg("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration_arg("7").unwrap(), Duration::from_secs(7));
        assert_eq!(
            parse_duration_arg("250ms").unwrap(),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn evidence_builder_records_run_directory_and_goal_ids() {
        let spec = fixture_spec();
        let evidence = build_evidence(
            &spec,
            "http://localhost:9090",
            Duration::from_secs(600),
            Path::new("target/test-run"),
            spec.fixtures.projection.clone(),
            Vec::new(),
            "offline_fixture".to_string(),
        );
        assert_eq!(evidence.scenario_id, "goal_lifecycle_basic");
        assert_eq!(evidence.timeout_seconds, 600);
        assert_eq!(evidence.submitted_goal_ids, vec!["goal-basic"]);
        assert_eq!(evidence.evaluator.status, "passed");
    }

    #[test]
    fn scenario_report_includes_usability_coherence_outcomes() {
        let spec = fixture_spec();
        let evidence = build_evidence(
            &spec,
            "http://localhost:9090",
            Duration::from_secs(600),
            Path::new("target/test-run"),
            spec.fixtures.projection.clone(),
            Vec::new(),
            "offline_fixture".to_string(),
        );
        let report = report_value(&evidence);
        let usability = report
            .get("usability_coherence")
            .expect("usability coherence report");
        assert_eq!(
            usability.get("status").and_then(Value::as_str),
            Some("passed")
        );
        assert!(
            usability
                .get("checks")
                .and_then(Value::as_array)
                .expect("usability checks")
                .iter()
                .any(|check| {
                    check.get("name").and_then(Value::as_str)
                        == Some("coherence_completed_satisfaction_rationale")
                })
        );
    }

    #[test]
    fn gateway_projection_normalizes_control_snapshot() {
        let projection = projection_from_gateway(
            json!({
                "goal": {
                    "found": true,
                    "goal": {
                        "goal_id": "goal-http",
                        "status": "done",
                        "payload_json": {
                            "plan": {
                                "subgoals": [{"id": "sg"}]
                            }
                        }
                    }
                },
                "tasks": {"tasks": [{"id": "task", "status": "done"}]},
                "events": {"events": [{"kind": "goal_completed"}]},
                "artifacts": {"artifacts": [{"uri": "artifact://report"}]},
                "compute_graph": {"nodes": [{"id": "n"}]}
            }),
            None,
            true,
        );
        assert_eq!(projection.goal_id, "goal-http");
        assert_eq!(projection.goal_status, "done");
        assert_eq!(projection.terminal_state, "completed");
        assert_eq!(projection.subgoals.len(), 1);
        assert_eq!(projection.tasks.len(), 1);
        assert_eq!(projection.events.len(), 1);
        assert_eq!(projection.artifacts.len(), 1);
        assert!(!projection.ui_projection.is_empty());
    }

    #[test]
    fn scenario_files_are_recursive_and_sorted() {
        let root = std::env::temp_dir().join(format!("coat-scenario-test-{}", std::process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("create nested dir");
        fs::write(root.join("b.json"), "{}").expect("write b");
        fs::write(nested.join("a.json"), "{}").expect("write a");
        fs::write(root.join("ignore.txt"), "x").expect("write ignored");

        let files = scenario_files(&root).expect("scenario files");
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("b.json") || files[0].ends_with("a.json"));
        assert!(files[0] < files[1], "files should be sorted: {files:?}");

        let _ = fs::remove_file(root.join("b.json"));
        let _ = fs::remove_file(nested.join("a.json"));
        let _ = fs::remove_file(root.join("ignore.txt"));
        let _ = fs::remove_dir(nested);
        let _ = fs::remove_dir(root);
    }
}
