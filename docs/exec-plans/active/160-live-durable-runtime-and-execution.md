# 160 Live Durable Runtime And Execution

## Objective

Coordinate the remaining live-runtime work across Restate durability, live workers, Kubernetes executors, memory, research, events, notifications, UI, release proof, and generated protocols.

This is the cross-cutting execution plan for the high-value follow-ups preserved from the completed subsystem plans. It does not reopen those plans; it sequences their residual proof work so the system moves from scaffold to proven durable runtime without duplicating subsystem ownership.

## Defaults

- Restate harness: Docker Testcontainers with a pinned Restate image.
- First live worker: Codex App Server.
- First Kubernetes proof: kind or k3d CI.
- Live test policy: env-gated live tests plus replay fixtures that always run in CI.
- First event and notification proof: SQS through LocalStack.
- Plan shape: one master active plan linked from subsystem plans.

## Subsumed Plans

The completed plans under `docs/exec-plans/completed/` remain the subsystem evidence record. Their residual live-runtime follow-ups are preserved here so `docs/exec-plans/active/` can stay focused on this single coordination plan.

- `000-bootstrap-harness`: repo and doc harness are complete; ongoing doc gardening stays as normal maintenance.
- `010-domain-task-tree`: core domain contracts are complete; future lifecycle contract growth is handled through this plan's protocol and test gates.
- `020-restate-coordinator`: coordinator scaffold is complete; real restart/resume and observability proof moves to `RuntimeVerifier`.
- `030-codex-worker`: Codex stub, registration, verification, and result refs are complete; live execution moves to `CodexWorker`.
- `040-staff-engineer-worker`: staff-engineer stub routing is complete; live package verification and issue-to-PR smoke run after Codex proves the live worker contract.
- `050-research-worker`: research contracts and replay fixture shape are complete; live source capture moves to `ResearchMemory`.
- `060-test-review-validator`: validator/reviewer contracts are complete; real live worker outputs become new reviewer fixtures through `Reviewer`.
- `070-sandbox-tooling`: sandbox/tool contracts are complete; Kubernetes execution and object upload promotion move to `Provisioner`.
- `080-deploy-and-cli`: deploy and CLI scaffold is complete; release proof and executor provisioning move to `ReleaseHardening` and `Provisioner`.
- `090-distributed-runners-mcp-notifications`: runner/MCP/notification contracts are complete; live provider profiles and SQS proof move to `CodexWorker` and `EventOps`.
- `100-steering-research-memory`: steering and memory contracts are complete; live adapters and browser memory workflow move to `ResearchMemory` and `UIE2E`.
- `110-protobuf-goal-store`: protobuf and goal-store projection are complete; SDK generation and Restate restart proof move to `ProtocolSDK` and `RuntimeVerifier`.
- `120-events-webhooks-schedules`: event gateway contracts are complete; provider adapters and topology proof move to `EventOps`.
- `130-restate-cloud-personal-corporate`: Restate Cloud support is complete; provider overlays and journal encryption guidance move to `ReleaseHardening`.
- `140-control-gateway-spa`: gateway and SPA scaffold are complete; full Compose browser E2E and token-broker smoke move to `UIE2E`.
- `150-durable-planning-mode`: durable planning mode is complete with no residual follow-ups.

## Subagent Lanes

- `RuntimeVerifier`: owns Restate restart/resume tests, durable projection idempotency, metrics, and traces.
- `CodexWorker`: owns live Codex App Server execution, Codex MCP fallback, and replayable fixtures.
- `Provisioner`: owns coordinator-approved Kubernetes executor Jobs, Job watching, result ingestion, and attestations.
- `ResearchMemory`: owns live research, source capture, Qdrant, Graphiti/Zep, and object-store snapshots.
- `EventOps`: owns SQS/LocalStack inbound and outbound proof, notifier outbox, retries, acknowledgements, and DLQ behavior.
- `UIE2E`: owns browser-level Compose workflows for goals, task graph, memory edits, approvals, runners, and event sources.
- `ReleaseHardening`: owns GitHub Release, Helm chart, kind/k3d, and published smoke evidence.
- `ProtocolSDK`: owns Buf-generated Rust and TypeScript SDK target selection and generation.
- `Reviewer`: reviews each lane for correctness, security, testing depth, and public-contract drift.
- `Unifier`: joins accepted lane outputs and decides whether the master plan can be moved to completed.

