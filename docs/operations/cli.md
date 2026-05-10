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
coat goal <draft|lint|submit|list|progress|tasks|steer|branch|restart|cancel>
coat human <approve|notify>
coat deploy local <preflight|up|config|down>
coat deploy cluster <render|apply|status|ephemeral-jobs|executor-job>
coat deploy chart <lint|template|upgrade|rollback|package>
coat deploy restate <cloud-env|tunnel-docker|register-cloud>
coat runner <list|status|register|dispatch|capacity-plan>
coat memory <write|search|context|join|retract|edit|preview-edit|repair|events>
coat event <sources|register|ingest|emit|webhook|poll-sqs|trigger|triggers>
coat store <policy|goals|plans|tasks|events|artifacts|checkpoints|approvals>
coat setup <config|local-auth|chat-client>
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
  `sandbox`, and `setup chat-client`. Prefer explicit endpoint flags for
  one-off routing; keep durable endpoint defaults in COAT config.
- Durable commands fail outside an initialized project when
  `config.cli.require_project_for_durable_commands=true`. Local authoring and
  Compose commands warn when `config.cli.warn_uninitialized=true`. Use
  `COAT_ALLOW_UNINITIALIZED=1` only for intentional one-off commands outside a
  COAT checkout.
- Use `coat deploy local preflight` before Compose automation. `up` runs the
  same preflight unless `--skip-preflight` is explicit.

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

## Help And Dialogue Surfaces

`coat` with no subcommand prints the root CLI help. It should not enter an
interactive dialogue or perform deployment work.

`coat guide --print` prints the canonical command map. `coat guide` opens a
small picker for the workflows where dialogue is useful: human feedback queue
inspection, approvals, project/user config, local provider auth, chat-client
integration, and active plan follow-up inspection. It does not bypass the normal
backend APIs or approval gates.

## Local Compose Preflight

`coat deploy local preflight` checks project initialization, Compose files,
Docker availability, Restate Cloud env files when requested, runner modes, and
model/provider environment. It fails when every agent lane is stubbed unless
the operator passes `--allow-stub-runners`.

When `--env-file` is omitted, local deploy commands read configured env files
from `.coat/project.json` and `~/.coat/config.json`, using only files that exist
and then falling back to `infra/compose/local-providers.env` when present.

For a smoke stack:

```sh
coat init
coat deploy local preflight --allow-stub-runners
coat deploy local up --allow-stub-runners
```

For live model/provider lanes:

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
Local model lanes query the configured OpenAI-compatible/Ollama endpoint for
currently served models and use a custom model-id prompt when discovery is
unavailable; the wizard can reuse that selected local model for the primary and
research model-provider lanes.
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
selecting the OpenAI hosted surface writes the generic model-provider lane and
can also write the research lane. `coat setup login` and `coat setup sso` own the provider CLI login steps
and can run the local preflight themselves, so operators are not left copying
raw `codex login`, `claude auth login`, or `aws sso login` commands from docs.
For Claude Code SSO or Console auth, use `coat setup login --claude
--claude-sso` or `coat setup login --claude --claude-console`; `--claude-email`
passes an email prefill to the underlying Claude auth command.
