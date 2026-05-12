SHELL := /bin/sh

CARGO ?= cargo
NPM ?= npm
NODE ?= node
BUF ?= buf
COAT ?= coat
BUF_GENERATE_HOME ?= $(CURDIR)/target/buf-home

COAT_BUILD_PROFILE ?= debug
ifeq ($(COAT_BUILD_PROFILE),release)
COAT_BUILD_ARGS := --release
COAT_BIN_DIR := target/release
else ifeq ($(COAT_BUILD_PROFILE),debug)
COAT_BUILD_ARGS :=
COAT_BIN_DIR := target/debug
else
$(error COAT_BUILD_PROFILE must be debug or release)
endif

SIDECAR_DIRS := \
	sidecars/codex-runner-ts \
	sidecars/claude-code-runner-ts \
	sidecars/staff-engineer-runner-ts \
	sidecars/model-provider-runner-ts

TS_DIRS := \
	$(SIDECAR_DIRS) \
	ui/control-plane-web

NPM_CI_FLAGS ?= --prefer-offline --no-audit --fund=false

.DEFAULT_GOAL := build

.PHONY: \
	build coat-cli coat-cli-release coat-path \
	ci ci-rust fmt fmt-check test check schemas proto-lint proto-format proto-check docs-check \
	proto-sdk-generate proto-sdk-check \
	event-gateway-smoke eventops-sqs-smoke runner-smoke compose-runner-smoke \
	release-binary-smoke release-helm-smoke \
	ts-install sidecars-build control-web-build control-web-smoke ts-build \
	helm-lint helm-package \
	compose-config compose-cloud-config compose-up compose-cloud-up compose-down compose-cloud-down \
	k8s-render

build: coat-cli

coat-cli:
	$(CARGO) build -p coat-cli $(COAT_BUILD_ARGS)

event-gateway-smoke:
	$(CARGO) build -p coat-event-gateway -p coat-goal-store $(COAT_BUILD_ARGS)
	COAT_EVENT_GATEWAY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-event-gateway-smoke.sh

eventops-sqs-smoke:
	COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-eventops-sqs-smoke.sh

runner-smoke:
	$(CARGO) build -p coat-cli -p coat-runner-registry $(COAT_BUILD_ARGS)
	COAT_RUNNER_REGISTRY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-runner-registry-smoke.sh

compose-runner-smoke:
	sh scripts/coat-compose-runner-smoke.sh

coat-cli-release:
	$(MAKE) coat-cli COAT_BUILD_PROFILE=release

coat-path:
	@printf '%s/coat\n' '$(COAT_BIN_DIR)'

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

test:
	$(CARGO) test --workspace

check:
	$(CARGO) check --workspace

ci-rust: fmt-check
	$(CARGO) test --workspace --all-targets
	$(CARGO) build -p coat-cli -p coat-event-gateway -p coat-goal-store -p coat-runner-registry $(COAT_BUILD_ARGS)
	COAT_EVENT_GATEWAY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-event-gateway-smoke.sh
	COAT_RUNNER_REGISTRY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-runner-registry-smoke.sh

schemas:
	$(CARGO) run -p coat-domain --bin generate-schemas -- schemas

proto-lint:
	$(BUF) lint

proto-format:
	$(BUF) format -w

proto-check:
	tmp_dir=$$(mktemp -d); \
	trap 'rm -rf "$$tmp_dir"' EXIT INT TERM; \
	cp -R schemas "$$tmp_dir/schemas.before"; \
	$(CARGO) run -p coat-domain --bin generate-schemas -- schemas; \
	diff -ru "$$tmp_dir/schemas.before" schemas
	$(BUF) lint
	$(BUF) format --diff --exit-code

proto-sdk-generate:
	mkdir -p "$(BUF_GENERATE_HOME)"
	HOME="$(BUF_GENERATE_HOME)" $(BUF) generate --template buf.gen.yaml

