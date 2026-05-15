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
use uuid::Uuid;

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
    #[command(about = "Seed a scenario fixture projection into the goal-store read model")]
    Seed(ScenarioSeedArgs),
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
pub struct ScenarioSeedArgs {
    #[arg(long)]
    pub file: PathBuf,
    #[arg(
        long,
        env = "COAT_GOAL_STORE_URL",
        default_value = "http://localhost:9088"
    )]
    pub goal_store_url: String,
    #[arg(long)]
    pub dry_run: bool,
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
    pub determinism: Value,
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
    pub service: String,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub task_id: String,
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
    pub attempt: Value,
    #[serde(default)]
    pub artifacts: Vec<Value>,
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
        ScenarioSubcommand::Seed(args) => seed_scenario(args).await,
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

async fn seed_scenario(args: ScenarioSeedArgs) -> anyhow::Result<()> {
    let spec = read_spec(&args.file)?;
    let projection = fixture_projection(&spec);
    let request = goal_store_seed_request(&spec, &projection)?;
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&request)?);
        return Ok(());
    }

    let base = args.goal_store_url.trim_end_matches('/');
    let url = format!("{base}/goal-store/snapshots");
    let response = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("seed scenario {} failed with {status}: {text}", spec.id);
    }
    println!(
        "seeded scenario {} into goal-store {}; response {}",
        spec.id, base, text
    );
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
    let projection = fixture_projection(spec);
    if uses_fixture_projection_replay(spec) {
        if projection_is_empty(&projection) {
            bail!(
                "scenario {} requests fixture projection replay but has no fixture projection",
                spec.id
            );
        }
        return Ok((
            projection.clone(),
            fixture_action_results(spec, &projection)?,
            "fixture_projection_replay".to_string(),
        ));
    }
    if gateway_reachable(&client, gateway_url).await {
        return drive_gateway(spec, gateway_url, timeout, client).await;
    }

    if projection_is_empty(&projection) {
        bail!(
            "gateway {gateway_url} is not reachable and scenario {} has no fixture projection",
            spec.id
        );
    }
    Ok((projection, Vec::new(), "offline_fixture".to_string()))
}

fn uses_fixture_projection_replay(spec: &ScenarioSpec) -> bool {
    let mode = first_string(
        &spec.determinism,
        &[&["mode"], &["projection_mode"], &["determinism", "mode"]],
    )
    .map(|value| normalize_token(&value));
    let projection_mode = first_string(
        &spec.determinism,
        &[
            &["projection_mode"],
            &["determinism", "projection_mode"],
            &["mode"],
        ],
    )
    .map(|value| normalize_token(&value));
    matches!(
        (mode.as_deref(), projection_mode.as_deref()),
        (Some("stub_projection_replay"), _) | (_, Some("fixture_replay"))
    )
}

fn fixture_action_results(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> anyhow::Result<Vec<ScenarioActionResult>> {
    let known_goal_ids = scenario_goal_ids(spec, projection);
    spec.actions
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let response = match action.kind {
                ScenarioActionKind::SubmitGoal => {
                    let goal_id = action_goal_id(action, &known_goal_ids).or_else(|_| {
                        spec.goals
                            .first()
                            .map(|goal| goal.id.clone())
                            .context("scenario has no goal for submit action")
                    })?;
                    json!({
                        "scenario_action": action.id,
                        "kind": action_name(action),
                        "fixture_only": true,
                        "goal_id": goal_id,
                        "status": "submitted",
                        "goal": goal_body(spec, action)?,
                    })
                }
                ScenarioActionKind::EmitEvent | ScenarioActionKind::EmitExternalEvent => json!({
                    "scenario_action": action.id,
                    "kind": action_name(action),
                    "fixture_only": true,
                    "event": event_body(spec, action),
                    "expect": action.expect,
                }),
                ScenarioActionKind::ResumeThunk | ScenarioActionKind::ResumeDelayedCompute => {
                    json!({
                        "scenario_action": action.id,
                        "kind": action_name(action),
                        "fixture_only": true,
                        "resume": action.resume,
                        "expect": action.expect,
                    })
                }
                ScenarioActionKind::Wait => json!({
                    "scenario_action": action.id,
                    "kind": action_name(action),
                    "fixture_only": true,
                    "wait": action.payload,
                }),
                ScenarioActionKind::WaitForProjection => json!({
                    "scenario_action": action.id,
                    "kind": action_name(action),
                    "fixture_only": true,
                    "projection_goal_id": projection.goal_id,
                }),
                _ => json!({
                    "scenario_action": action.id,
                    "kind": action_name(action),
                    "fixture_only": true,
                    "expect": action.expect,
                    "payload": action.payload,
                    "attempt": action.attempt,
                    "artifacts": action.artifacts,
                    "worker_result": action.worker_result,
                    "worker_results": action.worker_results,
                }),
            };
            Ok(ScenarioActionResult {
                index,
                kind: action.kind.clone(),
                label: action.label.clone(),
                url: None,
                status: None,
                ok: true,
                response,
            })
        })
        .collect()
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
            url: Some(format!("{}/api/operator/goals/{}", gateway_url, goal_id)),
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
        "{}/api/operator/goals/{}",
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

fn goal_store_seed_request(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> anyhow::Result<Value> {
    let goal_id = scenario_goal_ids(spec, projection)
        .first()
        .cloned()
        .filter(|value| !value.is_empty())
        .context("scenario seed requires a goal id")?;
    let title = scenario_goal_title(spec, projection);
    let objective = scenario_goal_objective(spec);
    let task_records = seed_task_records(projection)?;
    let total_tasks = task_records.len() as u32;
    let open_tasks = task_records
        .iter()
        .filter(|task| {
            let status = task
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            !matches!(status, "done" | "cancelled")
        })
        .count() as u32;
    let blocked_tasks = task_records
        .iter()
        .filter(|task| {
            matches!(
                task.get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                "blocked" | "waiting_input" | "waiting_approval"
            )
        })
        .count() as u32;
    let failed_tasks = task_records
        .iter()
        .filter(|task| task.get("status").and_then(Value::as_str) == Some("failed"))
        .count() as u32;
    let done_tasks = task_records
        .iter()
        .filter(|task| task.get("status").and_then(Value::as_str) == Some("done"))
        .count() as u32;
    let percent_done = if total_tasks == 0 {
        0.0
    } else {
        done_tasks as f32 / total_tasks as f32
    };
    let root_task_id = task_records
        .first()
        .and_then(|task| task.get("task_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let satisfied = projection.goal_status == "done" || projection.terminal_state == "completed";
    let payload_json = json!({
        "source": "scenario_seed",
        "scenario_id": spec.id,
        "scenario_title": spec.title,
        "terminal_state": projection.terminal_state,
    });

    Ok(json!({
        "metadata": {
            "protocol_version": "coat.v1",
            "idempotency_key": format!("scenario:{}:seed:{}", spec.id, goal_id),
            "trace_id": Value::Null,
            "causation_id": format!("scenario:{}:seed", spec.id),
            "correlation_id": goal_id,
            "created_at": Value::Null
        },
        "projection_reason": format!("scenario_seed:{}", spec.id),
        "snapshot": {
            "goal": {
                "goal_id": goal_id,
                "title": title,
                "objective": objective,
                "repo": Value::Null,
                "status": seed_goal_status(projection),
                "total_tasks": total_tasks,
                "open_tasks": open_tasks,
                "blocked_tasks": blocked_tasks,
                "failed_tasks": failed_tasks,
                "percent_done": percent_done,
                "root_task_id": root_task_id,
                "satisfied": satisfied,
                "satisfaction_score": if satisfied { json!(1.0) } else { Value::Null },
                "updated_at": Value::Null,
                "payload_json": payload_json
            },
            "compute_graph": seed_compute_graph(&goal_id, projection),
            "tasks": task_records,
            "artifacts": seed_artifact_records(&goal_id, projection),
            "approvals": seed_approval_records(&goal_id, projection),
            "events": seed_event_records(&goal_id, spec, projection),
            "full_state_json": {
                "source": "scenario_seed",
                "scenario_id": spec.id,
                "ui_projection": projection.ui_projection
            }
        }
    }))
}

fn scenario_goal_title(spec: &ScenarioSpec, projection: &ScenarioProjection) -> String {
    spec.goals
        .first()
        .and_then(|goal| {
            goal.spec
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| goal.payload.get("title").and_then(Value::as_str))
                .or_else(|| {
                    if goal.title.is_empty() {
                        None
                    } else {
                        Some(goal.title.as_str())
                    }
                })
        })
        .or_else(|| {
            projection
                .ui_projection
                .get("selected_goal")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
        })
        .unwrap_or_else(|| {
            if spec.title.is_empty() {
                &spec.id
            } else {
                &spec.title
            }
        })
        .to_string()
}

fn scenario_goal_objective(spec: &ScenarioSpec) -> String {
    spec.goals
        .first()
        .and_then(|goal| {
            goal.spec
                .get("objective")
                .and_then(Value::as_str)
                .or_else(|| goal.payload.get("objective").and_then(Value::as_str))
                .or_else(|| {
                    if goal.objective.is_empty() {
                        None
                    } else {
                        Some(goal.objective.as_str())
                    }
                })
        })
        .unwrap_or_else(|| {
            if spec.description.is_empty() {
                "Seeded deterministic scenario projection."
            } else {
                &spec.description
            }
        })
        .to_string()
}

fn seed_goal_status(projection: &ScenarioProjection) -> &'static str {
    match projection.goal_status.as_str() {
        "done" | "completed" => "done",
        "blocked" => "blocked",
        "failed" => "failed",
        "cancelled" | "canceled" => "cancelled",
        "waiting_approval" | "waiting-approval" => "waiting_approval",
        "paused" => "paused",
        _ => "running",
    }
}

fn seed_task_records(projection: &ScenarioProjection) -> anyhow::Result<Vec<Value>> {
    projection
        .tasks
        .iter()
        .map(|task| {
            let record = task.as_object().cloned().unwrap_or_default();
            let goal_id = required_string(&record, "goal_id")?;
            let task_id = required_string(&record, "task_id")?;
            let status = seed_task_status(record.get("status").and_then(Value::as_str));
            let purpose = seed_task_purpose(record.get("purpose").and_then(Value::as_str));
            let result_uri = record
                .get("worker_result")
                .and_then(|value| value.get("artifacts"))
                .and_then(Value::as_array)
                .and_then(|artifacts| artifacts.first())
                .and_then(|artifact| artifact.get("uri"))
                .and_then(Value::as_str)
                .map(Value::from)
                .unwrap_or(Value::Null);
            Ok(json!({
                "goal_id": goal_id,
                "task_id": task_id,
                "parent_task_id": Value::Null,
                "subgoal_id": Value::Null,
                "title": record.get("title").and_then(Value::as_str).unwrap_or("Scenario task"),
                "color": Value::Null,
                "role": seed_worker_role(record.get("role").and_then(Value::as_str)),
                "status": status,
                "purpose_kind": purpose,
                "depth": record.get("depth").and_then(Value::as_u64).unwrap_or(0),
                "priority": "normal",
                "priority_rank": 3,
                "attempts": 1,
                "runnable": matches!(status, "pending" | "runnable"),
                "tags": record.get("tags").cloned().unwrap_or_else(|| json!(["scenario", "bootstrap"])),
                "result_uri": result_uri,
                "payload_json": task
            }))
        })
        .collect()
}

fn seed_compute_graph(goal_id: &str, projection: &ScenarioProjection) -> Value {
    let nodes: Vec<Value> = projection
        .compute_graph_nodes
        .iter()
        .filter_map(|node| {
            let record = node.as_object()?;
            let kind = seed_compute_node_kind(record.get("kind").and_then(Value::as_str))?;
            let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
            let status = if let Some(status) = record.get("status").and_then(Value::as_str) {
                seed_compute_node_status(Some(status))
            } else if kind == "goal" {
                seed_compute_node_status(Some(seed_goal_status(projection)))
            } else if kind == "task" {
                seed_compute_node_status(task_status_for_projection_node(projection, id))
            } else {
                seed_compute_node_status(None)
            };
            Some(json!({
                "id": id,
                "kind": kind,
                "label": record.get("label").and_then(Value::as_str).unwrap_or(id),
                "status": status,
                "task_id": if kind == "task" { json!(id) } else { Value::Null },
                "thunk_id": if kind == "delayed_compute_thunk" { json!(id) } else { Value::Null },
                "continuation_id": record
                    .get("continuation_id")
                    .or_else(|| record.get("continuation_ref"))
                    .and_then(Value::as_str)
                    .map(Value::from)
                    .unwrap_or(Value::Null),
                "requested_input": record
                    .get("operator_action")
                    .or_else(|| record.get("requested_input"))
                    .and_then(Value::as_str)
                    .map(Value::from)
                    .unwrap_or(Value::Null),
                "wait_ref": normalized_wait_ref(record.get("wait_ref"))
            }))
        })
        .collect();
    let open_thunks = nodes
        .iter()
        .filter(|node| {
            node.get("kind").and_then(Value::as_str) == Some("delayed_compute_thunk")
                && node.get("status").and_then(Value::as_str) == Some("pending")
        })
        .count() as u32;
    let runnable_tasks = projection
        .tasks
        .iter()
        .filter(|task| task.get("status").and_then(Value::as_str) == Some("runnable"))
        .filter_map(|task| task.get("task_id").cloned())
        .collect::<Vec<_>>();
    let waiting_tasks = projection
        .tasks
        .iter()
        .filter(|task| {
            matches!(
                seed_task_status(task.get("status").and_then(Value::as_str)),
                "waiting_input" | "waiting_approval" | "blocked"
            )
        })
        .filter_map(|task| task.get("task_id").cloned())
        .collect::<Vec<_>>();
    json!({
        "goal_id": goal_id,
        "nodes": nodes,
        "edges": [],
        "open_thunks": open_thunks,
        "runnable_tasks": runnable_tasks,
        "waiting_tasks": waiting_tasks
    })
}

fn task_status_for_projection_node<'a>(
    projection: &'a ScenarioProjection,
    task_id: &str,
) -> Option<&'a str> {
    projection
        .tasks
        .iter()
        .find(|task| task.get("task_id").and_then(Value::as_str) == Some(task_id))
        .and_then(|task| task.get("status").and_then(Value::as_str))
}

