# COAT CLI

COAT is the Coordinator Of Agentic Tasks operator CLI.

The command tree is organized by operator intent, not by implementation detail.
Use the guided dialogue when you are not sure which command shape to use:

```sh
coat
coat guide
coat guide --print
```

In this checkout, `direnv allow` loads `.envrc`, which puts
`target/release/coat` and `target/debug/coat` on `PATH`. Run
`cargo build -p coat-cli` after a clean checkout, then use `coat` directly.
Put machine-local secrets or optional dotenv loading in `.envrc.local`, not in
the committed `.envrc`.

## Canonical Hierarchy

```text
coat plan <draft|list|show|revise|compile|follow-ups>
coat goal <draft|lint|submit|list|progress|tasks|steer|branch|restart|cancel>
coat human <approve|notify>
coat deploy local <preflight|up|config|down>
coat deploy cluster <render|apply|status|ephemeral-jobs|executor-job>
coat deploy chart <lint|template|upgrade|rollback|package>
coat deploy restate <cloud-env|tunnel-docker|register-cloud>
coat runner <list|status|register|dispatch>
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
- Prefer dialogue commands for setup and uncertain workflows; prefer explicit
  flags for automation.
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

## Dialogue Surfaces

`coat` with no subcommand opens the same guided picker as `coat guide`.

The guide can draft plan or goal JSON, inspect latest progress, show the human
queue, approve a request, start the local stack, configure provider auth,
install chat-client integration, or print the command map. It does not bypass
the normal backend APIs or approval gates.

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
coat deploy local preflight --env-file infra/compose/local-providers.env
coat deploy local up --env-file infra/compose/local-providers.env
```
