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
- `infra/helm/jattg/Chart.yaml` chart `version`;
- `infra/helm/jattg/Chart.yaml` chart `appVersion`.

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
- chart `version` and `appVersion` bump in `infra/helm/jattg/Chart.yaml`;
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

Trigger it manually only when the release was already cut locally, and pass the exact release tag or ref as the workflow `ref` input. Manual dispatches must build from the intended release tag, not from the branch tip that happens to be selected in GitHub Actions.

```sh
git push origin v0.2.0
```

The workflow builds release binaries for:

- `x86_64-unknown-linux-gnu`;
- `aarch64-unknown-linux-gnu`;
- `aarch64-apple-darwin`.

Linux release binaries are built on native GitHub-hosted runners instead of cross-compiling ARM from x64: x64 Linux uses `ubuntu-22.04`, ARM Linux uses `ubuntu-22.04-arm`, and macOS ARM uses `macos-latest`. The main CI workflow also has a runner-target compatibility lane across `ubuntu-latest`, `ubuntu-24.04`, `ubuntu-22.04`, `ubuntu-24.04-arm`, `ubuntu-22.04-arm`, and `macos-latest` so runner-label regressions are caught before release.

It uploads tarballs plus SHA-256 files to the GitHub Release. After the binary build matrix passes, the same workflow publishes multi-arch service images to GHCR under `ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/...` with `vX.Y.Z`, `X.Y.Z`, and `latest` tags.
Release binary jobs use Rust dependency caches plus `sccache` compiler-output caching. GHCR image publishing uses GitHub Actions BuildKit caches and registry-backed cache images by default so the large Rust and sidecar layers can survive normal Actions cache churn. Rust service image tags are produced from one shared service image build, while the toolbox image remains a separate target in the same visible lane; Node sidecar images fan out in parallel lanes so a slow or failed sidecar does not hide behind one opaque serial publish step. Set `BUILDX_CACHE=false` to disable GitHub Actions cache and `BUILDX_REGISTRY_CACHE=false` to disable GHCR-backed cache when manually debugging the image script without remote cache state.
Rust service images are built from the `service` target in
`infra/containers/rust-service.Dockerfile`. The released service image contains
all Rust service binaries; deployments select the process with
`COAT_SERVICE_BIN`, and the entrypoint falls back to the known `BIND_ADDR` port
mapping for existing manifests. The `jattg-agent-toolbox` image is built from
the same Dockerfile's `agent-toolbox` target so ephemeral Kubernetes Jobs can
share the released Rust binaries and runner sidecars without creating a second
Rust base image.

Published JATTG images:

- `jattg-coordinator`;
- `jattg-event-gateway`;
- `jattg-goal-store`;
- `jattg-memory-gateway`;
- `jattg-notifier`;
- `jattg-runner-registry`;
- `jattg-sandbox-runner`;
- `jattg-tool-registry`;
- `jattg-validator`;
- `jattg-agent-toolbox`;
- `jattg-control-web`;
- `jattg-codex-runner`;
- `jattg-claude-code-runner`;
- `jattg-model-provider-runner`;
- `jattg-staff-engineer-runner`.

Local binary packaging after a release build:

```sh
cargo build --workspace --release
VERSION=0.2.0 scripts/package-binaries.sh
```

## Published Binary Smoke

After the `vX.Y.Z` GitHub Release is published, smoke the release assets
separately from the Helm chart. Pick the target that matches the operator
machine or CI runner: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
or `aarch64-apple-darwin`.

The binary release workflow runs the `x86_64-unknown-linux-gnu` smoke after the
assets are attached to the GitHub Release and before GHCR service images are
published. Operators can repeat the same proof locally for any supported target:

```sh
make release-binary-smoke VERSION=0.2.0 TARGET=aarch64-apple-darwin
```

Retry tags with suffixes are supported. The smoke target downloads the suffixed release asset but calls `coat release plan` with the base semantic version and `--tag-suffix` internally:

```sh
make release-binary-smoke VERSION=0.2.0-ghcr.1 TARGET=aarch64-apple-darwin
```

Set `RELEASE_URL` only when validating a fork, mirror, or retry location.

```sh
VERSION=0.2.0
TARGET=aarch64-apple-darwin
RELEASE_URL="https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/v${VERSION}"
ARCHIVE="jattg-binaries-${VERSION}-${TARGET}.tar.gz"

mkdir -p /tmp/jattg-binary-release-smoke
cd /tmp/jattg-binary-release-smoke
curl -fsSLO "${RELEASE_URL}/${ARCHIVE}"
curl -fsSLO "${RELEASE_URL}/${ARCHIVE}.sha256"
EXPECTED_SHA="$(cut -d ' ' -f 1 "${ARCHIVE}.sha256")"
printf '%s  %s\n' "${EXPECTED_SHA}" "${ARCHIVE}" | shasum -a 256 -c -
tar -xzf "${ARCHIVE}"

EXTRACTED="./jattg-binaries-${VERSION}-${TARGET}"
python3 -m json.tool "${EXTRACTED}/manifest.json" >/dev/null
for binary in \
  coat \
  coat-coordinator \
  coat-event-gateway \
  coat-goal-store \
  coat-memory-gateway \
  coat-notifier \
  coat-runner-registry \
  coat-sandbox-runner \
  coat-tool-registry \
  coat-validator
do
  test -x "${EXTRACTED}/bin/${binary}"
done

"${EXTRACTED}/bin/coat" --help
"${EXTRACTED}/bin/coat" guide --print
BASE_VERSION="${VERSION%%-*}"
TAG_SUFFIX=""
if [ "${BASE_VERSION}" != "${VERSION}" ]; then
  TAG_SUFFIX="${VERSION#*-}"
fi
if [ -n "${TAG_SUFFIX}" ]; then
  "${EXTRACTED}/bin/coat" release plan --version "${BASE_VERSION}" --tag-suffix "${TAG_SUFFIX}"
else
  "${EXTRACTED}/bin/coat" release plan --version "${BASE_VERSION}"
fi
```