fn seed_artifact_records(goal_id: &str, projection: &ScenarioProjection) -> Vec<Value> {
    projection
        .artifacts
        .iter()
        .filter_map(|artifact| {
            let record = artifact.as_object()?;
            let uri = record.get("uri").and_then(Value::as_str)?;
            Some(json!({
                "goal_id": goal_id,
                "task_id": Value::Null,
                "artifact": {
                    "kind": seed_artifact_kind(record.get("kind").and_then(Value::as_str)),
                    "uri": uri,
                    "description": record
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("Scenario artifact"),
                    "sha256": Value::Null
                },
                "git_result": Value::Null,
                "object_artifact": Value::Null,
                "checkpoint": Value::Null,
                "created_at": Value::Null,
                "payload_json": artifact
            }))
        })
        .collect()
}

fn seed_approval_records(goal_id: &str, projection: &ScenarioProjection) -> Vec<Value> {
    projection
        .tasks
        .iter()
        .flat_map(|task| {
            let task_id = task
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            task.get("worker_result")
                .and_then(|result| result.get("delayed_compute_thunks"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |thunk| {
                    let kind = thunk
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if kind != "approval" {
                        return None;
                    }
                    let approval_ref = thunk
                        .get("approval_ref")
                        .and_then(Value::as_str)
                        .unwrap_or("scenario-approval");
                    let request = thunk
                        .get("approval_request")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let status = if thunk.get("status").and_then(Value::as_str) == Some("pending") {
                        "pending"
                    } else {
                        "approved"
                    };
                    Some(json!({
                        "approval_id": deterministic_uuid(&format!("{goal_id}:{approval_ref}")),
                        "goal_id": goal_id,
                        "task_id": if task_id.is_empty() { Value::Null } else { json!(task_id) },
                        "status": status,
                        "risk": "low",
                        "reason": thunk
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or("Scenario approval fixture"),
                        "requested_action": request
                            .get("question")
                            .and_then(Value::as_str)
                            .unwrap_or("Approve this scenario fixture."),
                        "updated_at": Value::Null,
                        "payload_json": thunk
                    }))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn seed_event_records(
    goal_id: &str,
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> Vec<Value> {
    projection
        .events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let event_type = event
                .get("event_type")
                .and_then(Value::as_str)
                .unwrap_or("scenario_event");
            json!({
                "event_id": deterministic_uuid(&format!("{}:{event_type}:{index}", spec.id)),
                "goal_id": goal_id,
                "task_id": event.get("task_id").cloned().unwrap_or(Value::Null),
                "sequence": index as u64 + 1,
                "kind": seed_event_kind(event_type),
                "message": event_type.replace('_', " "),
                "actor": "scenario-seed",
                "idempotency_key": format!("scenario:{}:{}:{}", spec.id, goal_id, index + 1),
                "created_at": Value::Null,
                "payload_json": event
            })
        })
        .collect()
}

fn required_string(record: &Map<String, Value>, key: &str) -> anyhow::Result<String> {
    record
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .with_context(|| format!("scenario seed task is missing {key}"))
}

fn seed_task_status(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default() {
        "runnable" => "runnable",
        "running" => "running",
        "needs_validation" | "needs-validation" => "needs_validation",
        "waiting_approval" | "waiting-approval" => "waiting_approval",
        "waiting_input" | "waiting-input" | "waiting" => "waiting_input",
        "done" | "completed" => "done",
        "blocked" => "blocked",
        "failed" => "failed",
        "cancelled" | "canceled" => "cancelled",
        _ => "pending",
    }
}

fn seed_worker_role(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default() {
        "planner" => "planner",
        "codex" => "codex",
        "claude_code" | "claude-code" => "claude_code",
        "staff_engineer_claude" | "staff-engineer-claude" => "staff_engineer_claude",
        "model_provider" | "model-provider" => "model_provider",
        "research" => "research",
        "reviewer" => "reviewer",
        "tester" => "tester",
        "formal_methods" | "formal-methods" => "formal_methods",
        "validator" => "validator",
        "patch_merger" | "patch-merger" => "patch_merger",
        "rust_tool" | "rust-tool" => "rust_tool",
        _ => "planner",
    }
}

fn seed_task_purpose(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default() {
        "review" => "review",
        "unification" => "unification",
        "actor_retry" | "actor-retry" => "actor_retry",
        "candidate_branch" | "candidate-branch" => "candidate_branch",
        "branch_vote" | "branch-vote" => "branch_vote",
        "branch_unification" | "branch-unification" => "branch_unification",
        "research" => "research",
        _ => "work",
    }
}

fn seed_compute_node_kind(value: Option<&str>) -> Option<&'static str> {
    match value.unwrap_or_default() {
        "goal" => Some("goal"),
        "task" => Some("task"),
        "delayed_compute_thunk" | "thunk" => Some("delayed_compute_thunk"),
        "continuation" => Some("continuation"),
        "wait_ref" | "wait" => Some("wait_ref"),
        "mechanism_round" | "mechanism" => Some("mechanism_round"),
        _ => None,
    }
}

fn seed_compute_node_status(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default() {
        "runnable" => "runnable",
        "running" => "running",
        "waiting" | "waiting_input" | "waiting_approval" => "waiting",
        "needs_validation" | "needs-validation" => "needs_validation",
        "done" | "completed" | "resumed" => "done",
        "blocked" => "blocked",
        "failed" => "failed",
        "cancelled" | "canceled" => "cancelled",
        "paused" => "paused",
        "expired" => "expired",
        _ => "pending",
    }
}

fn normalized_wait_ref(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Object(record))
            if record.get("kind").is_some() && record.get("reference").is_some() =>
        {
            Value::Object(record.clone())
        }
        _ => Value::Null,
    }
}

fn seed_artifact_kind(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default() {
        "patch" => "patch",
        "test_result" | "test-result" => "test_result",
        "report" | "required_artifact" | "approval_record" => "report",
        "pull_request" | "pull-request" => "pull_request",
        "workspace_snapshot" | "workspace-snapshot" => "workspace_snapshot",
        "checkpoint" | "scenario_checkpoint" => "checkpoint",
        "git_branch" | "git-branch" => "git_branch",
        "git_commit" | "git-commit" => "git_commit",
        "git_worktree" | "git-worktree" => "git_worktree",
        "object_storage_object" | "object-storage-object" => "object_storage_object",
        "object_storage_prefix" | "object-storage-prefix" => "object_storage_prefix",
        "artifact_manifest" | "artifact-manifest" => "artifact_manifest",
        "schema" => "schema",
        _ => "other",
    }
}

fn seed_event_kind(event_type: &str) -> &'static str {
    match event_type {
        "goal_submitted" | "submit_goal" => "submitted",
        "task_started" => "task_started",
        "task_completed" | "complete_root_task" | "complete_approved_task" => "task_completed",
        "task_blocked" | "request_human_input" | "request_approval" => "task_blocked",
        "approval_requested" => "approval_requested",
        "approval_granted" | "approve_request" => "approval_decided",
        "validation_passed" | "validate_goal" => "validation_recorded",
        "artifact_recorded" => "artifact_recorded",
        "goal_cancelled" | "cancel_goal" => "cancelled",
        "goal_failed" => "failed",
        _ => "other",
    }
}

fn deterministic_uuid(seed: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes()).to_string()
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
    checks.extend(forbidden_string_checks(
        "forbidden_event",
        &string_array_at(&spec.expected_terminal_state, &["forbidden_events"]),
        projection.events.iter(),
    ));
    checks.extend(forbidden_string_checks(
        "forbidden_artifact",
        &string_array_at(&spec.artifact_policy, &["forbidden_artifacts"]),
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
    checks.extend(custom_evaluator_checks(spec, projection));
    checks.extend(usability_coherence_checks(spec, projection));
    checks.extend(state_machine_contract_checks(spec, projection));

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

fn forbidden_string_checks<'a>(
    label: &str,
    forbidden: &[String],
    values: impl Iterator<Item = &'a Value>,
) -> Vec<EvaluatorCheck> {
    let haystack = values
        .map(|value| serde_json::to_string(value).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    forbidden
        .iter()
        .map(|needle| {
            check(
                format!("{label}:{needle}"),
                !haystack.contains(needle),
                json!({ "absent": needle }),
                json!(haystack),
                format!("forbidden {label} evidence {needle} was present"),
            )
        })
        .collect()
}

fn custom_evaluator_checks(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> Vec<EvaluatorCheck> {
    spec.evaluator_checks
        .iter()
        .map(|definition| custom_evaluator_check(spec, projection, definition))
        .collect()
}

fn custom_evaluator_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    definition: &Value,
) -> EvaluatorCheck {
    let id = first_string(definition, &[&["id"], &["name"], &["assertion"]])
        .unwrap_or_else(|| "unnamed".to_string());
    let kind = first_string(definition, &[&["kind"]]).unwrap_or_else(|| "unknown".to_string());
    let assertion = first_string(definition, &[&["assertion"]]).unwrap_or_default();
    let required = definition
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let expected = definition.get("expected").cloned().unwrap_or(json!(true));
    let (passed, actual, message) =
        evaluate_custom_check(spec, projection, &kind, &assertion, &expected);
    check(
        format!("scenario_check:{id}"),
        passed || !required,
        json!({
            "kind": kind,
            "assertion": assertion,
            "expected": expected,
            "required": required,
        }),
        actual,
        if passed || required {
            message
        } else {
            format!("optional scenario check {id} did not pass: {message}")
        },
    )
}

fn evaluate_custom_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    kind: &str,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    match normalize_token(kind).as_str() {
        "goal_status" => {
            let passed = expected.as_str().is_some_and(|status| {
                normalize_token(status) == normalize_token(&projection.goal_status)
            });
            (
                passed,
                json!({ "goal_status": projection.goal_status }),
                format!(
                    "scenario check {assertion:?} expected {:?} got {}",
                    expected, projection.goal_status
                ),
            )
        }
        "determinism" => determinism_check(spec, assertion, expected),
        "artifact_policy" => artifact_policy_check(spec, projection, assertion, expected),
        "state_transition" => state_transition_check(spec, projection, assertion, expected),
        "continuation" => continuation_behavior_check(spec, projection, assertion, expected),
        "approval" => approval_behavior_check(spec, projection, assertion, expected),
        "event_dedupe" => event_dedupe_check(spec, assertion, expected),
        "projection_lineage" => projection_lineage_check(spec, projection, assertion, expected),
        "child_tasks" => child_task_check(spec, assertion, expected),
        "terminal_frontier" => terminal_frontier_check(spec, projection, assertion, expected),
        "budget" => budget_check(spec, assertion, expected),
        "recovery" => recovery_check(spec, projection, assertion, expected),
        "review_gate" => review_gate_check(spec, projection, assertion, expected),
        "review_round" => review_round_check(spec, projection, assertion, expected),
        "control_loop" => control_loop_check(spec, assertion, expected),
        "satisfaction" => satisfaction_check(spec, assertion, expected),
        "task_count" => task_count_check(projection, assertion, expected),
        "task_graph" => task_graph_check(spec, projection, assertion, expected),
        "graph_shape" => graph_shape_check(projection, assertion, expected),
        "research" => research_check(spec, projection, assertion, expected),
        "state_machine" => state_machine_custom_check(spec, projection, assertion, expected),
        "queue_history" => queue_history_check(spec, projection, assertion, expected),
        other => (
            false,
            json!({ "unsupported_kind": other, "assertion": assertion }),
            format!("unsupported scenario evaluator check kind {other:?}"),
        ),
    }
}

fn expected_bool(value: &Value) -> bool {
    value.as_bool().unwrap_or(true)
}

fn determinism_check(
    spec: &ScenarioSpec,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let credential_requests = if spec
        .determinism
        .get("live_provider_credentials_required")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    let network_calls = if spec
        .determinism
        .get("network_required")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    let pass = if expected.is_number() {
        expected.as_i64() == Some(i64::from(credential_requests))
    } else {
        (!assertion.contains("credential_requests") || credential_requests == 0)
            && (!assertion.contains("network_calls") || network_calls == 0)
    } == expected_bool(expected);
    (
        pass,
        json!({
            "credential_requests": credential_requests,
            "network_calls": network_calls,
            "live_provider_credentials_required": spec.determinism.get("live_provider_credentials_required"),
            "network_required": spec.determinism.get("network_required"),
        }),
        format!("determinism check {assertion:?} failed"),
    )
}

fn artifact_policy_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let required = string_array_at(&spec.expected_terminal_state, &["required_artifacts"]);
    let artifact_text = serde_json::to_string(&projection.artifacts).unwrap_or_default();
    let target = assertion_contains_target(assertion);
    let required_present = required.iter().all(|uri| {
        artifact_text.contains(uri) || projection_artifact_uris(projection).contains(uri)
    });
    let target_present = target
        .as_deref()
        .map(|needle| {
            artifact_text.contains(needle) || required.iter().any(|uri| uri.contains(needle))
        })
        .unwrap_or(required_present);
    let pass = (required_present && target_present) == expected_bool(expected);
    (
        pass,
        json!({
            "required_artifacts": required,
            "artifact_uris": projection_artifact_uris(projection),
            "assertion_target": target,
            "required_present": required_present,
            "target_present": target_present,
        }),
        format!("artifact policy check {assertion:?} failed"),
    )
}

fn state_transition_check(
    spec: &ScenarioSpec,
    _projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let blocked_index = first_action_index(spec, |action| {
        action_worker_result_values(action)
            .into_iter()
            .any(|result| {
                matches!(
                    normalize_token(&first_string(result, &[&["status"]]).unwrap_or_default())
                        .as_str(),
                    "blocked" | "waiting" | "waiting_input" | "waiting_approval"
                )
            })
    });
    let resume_index = first_action_index(spec, |action| {
        matches!(
            action.kind,
            ScenarioActionKind::ResumeThunk | ScenarioActionKind::ResumeDelayedCompute
        )
    });
    let done_index = first_action_index(spec, |action| {
        normalize_token(&action_name(action)).contains("validate")
            || action
                .expect
                .get("goal_status")
                .and_then(Value::as_str)
                .is_some_and(|status| is_completed_goal_status(status))
    });
    let ordered = blocked_index
        .zip(resume_index)
        .is_some_and(|(blocked, resume)| blocked < resume)
        && resume_index
            .zip(done_index)
            .is_some_and(|(resume, done)| resume < done);
    (
        ordered == expected_bool(expected),
        json!({
            "blocked_action_index": blocked_index,
            "resume_action_index": resume_index,
            "done_action_index": done_index,
            "transitions": spec.actions.iter().map(action_name).collect::<Vec<_>>(),
        }),
        format!("state transition check {assertion:?} failed"),
    )
}

fn continuation_behavior_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let open_wait_refs = expected_usize_at(&spec.expected_terminal_state, &["open_wait_refs"])
        .unwrap_or_else(|| {
            projection
                .compute_graph_nodes
                .iter()
                .filter(|node| {
                    node.get("kind").and_then(Value::as_str) == Some("delayed_compute_thunk")
                        && !matches!(
                            normalize_token(&text_field(node, "status")).as_str(),
                            "done" | "resumed" | "completed"
                        )
                })
                .count()
        });
    let resumed_thunks = projection
        .compute_graph_nodes
        .iter()
        .filter(|node| {
            node.get("kind").and_then(Value::as_str) == Some("delayed_compute_thunk")
                && matches!(
                    normalize_token(&text_field(node, "status")).as_str(),
                    "resumed" | "done" | "completed"
                )
        })
        .count();
    let pass = open_wait_refs == 0 && resumed_thunks >= 1;
    (
        pass == expected_bool(expected),
        json!({
            "open_wait_refs": open_wait_refs,
            "resumed_thunks": resumed_thunks,
            "continuation_ref": continuation_ref_from_scenario(spec, projection),
        }),
        format!("continuation check {assertion:?} failed"),
    )
}

