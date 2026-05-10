# COAT Configuration

COAT uses JSON config so the same contracts can be validated with the generated
schemas in `schemas/`.

Primary schemas:

- `schemas/project-configuration.schema.json`
- `schemas/user-configuration.schema.json`
- `schemas/configuration.schema.json`
- `schemas/configuration-profile.schema.json`
- `schemas/cloud-configuration.schema.json`
- `schemas/kubernetes-configuration.schema.json`
- `schemas/runner-capacity-configuration.schema.json`

## Config Layers

Config is resolved in this order:

1. Built-in defaults.
2. Project config at `.coat/project.json`.
3. User or machine config at `~/.coat/config.json`, or `COAT_CONFIG` when a
   machine intentionally uses a non-default user config file.
4. The active config profile from `config.active_profile`,
   `coat --config-profile ...`, legacy `COAT_PROFILE`, or
   `coat setup config --show --profile ...`.
5. Environment variables and explicit CLI flags.

Later layers override scalar values and append unique list entries. Profiles
are named overlays, so the same checkout can carry clean defaults for `cli`,
`local`, `restate-cloud`, and `eks` without duplicating setup instructions.
Secrets
should stay out of both JSON files. Use environment variables, `SecretRef`,
Kubernetes Secrets, Vault, cloud secret managers, keychains, workload identity,
or MCP auth brokers for secret material.

Model catalogs are also config-adjacent but not secrets. `coat setup
local-auth` refreshes the default models.dev catalog cache at
`~/.coat/cache/models.dev.api.json` before it renders hosted model choices,
unless a catalog was refreshed in the last 60 minutes. `coat setup model-index
refresh` is still available for explicit cache warm-up. The setup wizard reads
model indexes in this order: `COAT_MODEL_INDEX`, `.coat/model-index.json`, then
the user cache; `COAT_MODEL_INDEX` is treated as an explicit operator-managed
catalog and is not overwritten by automatic refresh. This keeps hosted model
choices current without compiling provider model IDs into `coat`.

Model routing is config-adjacent too. Use `config.model_routing` for non-secret
defaults such as `direct_providers`, `shared_gateway`, or `hybrid`, gateway base
URLs, lane model names, and secret reference names. For local Compose,
`coat setup local-auth` writes the equivalent runtime env keys:
`COAT_LLM_GATEWAY_URL`, `COAT_LLM_GATEWAY_API_KEY`,
`COAT_LLM_GATEWAY_{WORK,RESEARCH,CHAT,DEFAULT}_MODEL`, and direct provider
keys when selected. Raw API keys still belong only in env files ignored by git,
Kubernetes Secrets, cloud secret stores, or auth brokers.

Keep gateway defaults separate from runner defaults. `COAT_CONTROL_CHAT_*`
selects the operator Chat tab backend. `MEMORY_GATEWAY_EMBEDDING_*` selects the
memory embedding provider. `MODEL_PROVIDER_*`, `MODEL_PROVIDER_RESEARCH_*`, and
`LOCAL_MODEL_PROVIDER_*` select durable runner capacity for task roles and
personas. The default `COAT_CONTROL_CHAT_BACKEND=configured` uses explicit
gateway chat settings or the stub; it does not infer that a registered local
runner should answer `/api/chat`. Use
`COAT_CONTROL_CHAT_BACKEND=runner_registry` only for an intentional
chat-labeled runner fallback.

Runner capacity scaling is standard COAT config, not a special env-file-only
surface. Use `config.runner_capacity.default_policy` for the default bounded
policy and `config.runner_capacity.lane_policies` for pool-specific overrides
such as `research`, `review`, `codex`, or `sre`. The policy is advisory until
the coordinator or an approved provisioner applies it; the runner registry
still only reports recommendations.

## Project Config

`.coat/project.json` is safe to commit. It defines shared non-secret profiles:

- project and package slugs;
- `cli`: operator output and local service endpoint defaults;
- `local`: Docker Compose service URLs, env-file locations, and local data/cache
  paths, plus recommend-only runner capacity defaults;
- `restate-cloud`: local services pointed at the Restate Cloud tunnel ingress;
- `eks`: Kubernetes namespace, manifest, Helm chart, image registry, workload
  identity, secret provider, S3 object-store defaults, and larger recommend-only
  runner capacity defaults for cluster pools.

Refresh it with:

```sh
coat setup config --write-project
```

## User Config

`~/.coat/config.json` is machine-local. Use it for workstation or node defaults
that should not be committed, such as a different goal-store URL, a local model
cluster endpoint, or a user-specific provider env-file path.

Create the template with:

```sh
coat setup config --write-user
```

Use `examples/coat-user-config.json` as the committed example. To keep multiple
machine profiles, prefer named profiles inside `~/.coat/config.json` and select
one with `coat --config-profile ...`. Set `COAT_CONFIG` only when a machine
should use a non-default user config file.

```sh
export COAT_CONFIG="$HOME/.coat/personal-laptop.json"
coat --config-profile local setup config --show
```

