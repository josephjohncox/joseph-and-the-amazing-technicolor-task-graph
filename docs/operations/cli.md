# COAT CLI

COAT is the Coordinator Of Agentic Tasks operator CLI.

The command tree is organized by operator intent, not by implementation detail.
Use the root help and printed command map to choose explicit subcommands:

```sh
coat
coat --help
coat guide --print
```

In this checkout, `direnv allow` loads `.envrc`, which puts the configured
checkout-local `coat` binary on `PATH`. The default build profile is `debug`, so
`make build` or `cargo build -p coat-cli` makes `coat` resolve to
`target/debug/coat` ahead of stale release or global installs. Set
`COAT_BUILD_PROFILE=release` in `.envrc.local` when you want `target/release/coat`
to win. Put machine-local secrets or optional dotenv loading in `.envrc.local`,
not in the committed `.envrc`.

## Canonical Hierarchy

```text
coat plan <draft|list|show|revise|compile|follow-ups>
coat goal <draft|lint|submit|list|progress|compute-graph|tasks|steer|vote|adversarial|mechanism|thunk|branch|restart|cancel>
coat human <approve|resume-thunk|notify>
coat deploy local <preflight|up|config|logs|down>
coat deploy cluster <render|apply|status|ephemeral-jobs|executor-job>
coat deploy chart <lint|template|upgrade|rollback|package>
coat deploy restate <cloud-env|tunnel-docker|register-cloud>
coat runner <list|status|register|dispatch|capacity-plan>
coat tool <list|call|web-search>
coat memory <write|search|context|join|retract|edit|preview-edit|repair|events>
coat event <sources|register|ingest|emit|webhook|poll-sqs|trigger|triggers>
coat store <policy|goals|plans|tasks|events|artifacts|checkpoints|approvals>
coat scenario <list|run|report>
coat setup <login|sso|model-index|config|local-auth|chat-client>
coat tui
```

## Rules

- Keep durable work under `goal` and pre-submission planning under `plan`.
- Keep approval and notification workflows under `human`.
- Keep local Compose, Kubernetes, Helm, and Restate Cloud under `deploy`.
- Keep command examples on the canonical hierarchy. Do not document duplicate
  top-level spellings for implementation tools.
- Prefer explicit subcommands by default. Use dialogue commands only where
  interaction is useful, such as setup, auth, chat-client installation, human
  feedback queues, and approvals.
- Use `coat tui` for an interactive terminal dashboard and chat surface. It is
  a Ratatui/Crossterm client over the control gateway APIs, not a separate
  engine or model runner.
- Run `coat init` once per checkout. It writes `.coat/project.json`, a
  non-secret project config that lets commands warn when they are outside an
  initialized COAT project and supplies standard `cli`, `local`,
  `restate-cloud`, and `eks` profiles.
- Use `coat --config-profile ...` for one-off profile selection. Use
  `COAT_CONFIG` only when a machine should use a non-default user config file
  outside the repo.
- Use `coat setup config --show` to inspect resolved config from built-in
  defaults, `.coat/project.json`, `~/.coat/config.json` or `COAT_CONFIG`, then
  environment variables and CLI flags.
  Use `coat setup config --list-profiles` to inspect configured profiles.
- Endpoint commands inherit the active profile when endpoint flags are omitted.
  This includes `goal`, `plan`, `store`, `human`, `event`, `memory`, `runner`,
  `tool`, `sandbox`, and `setup chat-client`. Prefer explicit endpoint flags for
  one-off routing; keep durable endpoint defaults in COAT config.
- Durable commands fail outside an initialized project when
  `config.cli.require_project_for_durable_commands=true`. Local authoring and
  Compose commands warn when `config.cli.warn_uninitialized=true`. Use
  `COAT_ALLOW_UNINITIALIZED=1` only for intentional one-off commands outside a
  COAT checkout.
- Use `coat deploy local preflight` before Compose automation. `up` runs the
  same preflight unless `--skip-preflight` is explicit.
- `coat deploy local up` rebuilds local images by default with
  `docker compose up --build` and a checkout-derived `COAT_SOURCE_FINGERPRINT`
  so changed Rust or TypeScript sources invalidate service-image cache without
  disabling cargo/npm dependency caches.
