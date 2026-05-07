# Local Overlay

This placeholder overlay is reserved for local image names, NodePorts, and development-only secrets.

The base manifest is intentionally self-contained for the first scaffold:

```sh
coat k8s render --output infra/k8s/rendered.yaml
kubectl apply --dry-run=client -f infra/k8s/rendered.yaml
```
