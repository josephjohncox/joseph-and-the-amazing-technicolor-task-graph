# Model And Runner Cluster Guide

This guide covers small personal clusters and production runner fleets for Joseph and the Amazing Technicolor Task Graph. Keep model serving, durable coordination, and executor sandboxes as separate pools even when they share the same physical hardware.

## Common Topology

Use four logical pools:

- `control`: Restate, coordinator, goal store, event gateway, notifier, runner registry, memory gateway.
- `memory`: Postgres/pgvector, Qdrant, Graphiti/Zep MCP, object storage, embedding services.
- `models`: Bedrock access points, vLLM, Ollama, llama.cpp, Hugging Face Text Embeddings Inference, rerankers, and local OpenAI-compatible APIs.
- `executors`: Codex, Claude Code, staff-engineer, generic model-provider, tester, reviewer, and sandbox Job runners.

Always-on runners can run as Deployments or host processes. Burst runners,
short-lived model experiments, and temporary Restate service executors should
run as Kubernetes Jobs from the `jattg-agent-toolbox` image when a cluster is
available. The toolbox carries the Rust services, `coat`, runner sidecars, and
common operator tools; inject only cluster-local env, scripts, or binaries
through mounted ConfigMaps/Secrets. See
`docs/operations/ephemeral-kubernetes-runners.md`.

Each runner registers with:

- node identity: `node_id`, `runner_id`, endpoint;
- roles: `codex`, `claude_code`, `model_provider`, `tester`, `reviewer`, `research`, `formal_methods`;
- model candidates: provider, model, endpoint, route label, context window, features;
- capabilities: `local_models`, `vllm`, `open_ai_compatible`, `gpu`, `workspace_sandbox`, `gvisor_sandbox`, `kata_sandbox`;
- labels: `node_pool`, `hardware`, `model_family`, `sandbox.backend`, `auth.locality`, `network.egress`.

The coordinator dispatches to capabilities and labels. Do not encode cluster topology into prompts.

## GB10 / DGX Spark Cluster

NVIDIA DGX Spark is the GB10-class personal AI system to model here. The current NVIDIA docs describe a compact Grace Blackwell system with 128 GB unified memory, 10 GbE, ConnectX-7, and support for large local AI model workflows. They also describe dual-Spark model support as a special case, so treat multi-node GB10 clustering as an inference-serving and routing problem first, not a shared-memory supercomputer assumption.

Recommended wiring:

- use 10 GbE for control, storage, and admin traffic;
- use the ConnectX-7 QSFP links through a compatible 200 GbE-capable switch for model-serving/data traffic when using more than point-to-point nodes;
- keep MTU, RDMA/RoCE, PFC/ECN, and switch buffer choices explicit if the model server depends on high-throughput collectives;
- label nodes as `hardware=gb10`, `accelerator=blackwell`, `memory.unified_gb=128`, `fabric=connectx7`;
- run vLLM/OpenAI-compatible model servers on the model pool and register them through the runner registry;
- run executor sandboxes on separate CPU or sandbox-capable nodes unless a task explicitly needs local GPU access.

Practical stack:

- Kubernetes: k3s, Talos, MicroK8s, RKE2, or upstream kubeadm with containerd.
- NVIDIA components: GPU Operator where supported, otherwise preinstalled driver/toolkit and the NVIDIA device plugin pattern.
- Model serving: vLLM for OpenAI-compatible high-throughput serving; TEI for embeddings; Ollama for simple interactive model hosting.
- Storage: local NVMe cache plus S3-compatible artifact storage.
- Networking: one VLAN for control, one VLAN for model traffic, one restricted executor egress path.

GB10 runners should advertise model capacity honestly. Use model routes with `context_window`, `features`, and labels instead of assuming every node can serve every model.

## Mac Mini Cluster

Mac mini clusters are useful for low-power always-on runners, Apple Silicon inference, and device-auth-local Codex/Claude sessions. They are not the right place for Linux kernel sandbox runtimes such as gVisor, Kata, or Firecracker unless the executor runs inside a separate Linux VM.

Recommended wiring:

- use wired Ethernet only; prefer 10 GbE Mac minis for model traffic;
- keep a management subnet separate from exposed model APIs;
- use LaunchAgents/systemd-like supervisors through launchd, nix-darwin, or a lightweight process manager;
- store Ollama models under a dedicated fast volume and set `OLLAMA_MODELS`;
- expose Ollama with `OLLAMA_HOST=0.0.0.0:11434` only on a trusted internal network or behind an authenticated gateway;
- register each Mac as a runner with labels such as `hardware=mac_mini`, `os=macos`, `auth.locality=runner_local`, `sandbox.backend=provider_sandbox` or `sandbox.backend=local_workspace`.

Good Mac mini roles:

- reviewer and research agents;
- local Ollama/MLX model endpoints;
- Codex or Claude Code runners with node-local device auth;
- model-provider runners for Ollama or OpenAI-compatible endpoints;
- embedding or reranking services for personal use;
- UI, notifier, and low-risk automation workers.