- Before starting containers, `up` runs the same resolved stack through
  `docker compose config --quiet`. It also passes `--remove-orphans` by default
  so old services from prior local topologies do not keep running invisibly.
  Use `--skip-config-check` or `--keep-orphans` only for intentional debugging.
- Use `coat deploy local logs` for Compose logs so env files, profiles, and
  Restate Cloud overlays are resolved the same way as the local stack.
- Use `coat scenario` for deterministic operator workflow evidence. Scenario
  runs write reviewable artifacts under `target/coat-scenarios` by default and
  are the CI entry point for PR-gated browser and backend E2E coverage.

## Runner Capacity

Use `coat runner capacity-plan --file examples/runner-scaling-request.json` to
ask the registry for a bounded scaling recommendation. The command does not
create or delete workers. It combines durable demand from the coordinator with
runner-registry heartbeat supply and `config.runner_capacity`. If the request
file includes a non-default `policy`, that explicit policy wins. Pass
`--ignore-config-policy` to inspect the raw request default instead. The command
returns advisory scale-up, scale-down, or steady-state actions.

The coordinator or an approved provisioner is responsible for turning a
recommendation into ephemeral Kubernetes Jobs, persistent runner changes, or no
action. Scale-down recommendations mean drain or TTL expiry, not killing active
task work.

## Tool Registry

Use `coat tool list` to inspect the MCP tools registered by the local Rust tool
registry. Use `coat tool call --name ... --file ...` for a generic tool call,
and `coat tool web-search --file examples/web-search-request.json` for the
first-class `coat_web_search` route.

`coat tool web-search` validates the file against `WebSearchRequest`, then
posts the original JSON to MCP `tools/call`. Leaving `route` out of the file
lets the tool registry apply its configured search route; setting
`"route": "coordinator_task"` or `"route": "runner_registry"` makes the
operator choice explicit.

Tool commands inherit `config.service_endpoints.tool_registry_url` and can also
use `--tool-registry-url`. If the registry requires auth, prefer
`COAT_TOOL_REGISTRY_TOKEN`; `MCP_TOOL_TOKEN` is accepted as the shared local
fallback for MCP-compatible clients.

## Help And Dialogue Surfaces

`coat` with no subcommand prints the root CLI help. It should not enter an
interactive dialogue or perform deployment work.

`coat guide --print` prints the canonical command map. `coat guide` opens a
small picker for the workflows where dialogue is useful: human feedback queue
inspection, approvals, project/user config, local provider auth, chat-client
integration, and active plan follow-up inspection. It does not bypass the normal
backend APIs or approval gates.

## Shortcut Flows

Shortcut flows are named operator recipes over the canonical commands. They
should create normal durable plans, goals, branch groups, reviews, votes,
research tasks, approvals, or steering directives; they must not introduce a
second orchestration path.

Recommended shortcuts:

- `strict_review`: draft or steer an actor task, run compile/test evidence,
  request critic review, then require unifier or validator acceptance.
- `red_team`: add security, safety, or policy critics to an existing goal before
  completion can be evaluated.
- `model_bakeoff`: create a branch group with model/persona variants, collect
  `branch_vote` outputs, and select or unify the winner.
- `research_first`: create a sourced research task and apply its
  `InformationUsePlan` as steering before implementation.
- `test_first`: inject a tester task that defines failing or missing evidence,
  then route actor work against that evidence.
- `cheap_then_deep`: run a fast candidate first and add deep review only when
  evidence is incomplete or high risk.
- `operator_review`: pause after critic or vote output and require a human
  approval, branch selection, or steering directive.

Near-term CLI shape should prefer explicit subcommands such as:

```sh
coat goal adversarial plan --goal-id <goal-id> --actor-count 3 --critic-check test_evidence --critic-check security --emit-only --out-dir /tmp/coat-adversarial
coat goal adversarial start --goal-id <goal-id> --actor-count 3 --critic-check test_evidence --research-topic "<topic>"
coat goal steer-standard --goal-id <goal-id> --check deep_research --topic "<topic>"
coat goal branch --goal-id <goal-id> --file examples/branch-request-root.json
coat goal select-branch --goal-id <goal-id> --file examples/branch-selection.json
coat goal vote --goal-id <goal-id> --direction up --reason "<why>"
coat goal compute-graph --goal-id <goal-id>
coat goal tasks --goal-id <goal-id>
```

