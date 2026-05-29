SHELL := /bin/sh

CARGO ?= cargo
NPM ?= npm
NODE ?= node
BUF ?= buf
HELM ?= helm
COAT ?= $(COAT_BIN_DIR)/coat
BUF_GENERATE_HOME ?= $(CURDIR)/target/buf-home
NODE_MIN_VERSION ?= 22.12.0
SCENARIO_E2E_OUT ?= target/coat-scenarios
SCENARIO_E2E_SPECS ?= scenarios/e2e/*.json
SCENARIO_E2E_STACK ?= auto
SCENARIO_E2E_SERVICES ?=
SCENARIO_E2E_KEEP_STACK ?= 1
ifneq ($(strip $(stack)),)
SCENARIO_E2E_STACK := $(stack)
endif
BOOTSTRAP_SCENARIO_SPECS ?= \
	scenarios/e2e/bootstrap_basic.json \
	scenarios/e2e/bootstrap_running.json \
	scenarios/e2e/bootstrap_pending_action.json \
	scenarios/e2e/bootstrap_human_input_thunk_resume.json \
	scenarios/e2e/bootstrap_approval.json \
	scenarios/e2e/bootstrap_fanout.json \
	scenarios/e2e/bootstrap_fork_join.json \
	scenarios/e2e/bootstrap_signal_driven.json \
	scenarios/e2e/bootstrap_blocked_retry_recovery.json \
	scenarios/e2e/bootstrap_cancelled_queue_history.json \
	scenarios/e2e/bootstrap_memory_research_evidence.json \
	scenarios/e2e/operator_usability_workbench.json \
	scenarios/e2e/blocked_and_resumed.json \
	scenarios/e2e/goal_lifecycle_basic.json
BOOTSTRAP_SCENARIO_OUT ?= target/coat-scenarios/bootstrap
TASK_GRAPH_SCENARIO_SPECS ?= \
	scenarios/e2e/fanout_until_done.json \
	scenarios/e2e/fork_join_review.json \
	scenarios/e2e/long_iterative_loop.json \
	scenarios/e2e/blocked_and_resumed.json
TASK_GRAPH_VALIDATION_OUT ?= target/coat-scenarios/task-graph
TASK_GRAPH_VALIDATION_TESTS ?= \
	initial_tasks_become_queryable_subgoal_tasks \
	child_task_inherits_execution_profile_with_new_role \
	compute_graph_projects_tasks_thunks_continuations_and_wait_refs \
	worker_waiting_result_materializes_delayed_compute_thunk \
	branch_group_spawns_candidates_votes_and_auto_selects \
	coordinator_rejects_branch_vote_for_unvalidated_candidate \
	patch_merger_selects_validated_checkpoint_branch_candidate
RESET_DRY_RUN ?= 0
RESET_BOOTSTRAP ?= 0
RESET_ARGS ?=
RESET_COMPOSE_ENV_FILE ?=
EXERCISE_MODE ?= quick
EXERCISE_OUT ?= target/coat-scenarios/latest
EXERCISE_ARGS ?=
RUNTIME_LIVE_SCAFFOLD_OUT ?= target/coat-runtime-live-scaffold

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
	ci ci-rust ci-node ci-pr fmt fmt-check test check schemas proto-lint proto-format proto-check docs-check \
	proto-sdk-generate proto-sdk-check \
	event-gateway-smoke event-gateway-compose-smoke eventops-sqs-smoke runner-smoke compose-runner-smoke \
	exercise-system exercise-quick exercise-demo exercise-e2e exercise-ui exercise-full exercise-dry-run \
	runtime-live-scaffold \
	scenario-e2e scenario-e2e-stack scenario-e2e-ui scenario-e2e-ui-live \
	bootstrap-scenarios task-graph-validation validate-task-graph-bootstraps \
	bootstrap-goals bootstrap-fixture-goals \
	reset-help reset-smoke scenario-reset scenario-reset-dry-run bootstrap-reset bootstrap-reset-dry-run compose-reset compose-reset-dry-run \
	release-binary-smoke release-helm-smoke \
	node-version-check ts-install sidecars-build control-web-build control-web-smoke ts-build \
	helm-lint helm-package \
	compose-config compose-cloud-config compose-up compose-cloud-up compose-down compose-cloud-down \
	k8s-render

build: coat-cli

coat-cli:
	$(CARGO) build -p coat-cli $(COAT_BUILD_ARGS)

event-gateway-smoke:
	$(CARGO) build -p coat-event-gateway -p coat-goal-store $(COAT_BUILD_ARGS)
	COAT_EVENT_GATEWAY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-event-gateway-smoke.sh

event-gateway-compose-smoke: coat-cli
	sh scripts/coat-event-gateway-compose-smoke.sh

eventops-sqs-smoke:
	COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-eventops-sqs-smoke.sh

runner-smoke:
	$(CARGO) build -p coat-cli -p coat-runner-registry $(COAT_BUILD_ARGS)
	COAT_RUNNER_REGISTRY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-runner-registry-smoke.sh

compose-runner-smoke:
	sh scripts/coat-compose-runner-smoke.sh

runtime-live-scaffold:
	COAT_RUNTIME_LIVE_SCAFFOLD_OUT="$(RUNTIME_LIVE_SCAFFOLD_OUT)" \
	sh scripts/coat-runtime-live-scaffold.sh

exercise-system:
	COAT_EXERCISE_OUT="$(EXERCISE_OUT)" \
	sh scripts/coat-exercise-system.sh --mode "$(EXERCISE_MODE)" $(EXERCISE_ARGS)

exercise-quick:
	$(MAKE) exercise-system EXERCISE_MODE=quick

exercise-demo:
	$(MAKE) exercise-system EXERCISE_MODE=demo

exercise-e2e:
	$(MAKE) exercise-system EXERCISE_MODE=e2e

exercise-ui:
	$(MAKE) exercise-system EXERCISE_MODE=ui

exercise-full:
	$(MAKE) exercise-system EXERCISE_MODE=full

exercise-dry-run:
	$(MAKE) exercise-quick EXERCISE_ARGS=--dry-run
	$(MAKE) exercise-demo EXERCISE_ARGS=--dry-run
	$(MAKE) exercise-e2e EXERCISE_ARGS=--dry-run
	$(MAKE) exercise-ui EXERCISE_ARGS=--dry-run
	$(MAKE) exercise-full EXERCISE_ARGS=--dry-run

scenario-e2e: coat-cli
	COAT="$(COAT)" \
	COAT_SCENARIO_E2E_OUT="$(SCENARIO_E2E_OUT)" \
	COAT_SCENARIO_E2E_SPECS="$(SCENARIO_E2E_SPECS)" \
	COAT_SCENARIO_E2E_STACK="$(SCENARIO_E2E_STACK)" \
	COAT_SCENARIO_E2E_SERVICES="$(SCENARIO_E2E_SERVICES)" \
	COAT_SCENARIO_E2E_KEEP_STACK="$(SCENARIO_E2E_KEEP_STACK)" \
	sh scripts/coat-scenario-e2e.sh

scenario-e2e-stack:
	COAT="$(COAT)" \
	COAT_SCENARIO_E2E_OUT="$(SCENARIO_E2E_OUT)" \
	COAT_SCENARIO_E2E_STACK=always \
	COAT_SCENARIO_E2E_STACK_ONLY=1 \
	COAT_SCENARIO_E2E_SERVICES="$(SCENARIO_E2E_SERVICES)" \
	COAT_SCENARIO_E2E_KEEP_STACK="$(SCENARIO_E2E_KEEP_STACK)" \
	sh scripts/coat-scenario-e2e.sh

scenario-e2e-ui: control-web-build
	$(NPM) run --prefix ui/control-plane-web test:e2e

scenario-e2e-ui-live: control-web-build
	$(MAKE) scenario-e2e-stack SCENARIO_E2E_KEEP_STACK=1
	@status=0; \
	COAT_CONTROL_E2E_USE_EXISTING_SERVER=1 \
	COAT_CONTROL_E2E_LIVE=1 \
	PLAYWRIGHT_BASE_URL=http://127.0.0.1:9090 \
	$(NPM) run --prefix ui/control-plane-web test:e2e:live || status=$$?; \
	$(COAT) deploy local down --env-file target/coat-scenarios/latest/stack/stub-local-providers.env || true; \
	exit $$status

bootstrap-scenarios: coat-cli
	COAT="$(COAT)" \
	COAT_BOOTSTRAP_SCENARIO_SPECS="$(BOOTSTRAP_SCENARIO_SPECS)" \
	COAT_BOOTSTRAP_SCENARIO_OUT="$(BOOTSTRAP_SCENARIO_OUT)" \
	COAT_BOOTSTRAP_SCENARIO_GATEWAY_URL="http://127.0.0.1:0" \
	COAT_BOOTSTRAP_SEED_GOALS=false \
	sh scripts/coat-bootstrap-scenarios.sh

bootstrap-goals: coat-cli
	COAT="$(COAT)" \
	sh scripts/coat-bootstrap-live-scenarios.sh

bootstrap-fixture-goals: coat-cli
	COAT="$(COAT)" \
	COAT_BOOTSTRAP_SCENARIO_SPECS="$(BOOTSTRAP_SCENARIO_SPECS)" \
	COAT_BOOTSTRAP_SCENARIO_OUT="$(BOOTSTRAP_SCENARIO_OUT)" \
	COAT_BOOTSTRAP_SCENARIO_GATEWAY_URL="http://127.0.0.1:0" \
	COAT_BOOTSTRAP_SEED_GOALS=true \
	sh scripts/coat-bootstrap-scenarios.sh

task-graph-validation: coat-cli
	@set -eu; \
	for test_name in $(TASK_GRAPH_VALIDATION_TESTS); do \
		echo "running coat-domain $$test_name"; \
		$(CARGO) test -p coat-domain "$$test_name"; \
	done
	$(MAKE) scenario-e2e \
		SCENARIO_E2E_SPECS="$(TASK_GRAPH_SCENARIO_SPECS)" \
		SCENARIO_E2E_OUT="$(TASK_GRAPH_VALIDATION_OUT)" \
		SCENARIO_E2E_STACK=never \
		SCENARIO_E2E_KEEP_STACK=0

validate-task-graph-bootstraps: bootstrap-scenarios task-graph-validation

reset-help:
	sh scripts/coat-local-reset.sh --help

reset-smoke:
	sh -n scripts/coat-local-reset.sh scripts/coat-bootstrap-scenarios.sh scripts/coat-bootstrap-live-scenarios.sh scripts/coat-scenario-e2e.sh scripts/coat-local-provider-setup.sh scripts/coat-exercise-system.sh scripts/coat-runtime-live-scaffold.sh
	$(MAKE) reset-help
	$(MAKE) exercise-dry-run
	$(MAKE) runtime-live-scaffold
	$(MAKE) scenario-reset-dry-run
	$(MAKE) bootstrap-reset-dry-run
	$(MAKE) compose-reset-dry-run

scenario-reset:
	@set -eu; \
	args="--mode scenario"; \
	if [ "$(RESET_BOOTSTRAP)" = "1" ]; then args="$$args --mode bootstrap"; fi; \
	if [ "$(RESET_DRY_RUN)" = "1" ]; then args="$$args --dry-run"; fi; \
	COAT_RESET_SCENARIO_OUT="$(SCENARIO_E2E_OUT)" \
	COAT_RESET_BOOTSTRAP_OUT="$(BOOTSTRAP_SCENARIO_OUT)" \
	COAT_RESET_SCENARIO_SPECS="$(SCENARIO_E2E_SPECS)" \
	COAT_RESET_BOOTSTRAP_SPECS="$(SCENARIO_E2E_SPECS)" \
	sh scripts/coat-local-reset.sh $$args $(RESET_ARGS)

scenario-reset-dry-run:
	$(MAKE) scenario-reset RESET_DRY_RUN=1

bootstrap-reset:
	@set -eu; \
	args="--mode bootstrap"; \
	if [ "$(RESET_DRY_RUN)" = "1" ]; then args="$$args --dry-run"; fi; \
	COAT_RESET_BOOTSTRAP_OUT="$(BOOTSTRAP_SCENARIO_OUT)" \
	COAT_RESET_BOOTSTRAP_SPECS="$(SCENARIO_E2E_SPECS)" \
	sh scripts/coat-local-reset.sh $$args $(RESET_ARGS)

bootstrap-reset-dry-run:
	$(MAKE) bootstrap-reset RESET_DRY_RUN=1

compose-reset:
	@set -eu; \
	args="--mode stack"; \
	if [ "$(RESET_DRY_RUN)" = "1" ]; then args="$$args --dry-run"; fi; \
	if [ -n "$(RESET_COMPOSE_ENV_FILE)" ]; then args="$$args --env-file $(RESET_COMPOSE_ENV_FILE)"; fi; \
	sh scripts/coat-local-reset.sh $$args $(RESET_ARGS)

compose-reset-dry-run:
	$(MAKE) compose-reset RESET_DRY_RUN=1

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

node-version-check:
	@$(NODE) -e 'const min = "$(NODE_MIN_VERSION)".split(".").map(Number); const got = process.versions.node.split(".").map(Number); const ok = got[0] > min[0] || (got[0] === min[0] && (got[1] > min[1] || (got[1] === min[1] && got[2] >= min[2]))); if (!ok) { console.error("Node " + process.versions.node + " is too old; COAT TypeScript builds require >= $(NODE_MIN_VERSION). Run `nvm use`, install the version in .nvmrc, or set NODE=/path/to/node."); process.exit(1); } console.log("Node " + process.versions.node + " satisfies >= $(NODE_MIN_VERSION)");'

ts-install: node-version-check
	@set -eu; \
	for dir in $(TS_DIRS); do \
		echo "installing $$dir"; \
		$(NPM) ci --prefix "$$dir" $(NPM_CI_FLAGS); \
	done

sidecars-build: node-version-check
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

control-web-build: node-version-check
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

ci-node: ts-install
	$(MAKE) sidecars-build
	$(MAKE) control-web-build
	$(NPM) run --prefix ui/control-plane-web smoke

helm-lint:
	$(COAT) deploy chart lint

helm-package:
	$(COAT) deploy chart package

release-binary-smoke:
	@test -n "$(VERSION)" || { echo "VERSION is required, for example: make release-binary-smoke VERSION=0.2.0 TARGET=aarch64-apple-darwin"; exit 2; }
	@set -eu; \
	target="$(TARGET)"; \
	host="$$(rustc -vV | awk '/host:/ { print $$2; exit }' 2>/dev/null || true)"; \
	if [ -z "$$target" ]; then \
		target="$$host"; \
	fi; \
	if [ -z "$$target" ]; then \
		echo "TARGET is required when rustc host detection is unavailable"; \
		exit 2; \
	fi; \
	archive="jattg-binaries-$(VERSION)-$$target.tar.gz"; \
	release_url="$(RELEASE_URL)"; \
	tmp_dir="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp_dir"' EXIT INT TERM; \
	cd "$$tmp_dir"; \
	if [ -z "$$release_url" ] && command -v gh >/dev/null 2>&1; then \
		gh release download "v$(VERSION)" --repo josephjohncox/joseph-and-the-amazing-technicolor-task-graph --pattern "$$archive" --pattern "$$archive.sha256" --dir "$$tmp_dir"; \
	else \
		if [ -z "$$release_url" ]; then \
			release_url="https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/v$(VERSION)"; \
		fi; \
		curl --retry 6 --retry-delay 5 --retry-all-errors -fsSLO "$$release_url/$$archive"; \
		curl --retry 6 --retry-delay 5 --retry-all-errors -fsSLO "$$release_url/$$archive.sha256"; \
	fi; \
	expected_sha="$$(cut -d ' ' -f 1 "$$archive.sha256")"; \
	printf '%s  %s\n' "$$expected_sha" "$$archive" | shasum -a 256 -c -; \
	tar -xzf "$$archive"; \
	extracted="./jattg-binaries-$(VERSION)-$$target"; \
	python3 -m json.tool "$$extracted/manifest.json" >/dev/null; \
	for binary in coat coat-coordinator coat-event-gateway coat-goal-store coat-memory-gateway coat-notifier coat-runner-registry coat-sandbox-runner coat-tool-registry coat-validator; do \
		test -x "$$extracted/bin/$$binary"; \
	done; \
	if [ "$$target" = "$$host" ]; then \
		COAT_ALLOW_UNINITIALIZED=1 "$$extracted/bin/coat" --help >/dev/null; \
		COAT_ALLOW_UNINITIALIZED=1 "$$extracted/bin/coat" guide --print >/dev/null; \
		base_version="$(VERSION)"; \
		tag_suffix=""; \
		case "$$base_version" in *-*) tag_suffix="$${base_version#*-}"; base_version="$${base_version%%-*}";; esac; \
		if [ -n "$$tag_suffix" ]; then \
			COAT_ALLOW_UNINITIALIZED=1 "$$extracted/bin/coat" release plan --version "$$base_version" --tag-suffix "$$tag_suffix" >/dev/null; \
		else \
			COAT_ALLOW_UNINITIALIZED=1 "$$extracted/bin/coat" release plan --version "$$base_version" >/dev/null; \
		fi; \
	else \
		echo "skipped executable smoke for $$target on host $${host:-unknown}"; \
	fi; \
	echo "smoked published binary release v$(VERSION) for $$target"

release-helm-smoke: coat-cli
	@test -n "$(CHART_VERSION)" || { echo "CHART_VERSION is required, for example: make release-helm-smoke CHART_VERSION=0.2.0 APP_VERSION=0.2.0"; exit 2; }
	@set -eu; \
	app_version="$(APP_VERSION)"; \
	helm_bin="$(HELM)"; \
	release="$(RELEASE)"; \
	if [ -z "$$release" ]; then release="jattg-smoke"; fi; \
	namespace="$(NAMESPACE)"; \
	if [ -z "$$namespace" ]; then namespace="jattg-smoke"; fi; \
	chart_url="$(CHART_URL)"; \
	tmp_dir="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp_dir"' EXIT INT TERM; \
	chart="$$tmp_dir/jattg-$(CHART_VERSION).tgz"; \
	if [ -z "$$chart_url" ] && command -v gh >/dev/null 2>&1; then \
		gh release download "chart-v$(CHART_VERSION)" --repo josephjohncox/joseph-and-the-amazing-technicolor-task-graph --pattern "jattg-$(CHART_VERSION).tgz" --pattern "jattg-$(CHART_VERSION).tgz.sha256" --dir "$$tmp_dir"; \
	else \
		if [ -z "$$chart_url" ]; then \
			chart_url="https://github.com/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/releases/download/chart-v$(CHART_VERSION)/jattg-$(CHART_VERSION).tgz"; \
		fi; \
		curl --retry 6 --retry-delay 5 --retry-all-errors -fsSL "$$chart_url" -o "$$chart"; \
		curl --retry 6 --retry-delay 5 --retry-all-errors -fsSL "$$chart_url.sha256" -o "$$chart.sha256"; \
	fi; \
	expected_sha="$$(cut -d ' ' -f 1 "$$chart.sha256")"; \
	printf '%s  %s\n' "$$expected_sha" "$$chart" | shasum -a 256 -c -; \
	if [ -z "$$app_version" ] && command -v "$$helm_bin" >/dev/null 2>&1; then \
		app_version="$$("$$helm_bin" show chart "$$chart" | awk -F': *' '$$1 == "appVersion" { gsub(/^"|"$$/, "", $$2); print $$2; exit }')"; \
	fi; \
	if [ -z "$$app_version" ]; then app_version="$(CHART_VERSION)"; fi; \
	$(COAT_BIN_DIR)/coat deploy chart lint --helm "$$helm_bin" --chart "$$chart"; \
	$(COAT_BIN_DIR)/coat deploy chart template --helm "$$helm_bin" --release "$$release" --namespace "$$namespace" --chart "$$chart" --set "global.imageTag=$$app_version" --output "$$tmp_dir/rendered.yaml"; \
	test -s "$$tmp_dir/rendered.yaml"; \
	if [ "$${HELM_SMOKE_UPGRADE_DRY_RUN:-false}" = "true" ]; then \
		$(COAT_BIN_DIR)/coat deploy chart upgrade --helm "$$helm_bin" --release "$$release" --namespace "$$namespace" --chart "$$chart" --set "global.imageTag=$$app_version" --dry-run; \
	else \
		echo "skipped Helm upgrade dry-run; set HELM_SMOKE_UPGRADE_DRY_RUN=true on a cluster-capable runner"; \
	fi; \
	if [ "$${HELM_SMOKE_APPLY:-false}" = "true" ]; then \
		$(COAT_BIN_DIR)/coat deploy chart upgrade --helm "$$helm_bin" --release "$$release" --namespace "$$namespace" --chart "$$chart" --set "global.imageTag=$$app_version" --wait --timeout "$${HELM_SMOKE_TIMEOUT:-5m}"; \
		$(COAT_BIN_DIR)/coat deploy cluster status --namespace "$$namespace" --timeout "$${CLUSTER_SMOKE_TIMEOUT:-180s}"; \
	fi; \
	echo "smoked published Helm chart $(CHART_VERSION) with image tag $$app_version"

ci: ci-rust proto-check docs-check runtime-live-scaffold ci-node
	git diff --check

ci-pr: ci-rust proto-check docs-check runtime-live-scaffold reset-smoke validate-task-graph-bootstraps ci-node scenario-e2e scenario-e2e-ui
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