Avoid running untrusted shell workloads directly on macOS host runners. For untrusted code, dispatch to Linux executor nodes with gVisor/Kata/Firecracker or to provider-backed sandboxes.

## vLLM

Use vLLM for GPU-backed OpenAI-compatible model serving. In Kubernetes, deploy vLLM on GPU model nodes and expose an internal service endpoint such as `http://vllm-qwen:8000/v1`. Register a runner with a model candidate:

```json
{
  "provider": "vllm",
  "model": "qwen3-coder-30b",
  "endpoint": "http://vllm-qwen:8000/v1",
  "route_label": "gb10-qwen-coder",
  "weight": 1
}
```

Set runner capabilities:

- `local_models`
- `vllm`
- `open_ai_compatible`
- `gpu`

For multi-model clusters, prefer one Deployment per hot model or a model gateway that owns admission control. COAT should route tasks; it should not overload a single vLLM server with unbounded model swaps.

## Ollama

Use Ollama for local interactive models, Mac mini clusters, and simple Linux model nodes. Set:

- `OLLAMA_HOST=0.0.0.0:11434` only on trusted internal networks;
- `OLLAMA_MODELS=/models/ollama` for shared or external model storage;
- `OLLAMA_KEEP_ALIVE` for warm model retention;
- `OLLAMA_FLASH_ATTENTION=1` where supported.

Register Ollama endpoints as OpenAI-compatible or Ollama-specific model candidates only when the runner adapter can call them. Keep high-risk executor tasks off Mac host shells; use Ollama nodes mainly for planning, review, research, and low-risk local inference.

## Embedding Models

Default production choices:

- hosted OpenAI embeddings for simplicity and quality;
- Hugging Face Text Embeddings Inference for self-hosted OpenAI-compatible embedding service;
- Qdrant for vector search;
- Graphiti/Zep for temporal knowledge graph memory;
- Postgres/pgvector for queryable read-model joins and smaller deployments.

Embedding servers should be separate from executor sandboxes. Use `coat setup local-auth` to select hosted OpenAI embeddings from the models.dev cache or local embeddings discovered from Ollama, vLLM, llama.cpp, Hugging Face, TEI, or another OpenAI-compatible `/models` endpoint. The wizard writes `MEMORY_GATEWAY_EMBEDDING_URL`, `MEMORY_GATEWAY_EMBEDDING_MODEL`, optional `MEMORY_GATEWAY_EMBEDDING_DIMENSIONS`, `MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS`, and store adapter settings for Qdrant or Graphiti/Zep MCP.

For GB10/GPU clusters, run TEI or another standard embedding server on model nodes. For Mac mini clusters, run smaller local embedding models through Ollama/MLX only when latency and quality are acceptable; otherwise use hosted embeddings and keep local Qdrant.

## Mixed Executor Sandboxing

Use a heterogeneous Kubernetes cluster when possible:

- normal model nodes: standard container runtime with GPU access;
- CPU executor nodes: gVisor RuntimeClass for untrusted tool execution;
- VM executor nodes: Kata RuntimeClass for stronger isolation;
- high-risk executor nodes: Firecracker-backed runtime or external microVM runner;
- GPU sandbox nodes: Kata plus NVIDIA GPU Operator sandbox mode where the platform supports it.

Every executor pool should have an explicit network profile. A typical split is
`control-plane-only`, `model-provider`, `research-gateway`, `object-store-only`,
and `restate-executor`. Register the same profile as a runner label and enforce
it with Kubernetes NetworkPolicy, Cilium, Calico, cloud firewall policy, or a
provider sandbox egress profile. Avoid routing open-network work to device-auth
runners unless the goal has a human approval and an output/security review gate.

Register each executor runner with its exact backend. Example labels:

```json
{
  "sandbox.backend": "kata",
  "sandbox.runtime_class": "kata-qemu-nvidia-gpu",
  "network.egress": "restricted",
  "node_pool": "gpu-kata-executors"
}
```

## Operational Rules

- Keep Restate and the goal store off executor nodes.
- Keep raw device-auth stores node-local unless a brokered user-auth lease is approved.
- Keep model endpoints internal; expose only through an authenticated gateway if external access is needed.
- Require `sandbox_attestation` and artifact manifests for untrusted executor tasks.
- Require output and security guardrail reviewers for tasks with open network, secrets, deploy authority, dependency changes, or unknown code execution.
- Put large traces, sim runs, and model artifacts in S3-compatible storage; keep workflow state to refs.

## References

- NVIDIA DGX Spark hardware overview: https://docs.nvidia.com/dgx/dgx-spark/hardware.html
- vLLM Kubernetes deployment: https://docs.vllm.ai/en/stable/deployment/k8s/
- Ollama FAQ: https://docs.ollama.com/faq
- NVIDIA GPU Operator: https://docs.nvidia.com/datacenter/cloud-native/gpu-operator/latest/getting-started.html
- NVIDIA GPU Operator with Kata: https://docs.nvidia.com/datacenter/cloud-native/gpu-operator/latest/deploy-kata-containers.html
