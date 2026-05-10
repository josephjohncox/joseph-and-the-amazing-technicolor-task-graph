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
	ci fmt fmt-check test check schemas proto-lint proto-format docs-check \
	sidecars-build control-web-build ts-build \
	helm-lint helm-package \
	compose-config compose-cloud-config compose-up compose-cloud-up compose-down compose-cloud-down \
	k8s-render

build: coat-cli

coat-cli:
	$(CARGO) build -p coat-cli $(COAT_BUILD_ARGS)

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
	if [ -x "$$dir/node_modules/.bin/tsc" ]; then \
		"$$dir/node_modules/.bin/tsc" -p "$$dir/tsconfig.json"; \
	elif [ -f sidecars/codex-runner-ts/node_modules/typescript/bin/tsc ]; then \
		$(NODE) sidecars/codex-runner-ts/node_modules/typescript/bin/tsc -p "$$dir/tsconfig.json"; \
	else \
		$(NPM) install --prefix "$$dir"; \
		$(NPM) run --prefix "$$dir" build; \
	fi

ts-build: sidecars-build control-web-build

helm-lint:
	$(COAT) deploy chart lint

helm-package:
	$(COAT) deploy chart package

ci: fmt-check check test schemas proto-lint docs-check ts-build
	git diff --check
	git diff --exit-code schemas

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
