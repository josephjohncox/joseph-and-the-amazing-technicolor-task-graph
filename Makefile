.PHONY: ci fmt fmt-check test check schemas proto-lint proto-format docs-check sidecars-build control-web-build ts-build helm-lint helm-package compose-config compose-cloud-config compose-up compose-cloud-up compose-down compose-cloud-down k8s-render

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

test:
	cargo test --workspace

check:
	cargo check --workspace

schemas:
	cargo run -p coat-domain --bin generate-schemas -- schemas

proto-lint:
	buf lint

proto-format:
	buf format -w

docs-check:
	sh scripts/coat-doc-gardener.sh

sidecars-build:
	if [ -f sidecars/codex-runner-ts/node_modules/typescript/bin/tsc ]; then node sidecars/codex-runner-ts/node_modules/typescript/bin/tsc -p sidecars/codex-runner-ts/tsconfig.json; else npm ci --prefix sidecars/codex-runner-ts && npm run --prefix sidecars/codex-runner-ts build; fi
	if [ -f sidecars/staff-engineer-runner-ts/node_modules/typescript/bin/tsc ]; then node sidecars/staff-engineer-runner-ts/node_modules/typescript/bin/tsc -p sidecars/staff-engineer-runner-ts/tsconfig.json; else npm ci --prefix sidecars/staff-engineer-runner-ts && npm run --prefix sidecars/staff-engineer-runner-ts build; fi

control-web-build:
	if [ -x ui/control-plane-web/node_modules/.bin/tsc ]; then ui/control-plane-web/node_modules/.bin/tsc -p ui/control-plane-web/tsconfig.json; elif [ -f sidecars/codex-runner-ts/node_modules/typescript/bin/tsc ]; then node sidecars/codex-runner-ts/node_modules/typescript/bin/tsc -p ui/control-plane-web/tsconfig.json; else npm install --prefix ui/control-plane-web && npm run --prefix ui/control-plane-web build; fi

ts-build: sidecars-build control-web-build

helm-lint:
	coat deploy chart lint

helm-package:
	coat deploy chart package

ci: fmt-check check test schemas proto-lint docs-check ts-build
	git diff --check
	git diff --exit-code schemas

compose-config:
	coat deploy local config

compose-cloud-config:
	coat deploy local config --restate-cloud --restate-cloud-env-file infra/compose/restate-cloud.env.example --allow-placeholder-env

compose-up:
	coat deploy local up --allow-stub-runners

compose-cloud-up:
	coat deploy local up --restate-cloud --allow-stub-runners

compose-down:
	coat deploy local down

compose-cloud-down:
	coat deploy local down --restate-cloud

k8s-render:
	coat deploy cluster render --output infra/k8s/rendered.yaml