proto-sdk-check: proto-sdk-generate
	test -d target/generated-sdks/rust
	test -d target/generated-sdks/typescript
	test -s target/generated-sdks/rust/coat/v1/coat.v1.rs
	test -s target/generated-sdks/rust/coat/v1/coat.v1.tonic.rs
	test -s target/generated-sdks/typescript/coat/v1/common_pb.js
	test -s target/generated-sdks/typescript/coat/v1/common_pb.d.ts

docs-check:
	sh scripts/coat-doc-gardener.sh

ts-install:
	@set -eu; \
	for dir in $(TS_DIRS); do \
		echo "installing $$dir"; \
		$(NPM) ci --prefix "$$dir" $(NPM_CI_FLAGS); \
	done

sidecars-build:
	@set -eu; \
	for dir in $(SIDECAR_DIRS); do \
		echo "building $$dir"; \
		if [ -x "$$dir/node_modules/.bin/tsc" ]; then \
			"$$dir/node_modules/.bin/tsc" -p "$$dir/tsconfig.json"; \
		elif [ -f "$$dir/node_modules/typescript/bin/tsc" ]; then \
			$(NODE) "$$dir/node_modules/typescript/bin/tsc" -p "$$dir/tsconfig.json"; \
		else \
			$(NPM) ci --prefix "$$dir" $(NPM_CI_FLAGS); \
			$(NPM) run --prefix "$$dir" build; \
		fi; \
	done

control-web-build:
	@set -eu; \
	dir=ui/control-plane-web; \
	echo "building $$dir"; \
	if [ ! -d "$$dir/node_modules" ]; then \
		$(NPM) ci --prefix "$$dir" $(NPM_CI_FLAGS); \
	fi; \
	$(NPM) run --prefix "$$dir" build

control-web-smoke: control-web-build
	$(NPM) run --prefix ui/control-plane-web smoke

ts-build: sidecars-build control-web-build

helm-lint:
	$(COAT) deploy chart lint

helm-package:
	$(COAT) deploy chart package

release-binary-smoke:
	@test -n "$(VERSION)" || { echo "VERSION is required, for example: make release-binary-smoke VERSION=0.2.0 TARGET=aarch64-apple-darwin"; exit 2; }
	@target="$(TARGET)"; \
	if [ -z "$$target" ]; then \
		target="$$(rustc -vV | awk '/host:/ { print $$2; exit }')"; \
	fi; \
	archive="jattg-binaries-$(VERSION)-$$target.tar.gz"; \
	release_url="$(RELEASE_URL)"; \
	if [ -z "$$release_url" ]; then \
		release_url="https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/v$(VERSION)"; \
	fi; \
	tmp_dir="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp_dir"' EXIT INT TERM; \
	cd "$$tmp_dir"; \
	curl --retry 6 --retry-delay 5 --retry-all-errors -fsSLO "$$release_url/$$archive"; \
	curl --retry 6 --retry-delay 5 --retry-all-errors -fsSLO "$$release_url/$$archive.sha256"; \
	expected_sha="$$(cut -d ' ' -f 1 "$$archive.sha256")"; \
	printf '%s  %s\n' "$$expected_sha" "$$archive" | shasum -a 256 -c -; \
	tar -xzf "$$archive"; \
	extracted="./jattg-binaries-$(VERSION)-$$target"; \
	python3 -m json.tool "$$extracted/manifest.json" >/dev/null; \
	for binary in coat coat-coordinator coat-event-gateway coat-goal-store coat-memory-gateway coat-notifier coat-runner-registry coat-sandbox-runner coat-tool-registry coat-validator; do \
		test -x "$$extracted/bin/$$binary"; \
	done; \
	"$$extracted/bin/coat" --help >/dev/null; \
	"$$extracted/bin/coat" guide --print >/dev/null; \
	base_version="$(VERSION)"; \
	tag_suffix=""; \
	case "$$base_version" in *-*) tag_suffix="$${base_version#*-}"; base_version="$${base_version%%-*}";; esac; \
	if [ -n "$$tag_suffix" ]; then \
		"$$extracted/bin/coat" release plan --version "$$base_version" --tag-suffix "$$tag_suffix" >/dev/null; \
	else \
		"$$extracted/bin/coat" release plan --version "$$base_version" >/dev/null; \
	fi; \
	echo "smoked published binary release v$(VERSION) for $$target"

