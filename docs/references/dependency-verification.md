# Dependency Verification

Verified on 2026-05-06 from the npm registry:

- `@openai/codex-sdk`: `0.128.0`
- `@ctxr/agent-staff-engineer`: `1.0.3`

The staff-engineer package is still treated as an integration dependency, not foundational infrastructure. Keep stub mode available even when the package is installed.

The `claude-code-runner-ts` and `model-provider-runner-ts` wrappers do not add package-level runtime dependencies in this scaffold. They verify configured CLIs, endpoints, or cloud identity through `/verify` and keep live execution gated by environment.
