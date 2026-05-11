SHELL := /bin/sh

CARGO ?= cargo
NPM ?= npm
NODE ?= node
BUF ?= buf
COAT ?= coat

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

.DEFAULT_GOAL := build

.PHONY: \
	build coat-cli coat-cli-release coat-path \
	ci fmt fmt-check test check schemas proto-lint proto-format proto-check docs-check \
	event-gateway-smoke runner-smoke compose-runner-smoke \
	sidecars-build control-web-build control-web-smoke ts-build \
	helm-lint helm-package \
	compose-config compose-cloud-config compose-up compose-cloud-up compose-down compose-cloud-down \
	k8s-render

build: coat-cli

coat-cli:
	$(CARGO) build -p coat-cli $(COAT_BUILD_ARGS)

event-gateway-smoke:
	$(CARGO) build -p coat-event-gateway -p coat-goal-store $(COAT_BUILD_ARGS)
	COAT_EVENT_GATEWAY_SMOKE_SKIP_BUILD=1 COAT_BUILD_PROFILE=$(COAT_BUILD_PROFILE) sh scripts/coat-event-gateway-smoke.sh

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

docs-check:
	sh scripts/coat-doc-gardener.sh

sidecars-build:
	@set -eu; \
	for dir in $(SIDECAR_DIRS); do \
		echo "building $$dir"; \
		if [ -x "$$dir/node_modules/.bin/tsc" ]; then \
			"$$dir/node_modules/.bin/tsc" -p "$$dir/tsconfig.json"; \
		elif [ -f "$$dir/node_modules/typescript/bin/tsc" ]; then \
			$(NODE) "$$dir/node_modules/typescript/bin/tsc" -p "$$dir/tsconfig.json"; \
		else \
			$(NPM) ci --prefix "$$dir"; \
			$(NPM) run --prefix "$$dir" build; \
		fi; \
	done

control-web-build:
	@set -eu; \
	dir=ui/control-plane-web; \
	echo "building $$dir"; \
	if [ ! -d "$$dir/node_modules" ]; then \
		$(NPM) install --prefix "$$dir"; \
	fi; \
	$(NPM) run --prefix "$$dir" build

control-web-smoke: control-web-build
	$(NPM) run --prefix ui/control-plane-web smoke

ts-build: sidecars-build control-web-build

helm-lint:
	$(COAT) deploy chart lint

helm-package:
	$(COAT) deploy chart package

ci: fmt-check check test event-gateway-smoke runner-smoke proto-check docs-check ts-build control-web-smoke
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