fn approval_behavior_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let approval_refs = approval_request_refs(spec, projection);
    let approval_events = projection
        .events
        .iter()
        .filter_map(|event| {
            let event_type = text_field(event, "event_type");
            matches!(
                normalize_token(&event_type).as_str(),
                "approval_granted" | "approval_decided" | "approval_resolved"
            )
            .then_some(event_type)
        })
        .collect::<Vec<_>>();
    let resumed_approval_thunks = projection
        .compute_graph_nodes
        .iter()
        .filter(|node| {
            node.get("kind").and_then(Value::as_str) == Some("delayed_compute_thunk")
                && approval_ref_from_value(node).is_some()
                && matches!(
                    normalize_token(&text_field(node, "status")).as_str(),
                    "resumed" | "done" | "completed" | "approved"
                )
        })
        .count();
    let approval_resume_actions = spec
        .actions
        .iter()
        .filter(|action| is_approval_resume_action(action))
        .count();
    let requested_approved = normalize_token(assertion).contains("approved")
        || normalize_token(assertion).contains("approval_status_approved");
    let approved = !approval_refs.is_empty()
        && approval_resume_actions > 0
        && (!approval_events.is_empty() || resumed_approval_thunks > 0);
    let pass = if requested_approved {
        approved
    } else {
        !approval_refs.is_empty()
    };
    (
        pass == expected_bool(expected),
        json!({
            "approval_refs": approval_refs,
            "approval_events": approval_events,
            "approval_resume_actions": approval_resume_actions,
            "resumed_approval_thunks": resumed_approval_thunks,
        }),
        format!("approval behavior check {assertion:?} failed"),
    )
}

fn event_dedupe_check(
    spec: &ScenarioSpec,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let created_goals = expected_usize_at(&spec.expected_terminal_state, &["created_goals"])
        .or_else(|| max_expect_usize(spec, "created_goals"))
        .unwrap_or(0);
    let dedupe_hits = expected_usize_at(&spec.expected_terminal_state, &["dedupe_hits"])
        .or_else(|| max_expect_usize(spec, "dedupe_hits"))
        .unwrap_or(0);
    let pass = created_goals == 1 && dedupe_hits >= 1;
    (
        pass == expected_bool(expected),
        json!({
            "created_goals": created_goals,
            "dedupe_hits": dedupe_hits,
        }),
        format!("event dedupe check {assertion:?} failed"),
    )
}

fn projection_lineage_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let expected_text = expected.as_str().unwrap_or_default();
    let searchable = serde_json::to_string(&json!({
        "spec": spec,
        "projection": projection,
    }))
    .unwrap_or_default();
    let pass = !expected_text.is_empty() && searchable.contains(expected_text);
    (
        pass,
        json!({
            "expected_lineage": expected_text,
            "present": pass,
        }),
        format!("projection lineage check {assertion:?} failed"),
    )
}

fn child_task_check(
    spec: &ScenarioSpec,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let requested_children = child_request_count(spec);
    let created_children =
        expected_usize_at(&spec.expected_terminal_state, &["created_child_tasks"])
            .or_else(|| max_expect_usize(spec, "created_child_tasks"))
            .unwrap_or(requested_children);
    let native_subagents_spawned = max_expect_usize(spec, "native_subagents_spawned").unwrap_or(0);
    let pass = requested_children == created_children && native_subagents_spawned == 0;
    (
        pass == expected_bool(expected),
        json!({
            "requested_child_tasks": requested_children,
            "created_child_tasks": created_children,
            "native_subagents_spawned": native_subagents_spawned,
        }),
        format!("child task check {assertion:?} failed"),
    )
}

fn terminal_frontier_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let active = projection
        .tasks
        .iter()
        .filter(|task| {
            matches!(
                normalize_token(&text_field(task, "status")).as_str(),
                "pending"
                    | "runnable"
                    | "running"
                    | "needs_validation"
                    | "blocked"
                    | "waiting"
                    | "waiting_input"
                    | "waiting_approval"
            )
        })
        .count();
    let done_tasks = projection
        .tasks
        .iter()
        .filter(|task| normalize_token(&text_field(task, "status")) == "done")
        .count();
    let expected_done = expected_task_counts(spec)
        .get("done")
        .copied()
        .unwrap_or(done_tasks);
    let pass = active == 0 && done_tasks >= expected_done;
    (
        pass == expected_bool(expected),
        json!({
            "frontier_empty": active == 0,
            "active_tasks": active,
            "done_tasks": done_tasks,
            "expected_done_tasks": expected_done,
        }),
        format!("terminal frontier check {assertion:?} failed"),
    )
}

fn budget_check(spec: &ScenarioSpec, assertion: &str, expected: &Value) -> (bool, Value, String) {
    let created_children =
        expected_usize_at(&spec.expected_terminal_state, &["created_child_tasks"])
            .or_else(|| max_expect_usize(spec, "created_child_tasks"))
            .unwrap_or_else(|| child_request_count(spec));
    let max_child_tasks = spec
        .goals
        .first()
        .and_then(|goal| expected_usize_at(&goal.payload, &["root_budget", "max_child_tasks"]))
        .or_else(|| {
            spec.goals
                .first()
                .and_then(|goal| expected_usize_at(&goal.spec, &["root_budget", "max_child_tasks"]))
        })
        .unwrap_or(usize::MAX);
    let pass = created_children <= max_child_tasks;
    (
        pass == expected_bool(expected),
        json!({
            "created_child_tasks": created_children,
            "max_child_tasks": max_child_tasks,
        }),
        format!("budget check {assertion:?} failed"),
    )
}

fn recovery_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let blocked_results = spec
        .actions
        .iter()
        .flat_map(action_worker_result_values)
        .filter(|result| normalize_token(&text_field(result, "status")) == "blocked")
        .collect::<Vec<_>>();
    let recovery_actions = blocked_results
        .iter()
        .flat_map(|result| string_array(result.get("recovery_actions")))
        .map(|action| normalize_token(&action))
        .collect::<BTreeSet<_>>();
    let has_restart_or_retry =
        recovery_actions.contains("restart") || recovery_actions.contains("retry");
    let blocked_thunks = blocked_results
        .iter()
        .flat_map(|result| delayed_compute_thunks_from_value(result))
        .count();
    let done_tasks = projection
        .tasks
        .iter()
        .filter(|task| normalize_token(&text_field(task, "status")) == "done")
        .count();
    let expected_done = expected_task_counts(spec)
        .get("done")
        .copied()
        .unwrap_or(done_tasks);
    let pass = !blocked_results.is_empty()
        && has_restart_or_retry
        && blocked_thunks == 0
        && done_tasks >= expected_done;
    (
        pass == expected_bool(expected),
        json!({
            "blocked_results": blocked_results.len(),
            "recovery_actions": recovery_actions,
            "blocked_delayed_compute_thunks": blocked_thunks,
            "done_tasks": done_tasks,
            "expected_done_tasks": expected_done,
        }),
        format!("recovery check {assertion:?} failed"),
    )
}