## Inspection

List available profiles or show the resolved configuration and which files were
loaded:

```sh
coat setup config --list-profiles
coat setup config --show
coat setup config --show --profile restate-cloud
coat setup config --show --profile eks
```

The output intentionally redacts goal identity and does not print raw provider
tokens. CLI flags and environment variables still win for one-off runs.

## Runtime Endpoint Defaults

Endpoint-bearing commands inherit the active profile when their endpoint flag
is omitted or still equals the built-in localhost default. This keeps daily
operator commands aligned with the same `cli`, `local`, `restate-cloud`, or
`eks` profile used for deployment:

```sh
coat --config-profile restate-cloud goal progress --latest
coat --config-profile restate-cloud plan list
coat --config-profile restate-cloud human notify --queue
coat --config-profile local memory search --file examples/memory-search.json
coat --config-profile local runner list
```

Explicit endpoint flags still override profile values:

```sh
coat goal progress \
  --latest \
  --restate-ingress http://localhost:8080 \
  --goal-store-url http://localhost:9088
```

The resolved config can set defaults for Restate ingress, goal store, event
gateway, memory gateway, runner registry, notifier, sandbox runner, and the
control MCP gateway. Avoid placing endpoint defaults in direnv. Environment
variables remain available for secrets, process runtime boundaries, and rare
one-off overrides, but project and operator defaults should live in COAT config.

## Initialization Policy

`coat init` writes `.coat/project.json`. Commands that submit, mutate, inspect,
or deploy durable project state check for that marker before running.

By default:

- durable commands fail outside an initialized project;
- local-only authoring and Compose commands warn but continue;
- `setup`, `init`, and the guided command picker are allowed.

The behavior is controlled by `config.cli.warn_uninitialized` and
`config.cli.require_project_for_durable_commands`. Use
`COAT_ALLOW_UNINITIALIZED=1` only for intentional one-off operator commands
outside a project checkout.

## Local Deploy Defaults

`coat deploy local preflight`, `config`, `up`, and `down` read the standard
config. If no `--env-file` is provided, the CLI looks for configured env files
that exist, then falls back to `infra/compose/local-providers.env` when present.

Useful fields:

```json
{
  "config": {
    "local_deploy": {
      "env_files": ["infra/compose/local-providers.env"],
      "restate_cloud_env_file": "infra/compose/restate-cloud.env",
      "allow_stub_runners": false,
      "allow_uninitialized": false,
      "profiles": []
    }
  }
}
```

Keep `allow_stub_runners=false` in committed project config unless the repo is
only meant for stub smoke tests. A user config can opt into stubs locally.

## Runner Capacity Defaults

`config.runner_capacity` holds the default scaling envelope used by
coordinator/provisioner code and by `coat runner capacity-plan` when the request
file omits a policy or carries the disabled default. This keeps local setup,
Kubernetes profiles, and operator diagnostics on the same policy.

Example user override:

```json
{
  "config": {
    "runner_capacity": {
      "default_policy": {
        "enabled": true,
        "mode": "recommend_only",
        "max_runners": 4,
        "max_scale_up_step": 1,
        "cooldown_seconds": 300
      },
      "lane_policies": {
        "research": {
          "enabled": true,
          "mode": "recommend_only",
          "max_runners": 3,
          "scale_from_events": true,
          "event_weight": 2
        }
      }
    }
  }
}
```

Fields omitted from a `CapacityScalingPolicy` inherit safe defaults. Use
`mode=recommend_only` for local and development profiles. Use
`mode=provision_ephemeral` only for sandboxed lanes with template refs, approval
policy, cooldowns, and finite `max_runners`. Per-lane policies override the
default by runner pool key; otherwise the default policy applies.

## Restate Cloud Defaults

The `restate-cloud` profile keeps the same local service stack but points
Restate ingress/admin defaults at the tunnel ports:

```sh
coat --config-profile restate-cloud setup config --show
coat --config-profile restate-cloud deploy local up --restate-cloud --init-env
coat --config-profile restate-cloud deploy local up --restate-cloud --register-cloud --allow-stub-runners
```

The Restate Cloud env file path, tunnel name, and coordinator service URL come
from `config.cloud.restate_cloud` unless overridden by flags or environment.

## EKS Defaults

The `eks` profile standardizes Kubernetes and AWS deployment assumptions:

```sh
coat --config-profile eks setup config --show
coat --config-profile eks deploy cluster render
coat --config-profile eks deploy cluster apply --dry-run=client
coat --config-profile eks deploy chart template --output /tmp/jattg.yaml
```

The profile defaults to namespace `jattg`, chart `infra/helm/jattg`, image
registry `ghcr.io/josephjohncox`, AWS secret providers, IRSA or EKS Pod
Identity, and S3-compatible object artifacts. Put cluster-specific context,
kubeconfig, account IDs, values files, and secret references in `~/.coat` or
`COAT_CONFIG`, not in the committed project file.
