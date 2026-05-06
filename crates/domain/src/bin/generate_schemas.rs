use std::{fs, path::PathBuf};

use anyhow::Context;
use jattg_domain::{
    AgentRunRequest, AgentRunResult, ChildTaskRequest, ExecutionProfile, GoalSpec, GoalState,
    HumanApproval, HumanFeedback, McpContextRef, ModelRoute, NotificationPolicy,
    NotificationRequest, RunnerDispatchDecision, RunnerDispatchRequest, RunnerRegistration,
    SandboxProfile, TaskNode, ValidationReport, ValidationRequest,
};
use schemars::schema_for;

fn main() -> anyhow::Result<()> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("schemas"));
    fs::create_dir_all(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;

    write_schema(&out_dir, "goal-spec.schema.json", schema_for!(GoalSpec))?;
    write_schema(&out_dir, "goal-state.schema.json", schema_for!(GoalState))?;
    write_schema(&out_dir, "task-node.schema.json", schema_for!(TaskNode))?;
    write_schema(
        &out_dir,
        "agent-run-request.schema.json",
        schema_for!(AgentRunRequest),
    )?;
    write_schema(
        &out_dir,
        "agent-run-result.schema.json",
        schema_for!(AgentRunResult),
    )?;
    write_schema(
        &out_dir,
        "child-task-request.schema.json",
        schema_for!(ChildTaskRequest),
    )?;
    write_schema(
        &out_dir,
        "validation-request.schema.json",
        schema_for!(ValidationRequest),
    )?;
    write_schema(
        &out_dir,
        "validation-report.schema.json",
        schema_for!(ValidationReport),
    )?;
    write_schema(
        &out_dir,
        "sandbox-profile.schema.json",
        schema_for!(SandboxProfile),
    )?;
    write_schema(
        &out_dir,
        "execution-profile.schema.json",
        schema_for!(ExecutionProfile),
    )?;
    write_schema(&out_dir, "model-route.schema.json", schema_for!(ModelRoute))?;
    write_schema(
        &out_dir,
        "mcp-context.schema.json",
        schema_for!(McpContextRef),
    )?;
    write_schema(
        &out_dir,
        "notification-policy.schema.json",
        schema_for!(NotificationPolicy),
    )?;
    write_schema(
        &out_dir,
        "notification-request.schema.json",
        schema_for!(NotificationRequest),
    )?;
    write_schema(
        &out_dir,
        "runner-registration.schema.json",
        schema_for!(RunnerRegistration),
    )?;
    write_schema(
        &out_dir,
        "runner-dispatch-request.schema.json",
        schema_for!(RunnerDispatchRequest),
    )?;
    write_schema(
        &out_dir,
        "runner-dispatch-decision.schema.json",
        schema_for!(RunnerDispatchDecision),
    )?;
    write_schema(
        &out_dir,
        "human-feedback.schema.json",
        schema_for!(HumanFeedback),
    )?;
    write_schema(
        &out_dir,
        "human-approval.schema.json",
        schema_for!(HumanApproval),
    )?;
    Ok(())
}

fn write_schema(out_dir: &PathBuf, name: &str, schema: schemars::Schema) -> anyhow::Result<()> {
    let path = out_dir.join(name);
    let json = serde_json::to_string_pretty(&schema)?;
    fs::write(&path, format!("{json}\n")).with_context(|| format!("write {}", path.display()))
}
