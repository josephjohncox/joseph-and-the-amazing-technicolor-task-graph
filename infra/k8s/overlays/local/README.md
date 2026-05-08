# Local Overlay

This placeholder overlay is reserved for local image names, NodePorts, and development-only secrets.

The base manifest is intentionally self-contained for the first scaffold:

```sh
coat deploy cluster render --output infra/k8s/rendered.yaml
coat deploy cluster apply --file infra/k8s/rendered.yaml --dry-run=client
```