Interactive surfaces such as `coat guide` and `coat tui` may expose these
shortcuts as buttons or menu actions, but the resulting action should still be
shown as the underlying command or workflow handler before it mutates durable
state.

## Terminal Dashboard And Chat

`coat tui` opens a terminal dashboard with gateway-backed chat. It uses
Ratatui and Crossterm for terminal rendering and keyboard handling, and it
talks to the same backend routes used by the TypeScript SPA:

- `GET /api/config`
- `GET /api/overview`
- `GET /api/chat/session`
- `POST /api/chat`
- `POST /api/goals/submit` only when the operator explicitly accepts a goal draft

The TUI never calls model providers directly. Chat requests are operator-chat
requests routed through the control gateway, which handles backend selection,
chat-turn journaling, and stub fallback policy. Chat alone is a drafting
surface; pressing `F5` or `Ctrl-G` accepts only the last `drafts.goal_spec`
payload through the same gateway endpoint used by the SPA and regular CLI.
When a chat turn returns a goal draft, the TUI shows the exact draft summary in
two places before acceptance: the chat log receives a `Goal draft ready`
preview, and the left dashboard shows an `active goal draft`
section with title, objective, initial task count, done criteria, and the
accept binding. The draft stays visible until the operator accepts it with
`F5`/`Ctrl-G` or discards it with `Ctrl-D`. After acceptance, the chat log
echoes the accepted goal id and the same draft summary, selects that goal, and
reloads the goal-scoped session.

Goal context is selected in the TUI, not retyped into every prompt. `Ctrl-N`
and `Ctrl-P` cycle through projected goals, `Ctrl-O` clears the selection, and
`Ctrl-R` refreshes the dashboard projection. When
a goal is selected, chat uses `goal:<goal_id>` as the session and sends the
same goal id to `/api/chat`; without a selected goal it uses the operator
workspace session.

The left control panel is organized around operator intent, not CLI coverage:
Overview, Goals, Approvals, Events, Adversarial, and secondary command help.
Overview shows service health, runner count, selected-goal state, blockers,
next action, evidence, and any active goal draft. Goals is the navigable goal
list. Approvals is an action queue: approval rows can be approved directly,
waiting continuations render a human prompt with a concrete question, explicit
actions, and a focused context field, and blocked or failed task rows retry the
recoverable work through the coordinator restart path. Events shows recent gateway or goal-store events, plus registered event
sources when the projection includes them. Adversarial shows actor, critic,
research, vote, and unifier context for actor/critic workflows. Command help is
a secondary reference for raw CLI names and contract inspection; normal
operation should happen through selected-goal navigation, direct human-queue
actions, and intent-grouped recovery or review controls. Both the chat panel
and the control panel render scroll progress and a scrollbar when content
exceeds the visible area.

Human prompts are the terminal form of delayed compute thunks and approval or
recovery waits. The TUI should show the prompt title, why the coordinator is
waiting, and the exact action choices before any raw payload. Safe continuations
should be a direct Continue action. Information waits should label the input
with the requested answer or context. Recovery waits should offer Retry or
Replan with context. Operators should not need to know `resume_thunk`, Restate
handler names, or raw JSON to resolve the queue item.

Cancel is the explicit terminal stop path. Use `coat goal cancel --goal-id
<goal-id> --reason "..."` when the operator wants the coordinator to stop the
goal and mark remaining recoverable work cancelled. For normal recovery, use the
action queue, `coat goal restart`, `coat human approve`, `coat human
resume-thunk`, or `coat goal steer`; blocked, failed, waiting, and
budget-exhausted states are meant to remain recoverable.

The selected-goal outline includes projected subgoals, visible tasks, and
compute graph nodes such as wait states. This is the terminal counterpart to
the SPA work graph, meant for navigation and status comprehension rather than
raw JSON inspection.

```sh
coat tui
coat tui --control-gateway-url http://localhost:9090
```

Key bindings:

- `Tab`, `Shift-Tab`: move focus across dashboard, chat, and input panels.
- `Left`, `Right`, or `1` through `6` while the control panel is focused:
  switch Overview, Goals, Approvals, Events, Adversarial, and Commands.