Workers in these lanes use COAT durable child tasks. They must not use hidden native Codex, Claude Code, Agents SDK, or MCP subagent spawning. Any request for more work returns `ChildTaskRequest` values for coordinator approval.

## Workstreams

### Runtime Proof

- Add a Docker Testcontainers-based Restate integration harness for `coat-coordinator`.
- Scaffold the ignored `coat-coordinator` RuntimeVerifier entrypoint at `crates/coordinator/tests/restate_restart_resume.rs`; normal CI only compiles gate/default tests, while the live proof requires `COAT_RESTATE_RESTART_RESUME_TEST=1`, Docker availability, and a pinned `COAT_RESTATE_TESTCONTAINERS_IMAGE`.
- Evidence 2026-05-11: `crates/coordinator/tests/restate_restart_resume.rs` now has deterministic config, harness-step ordering, projection idempotency, and transition-counter assertions around the live proof gate.
- Evidence 2026-05-11: coordinator transition observations now include waiting-input counts, pending delayed thunks, mechanism-round counts, compute-graph node/edge counts, and a `coordinator.transition` tracing span; RuntimeVerifier projection counters now assert persisted compute-graph nodes, edges, open thunks, and waiting tasks.
- Start Restate with persistent data, start coordinator on a dynamic local port, register the deployment, and drive workflow calls through Restate ingress.
- Prove coordinator restart against existing workflow state.
- Prove Restate process restart with persisted journal data.
- Assert completed durable steps are not re-executed after replay.
- Add transition metrics and spans for workflow run, task dispatch, runner calls, validation, restart, approval pause/resume, projection attempts, and projection failures.
- Populate trace IDs already present in worker/protocol metadata instead of creating a separate observability model.

### Codex Live Worker

- Make distributed/operator profiles truthfully distinguish stub execution from live execution.
- Prevent placeholder actor, critic, unifier, or research results from satisfying non-stub goals.
- Implement Codex App Server `/run-task` as the first live runner path.
- Evidence 2026-05-11: `sidecars/codex-runner-ts` now has explicit `stub`, `replay`, `live`, and `mcp-healthcheck` modes; live App Server mode blocks rather than fabricating work when auth, URL, sandbox, or workspace gates are missing; replay mode consumes `examples/codex-app-server-replay.json`.
- Preserve sandbox profile, memory context refs, thread/session IDs, child-task requests, git refs, checkpoints, object refs, and artifact manifests in `AgentRunResult`.
- Add Codex MCP as the fallback callable-tool path after App Server behavior is proven.
- Evidence 2026-05-11: `sidecars/codex-runner-ts` now has `mcp-replay` mode and `examples/codex-mcp-fallback-replay.json`, so Codex MCP fallback parsing, diagnostics, and structured result extraction are covered by deterministic CI tests.
- Capture replay fixtures with thread IDs, checkpoint refs, git refs, artifact manifests, structured results, and diagnostics.
- Verify live provider profiles after auth setup is exercised on real nodes, including Codex App Server, Claude Code, Bedrock, vLLM, Ollama, Hugging Face, and OpenAI-compatible gateways.
- Evidence 2026-05-11: `/verify` now returns provider-profile entries with explicit `verified`, `skipped`, or `failed` state so unavailable live lanes produce reviewable skipped evidence instead of silent absence.
- Keep staff-engineer live execution second-phase until current `@ctxr/kit` and `@ctxr/agent-staff-engineer` behavior, isolated target repo install, tracker auth, and Claude Code auth distribution are verified.
- Add a live staff-engineer issue-to-PR smoke test only after those staff-engineer gates pass.

