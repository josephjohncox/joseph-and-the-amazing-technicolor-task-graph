# Release Operations

COAT publishes two independent GitHub Release streams:

- binary releases on tags shaped like `v0.1.0`;
- Helm chart releases on tags shaped like `chart-v0.1.0`.

Keep them separate. A binary release proves the Rust service and CLI binaries can be downloaded directly. A chart release proves the Kubernetes install bundle can be consumed by Helm.

## Bump Versions

Use the CLI to preview the release shape:

```sh
coat release plan --version 0.2.0
```

Use the CLI to bump files without committing or tagging:

```sh
coat release bump --version 0.2.0 --allow-dirty
```

The bump command updates:

- `Cargo.toml` workspace package version;
- `Cargo.lock` workspace package versions;
- `infra/helm/coat/Chart.yaml` chart `version`;
- `infra/helm/coat/Chart.yaml` chart `appVersion`.

By default `release bump` refuses to run in a dirty worktree. Pass `--allow-dirty` while preparing an unreleased branch or active scaffold.

If the chart version must diverge from the app version, use:

```sh
coat release bump \
  --version 0.2.0 \
  --chart-version 0.2.1 \
  --app-version 0.2.0
```

## Cut A Release

Use `release cut` from a clean worktree to bump versions, create the release commit, and create both local release tags:

```sh
coat release cut --version 0.2.0
```

That command performs:

- version bump in `Cargo.toml`;
- lockfile refresh in `Cargo.lock`;
- chart `version` and `appVersion` bump in `infra/helm/coat/Chart.yaml`;
- release commit `chore(release): v0.2.0`;
- annotated binary tag `v0.2.0`;
- annotated chart tag `chart-v0.2.0`.

If the chart version must be cut separately from the app version, pass it to `release cut` as well:

```sh
coat release cut \
  --version 0.2.0 \
  --chart-version 0.2.1 \
  --app-version 0.2.0
```

That creates binary tag `v0.2.0` and chart tag `chart-v0.2.1`.

If a release tag already ran but a build/publish issue requires a fresh cut from the fixed commit without changing the app or chart versions, add a tag suffix:

```sh
coat release cut --version 0.2.0 --tag-suffix ghcr.1
```

This creates retry tags such as `v0.2.0-ghcr.1` and `chart-v0.2.0-ghcr.1`. If `Cargo.toml`, `Cargo.lock`, and `Chart.yaml` already match the requested versions, the CLI tags the current commit instead of requiring a no-op release commit.

Push the release commit and tags, triggering both GitHub release workflows, by passing `--push`:

```sh
coat release cut --version 0.2.0 --push
```

Use `--dry-run` to print the release shape without modifying files. Use `--no-verify` only when release hooks are unavailable and the validation has been run separately.

## Binary Release

Binary releases are handled by `.github/workflows/release-binaries.yml`.

Trigger it manually only when the release was already cut locally:

```sh
git tag v0.2.0
git push origin v0.2.0
```

The workflow builds release binaries for:

- `x86_64-unknown-linux-gnu`;
- `aarch64-unknown-linux-gnu`;
- `aarch64-apple-darwin`.

It uploads tarballs plus SHA-256 files to the GitHub Release. After the binary build matrix passes, the same workflow publishes multi-arch service images to GHCR under `ghcr.io/<owner>/<repo>/...` with `vX.Y.Z`, `X.Y.Z`, and `latest` tags.

Published COAT images:

- `coat-coordinator`;
- `coat-event-gateway`;
- `coat-goal-store`;
- `coat-memory-gateway`;
- `coat-notifier`;
- `coat-runner-registry`;
- `coat-sandbox-runner`;
- `coat-tool-registry`;
- `coat-validator`;
- `coat-control-web`;
- `coat-codex-runner`;
- `coat-staff-engineer-runner`.

Local binary packaging after a release build:

```sh
cargo build --workspace --release
VERSION=0.2.0 scripts/package-binaries.sh
```

## Helm Chart Release

Helm chart releases are handled by `.github/workflows/release-helm.yml`.
When the workflow runs from a `chart-v*` tag, it packages the chart with the `appVersion` already committed in `infra/helm/coat/Chart.yaml`; `workflow_dispatch` can still override it with `app_version`.

Trigger it manually only when the release was already cut locally:

```sh
git tag chart-v0.2.0
git push origin chart-v0.2.0
```

The workflow runs `helm lint`, packages `infra/helm/coat`, generates `index.yaml`, and uploads the chart, index, and SHA-256 files to a separate GitHub Release.

Local chart packaging:

```sh
CHART_VERSION=0.2.0 APP_VERSION=0.2.0 scripts/package-helm-chart.sh
```

Install from a chart release:

```sh
helm install coat https://github.com/OWNER/REPO/releases/download/chart-v0.2.0/coat-0.2.0.tgz
```

## Guardrails

- Do not publish from unreviewed dirty state.
- Do not put live secrets in chart defaults.
- Keep binary and chart tags separate.
- Prefer `coat release cut --version X --push` over hand-written release tags.
- Run `cargo test --workspace`, `buf lint`, `make ts-build`, and `helm lint infra/helm/coat` before tagging when local tools are available.