The smoke passes when the checksum verifies, the archive expands, the released
manifest parses, every shipped binary is executable, and the extracted `coat`
CLI can print its operator surface and plan the same release version. Do not run
service binaries in this smoke; without a dedicated help/version flag they start
listeners. This does not validate the Helm install path and does not require
local Rust builds.

## Helm Chart Release

Helm chart releases are handled by `.github/workflows/release-helm.yml`.
When the workflow runs from a `chart-v*` tag, it packages the chart with the `appVersion` already committed in `infra/helm/jattg/Chart.yaml`; `workflow_dispatch` can still override it with `app_version`.

Trigger it manually only when the release was already cut locally, and pass the exact chart tag or ref as the workflow `ref` input. Manual dispatches must package from the intended chart release tag, not from the selected branch tip.

```sh
git push origin chart-v0.2.0
```

The workflow runs `helm lint`, packages `infra/helm/jattg`, generates `index.yaml`, and uploads the chart, index, and SHA-256 files to a separate GitHub Release.

Local chart packaging:

```sh
coat deploy chart package --chart-version 0.2.0 --app-version 0.2.0
```

Install from a chart release:

```sh
coat deploy chart upgrade \
  --chart https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/chart-v0.2.0/jattg-0.2.0.tgz \
  --wait
```

## Published Helm Chart Smoke

After the `chart-vX.Y.Z` GitHub Release is published, smoke the packaged chart
against a disposable namespace or an existing smoke release. Keep this separate
from binary release validation because it proves Helm consumption, image tag
selection, rollout behavior, and rollback mechanics.

The chart release workflow downloads the just-published `jattg` chart asset,
verifies its checksum, runs `coat deploy chart lint`, and renders a smoke
manifest. Operators can repeat the local no-cluster smoke with one command; the
target builds `target/debug/coat` first and uses that binary for chart lint,
template, and dry-run upgrade checks:

```sh
make release-helm-smoke CHART_VERSION=0.2.0 APP_VERSION=0.2.0
```

Set `HELM_SMOKE_APPLY=true` only when the active Kubernetes context points at a
disposable namespace or a cluster explicitly reserved for release validation.

```sh
CHART_VERSION=0.2.0
APP_VERSION=0.2.0
RELEASE=jattg-smoke
NAMESPACE=jattg-smoke
CHART_URL="https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/chart-v${CHART_VERSION}/jattg-${CHART_VERSION}.tgz"

coat deploy chart template \
  --release "${RELEASE}" \
  --namespace "${NAMESPACE}" \
  --chart "${CHART_URL}" \
  --set "global.imageTag=${APP_VERSION}" \
  --output /tmp/jattg-chart-release-smoke.yaml

coat deploy chart upgrade \
  --release "${RELEASE}" \
  --namespace "${NAMESPACE}" \
  --chart "${CHART_URL}" \
  --set "global.imageTag=${APP_VERSION}" \
  --dry-run

coat deploy chart upgrade \
  --release "${RELEASE}" \
  --namespace "${NAMESPACE}" \
  --chart "${CHART_URL}" \
  --set "global.imageTag=${APP_VERSION}" \
  --wait \
  --timeout 5m

coat deploy cluster status --namespace "${NAMESPACE}" --timeout 180s
```

For an upgrade smoke with a previous Helm revision, validate rollback before
promoting the chart:

```sh
coat deploy chart rollback \
  --release "${RELEASE}" \
  --namespace "${NAMESPACE}" \
  --wait \
  --timeout 5m

coat deploy cluster status --namespace "${NAMESPACE}" --timeout 180s

coat deploy chart upgrade \
  --release "${RELEASE}" \
  --namespace "${NAMESPACE}" \
  --chart "${CHART_URL}" \
  --set "global.imageTag=${APP_VERSION}" \
  --wait \
  --timeout 5m
```

Fresh installs have no prior revision to roll back to. Treat a failed first
install as a failed chart smoke, fix the release, and cut a retry tag instead of
marking the release healthy.

## Release Smoke Evidence

Record the first successful published smoke in this section or in the release
notes before marking the `ReleaseHardening` follow-up done. Include:

- binary version, target triple, release URL, workflow run URL or local command,
  and checksum result;
- chart version, app image tag, chart URL, namespace, whether apply was dry-run
  or live, rendered manifest path or workflow artifact, and rollback result when
  a prior revision existed;
- any skipped proof with the exact reason, such as no published release yet, no
  disposable cluster, or no rollback revision.

## Guardrails

- Do not publish from unreviewed dirty state.
- Do not put live secrets in chart defaults.
- Keep binary and chart tags separate.
- Prefer `coat release cut --version X --push` over hand-written release tags.
- Run `cargo test --workspace`, `buf lint`, `make ts-build`, and `coat deploy chart lint` before tagging when local tools are available.