### Executor Provisioning

- Connect coordinator-approved runnable task state and capacity policy to sandbox-runner Kubernetes executor Job provisioning. The coordinator must approve budget, approval policy, sandbox profile, local tools, `ExecutionProfile.capacity`, and template refs before any Job is materialized.
- Use kind or k3d in CI to prove the normal backend path: coordinator capacity approval, sandbox-runner provision request, Kubernetes server-side dry-run, live apply, Job/Pod watch, result ingestion, and goal-store projection.
- Implement the `jattg-agent-toolbox` executor contract: read `sandbox-launch-plan.json`, run bounded work, write command/artifact/checkpoint/git/object-store manifests, emit the structured result, and produce sandbox attestation evidence.
- Watch Job and Pod state, collect logs and applied/final manifests, classify image-pull, scheduling, runtime-class, admission, timeout, deadline, and cleanup failures, enforce TTL/cleanup, and project results into goal-store.
- Preserve provision request ID, goal/task IDs, capacity decision ref, ConfigMap/Job/Pod UIDs, phase transition timestamps, log refs, result manifest refs, and attestation evidence as validator-reviewable artifacts.
- Evidence 2026-05-11: `crates/sandbox-runner` now mounts `provisioner-evidence.json`, injects `COAT_*` evidence paths into executor Jobs, and rejects live modes before cluster contact when coordinator evidence refs are absent.
- Keep rendered Job manifests as operator fixtures and escape hatches; the normal backend path uses Rust `kube`/`k8s-openapi` clients.

### Memory, Research, And Object Artifacts

- Run live Qdrant and Graphiti/Zep tests only when explicit service URLs and credentials are present.
- Keep replay fixtures in normal CI so research provenance can be reviewed without live web or model access.
- Route live research through a bounded runner/tool path that requires citations, source artifacts, and an `InformationUsePlan`.
- Capture raw source snapshots, fetch metadata, and large artifacts into MinIO/S3-compatible storage.
- Promote local object refs to real uploaded snapshots once the uploader path is live.
- Preserve memory context by reference; do not dump large memory payloads into task prompts.
- Evidence 2026-05-11: `coat-memory-gateway` now preserves research source/object artifact refs in Qdrant payloads and Graphiti source descriptions, and `examples/research-output-memory-substrate.json` carries raw source snapshot and fetch metadata object refs for replay review.
- Evidence 2026-05-11: live Qdrant, Graphiti, and Zep tests are explicitly env-gated while replay fixture coverage remains deterministic in normal CI.

### Events And Notifications

- Add SQS LocalStack inbound event-source and outbound notification smoke tests. First proof slice: `make eventops-sqs-smoke` starts LocalStack when Docker is available, reuses the SQS event-source and notification examples with local queue URLs, proves inbound poll/delete through `coat-event-gateway`, and proves outbound SQS delivery through `coat-notifier`.
- Evidence 2026-05-11: `coat-notifier` now has a journaled outbox with `pending`, `delivered`, `awaiting_ack`, `acknowledged`, `retry_scheduled`, and `dead_lettered` states plus `/outbox`, `/outbox/{id}/ack`, `/outbox/{id}/retry`, `/outbox/retry-due`, and `/dlq` endpoints.
- Evidence 2026-05-11: `make eventops-sqs-smoke` passed against `localstack/localstack:3.8.1`, proving inbound SQS poll/delete, outbound SQS delivery, notifier journal replay shape, and an `awaiting_ack` outbox entry.
- Evidence 2026-05-11: event sources now include explicit `pull_request_check`, `github_actions_check`, and `gitlab_pipeline_check` kinds with examples and normalization tests, so PR required checks, GitHub Actions runs, and GitLab pipelines all project provider-neutral `_coat_change_activity` metadata before routing durable goals.
- Closure 2026-05-11: the SQS/LocalStack residuals inherited from the distributed-runners and events plans are satisfied by the smoke above; remaining EventOps work is topology proof plus additional provider adapters.
- Normalize recurrent observability events into durable gateway events before creating or steering goals.
- Keep event activation behind coordinator policy and human approval when sources add external callbacks, cost-bearing polling, or broad network access.
- Prove event-gateway projection against the same Compose or cluster topology operators run, not only isolated local smoke scripts.
- Evidence 2026-05-11: `coat-event-gateway` now projects create-goal trigger decisions with concrete `goal_id` values into `coat-goal-store`; `make event-gateway-smoke` verifies source registration, normalized event ingestion, trigger creation, goal-store projection, and dedupe behavior.
- Add Slack, PagerDuty, Google Calendar, Outlook, and OpenTelemetry provider adapters after the SQS proof is stable and credentials are approved.

