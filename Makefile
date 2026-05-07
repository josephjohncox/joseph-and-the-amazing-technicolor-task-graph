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
	helm lint infra/helm/coat

helm-package:
	scripts/package-helm-chart.sh

ci: fmt-check check test schemas proto-lint docs-check ts-build
	git diff --check
	git diff --exit-code schemas

compose-config:
	docker compose -f infra/compose/docker-compose.yml config

compose-cloud-config:
	docker compose --env-file infra/compose/restate-cloud.env.example -f infra/compose/docker-compose.yml -f infra/compose/docker-compose.restate-cloud.yml --profile restate-cloud config

compose-up:
	docker compose -f infra/compose/docker-compose.yml up --build

compose-cloud-up:
	docker compose --env-file infra/compose/restate-cloud.env -f infra/compose/docker-compose.yml -f infra/compose/docker-compose.restate-cloud.yml --profile restate-cloud up --build

compose-down:
	docker compose -f infra/compose/docker-compose.yml down

compose-cloud-down:
	docker compose --env-file infra/compose/restate-cloud.env -f infra/compose/docker-compose.yml -f infra/compose/docker-compose.restate-cloud.yml down

k8s-render:
	coat k8s render --output infra/k8s/rendered.yaml
