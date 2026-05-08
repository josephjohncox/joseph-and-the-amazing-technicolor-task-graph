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

## Project Config

`.coat/project.json` is safe to commit. It defines shared non-secret profiles:

- project and package slugs;
- `cli`: operator output and local service endpoint defaults;
- `local`: Docker Compose service URLs, env-file locations, and local data/cache
  paths;
- `restate-cloud`: local services pointed at the Restate Cloud tunnel ingress;
- `eks`: Kubernetes namespace, manifest, Helm chart, image registry, workload
  identity, secret provider, and S3 object-store defaults.

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