### Operator UI And MCP

- Add browser E2E over the full Compose stack for goal selection, task graph inspection, memory preview/apply, approval/reject/comment, runner status, and event source management.
- Evidence 2026-05-11: `ui/control-plane-web` smoke coverage now renders event-source activation, approval queues, runner capacity, memory events, goal progress, and task graph filters against gateway-backed fixtures.
- Evidence 2026-05-11: gateway goal snapshots now read `GoalWorkflow/compute_graph`, expose `workflow_compute_graph` through `/api/goals/{goal_id}` and MCP `coat_goal_snapshot`, and the SPA renders `waiting_input` continuation state plus compute-graph node/edge/thunk counters in task summaries.
- Evidence 2026-05-11: operator continuations are now actionable in the SPA: open delayed-compute thunk nodes render reason, task ID, thunk ID, continuation ID, wait ref, response-summary input, and a guarded `resume_thunk` backend mutation; resumed/cancelled thunks are filtered out of the actionable queue.
- Verify UI mutations use backend APIs only and never mutate goal-store projections directly.
- Keep existing gateway contract smoke tests as the fast CI path.
- Add token-broker-backed multi-user MCP smoke only after a broker implementation is selected.

### Release And Deployment Proof

- Fix release workflow command drift whenever CLI hierarchy changes.
- Evidence 2026-05-11: `.github/workflows/release-helm.yml` now uses `coat deploy chart package` for chart release packaging.
- Run published binary smoke and Helm chart smoke after the first GitHub Release and record evidence in `docs/operations/releases.md`.
- Evidence 2026-05-11: `.github/workflows/release-binaries.yml` and `.github/workflows/release-helm.yml` now include published-asset smoke jobs; `Makefile` exposes `release-binary-smoke` and `release-helm-smoke` for local/operator replay of those checks.
- Evidence 2026-05-11: CI, release-binary, and release-Helm workflows now use stable Rust cache keys plus `sccache`; GHCR publishing uses both GitHub Actions BuildKit cache and registry-backed `jattg-build-cache` refs; Rust and Node Dockerfiles use BuildKit cache mounts for target and npm caches.
- Evidence 2026-05-11: release binaries now build Linux ARM on the native `ubuntu-22.04-arm` runner and macOS ARM on `macos-latest`; CI adds a cached runner-target compatibility lane for `ubuntu-latest`, `ubuntu-24.04`, `ubuntu-22.04`, `ubuntu-24.04-arm`, `ubuntu-22.04-arm`, and `macos-latest`.
- Evidence 2026-05-11: GHCR image publishing can now target a single image or image group; the release workflow keeps Rust service/toolbox publishing in one cache-sharing lane and fans Node sidecar images out into parallel lanes.
- Evidence 2026-05-11: Rust service release tags now share one multi-binary service image build; `COAT_SERVICE_BIN` selects the process, and the entrypoint preserves compatibility by mapping known `BIND_ADDR` ports to service binaries.
- Promote `make compose-runner-smoke` to required or scheduled CI once Docker build capacity and image caches are available.
- Add provider overlays after the first target is chosen; the first executor proof remains kind/k3d.
- Add Restate Cloud journal encryption guidance when the Rust service path and provider documentation support it.