- `Enter`: send the chat input to `/api/chat` only when the input panel is focused; from another panel it focuses the input first.
- `Enter` or `a` in the Approvals view: apply the selected action queue row.
- `Ctrl-T`: switch chat mode across general, goal, plan, and search.
- `Ctrl-N`, `Ctrl-P`: cycle to the next or previous projected goal.
- `Ctrl-O`: clear the selected goal and return chat to the operator workspace session.
- `Ctrl-R`: refresh the dashboard projection for the current goal.
- `Up`, `Down`: scroll the focused control view, scroll chat history, or
  cycle projected goals while the Goals view is focused; in Approvals they
  select the action queue row.
- `PageUp`, `PageDown`, `Home`, `End`: scroll the focused control view or
  chat history.
- `F5` or `Ctrl-G`: accept the last chat-authored GoalSpec draft and select the returned goal.
- `Ctrl-D`: discard the active chat-authored GoalSpec draft.
- `Ctrl-U`: clear the input.
- `Esc`, `Ctrl-C`, or `q` with an empty input: quit.

## Scenario E2E

`coat scenario` is the operator command for deterministic end-to-end evidence.
It is separate from live autonomous execution: normal CI scenarios use stubbed
runners, bounded clocks, fixed seeds, local ports, and explicit fixtures so a PR
cannot pass because a live model, credential, or external service happened to
respond.

```sh
coat scenario list
coat scenario run --file scenarios/e2e/goal_lifecycle_basic.json --output-dir target/coat-scenarios
coat scenario report --run-dir target/coat-scenarios/goal_lifecycle_basic
```

The default PR gate runs every checked-in E2E scenario spec:

```sh
for scenario in scenarios/e2e/*.json; do
  target/debug/coat scenario run --file "$scenario" --output-dir target/coat-scenarios
done
```

Use the checkout-built binary in CI so the scenario command matches the Rust
contracts, gateway, and SPA produced by the same workflow run. Local operators
who have run `direnv allow` can use plain `coat scenario ...` after `make build`.

Scenario evidence is a directory, not a console transcript. Each run writes
`target/coat-scenarios/<scenario-id>/spec.json`, `evidence.json`, and
`report.json`. Evidence should include enough deterministic fixture and
projection detail for a reviewer to reconstruct the run: step results, command
evidence, service endpoint health, goal IDs created by the run, selected-goal
state, API snapshots, relevant Compose logs, and any browser artifacts.
Browser-driven scenarios should place Playwright `test-results`, traces,
screenshots, and reports under the scenario evidence tree or the standard
control-web Playwright paths so CI can upload them on failure.

Reset generated scenario evidence with the checkout helper when a local run
needs a clean evidence tree. Dry-run targets print the exact generated run
directories first, and reset modes remove known scenario directories instead of
deleting the whole evidence root:

```sh
make reset-help
make scenario-reset-dry-run
make scenario-reset
make bootstrap-reset-dry-run
RESET_BOOTSTRAP=1 make scenario-reset
sh scripts/coat-local-reset.sh --mode evidence --dry-run
```

Use `make scenario-e2e-ui-live` when you need the browser to talk to the real
local Compose control gateway instead of Playwright API fixtures. That target
starts or reuses the deterministic stub-runner Compose stack, points Playwright
at `http://127.0.0.1:9090`, submits a chat-authored goal through the gateway,
waits for the goal-store projection to appear in the selected-goal and goal
list surfaces, then verifies memory preview/apply, human queue visibility,
runner status, and event-source registration through backend APIs.