fn review_gate_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let event_names = projection
        .events
        .iter()
        .map(|event| text_field(event, "event_type"))
        .collect::<Vec<_>>();
    let review_index = event_names
        .iter()
        .position(|event| event == "review_unification_completed")
        .or_else(|| first_action_index(spec, |action| action_name(action).contains("join")));
    let satisfied_index = event_names
        .iter()
        .position(|event| event == "goal_satisfied")
        .or_else(|| first_action_index(spec, |action| action_name(action).contains("validate")));
    let pass = review_index
        .zip(satisfied_index)
        .is_some_and(|(review, satisfied)| satisfied > review);
    (
        pass == expected_bool(expected),
        json!({
            "review_unification_index": review_index,
            "goal_satisfied_index": satisfied_index,
            "event_names": event_names,
        }),
        format!("review gate check {assertion:?} failed"),
    )
}

fn review_round_check(
    spec: &ScenarioSpec,
    _projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let completed_review_tasks = review_result_count(spec);
    let created_unifier_tasks =
        expected_usize_at(&spec.expected_terminal_state, &["unifier_tasks"])
            .or_else(|| max_expect_usize(spec, "created_unifier_tasks"))
            .unwrap_or(0);
    let pass = created_unifier_tasks == 1 && completed_review_tasks >= 2;
    (
        pass == expected_bool(expected),
        json!({
            "created_unifier_tasks": created_unifier_tasks,
            "completed_review_tasks": completed_review_tasks,
        }),
        format!("review round check {assertion:?} failed"),
    )
}

fn control_loop_check(
    spec: &ScenarioSpec,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let attempts_used = expected_usize_at(&spec.expected_terminal_state, &["attempts_used"])
        .unwrap_or_else(|| iteration_attempts(spec).len());
    let max_attempts = expected_usize_at(&spec.expected_terminal_state, &["max_attempts"])
        .or_else(|| {
            spec.setup
                .get("stub_projections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find_map(|item| expected_usize_at(item, &["value", "max_attempts"]))
        })
        .unwrap_or(attempts_used);
    let pass = attempts_used > 0 && attempts_used < max_attempts;
    (
        pass == expected_bool(expected),
        json!({
            "attempts_used": attempts_used,
            "max_attempts": max_attempts,
        }),
        format!("control loop check {assertion:?} failed"),
    )
}

fn satisfaction_check(
    spec: &ScenarioSpec,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let actual_score = number_at(
        &spec.expected_terminal_state,
        &["satisfaction", "actual_score"],
    )
    .or_else(|| {
        number_at(
            &spec.expected_terminal_state,
            &["satisfaction", "min_score"],
        )
    })
    .unwrap_or(0.0);
    let validator_score_min = spec
        .goals
        .first()
        .and_then(|goal| number_at(&goal.spec, &["done_criteria", "validator_score_min"]))
        .or_else(|| {
            spec.goals.first().and_then(|goal| {
                number_at(&goal.payload, &["done_criteria", "validator_score_min"])
            })
        })
        .unwrap_or(0.0);
    let retry_created_after_terminal = iteration_attempts(spec)
        .last()
        .and_then(|attempt| attempt.get("retry_created").and_then(Value::as_bool))
        .unwrap_or(false);
    let pass = actual_score >= validator_score_min && !retry_created_after_terminal;
    (
        pass == expected_bool(expected),
        json!({
            "actual_score": actual_score,
            "validator_score_min": validator_score_min,
            "retry_created_after_terminal": retry_created_after_terminal,
        }),
        format!("satisfaction check {assertion:?} failed"),
    )
}

fn task_count_check(
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let status = if normalize_token(assertion).contains("done") {
        "done"
    } else if normalize_token(assertion).contains("blocked") {
        "blocked"
    } else if normalize_token(assertion).contains("failed") {
        "failed"
    } else {
        "all"
    };
    let actual = projection
        .tasks
        .iter()
        .filter(|task| status == "all" || normalize_token(&text_field(task, "status")) == status)
        .count();
    let expected_count = expected
        .as_u64()
        .map(|value| value as usize)
        .unwrap_or(actual);
    (
        actual == expected_count,
        json!({
            "status": status,
            "actual": actual,
            "expected": expected_count,
        }),
        format!("task count check {assertion:?} failed"),
    )
}

fn task_graph_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let done_tasks = projection
        .tasks
        .iter()
        .filter(|task| normalize_token(&text_field(task, "status")) == "done")
        .count();
    let expected_done = expected_task_counts(spec)
        .get("done")
        .copied()
        .unwrap_or(done_tasks);
    let review_rounds = expected_usize_at(&spec.expected_terminal_state, &["review_rounds"])
        .unwrap_or_else(|| review_result_count(spec));
    let pass = done_tasks >= expected_done && review_rounds >= 1;
    (
        pass == expected_bool(expected),
        json!({
            "done_tasks": done_tasks,
            "expected_done_tasks": expected_done,
            "review_rounds": review_rounds,
            "compute_graph_nodes": projection.compute_graph_nodes.len(),
        }),
        format!("task graph check {assertion:?} failed"),
    )
}

fn graph_shape_check(
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let normalized = normalize_token(assertion);
    let target = if normalized.contains("review_unifier") {
        "review_unifier"
    } else if normalized.contains("fanout") {
        "fanout"
    } else if normalized.contains("thunk") {
        "delayed_compute_thunk"
    } else {
        ""
    };
    let contains_target = !target.is_empty()
        && projection.compute_graph_nodes.iter().any(|node| {
            normalize_token(&text_field(node, "kind")) == target || value_contains(node, target)
        });
    (
        contains_target == expected_bool(expected),
        json!({
            "target": target,
            "contains_target": contains_target,
            "compute_graph_nodes": projection.compute_graph_nodes,
        }),
        format!("graph shape check {assertion:?} failed"),
    )
}

fn research_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let events = projection
        .events
        .iter()
        .map(|event| text_field(event, "event_type"))
        .collect::<Vec<_>>();
    let artifact_text = projection
        .artifacts
        .iter()
        .map(|artifact| serde_json::to_string(artifact).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let task_text = spec
        .actions
        .iter()
        .flat_map(action_worker_result_values)
        .map(|result| serde_json::to_string(&result).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let has_sources = events
        .iter()
        .any(|event| event == "research_sources_captured")
        || artifact_text.contains("research-output")
        || task_text.contains("sources");
    let has_memory_write = events.iter().any(|event| event == "memory_write_proposed")
        || artifact_text.contains("memory-write")
        || task_text.contains("proposed_memory");
    let pass = has_sources && has_memory_write;
    (
        pass == expected_bool(expected),
        json!({
            "has_sources": has_sources,
            "has_memory_write": has_memory_write,
            "events": events,
            "artifact_text": artifact_text,
        }),
        format!("research check {assertion:?} failed"),
    )
}

fn state_machine_custom_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let checks = state_machine_contract_checks(spec, projection);
    let failed = checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.name.clone())
        .collect::<Vec<_>>();
    (
        failed.is_empty() == expected_bool(expected),
        json!({ "failed_checks": failed, "check_count": checks.len() }),
        format!("state machine check {assertion:?} failed"),
    )
}

fn queue_history_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
    assertion: &str,
    expected: &Value,
) -> (bool, Value, String) {
    let events = queue_history_events(spec);
    let has_dispatch = events
        .iter()
        .any(|event| event == "queued" || event == "dispatched");
    let has_cancel = events
        .iter()
        .any(|event| event == "cancel_requested" || event == "task_cancelled");
    let has_drain = events.iter().any(|event| event == "dispatch_drained");
    let cancelled = is_cancelled_goal_status(&projection.goal_status)
        || is_cancelled_goal_status(
            first_string(&spec.expected_terminal_state, &[&["goal_status"]])
                .unwrap_or_default()
                .as_str(),
        );
    let pass = cancelled && has_dispatch && has_cancel && has_drain;
    (
        pass == expected_bool(expected),
        json!({
            "goal_status": projection.goal_status,
            "queue_history_events": events,
            "has_dispatch": has_dispatch,
            "has_cancel": has_cancel,
            "has_drain": has_drain,
        }),
        format!("queue history check {assertion:?} failed"),
    )
}

fn assertion_contains_target(assertion: &str) -> Option<String> {
    let (_, target) = assertion.split_once("contains")?;
    let target = target.trim().trim_matches('"').trim_matches('\'');
    if target.is_empty() {
        None
    } else {
        Some(target.to_string())
    }
}

fn projection_artifact_uris(projection: &ScenarioProjection) -> BTreeSet<String> {
    projection
        .artifacts
        .iter()
        .filter_map(|artifact| first_string(artifact, &[&["uri"]]))
        .collect()
}

fn first_action_index(
    spec: &ScenarioSpec,
    mut predicate: impl FnMut(&ScenarioAction) -> bool,
) -> Option<usize> {
    spec.actions.iter().position(|action| predicate(action))
}

fn expected_usize_at(value: &Value, path: &[&str]) -> Option<usize> {
    value_at_path(value, path)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn max_expect_usize(spec: &ScenarioSpec, key: &str) -> Option<usize> {
    spec.actions
        .iter()
        .filter_map(|action| expected_usize_at(&action.expect, &[key]))
        .max()
}

fn number_at(value: &Value, path: &[&str]) -> Option<f64> {
    value_at_path(value, path).and_then(Value::as_f64)
}

fn child_request_count(spec: &ScenarioSpec) -> usize {
    spec.actions
        .iter()
        .flat_map(action_worker_result_values)
        .map(|result| first_array(result, &[&["child_requests"], &["child_task_requests"]]).len())
        .sum()
}

fn review_result_count(spec: &ScenarioSpec) -> usize {
    spec.actions
        .iter()
        .flat_map(action_worker_result_values)
        .filter(|result| result.get("review").is_some() || text_field(result, "role") == "reviewer")
        .count()
}

fn iteration_attempts(spec: &ScenarioSpec) -> Vec<&Value> {
    spec.actions
        .iter()
        .filter_map(|action| {
            if matches!(action.kind, ScenarioActionKind::IterationFixture)
                && !action.attempt.is_null()
            {
                Some(&action.attempt)
            } else {
                None
            }
        })
        .collect()
}

fn queue_history_events(spec: &ScenarioSpec) -> BTreeSet<String> {
    let mut events = BTreeSet::new();
    for action in &spec.actions {
        for source in [&action.payload, &action.body, &action.expect] {
            for item in first_array(source, &[&["queue_history"]]) {
                if let Some(event) = first_string(&item, &[&["event"]]) {
                    events.insert(event);
                }
            }
        }
    }
    events
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
            action_worker_result_values(action)
                .into_iter()
                .any(|result| {
                    matches!(
                        normalize_token(&first_string(result, &[&["status"]]).unwrap_or_default())
                            .as_str(),
                        "blocked" | "waiting" | "waiting_input" | "waiting_approval"
                    ) || !delayed_compute_thunks_from_value(result).is_empty()
                })
        })
}

fn blocked_operator_action(spec: &ScenarioSpec, projection: &ScenarioProjection) -> Option<String> {
    scenario_requirement_values(spec, projection)
        .into_iter()
        .find_map(operator_action_from_value)
        .or_else(|| plain_blocked_recovery_action(spec, projection))
}

const REQUIREMENT_SEARCH_DEPTH: usize = 3;

const OPERATOR_ACTION_TEXT_PATHS: &[&[&str]] = &[
    &["operator_action"],
    &["required_operator_action"],
    &["action_required"],
    &["resume_instruction"],
    &["next_action"],
    &["wait", "operator_action"],
    &["blocked", "operator_action"],
];

const OPERATOR_ACTION_LIST_PATHS: &[&[&str]] = &[
    &["next_actions"],
    &["operator_actions"],
    &["required_operator_actions"],
    &["allowed_actions"],
    &["recovery_actions"],
    &["available_recovery_actions"],
    &["allowed_recovery_actions"],
];