### Protocol And SDK

- Keep protobuf, JSON schemas, docs, and Rust domain contracts aligned through `make proto-check`.
- Add generated Rust and TypeScript SDKs from Buf only after package names, output locations, and compatibility rules are selected.
- Evidence 2026-05-11: Buf SDK generation is scaffolded through `buf.gen.yaml`, `make proto-sdk-generate`, and `make proto-sdk-check`, generating Rust and TypeScript outputs under `target/generated-sdks` without committing generated artifacts.
- Decision 2026-05-11: generated SDK wrappers are internal validation artifacts for now, not published packages. Reserve `coat-protocol-sdk` and `@coat/protocol-sdk`, keep generation under `target/generated-sdks/`, and defer package metadata/release jobs until a published-SDK compatibility milestone is selected.
- Add or change public types only where needed for Kubernetes provision/result records, executor attestations, object upload status, retryable event delivery, and observability correlation.
- Evidence 2026-05-11: `crates/domain` now models goal ranking votes as an opt-in extension with upvote/downvote promotion or demotion decisions, plus first-class delayed compute thunks for human input, approvals, timers, callbacks, resource waits, model availability, delimited continuation refs, worker `waiting` results, and derived compute graph snapshots; `coat-coordinator` exposes `vote`, `create_thunk`, `resume_thunk`, and `compute_graph`, and the CLI exposes `coat goal vote`, `coat goal thunk create`, `coat goal compute-graph`, and `coat human resume-thunk`.
- Evidence 2026-05-11: `crates/domain` now includes opt-in `mechanism_policy` and `MechanismRound` state for distributed consensus, voting, Delphi-style rounds, sealed-bid/Vickrey auctions, and contract-net allocation; `coat-coordinator` exposes `mechanism_start` and `mechanism_ballot`, and the CLI exposes `coat goal mechanism start|ballot`.
- Before moving any linked plan to completed, preserve every remaining follow-up here, record direct evidence, or write an explicit supersession note.

### Reviewer And Satisfaction Gates

- Evidence 2026-05-11: `examples/reviewer-fixtures/live-replay-worker-fixture.json` now consumes accepted Codex replay output as a reviewer/validator fixture with command evidence, child tester requests, git refs, worktree paths, and reviewer checkpoint branch evidence.
- Evidence 2026-05-11: `crates/domain` now rejects metadata-only checkpoints for code tasks that require git checkpointing, and the behavioral fixture proves actor output, reviewer output, tester child requests, and checkpoint branch projection into the goal-store snapshot.

## Tests

- Unit tests cover task lifecycle, restart policy, projection idempotency, attestation validation, event retry states, object artifact refs, stub-output rejection, and public-contract serialization.
- Unit tests cover opt-in goal ranking vote promotion/demotion, delayed compute thunk pause/resume behavior, worker waiting-result thunk materialization, and compute graph projection of tasks, thunks, wait refs, and continuations.
- Unit tests cover opt-in mechanism rounds for consensus tallies and Vickrey auction decisions with human-ratification state.
- Restate integration tests cover coordinator restart, Restate journal recovery, durable projection replay, approval pause/resume, and timeout restart.
- Worker tests cover Codex App Server live execution behind env gates, Codex MCP replay, structured result extraction, and stub mode as an explicit smoke path only.
- Reviewer tests consume accepted live worker outputs as real-world fixtures and include git checkpoint branch/worktree coverage where branch workflows are involved.
- Kubernetes tests cover kind/k3d Job provision, execution, log/result collection, attestation, timeout, image-pull failure, runtime-class failure, scheduling failure, and artifact upload failure.
- Memory and research tests cover env-gated Qdrant/Graphiti/Zep, offline replay fixtures, source snapshots, citation validation, and memory repair/replay after adapter outage.
- Event and notification tests cover LocalStack SQS ingest, outbound notification, retry, ack, DLQ, event dedupe, and triggered-goal projection.
- UI tests cover existing gateway smoke plus browser E2E for the full Compose operator workflows.
- Release tests cover binary unpack/checksum/operator-surface smoke and Helm template/install/rollout/rollback smoke after publication.

