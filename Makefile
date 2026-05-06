.PHONY: fmt test check schemas compose-up compose-down k8s-render

fmt:
	cargo fmt --all

test:
	cargo test --workspace

check:
	cargo check --workspace

schemas:
	cargo run -p jattg-domain --bin generate-schemas -- schemas

compose-up:
	docker compose -f infra/compose/docker-compose.yml up --build

compose-down:
	docker compose -f infra/compose/docker-compose.yml down

k8s-render:
	cargo run -p jattg-cli -- k8s render --output infra/k8s/rendered.yaml