release-helm-smoke: coat-cli
	@test -n "$(CHART_VERSION)" || { echo "CHART_VERSION is required, for example: make release-helm-smoke CHART_VERSION=0.2.0 APP_VERSION=0.2.0"; exit 2; }
	@app_version="$(APP_VERSION)"; \
	if [ -z "$$app_version" ]; then app_version="$(CHART_VERSION)"; fi; \
	release="$(RELEASE)"; \
	if [ -z "$$release" ]; then release="jattg-smoke"; fi; \
	namespace="$(NAMESPACE)"; \
	if [ -z "$$namespace" ]; then namespace="jattg-smoke"; fi; \
	chart_url="$(CHART_URL)"; \
	if [ -z "$$chart_url" ]; then \
		chart_url="https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/chart-v$(CHART_VERSION)/jattg-$(CHART_VERSION).tgz"; \
	fi; \
	tmp_dir="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp_dir"' EXIT INT TERM; \
	chart="$$tmp_dir/jattg-$(CHART_VERSION).tgz"; \
	curl --retry 6 --retry-delay 5 --retry-all-errors -fsSL "$$chart_url" -o "$$chart"; \
	curl --retry 6 --retry-delay 5 --retry-all-errors -fsSL "$$chart_url.sha256" -o "$$chart.sha256"; \
	expected_sha="$$(cut -d ' ' -f 1 "$$chart.sha256")"; \
	printf '%s  %s\n' "$$expected_sha" "$$chart" | shasum -a 256 -c -; \
	$(COAT_BIN_DIR)/coat deploy chart lint --chart "$$chart"; \
	$(COAT_BIN_DIR)/coat deploy chart template --release "$$release" --namespace "$$namespace" --chart "$$chart" --set "global.imageTag=$$app_version" --output "$$tmp_dir/rendered.yaml"; \
	test -s "$$tmp_dir/rendered.yaml"; \
	$(COAT_BIN_DIR)/coat deploy chart upgrade --release "$$release" --namespace "$$namespace" --chart "$$chart" --set "global.imageTag=$$app_version" --dry-run; \
	if [ "$${HELM_SMOKE_APPLY:-false}" = "true" ]; then \
		$(COAT_BIN_DIR)/coat deploy chart upgrade --release "$$release" --namespace "$$namespace" --chart "$$chart" --set "global.imageTag=$$app_version" --wait --timeout "$${HELM_SMOKE_TIMEOUT:-5m}"; \
		$(COAT_BIN_DIR)/coat deploy cluster status --namespace "$$namespace" --timeout "$${CLUSTER_SMOKE_TIMEOUT:-180s}"; \
	fi; \
	echo "smoked published Helm chart $(CHART_VERSION) with image tag $$app_version"

ci: ci-rust proto-check docs-check ts-install ts-build control-web-smoke
	git diff --check

compose-config:
	$(COAT) deploy local config

compose-cloud-config:
	$(COAT) deploy local config --restate-cloud --restate-cloud-env-file infra/compose/restate-cloud.env.example --allow-placeholder-env

compose-up:
	$(COAT) deploy local up --allow-stub-runners

compose-cloud-up:
	$(COAT) deploy local up --restate-cloud --allow-stub-runners

compose-down:
	$(COAT) deploy local down

compose-cloud-down:
	$(COAT) deploy local down --restate-cloud

k8s-render:
	$(COAT) deploy cluster render --output infra/k8s/rendered.yaml
