# Design Doc: System Shape

COAT is a durable control plane, not a monolithic agent.

The coordinator stores the task tree and chooses the next runnable frontier. Workers receive bounded tasks and return structured results. The validator translates those results into state transitions. Human approvals are workflow signals, not ad hoc chat messages.

## Key Decisions

- Rust is the contract and control-plane language.
- Restate is the durable execution substrate.
- TypeScript sidecars wrap agent ecosystems and model-provider APIs where TS support is stronger.
- Compose and Kubernetes share the same service boundaries.
- Stub mode is required for every live-agent worker.

## First Scaffold

The first scaffold makes the contracts and deploy surfaces concrete. Live Codex App Server, Claude Code, staff-engineer, Bedrock, vLLM, Ollama, llama.cpp, Hugging Face, and other provider execution remains behind verification gates and stub-safe wrappers.