const OPERATOR_ACTION_NESTED_PATHS: &[&[&str]] = &[
    &["delayed_compute_thunks"],
    &["delayed_compute", "thunks"],
    &["approval_requests"],
    &["human_prompts"],
    &["operator_prompts"],
    &["human_prompt"],
    &["operator_prompt"],
    &["approval_request"],
    &["wait"],
    &["blocked"],
];

const RECOVERY_ACTION_TEXT_PATHS: &[&[&str]] = &[
    &["recovery_action"],
    &["recovery", "action"],
    &["blocked", "recovery_action"],
    &["action"],
    &["kind"],
    &["type"],
    &["key"],
    &["id"],
    &["label"],
    &["title"],
    &["operator_action"],
    &["description"],
];

const RECOVERY_ACTION_LIST_PATHS: &[&[&str]] = &[
    &["recovery_actions"],
    &["available_recovery_actions"],
    &["allowed_recovery_actions"],
    &["recovery", "actions"],
    &["allowed_actions"],
    &["next_actions"],
    &["operator_actions"],
    &["required_operator_actions"],
];

const RECOVERY_ACTION_NESTED_PATHS: &[&[&str]] = &[
    &["blocked"],
    &["wait"],
    &["recovery"],
    &["human_prompt"],
    &["operator_prompt"],
];

const CONTINUATION_REF_PATHS: &[&[&str]] = &[
    &["continuation_ref"],
    &["continuationRef"],
    &["continuation"],
    &["resume", "continuation_ref"],
    &["resume", "continuation"],
    &["human_prompt", "continuation_ref"],
    &["operator_prompt", "continuation_ref"],
    &["approval_request", "continuation_ref"],
    &["wait", "continuation_ref"],
];

const CONTINUATION_REF_NESTED_PATHS: &[&[&str]] = &[
    &["delayed_compute_thunks"],
    &["delayed_compute", "thunks"],
    &["approval_requests"],
    &["human_prompts"],
    &["operator_prompts"],
];

const APPROVAL_REF_PATHS: &[&[&str]] = &[
    &["approval_ref"],
    &["approvalRef"],
    &["approval", "ref"],
    &["approval", "approval_ref"],
    &["approval_request", "approval_ref"],
    &["resume", "approval_ref"],
];

const APPROVAL_REF_NESTED_PATHS: &[&[&str]] = &[
    &["delayed_compute_thunks"],
    &["delayed_compute", "thunks"],
    &["approval_requests"],
    &["approval_request"],
];

fn operator_action_from_value(value: &Value) -> Option<String> {
    find_requirement_text(
        value,
        OPERATOR_ACTION_TEXT_PATHS,
        OPERATOR_ACTION_LIST_PATHS,
        OPERATOR_ACTION_NESTED_PATHS,
        false,
        is_concrete_operator_action,
    )
}

fn is_concrete_operator_action(text: &str) -> bool {
    let trimmed = text.trim();
    is_operator_control_action_text(trimmed)
        || (trimmed.split_whitespace().count() >= 2 && trimmed.chars().any(char::is_alphabetic))
}

fn is_operator_control_action_text(text: &str) -> bool {
    matches!(
        normalize_action_key(text).as_str(),
        "answer"
            | "continue"
            | "approve"
            | "reject"
            | "add_context"
            | "retry"
            | "restart"
            | "replan"
            | "steer"
            | "cancel"
            | "create_human_prompt"
            | "create_operator_prompt"
            | "create_delayed_compute_thunk"
            | "create_thunk"
    )
}

fn is_plain_blocked_recovery_action_text(text: &str) -> bool {
    let key = normalize_action_key(text);
    matches!(
        key.as_str(),
        "retry"
            | "restart"
            | "replan"
            | "steer"
            | "cancel"
            | "create_human_prompt"
            | "create_operator_prompt"
            | "create_delayed_compute_thunk"
            | "create_thunk"
    ) || key.contains("retry")
        || key.contains("restart")
        || key.contains("replan")
        || key.contains("steer")
        || key.contains("cancel")
        || key.contains("create_human_prompt")
        || key.contains("create_operator_prompt")
        || key.contains("create_delayed_compute_thunk")
        || key.contains("create_thunk")
}

fn normalize_action_key(value: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_separator = false;
    for ch in value.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            last_was_separator = false;
        } else if !last_was_separator && !normalized.is_empty() {
            normalized.push('_');
            last_was_separator = true;
        }
    }
    while normalized.ends_with('_') {
        normalized.pop();
    }
    normalized
}

fn validate_action_required_contract(spec: &ScenarioSpec) -> anyhow::Result<()> {
    let projection = &spec.fixtures.projection;

    if action_required_wait_present(spec, projection) {
        let mut missing = Vec::new();
        if blocked_operator_action(spec, projection).is_none() {
            missing.push("concrete recovery action");
        }
        if continuation_ref_from_scenario(spec, projection).is_none() {
            missing.push("continuation_ref");
        }
        if !missing.is_empty() {
            bail!(
                "scenario {} has an action-required state but is missing {}",
                spec.id,
                missing.join(" and ")
            );
        }
    }

    if plain_blocked_without_thunk_present(spec, projection)
        && plain_blocked_recovery_action(spec, projection).is_none()
    {
        bail!(
            "scenario {} has a plain blocked task without a delayed compute thunk but no retry/replan/cancel/create-human-prompt recovery action",
            spec.id
        );
    }
    Ok(())
}

fn validate_state_machine_contract(spec: &ScenarioSpec) -> anyhow::Result<()> {
    let projection = &spec.fixtures.projection;
    let failed = state_machine_contract_checks(spec, projection)
        .into_iter()
        .filter(|check| !check.passed)
        .map(|check| check.message)
        .collect::<Vec<_>>();
    if !failed.is_empty() {
        bail!(
            "scenario {} violates state-machine contract: {}",
            spec.id,
            failed.join("; ")
        );
    }
    Ok(())
}

fn state_machine_contract_checks(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> Vec<EvaluatorCheck> {
    vec![
        action_needed_state_machine_check(spec, projection),
        resumable_wait_continuation_check(spec, projection),
        approval_ref_integrity_check(spec, projection),
        terminal_state_contract_check(spec, projection),
    ]
}

fn action_needed_state_machine_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> EvaluatorCheck {
    let action_needed = action_needed_state_present(spec, projection);
    let action = blocked_operator_action(spec, projection);
    check(
        "state_machine_action_needed_has_operator_action",
        !action_needed || action.is_some(),
        json!("concrete operator action for every blocked/waiting/failed/budget-exhausted state"),
        json!({
            "action_needed_state_present": action_needed,
            "operator_action": action,
        }),
        "blocked, waiting, failed, or budget-exhausted task graph state requires a concrete coordinator action".to_string(),
    )
}

fn resumable_wait_continuation_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> EvaluatorCheck {
    let resumable_wait = resumable_wait_present(spec, projection);
    let continuation_ref = continuation_ref_from_scenario(spec, projection);
    check(
        "state_machine_resumable_wait_has_continuation_ref",
        !resumable_wait || continuation_ref.is_some(),
        json!("continuation_ref for waiting/waiting_input/waiting_approval or thunk-backed waits"),
        json!({
            "resumable_wait_present": resumable_wait,
            "continuation_ref": continuation_ref,
        }),
        "resumable waiting state requires a concrete continuation_ref".to_string(),
    )
}

fn approval_ref_integrity_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> EvaluatorCheck {
    let request_refs = approval_request_refs(spec, projection);
    let resume_refs = approval_resume_refs(spec);
    let missing_resume_refs = approval_resume_actions_missing_refs(spec);
    let approval_wait = approval_wait_present(spec, projection);
    let missing_request_ref = approval_wait && request_refs.is_empty();
    let unmatched_resume_refs = resume_refs
        .difference(&request_refs)
        .cloned()
        .collect::<Vec<_>>();
    check(
        "state_machine_approval_refs_valid",
        !missing_request_ref && missing_resume_refs == 0 && unmatched_resume_refs.is_empty(),
        json!(
            "approval waits carry approval_ref and approval resumes reference an existing approval request"
        ),
        json!({
            "approval_wait_present": approval_wait,
            "approval_request_refs": request_refs,
            "approval_resume_refs": resume_refs,
            "approval_resume_actions_missing_refs": missing_resume_refs,
            "unmatched_resume_refs": unmatched_resume_refs,
        }),
        "approval wait/resume state requires concrete matching approval_ref values".to_string(),
    )
}

fn terminal_state_contract_check(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> EvaluatorCheck {
    let goal_status = expected_or_projected_goal_status(spec, projection);
    let terminal_state = expected_or_projected_terminal_state(spec, projection);
    let task_counts = expected_task_counts(spec);
    let active_terminal_misuse_counts = task_counts
        .iter()
        .filter(|(status, count)| **count > 0 && is_recoverable_action_needed_status(status))
        .map(|(status, count)| json!({ "status": status, "count": count }))
        .collect::<Vec<_>>();
    let terminal_goal =
        is_completed_goal_status(&goal_status) || is_cancelled_goal_status(&goal_status);
    let terminal_state_is_failed = matches!(
        normalize_terminal(&terminal_state).as_str(),
        "failed" | "budget_exhausted"
    );
    let failed_goal_marked_terminal = matches!(
        normalize_token(&goal_status).as_str(),
        "failed" | "budget_exhausted"
    );
    let terminal_goal_with_active_action_needed = terminal_goal
        && (!active_terminal_misuse_counts.is_empty()
            || projection
                .tasks
                .iter()
                .any(|task| is_recoverable_action_needed_status(&text_field(task, "status"))));
    check(
        "state_machine_terminal_state_contract",
        !terminal_state_is_failed && !failed_goal_marked_terminal && !terminal_goal_with_active_action_needed,
        json!("only done/satisfied and cancelled are terminal; action-needed states stay recoverable"),
        json!({
            "goal_status": goal_status,
            "terminal_state": terminal_state,
            "active_action_needed_task_counts": active_terminal_misuse_counts,
            "terminal_goal_with_action_needed_tasks": terminal_goal_with_active_action_needed,
        }),
        "terminal state misuse: failed/budget-exhausted are recoverable states, and completed/cancelled goals cannot retain action-needed tasks".to_string(),
    )
}

fn action_needed_state_present(spec: &ScenarioSpec, projection: &ScenarioProjection) -> bool {
    is_recoverable_action_needed_status(&projection.goal_status)
        || is_recoverable_action_needed_status(&projection.terminal_state)
        || projection
            .tasks
            .iter()
            .any(|task| is_recoverable_action_needed_status(&text_field(task, "status")))
        || spec.actions.iter().any(|action| {
            action_worker_result_values(action)
                .into_iter()
                .any(|result| {
                    is_recoverable_action_needed_status(
                        &first_string(result, &[&["status"]]).unwrap_or_default(),
                    )
                })
        })
}

fn resumable_wait_present(spec: &ScenarioSpec, projection: &ScenarioProjection) -> bool {
    action_required_wait_present(spec, projection)
        || projection
            .tasks
            .iter()
            .any(|task| is_resumable_wait_status(&text_field(task, "status")))
}

fn is_resumable_wait_status(status: &str) -> bool {
    matches!(
        normalize_token(status).as_str(),
        "waiting" | "waiting_input" | "waiting_approval"
    )
}

fn approval_wait_present(spec: &ScenarioSpec, projection: &ScenarioProjection) -> bool {
    approval_request_values(spec, projection)
        .into_iter()
        .any(value_is_approval_wait)
}

fn approval_request_refs(spec: &ScenarioSpec, projection: &ScenarioProjection) -> BTreeSet<String> {
    approval_request_values(spec, projection)
        .into_iter()
        .filter_map(approval_ref_from_value)
        .collect()
}

fn approval_request_values<'a>(
    spec: &'a ScenarioSpec,
    projection: &'a ScenarioProjection,
) -> Vec<&'a Value> {
    let mut values = Vec::new();
    for action in &spec.actions {
        values.extend(action_worker_result_values(action));
    }
    values.extend(projection_requirement_values(projection));
    values
}

fn approval_resume_refs(spec: &ScenarioSpec) -> BTreeSet<String> {
    spec.actions
        .iter()
        .filter(|action| is_approval_resume_action(action))
        .filter_map(|action| approval_ref_from_value(&action.resume))
        .collect()
}

