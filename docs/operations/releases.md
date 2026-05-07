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

Use the CLI to bump files:

```sh
coat release bump --version 0.2.0 --allow-dirty
```

The bump command updates:

- `Cargo.toml` workspace package version;
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

## Binary Release

Binary releases are handled by `.github/workflows/release-binaries.yml`.

Trigger it by pushing:

```sh
git tag v0.2.0
git push origin v0.2.0
```

The workflow builds release binaries for:

- `x86_64-unknown-linux-gnu`;
- `aarch64-unknown-linux-gnu`;
- `aarch64-apple-darwin`;
- `x86_64-apple-darwin`.

It uploads tarballs plus SHA-256 files to the GitHub Release.

Local binary packaging after a release build:

```sh
cargo build --workspace --release
VERSION=0.2.0 scripts/package-binaries.sh
```

## Helm Chart Release

Helm chart releases are handled by `.github/workflows/release-helm.yml`.

Trigger it by pushing:

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
- Run `cargo test --workspace`, `buf lint`, `make ts-build`, and `helm lint infra/helm/coat` before tagging when local tools are available.