PR-gated scenarios must prove the operator workflow, not only endpoint
availability. The baseline browser scenario covers creating or selecting a
goal, verifying the SPA current-goal selector drives Chat, Work Graph, Human
Queue, Memory, and intent-grouped controls, verifying `coat tui` uses the same
selected-goal model through gateway APIs, and capturing enough evidence for a
reviewer to distinguish a real workflow failure from a harness or fixture
failure. Use
`docs/exec-plans/completed/170-usability-coherence-evaluation.md` as the
scenario usability rubric for operator comprehension and SPA/TUI coherence.
Residual UIE2E runtime proof belongs in
`docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.

## Local Compose Preflight

`coat deploy local preflight` checks project initialization, Compose files,
Docker availability, Restate Cloud env files when requested, runner modes, and
model/provider environment. It fails when every configured Compose runner is stubbed unless
the operator passes `--allow-stub-runners`.

When `--env-file` is omitted, local deploy commands read configured env files
from `.coat/project.json` and `~/.coat/config.json`, using only files that exist
and then falling back to `infra/compose/local-providers.env` when present.

For a smoke stack:

```sh
coat init
coat deploy local preflight --allow-stub-runners
coat deploy local up --allow-stub-runners
coat deploy local logs --follow coordinator runner-registry control-web
```

For a local reset, use the checkout helper rather than deleting Docker state by
hand. It stops the stack without deleting volumes by default, and deletes COAT
local stack volumes only when `--delete-volumes` is explicit. Use the dry-run
target before passing env files or project-name overrides:

```sh
make compose-reset-dry-run
sh scripts/coat-local-reset.sh --compose-stack
sh scripts/coat-local-reset.sh --compose-stack --delete-volumes
```

For live model/provider runners:

```sh
coat setup local-auth
coat setup login --codex --claude --preflight
coat setup sso --profile my-aws-sso-profile --write-env --bedrock-live --preflight
coat deploy local up --env-file infra/compose/local-providers.env
```

`coat setup local-auth` is dialogue-driven because provider kind, model choice,
device auth, and fast/balanced/deep runtime params are easier to select than to
type by hand. The wizard starts from the existing output env file, normally
`infra/compose/local-providers.env`, when it exists. Existing auth modes,
endpoints, model IDs, model params, memory-store URLs, and chat settings become
interactive defaults, so operators can press enter through known-good values and
only override the field they are changing. Before it shows hosted model or
embedding choices, the wizard refreshes the models.dev catalog unless
`COAT_MODEL_INDEX` is explicit or a cache was refreshed in the last 60 minutes.
`coat setup model-index refresh` remains available for explicit cache warm-up,
and the setup wizard reads that external index for hosted model choices instead
of compiled-in IDs.
Local model setup queries the configured OpenAI-compatible/Ollama endpoint for
currently served models and use a custom model-id prompt when discovery is
unavailable; the wizard can reuse that selected local model for the primary and
research model-provider runners.
The same wizard configures memory stores and embedding models: Qdrant,
Graphiti/Zep MCP, OpenAI hosted embeddings, Ollama, vLLM, llama.cpp, Hugging
Face, and custom OpenAI-compatible embedding endpoints are selected through
dialogue instead of hand-authored env values. Hosted embedding choices come from
the external models.dev cache; local choices come from the configured endpoint's
live `/models` or Ollama tags response. Use `coat setup model-index show
--provider openai --embeddings` to inspect hosted embedding choices from the
cache. Runtime parameter pickers include fast, provider speed tier, fast
completions, balanced, deep review, xhigh reasoning, deterministic JSON/tool
output, provider defaults, and custom values.

Codex runner setup is not the same as OpenAI hosted model-provider setup;
selecting the OpenAI hosted surface writes the generic model-provider runner and
can also write the research provider runner. `coat setup login` and `coat setup sso` own the provider CLI login steps
and can run the local preflight themselves, so operators are not left copying
raw `codex login`, `claude auth login`, or `aws sso login` commands from docs.
For Claude Code SSO or Console auth, use `coat setup login --claude
--claude-sso` or `coat setup login --claude --claude-console`; `--claude-email`
passes an email prefill to the underlying Claude auth command.

## Local Logs

`coat deploy local logs` wraps `docker compose logs` with the resolved COAT
local-deploy config. It supports `--tail`, `--follow`, `--profile`,
`--env-file`, `--restate-cloud`, and explicit service names:

```sh
coat deploy local logs --tail 200 coordinator runner-registry control-web
coat deploy local logs --follow --profile db goal-store postgres
coat --config-profile restate-cloud deploy local logs --restate-cloud --follow coordinator
```

Compose defaults local COAT services to debug request and task logs. Set
`COAT_LOG_FORMAT=json` for structured log capture, or override
`COAT_RUST_LOG` when a specific Rust service needs trace-level diagnostics.
See `docs/operations/local-observability.md` for the full local logging surface.