fn approval_resume_actions_missing_refs(spec: &ScenarioSpec) -> usize {
    spec.actions
        .iter()
        .filter(|action| is_approval_resume_action(action))
        .filter(|action| approval_ref_from_value(&action.resume).is_none())
        .count()
}

fn is_approval_resume_action(action: &ScenarioAction) -> bool {
    matches!(action.kind, ScenarioActionKind::Approve)
        || matches!(
            action.kind,
            ScenarioActionKind::ResumeThunk | ScenarioActionKind::ResumeDelayedCompute
        ) && (approval_ref_from_value(&action.resume).is_some()
            || normalize_action_key(
                &first_string(&action.resume, &[&["action"], &["decision"], &["kind"]])
                    .unwrap_or_default(),
            ) == "approve")
}

fn value_is_approval_wait(value: &Value) -> bool {
    normalize_token(&first_string(value, &[&["status"]]).unwrap_or_default()) == "waiting_approval"
        || value_contains(value, "\"kind\":\"approval\"")
        || value_contains(value, "\"kind\": \"approval\"")
        || value.get("approval_request").is_some()
        || value
            .get("approval_requests")
            .and_then(Value::as_array)
            .is_some()
}

fn approval_ref_from_value(value: &Value) -> Option<String> {
    approval_ref_from_value_depth(value, 0)
}

fn approval_ref_from_value_depth(value: &Value, depth: usize) -> Option<String> {
    for path in APPROVAL_REF_PATHS {
        if let Some(reference) = value_at_path(value, path).and_then(continuation_ref_text) {
            return Some(reference);
        }
    }
    if depth < REQUIREMENT_SEARCH_DEPTH {
        for path in APPROVAL_REF_NESTED_PATHS {
            if let Some(item) = value_at_path(value, path) {
                if let Some(items) = item.as_array() {
                    for item in items {
                        if let Some(reference) = approval_ref_from_value_depth(item, depth + 1) {
                            return Some(reference);
                        }
                    }
                } else if let Some(reference) = approval_ref_from_value_depth(item, depth + 1) {
                    return Some(reference);
                }
            }
        }
    }
    None
}

fn expected_or_projected_goal_status(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> String {
    first_string(
        &spec.expected_terminal_state,
        &[&["goal_status"], &["status"]],
    )
    .unwrap_or_else(|| projection.goal_status.clone())
}

fn expected_or_projected_terminal_state(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> String {
    first_string(
        &spec.expected_terminal_state,
        &[&["terminal_state"], &["workflow_status"]],
    )
    .unwrap_or_else(|| projection.terminal_state.clone())
}

fn expected_task_counts(spec: &ScenarioSpec) -> BTreeMap<String, usize> {
    spec.expected_terminal_state
        .get("task_counts")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|counts| counts.iter())
        .filter_map(|(status, count)| {
            count
                .as_u64()
                .map(|count| (status.clone(), usize::try_from(count).unwrap_or(usize::MAX)))
        })
        .collect()
}

fn is_recoverable_action_needed_status(status: &str) -> bool {
    matches!(
        normalize_token(status).as_str(),
        "blocked"
            | "waiting"
            | "waiting_input"
            | "waiting_approval"
            | "failed"
            | "budget_exhausted"
    )
}

fn is_completed_goal_status(status: &str) -> bool {
    matches!(
        normalize_token(status).as_str(),
        "done" | "satisfied" | "completed"
    )
}

fn is_cancelled_goal_status(status: &str) -> bool {
    matches!(normalize_token(status).as_str(), "cancelled" | "canceled")
}

fn action_required_wait_present(spec: &ScenarioSpec, projection: &ScenarioProjection) -> bool {
    spec.expectations.action_required_expected
        || spec_has_action_required_wait(spec)
        || projection_requirement_values(projection)
            .into_iter()
            .any(value_has_action_required_marker)
}

fn spec_has_action_required_wait(spec: &ScenarioSpec) -> bool {
    spec.actions
        .iter()
        .flat_map(|action| action_worker_result_values(action))
        .any(action_required_wait_value)
}

fn action_required_wait_value(value: &Value) -> bool {
    let status = normalize_token(&first_string(value, &[&["status"]]).unwrap_or_default());
    matches!(
        status.as_str(),
        "waiting" | "waiting_input" | "waiting_approval"
    ) || !delayed_compute_thunks_from_value(value).is_empty()
        || value_has_action_required_marker(value)
}

fn value_has_action_required_marker(value: &Value) -> bool {
    value_contains(value, "action_required")
        || value_contains(value, "delayed_compute_thunk")
        || value_contains(value, "waiting_input")
        || value_contains(value, "waiting_approval")
}

fn plain_blocked_without_thunk_present(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> bool {
    spec.actions
        .iter()
        .flat_map(|action| action_worker_result_values(action))
        .any(plain_blocked_without_thunk_value)
        || projection
            .tasks
            .iter()
            .any(plain_blocked_without_thunk_value)
}

fn plain_blocked_without_thunk_value(value: &Value) -> bool {
    normalize_token(&text_field(value, "status")) == "blocked"
        && delayed_compute_thunks_from_value(value).is_empty()
        && !value_has_action_required_marker(value)
}

fn plain_blocked_recovery_action(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> Option<String> {
    scenario_requirement_values(spec, projection)
        .into_iter()
        .find_map(plain_blocked_recovery_action_from_value)
}

fn plain_blocked_recovery_action_from_value(value: &Value) -> Option<String> {
    find_requirement_text(
        value,
        RECOVERY_ACTION_TEXT_PATHS,
        RECOVERY_ACTION_LIST_PATHS,
        RECOVERY_ACTION_NESTED_PATHS,
        true,
        is_plain_blocked_recovery_action_text,
    )
}

fn continuation_ref_from_scenario(
    spec: &ScenarioSpec,
    projection: &ScenarioProjection,
) -> Option<String> {
    scenario_requirement_values(spec, projection)
        .into_iter()
        .find_map(continuation_ref_from_value)
}

fn continuation_ref_from_value(value: &Value) -> Option<String> {
    continuation_ref_from_value_depth(value, 0)
}

fn continuation_ref_from_value_depth(value: &Value, depth: usize) -> Option<String> {
    for path in CONTINUATION_REF_PATHS {
        if let Some(reference) = value_at_path(value, path).and_then(continuation_ref_text) {
            return Some(reference);
        }
    }
    if depth < REQUIREMENT_SEARCH_DEPTH {
        for path in CONTINUATION_REF_NESTED_PATHS {
            if let Some(items) = value_at_path(value, path).and_then(Value::as_array) {
                for item in items {
                    if let Some(reference) = continuation_ref_from_value_depth(item, depth + 1) {
                        return Some(reference);
                    }
                }
            }
        }
    }
    None
}

fn continuation_ref_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().filter(|text| is_concrete_ref(text)) {
        return Some(text.to_string());
    }
    first_string(
        value,
        &[
            &["ref"],
            &["id"],
            &["uri"],
            &["token_ref"],
            &["resume_token_ref"],
            &["continuation_id"],
        ],
    )
    .filter(|text| is_concrete_ref(text))
}

fn is_concrete_ref(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && (trimmed.contains("://")
            || trimmed
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .count()
                >= 6)
}

fn find_requirement_text(
    value: &Value,
    text_paths: &[&[&str]],
    list_paths: &[&[&str]],
    nested_paths: &[&[&str]],
    include_current_value: bool,
    predicate: fn(&str) -> bool,
) -> Option<String> {
    find_requirement_text_depth(
        value,
        text_paths,
        list_paths,
        nested_paths,
        include_current_value,
        predicate,
        0,
    )
}

fn find_requirement_text_depth(
    value: &Value,
    text_paths: &[&[&str]],
    list_paths: &[&[&str]],
    nested_paths: &[&[&str]],
    include_current_value: bool,
    predicate: fn(&str) -> bool,
    depth: usize,
) -> Option<String> {
    if include_current_value && let Some(text) = value.as_str().filter(|text| predicate(text)) {
        return Some(text.to_string());
    }
    for path in text_paths {
        if let Some(text) = first_string(value, &[path]).filter(|text| predicate(text)) {
            return Some(text);
        }
    }
    if depth >= REQUIREMENT_SEARCH_DEPTH {
        return None;
    }
    for path in list_paths {
        if let Some(items) = value_at_path(value, path).and_then(Value::as_array) {
            for item in items {
                if let Some(text) = find_requirement_text_depth(
                    item,
                    text_paths,
                    list_paths,
                    nested_paths,
                    true,
                    predicate,
                    depth + 1,
                ) {
                    return Some(text);
                }
            }
        }
    }
    for path in nested_paths {
        if let Some(item) = value_at_path(value, path) {
            if let Some(items) = item.as_array() {
                for item in items {
                    if let Some(text) = find_requirement_text_depth(
                        item,
                        text_paths,
                        list_paths,
                        nested_paths,
                        include_current_value,
                        predicate,
                        depth + 1,
                    ) {
                        return Some(text);
                    }
                }
            } else if let Some(text) = find_requirement_text_depth(
                item,
                text_paths,
                list_paths,
                nested_paths,
                include_current_value,
                predicate,
                depth + 1,
            ) {
                return Some(text);
            }
        }
    }
    None
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
        } else if is_scenario_file(&path) {
            paths.push(path);
        }
    }
    Ok(())
}

fn is_scenario_file(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("json")
        && !is_invalid_scenario_fixture(path)
}