## Follow-Ups

- `RuntimeVerifier`: complete the ignored entrypoint's deterministic scaffold with a Docker Testcontainers Restate harness and restart/resume proof.
- `RuntimeVerifier`: wire the deterministic transition/projection observation assertions into the live Docker Testcontainers harness and, once an OpenTelemetry sink is selected, assert exported spans instead of only local tracing fields.
- `CodexWorker`: run an env-gated live Codex App Server smoke with real thread/turn IDs, then capture the live result as a replay fixture.
- `CodexWorker`: run live provider verification on real configured nodes and archive one profile result per enabled lane.
- `CodexWorker`: verify `@ctxr/kit` and `@ctxr/agent-staff-engineer` before staff-engineer live smoke work.
- `Provisioner`: run kind/k3d watch proof from sandbox-runner provision request through result ingestion, failure taxonomy, cleanup, and attestation projection.
- `Provisioner`: add provider-backed sandbox adapters only when they return validator-reviewable attestations, or write a supersession note if provider sandboxes remain out of scope.
- `ResearchMemory`: run live Qdrant, Graphiti, Zep, and object-store adapter smoke with approved credentials, then capture replayable fixture evidence.
- `ResearchMemory`: promote replay object refs to real S3/MinIO uploads with immutable version or digest evidence for source snapshots and large artifacts.
- `EventOps`: run event-gateway projection proof against full Compose or cluster topology, reusing the local projection smoke as the fast path.
- `EventOps`: add live Slack, tracker, PagerDuty, Google Calendar, Outlook, OpenTelemetry, and provider-adapter smoke tests behind credentials and explicit approval gates.
- `UIE2E`: upgrade gateway fixture smoke into full Compose browser workflows for goals, memory, approvals, runners, and events.
- `UIE2E`: add token-broker-backed multi-user MCP smoke after broker design is selected.
- `ReleaseHardening`: run the first published binary and Helm chart smoke and record evidence.
- `ReleaseHardening`: promote `make compose-runner-smoke` to required or scheduled CI once Docker build capacity and image caches are available.
- `ReleaseHardening`: add provider-specific deploy overlays and Restate Cloud journal-encryption guidance once the first cloud target and supported SDK path are selected.

## Acceptance

- `coat plan follow-ups` lists this master plan as the live-runtime coordination point.
- Completed subsystem plans preserve their implementation evidence while this master plan owns residual live-runtime follow-ups.
- Replay tests run in normal CI without live credentials.
- Live tests are env-gated and skip clearly when credentials or external services are unavailable.
- A real Restate restart/resume test proves completed durable steps are not re-executed.
- A real Codex App Server task returns structured evidence, checkpoints, and replayable artifacts.
- A kind/k3d executor Job runs from coordinator-approved state through the sandbox-runner provision API, is watched to completion, returns structured result and attestation evidence, and is projected into goal-store.
- SQS LocalStack proves durable inbound and outbound event/notification behavior.
- Event gateway proof runs against Compose or cluster topology.
- Live provider verification records one profile result per enabled provider lane.
- Staff-engineer remains gated until package behavior, isolated target repo install, tracker auth, and Claude Code auth distribution are verified.
- Live worker outputs are captured as reviewer/validator fixtures with git checkpoint/worktree evidence where branch workflows are involved.
- Browser E2E proves operator workflows against backend APIs rather than projection mutation.
- Published binary and Helm chart smoke evidence is recorded after the first release.
