.PHONY: fmt test check schemas proto-lint proto-format compose-up compose-down k8s-render

fmt:
	cargo fmt --all

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

compose-up:
	docker compose -f infra/compose/docker-compose.yml up --build

compose-down:
	docker compose -f infra/compose/docker-compose.yml down

k8s-render:
	cargo run -p coat-cli -- k8s render --output infra/k8s/rendered.yaml