fn is_invalid_scenario_fixture(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".invalid.json"))
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
    validate_action_required_contract(spec)?;
    validate_state_machine_contract(spec)?;
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
    if spec_has_action_required_wait(spec) {
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
    if spec_has_action_required_wait(spec) {
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
            .flat_map(|action| action_worker_result_values(action))
            .find_map(operator_action_from_value),
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
    let mut seen = BTreeMap::new();
    for action in &spec.actions {
        for result in action_worker_results(action) {
            let task_id = first_string(&result, &[&["task_id"]])
                .or_else(|| {
                    if action.task_id.trim().is_empty() {
                        None
                    } else {
                        Some(action.task_id.clone())
                    }
                })
                .unwrap_or_else(|| format!("{}-task-{}", spec.id, tasks.len() + 1));
            let status = task_status_from_worker_result(&result);
            let task = json!({
                "goal_id": goal_id,
                "task_id": task_id,
                "title": first_string(&result, &[&["summary"]]).unwrap_or_else(|| action_name(action)),
                "status": status,
                "purpose": task_purpose_from_action(action),
                "role": first_string(&result, &[&["role"], &["worker_kind"]]).unwrap_or_else(|| "codex".to_string()),
                "worker_result": result,
            });
            if let Some(existing) = seen.get(&task_id).copied() {
                tasks[existing] = task;
            } else {
                seen.insert(task_id, tasks.len());
                tasks.push(task);
            }
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

fn task_status_from_worker_result(result: &Value) -> String {
    match normalize_token(&first_string(result, &[&["status"]]).unwrap_or_default()).as_str() {
        "waiting" | "waiting_input" | "waiting_approval" => "blocked".to_string(),
        "blocked" => "blocked".to_string(),
        "failed" => "failed".to_string(),
        "cancelled" | "canceled" => "cancelled".to_string(),
        "needs_validation" => "needs_validation".to_string(),
        "running" => "running".to_string(),
        "runnable" => "runnable".to_string(),
        "pending" => "pending".to_string(),
        _ => "done".to_string(),
    }
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
        let mut event = json!({
            "event_type": action_name(action),
            "scenario_action": action.id,
            "goal_id": action.goal_ref,
        });
        if let Some(record) = event.as_object_mut() {
            if !action.event.is_null() {
                record.insert("event".to_string(), event_body(spec, action));
            }
            if !action.payload.is_null() {
                record.insert("payload".to_string(), action.payload.clone());
            }
            if !action.expect.is_null() {
                record.insert("expect".to_string(), action.expect.clone());
            }
            if !action.attempt.is_null() {
                record.insert("attempt".to_string(), action.attempt.clone());
            }
        }
        events.push(event);
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
        artifacts.extend(action.artifacts.iter().cloned());
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
    nodes.extend(delayed_compute_nodes_from_scenario(spec, goal_id));
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

fn delayed_compute_nodes_from_scenario(spec: &ScenarioSpec, goal_id: &str) -> Vec<Value> {
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();
    for action in &spec.actions {
        for result in action_worker_results(action) {
            for thunk in delayed_compute_thunks_from_value(&result) {
                let thunk_id = first_string(&thunk, &[&["thunk_id"], &["id"]])
                    .unwrap_or_else(|| format!("{}-thunk-{}", spec.id, nodes.len() + 1));
                if !seen.insert(thunk_id.clone()) {
                    continue;
                }
                let continuation_ref = continuation_ref_from_value(&thunk)
                    .or_else(|| continuation_ref_from_value(&result))
                    .unwrap_or_default();
                let wait_ref = first_string(&thunk, &[&["wait_ref"], &["wait", "ref"]])
                    .or_else(|| first_string(&result, &[&["wait_ref"], &["wait", "ref"]]));
                let operator_action = operator_action_from_value(&thunk)
                    .or_else(|| operator_action_from_value(&result));
                let approval_ref =
                    approval_ref_from_value(&thunk).or_else(|| approval_ref_from_value(&result));
                let status = if thunk_resumed_by_action(spec, &thunk_id, &continuation_ref) {
                    "resumed"
                } else {
                    "waiting"
                };
                nodes.push(json!({
                    "id": thunk_id,
                    "kind": "delayed_compute_thunk",
                    "status": status,
                    "label": "action_required",
                    "goal_id": goal_id,
                    "wait_ref": wait_ref,
                    "continuation_ref": continuation_ref,
                    "approval_ref": approval_ref,
                    "operator_action": operator_action,
                }));
            }
        }
    }
    nodes
}

fn delayed_compute_thunks_from_value(value: &Value) -> Vec<Value> {
    let mut thunks = first_array(
        value,
        &[
            &["delayed_compute_thunks"],
            &["delayed_compute", "thunks"],
            &["thunks"],
        ],
    );
    thunks.extend(first_array(
        value,
        &[
            &["approval_requests"],
            &["human_prompts"],
            &["operator_prompts"],
        ],
    ));
    thunks
}

fn thunk_resumed_by_action(spec: &ScenarioSpec, thunk_id: &str, continuation_ref: &str) -> bool {
    spec.actions.iter().any(|action| {
        matches!(
            action.kind,
            ScenarioActionKind::ResumeThunk
                | ScenarioActionKind::ResumeDelayedCompute
                | ScenarioActionKind::Approve
        ) && (first_string(&action.resume, &[&["thunk_id"], &["id"]]).as_deref() == Some(thunk_id)
            || (!continuation_ref.is_empty()
                && continuation_ref_from_value(&action.resume).as_deref()
                    == Some(continuation_ref)))
    })
}

fn scenario_requirement_values<'a>(
    spec: &'a ScenarioSpec,
    projection: &'a ScenarioProjection,
) -> Vec<&'a Value> {
    let mut values = Vec::new();
    for action in &spec.actions {
        values.extend(action_requirement_values(action));
    }
    values.extend(projection_requirement_values(projection));
    values
}

fn projection_requirement_values(projection: &ScenarioProjection) -> Vec<&Value> {
    projection
        .tasks
        .iter()
        .chain(projection.compute_graph_nodes.iter())
        .chain(projection.events.iter())
        .chain(projection.ui_projection.values())
        .collect()
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

fn action_worker_result_values(action: &ScenarioAction) -> Vec<&Value> {
    let mut values = Vec::new();
    if !action.worker_result.is_null() {
        values.push(&action.worker_result);
    }
    values.extend(action.worker_results.iter());
    values
}

fn action_worker_results(action: &ScenarioAction) -> Vec<Value> {
    action_worker_result_values(action)
        .into_iter()
        .cloned()
        .collect()
}

fn action_requirement_values(action: &ScenarioAction) -> Vec<&Value> {
    let mut values = vec![
        &action.payload,
        &action.body,
        &action.event,
        &action.resume,
        &action.worker_result,
        &action.attempt,
        &action.expect,
    ];
    values.extend(action.worker_results.iter());
    values.extend(action.artifacts.iter());
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
            &["summary", "status"],
            &["snapshot", "goal_status"],
            &["workflow_status", "status"],
            &["snapshot", "workflow_status", "status"],
        ],
    )
    .unwrap_or_default();
    let terminal_state = first_string(
        &value,
        &[
            &["terminal_state"],
            &["goal", "terminal_state"],
            &["workflow_status", "terminal_state"],
            &["snapshot", "workflow_status", "terminal_state"],
        ],
    )
    .unwrap_or_else(|| terminal_state_from_goal_status(&goal_status));
    let goal_id = first_string(
        &value,
        &[
            &["goal_id"],
            &["goal", "goal_id"],
            &["goal", "goal", "goal_id"],
            &["summary", "goal_id"],
            &["workflow_status", "goal_id"],
            &["snapshot", "goal_id"],
            &["snapshot", "workflow_status", "goal_id"],
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
            &[
                "snapshot",
                "goal_store_goal",
                "data",
                "goal",
                "payload_json",
                "plan",
                "subgoals",
            ],
            &["snapshot", "goal", "payload_json", "plan", "subgoals"],
        ],
    );
    let tasks = first_array(
        &value,
        &[
            &["tasks", "tasks"],
            &["tasks"],
            &["agent_activity"],
            &["snapshot", "tasks", "data", "tasks"],
            &["snapshot", "agent_activity"],
        ],
    );
    let events = first_array(
        &value,
        &[
            &["events", "events"],
            &["events"],
            &["snapshot", "events", "data", "events"],
        ],
    );
    let artifacts = first_array(
        &value,
        &[
            &["artifacts", "artifacts"],
            &["artifacts"],
            &["snapshot", "artifacts", "data", "artifacts"],
        ],
    );
    let checkpoints = first_array(
        &value,
        &[
            &["checkpoints", "checkpoints"],
            &["checkpoints"],
            &["snapshot", "checkpoints", "data", "checkpoints"],
        ],
    );
    let compute_graph_nodes = first_array(
        &value,
        &[
            &["graph", "nodes"],
            &["compute_graph", "nodes"],
            &["compute_graph_nodes"],
            &["progress", "compute_graph", "nodes"],
            &["snapshot", "workflow_compute_graph", "data", "nodes"],
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
        ScenarioActionKind::SubmitGoal => Ok("/api/operator/goals".to_string()),
        ScenarioActionKind::EmitEvent | ScenarioActionKind::EmitExternalEvent => {
            Ok("/api/events?route=true".to_string())
        }
        ScenarioActionKind::Approve => Ok(format!(
            "/api/operator/goals/{}/approve",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::ResumeThunk | ScenarioActionKind::ResumeDelayedCompute => Ok(format!(
            "/api/operator/goals/{}/resume_thunk",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::Steer => Ok(format!(
            "/api/operator/goals/{}/steer",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::Vote => Ok(format!(
            "/api/operator/goals/{}/vote",
            action_goal_id(action, known_goal_ids)?
        )),
        ScenarioActionKind::BranchSelect => Ok(format!(
            "/api/operator/goals/{}/select_branch",
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
    fn core_scenario_specs_execute_behavioral_evaluator_checks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let scenarios = [
            "scenarios/e2e/goal_lifecycle_basic.json",
            "scenarios/e2e/blocked_and_resumed.json",
            "scenarios/e2e/signal_driven_goal.json",
            "scenarios/e2e/fanout_until_done.json",
            "scenarios/e2e/fork_join_review.json",
            "scenarios/e2e/long_iterative_loop.json",
            "scenarios/e2e/bootstrap_cancelled_queue_history.json",
        ];
        for scenario in scenarios {
            let spec = read_spec(&root.join(scenario)).expect(scenario);
            let verdict = evaluate(&spec, &spec.fixtures.projection);
            assert_eq!(
                verdict.status, "passed",
                "{scenario} findings: {:?}",
                verdict.findings
            );
            let scenario_check_count = verdict
                .checks
                .iter()
                .filter(|check| check.name.starts_with("scenario_check:"))
                .count();
            assert_eq!(
                scenario_check_count,
                spec.evaluator_checks.len(),
                "{scenario} should execute every authored evaluator_check"
            );
        }
    }

    #[test]
    fn behavioral_evaluator_rejects_presence_only_fanout() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios/e2e/fanout_until_done.json");
        let mut spec = read_spec(&path).expect("fanout spec");
        let planner = spec
            .actions
            .iter_mut()
            .find(|action| action.id == "planner_fanout")
            .expect("planner action");
        planner.worker_result["child_requests"] = json!([]);
        spec.fixtures.projection = projection_from_scenario_spec(&spec);

        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict.checks.iter().any(|check| check.name
                == "scenario_check:bounded_child_materialization"
                && !check.passed),
            "fanout should fail when child requests are absent: {:?}",
            verdict.findings
        );
    }

    #[test]
    fn cancellation_scenario_projects_drained_queue_without_stale_blocker() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios/e2e/bootstrap_cancelled_queue_history.json");
        let spec = read_spec(&path).expect("cancelled queue spec");
        let projection = &spec.fixtures.projection;
        let statuses = projection
            .tasks
            .iter()
            .map(|task| text_field(task, "status"))
            .collect::<Vec<_>>();
        assert!(statuses.iter().any(|status| status == "cancelled"));
        assert!(
            !statuses.iter().any(|status| status == "blocked"),
            "cancelled queue projection must not leave stale blocked tasks: {statuses:?}"
        );
        let verdict = evaluate(&spec, projection);
        assert_eq!(verdict.status, "passed", "{:?}", verdict.findings);
        assert!(verdict.checks.iter().any(|check| {
            check.name == "scenario_check:cancelled_queue_history" && check.passed
        }));
    }

    #[test]
    fn fixture_replay_specs_do_not_require_live_gateway_actions() {
        let mut spec = fixture_spec();
        spec.determinism = json!({
            "mode": "stub_projection_replay",
            "projection_mode": "fixture_replay"
        });
        spec.actions.push(
            serde_json::from_value(json!({
                "id": "resume",
                "kind": "resume_delayed_compute",
                "goal_id": "goal-basic",
                "resume": {"thunk_id": "thunk-1", "response_summary": "continue"}
            }))
            .expect("resume action"),
        );

        assert!(uses_fixture_projection_replay(&spec));
        let results =
            fixture_action_results(&spec, &spec.fixtures.projection).expect("fixture actions");
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].kind, ScenarioActionKind::ResumeDelayedCompute);
        assert_eq!(
            results[1]
                .response
                .get("fixture_only")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn scenario_seed_request_matches_goal_store_contract() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/e2e/bootstrap_basic.json");
        let spec = read_spec(&path).expect("bootstrap spec");
        let request =
            goal_store_seed_request(&spec, &fixture_projection(&spec)).expect("seed request");
        let parsed: coat_domain::GoalStoreSnapshotUpsertRequest =
            serde_json::from_value(request).expect("goal-store snapshot request");
        assert_eq!(parsed.snapshot.goal.title, "Bootstrap one task");
        assert_eq!(parsed.snapshot.tasks.len(), 1);
        assert_eq!(parsed.snapshot.artifacts.len(), 2);
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
    fn validate_spec_rejects_action_required_without_continuation_ref() {
        let mut spec: ScenarioSpec = serde_json::from_value(json!({
            "id": "blocked_without_continuation",
            "title": "Blocked without continuation",
            "determinism": {"mode": "stub_projection_replay"},
            "goals": [{
                "goal_id": "goal-blocked",
                "spec": {
                    "title": "Blocked goal",
                    "objective": "Wait for an operator.",
                    "initial_tasks": [{"role": "codex", "title": "Wait", "prompt": "Wait"}]
                }
            }],
            "actions": [{
                "id": "wait_for_input",
                "type": "inject_worker_result",
                "worker_result": {
                    "task_id": "task-blocked",
                    "status": "waiting",
                    "next_actions": ["Answer the prompt and resume the task"],
                    "delayed_compute_thunks": [{
                        "thunk_id": "thunk-blocked",
                        "kind": "human_input",
                        "operator_action": "Answer the prompt and resume the task"
                    }]
                }
            }],
            "expected_terminal_state": {
                "goal_status": "blocked",
                "task_counts": {"blocked": 1},
                "required_events": ["task_blocked"],
                "required_artifacts": []
            },
            "evaluator_checks": [{
                "id": "blocked",
                "kind": "goal_status",
                "assertion": "goal.status == blocked",
                "expected": "blocked",
                "required": true
            }]
        }))
        .expect("scenario spec");

        let error = validate_spec(&mut spec).expect_err("missing continuation_ref fails");
        assert!(error.to_string().contains("continuation_ref"));
    }

    #[test]
    fn negative_fixture_rejects_action_required_without_recovery_or_continuation() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios/e2e/bootstrap_action_required_missing_recovery.invalid.json");
        let error = read_spec(&path).expect_err("negative action-required fixture fails");
        let message = format!("{error:#}");
        assert!(message.contains("concrete recovery action"));
        assert!(message.contains("continuation_ref"));
    }

    #[test]
    fn evaluator_rejects_waiting_task_without_concrete_operator_action() {
        let mut spec = fixture_spec();
        spec.expected_terminal_state = json!({
            "goal_status": "blocked",
            "task_counts": {"waiting": 1},
            "required_events": ["task_waiting"],
            "required_artifacts": []
        });
        spec.fixtures.projection.goal_status = "blocked".to_string();
        spec.fixtures.projection.terminal_state = "blocked".to_string();
        spec.fixtures.projection.tasks =
            vec![json!({"id": "task-waiting", "status": "waiting", "purpose": "work"})];
        spec.fixtures.projection.compute_graph_nodes.clear();
        spec.fixtures.projection.ui_projection.insert(
            "operator_terms".to_string(),
            json!("goal subgoal task thunk fork review evidence action completed"),
        );

        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "failed");
        assert!(
            verdict.findings.iter().any(|finding| finding.contains(
                "blocked, waiting, failed, or budget-exhausted task graph state requires"
            ))
        );
    }

    #[test]
    fn validate_spec_rejects_invalid_approval_ref() {
        let mut spec: ScenarioSpec = serde_json::from_value(json!({
            "id": "invalid_approval_ref",
            "title": "Invalid approval ref",
            "determinism": {"mode": "stub_projection_replay"},
            "goals": [{
                "goal_id": "goal-approval",
                "spec": {
                    "title": "Approval goal",
                    "objective": "Reject mismatched approval refs.",
                    "initial_tasks": [{"role": "codex", "title": "Approve", "prompt": "Approve"}]
                }
            }],
            "actions": [
                {
                    "id": "request_approval",
                    "type": "inject_worker_result",
                    "worker_result": {
                        "task_id": "task-approval",
                        "status": "waiting_approval",
                        "delayed_compute_thunks": [{
                            "thunk_id": "thunk-approval",
                            "kind": "approval",
                            "approval_ref": "approval://scenario/requested",
                            "continuation_ref": "continuation://scenario/approval",
                            "operator_action": "Approve the request and resume the continuation"
                        }]
                    }
                },
                {
                    "id": "approve_wrong_ref",
                    "type": "resume_delayed_compute",
                    "resume": {
                        "thunk_id": "thunk-approval",
                        "approval_ref": "approval://scenario/wrong",
                        "continuation_ref": "continuation://scenario/approval",
                        "action": "approve",
                        "operator_action": "Approve the request and resume the continuation"
                    }
                }
            ],
            "expected_terminal_state": {
                "goal_status": "blocked",
                "task_counts": {"blocked": 1},
                "required_events": ["approval_requested"],
                "required_artifacts": []
            }
        }))
        .expect("scenario spec");

        let error = validate_spec(&mut spec).expect_err("invalid approval_ref fails");
        let message = error.to_string();
        assert!(message.contains("approval_ref"));
    }

    #[test]
    fn negative_fixture_rejects_invalid_approval_ref() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios/e2e/bootstrap_invalid_approval_ref.invalid.json");
        let error = read_spec(&path).expect_err("negative approval fixture fails");
        assert!(format!("{error:#}").contains("approval_ref"));
    }

    #[test]
    fn validate_spec_rejects_terminal_state_misuse() {
        let mut spec: ScenarioSpec = serde_json::from_value(json!({
            "id": "terminal_state_misuse",
            "title": "Terminal state misuse",
            "determinism": {"mode": "stub_projection_replay"},
            "goals": [{
                "goal_id": "goal-terminal-misuse",
                "spec": {
                    "title": "Terminal misuse",
                    "objective": "Reject failed as a terminal workflow state.",
                    "initial_tasks": [{"role": "codex", "title": "Fail", "prompt": "Fail"}]
                }
            }],
            "actions": [{
                "id": "fail_task",
                "type": "inject_worker_result",
                "worker_result": {
                    "task_id": "task-failed",
                    "status": "failed",
                    "summary": "Failure still has coordinator recovery controls.",
                    "operator_action": "Retry the task from coordinator state.",
                    "recovery_actions": ["retry", "replan", "cancel"]
                }
            }],
            "expected_terminal_state": {
                "goal_status": "failed",
                "terminal_state": "failed",
                "task_counts": {"failed": 1},
                "required_events": ["task_failed"],
                "required_artifacts": []
            }
        }))
        .expect("scenario spec");

        let error = validate_spec(&mut spec).expect_err("failed terminal state fails");
        assert!(error.to_string().contains("terminal state misuse"));
    }

    #[test]
    fn negative_fixture_rejects_terminal_state_misuse() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scenarios/e2e/bootstrap_terminal_state_misuse.invalid.json");
        let error = read_spec(&path).expect_err("negative terminal fixture fails");
        assert!(format!("{error:#}").contains("terminal state misuse"));
    }

    #[test]
    fn plain_blocked_without_thunk_accepts_explicit_recovery_controls() {
        let mut spec: ScenarioSpec = serde_json::from_value(json!({
            "id": "plain_blocked_with_recovery",
            "title": "Plain blocked with recovery",
            "determinism": {"mode": "stub_projection_replay"},
            "goals": [{
                "goal_id": "goal-plain-blocked",
                "spec": {
                    "title": "Plain blocked goal",
                    "objective": "Wait without a thunk but expose coordinator recovery controls.",
                    "initial_tasks": [{"role": "codex", "title": "Block", "prompt": "Block"}]
                }
            }],
            "actions": [{
                "id": "block_without_thunk",
                "type": "inject_worker_result",
                "worker_result": {
                    "task_id": "task-plain-blocked",
                    "status": "blocked",
                    "summary": "Blocked until an operator chooses a recovery path.",
                    "recovery_actions": ["retry", "replan", "cancel", "create-human-prompt"]
                }
            }],
            "expected_terminal_state": {
                "goal_status": "blocked",
                "task_counts": {"blocked": 1},
                "required_events": ["task_blocked"],
                "required_artifacts": []
            }
        }))
        .expect("scenario spec");

        validate_spec(&mut spec).expect("plain blocked recovery is valid");
        assert!(plain_blocked_recovery_action(&spec, &spec.fixtures.projection).is_some());
        assert!(continuation_ref_from_scenario(&spec, &spec.fixtures.projection).is_none());
        assert!(
            !spec
                .fixtures
                .projection
                .compute_graph_nodes
                .iter()
                .any(|node| node.get("kind").and_then(Value::as_str)
                    == Some("delayed_compute_thunk"))
        );
        let verdict = evaluate(&spec, &spec.fixtures.projection);
        assert_eq!(verdict.status, "passed", "{:?}", verdict.findings);
    }

    #[test]
    fn plain_blocked_without_thunk_rejects_missing_recovery_controls() {
        let mut spec: ScenarioSpec = serde_json::from_value(json!({
            "id": "plain_blocked_without_recovery",
            "title": "Plain blocked without recovery",
            "determinism": {"mode": "stub_projection_replay"},
            "goals": [{
                "goal_id": "goal-plain-blocked",
                "spec": {
                    "title": "Plain blocked goal",
                    "objective": "Block without exposing recovery controls.",
                    "initial_tasks": [{"role": "codex", "title": "Block", "prompt": "Block"}]
                }
            }],
            "actions": [{
                "id": "block_without_thunk",
                "type": "inject_worker_result",
                "worker_result": {
                    "task_id": "task-plain-blocked",
                    "status": "blocked",
                    "summary": "Blocked but no recovery action is available."
                }
            }],
            "expected_terminal_state": {
                "goal_status": "blocked",
                "task_counts": {"blocked": 1},
                "required_events": ["task_blocked"],
                "required_artifacts": []
            }
        }))
        .expect("scenario spec");

        let error = validate_spec(&mut spec).expect_err("missing recovery controls fail");
        assert!(
            error
                .to_string()
                .contains("retry/replan/cancel/create-human-prompt")
        );
    }

    #[test]
    fn generated_projection_keeps_delayed_compute_continuation_nodes() {
        let mut spec: ScenarioSpec = serde_json::from_value(json!({
            "id": "human_input_thunk_resume",
            "title": "Human input thunk resume",
            "determinism": {"mode": "stub_projection_replay"},
            "goals": [{
                "goal_id": "goal-human-input",
                "spec": {
                    "title": "Human input goal",
                    "objective": "Resume through a continuation.",
                    "initial_tasks": [{"role": "codex", "title": "Ask", "prompt": "Ask"}]
                }
            }],
            "actions": [
                {
                    "id": "wait_for_input",
                    "type": "inject_worker_result",
                    "worker_result": {
                        "task_id": "task-human-input",
                        "status": "waiting",
                        "delayed_compute_thunks": [{
                            "thunk_id": "thunk-human-input",
                            "kind": "human_input",
                            "continuation_ref": "continuation://scenario/human-input",
                            "operator_action": "Answer the prompt and resume the continuation"
                        }]
                    }
                },
                {
                    "id": "resume_input",
                    "type": "resume_delayed_compute",
                    "resume": {
                        "thunk_id": "thunk-human-input",
                        "continuation_ref": "continuation://scenario/human-input",
                        "operator_action": "Resume the continuation with the selected answer"
                    }
                },
                {
                    "id": "complete_after_resume",
                    "type": "inject_worker_result",
                    "worker_result": {
                        "task_id": "task-human-input",
                        "status": "done",
                        "summary": "Evidence after resume.",
                        "artifacts": [{"uri": "artifact://scenario/human-input"}]
                    }
                }
            ],
            "expected_terminal_state": {
                "goal_status": "done",
                "task_counts": {"done": 1},
                "required_events": ["goal_satisfied"],
                "required_artifacts": ["artifact://scenario/human-input"],
                "satisfaction": {
                    "rationale": "The resumed continuation produced terminal evidence."
                }
            },
            "evaluator_checks": [{
                "id": "done",
                "kind": "goal_status",
                "assertion": "goal.status == done",
                "expected": "done",
                "required": true
            }]
        }))
        .expect("scenario spec");

        validate_spec(&mut spec).expect("valid action-required scenario");
        let thunk = spec
            .fixtures
            .projection
            .compute_graph_nodes
            .iter()
            .find(|node| node.get("kind").and_then(Value::as_str) == Some("delayed_compute_thunk"))
            .expect("delayed compute node");
        assert_eq!(
            thunk.get("continuation_ref").and_then(Value::as_str),
            Some("continuation://scenario/human-input")
        );
        assert_eq!(thunk.get("status").and_then(Value::as_str), Some("resumed"));
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
        fs::write(root.join("bad.invalid.json"), "{}").expect("write invalid fixture");
        fs::write(nested.join("a.json"), "{}").expect("write a");
        fs::write(root.join("ignore.txt"), "x").expect("write ignored");

        let files = scenario_files(&root).expect("scenario files");
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("b.json") || files[0].ends_with("a.json"));
        assert!(files[0] < files[1], "files should be sorted: {files:?}");
        assert!(
            !files.iter().any(|path| is_invalid_scenario_fixture(path)),
            "invalid fixtures should not be collected: {files:?}"
        );
        assert!(!is_scenario_file(&root.join("bad.invalid.json")));
        assert!(is_scenario_file(&root.join("b.json")));

        let _ = fs::remove_file(root.join("b.json"));
        let _ = fs::remove_file(root.join("bad.invalid.json"));
        let _ = fs::remove_file(nested.join("a.json"));
        let _ = fs::remove_file(root.join("ignore.txt"));
        let _ = fs::remove_dir(nested);
        let _ = fs::remove_dir(root);
    }
}
