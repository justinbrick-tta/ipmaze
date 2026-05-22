# ipmaze-controller deployment

Apply the CRD first, then the controller RBAC and deployment manifests.

```sh
kubectl apply -f config/crd/cidrpolicies.ipmaze.k8s.justin.directory.yaml
kubectl apply -f config/controller/rbac.yaml
kubectl apply -f config/controller/deployment.yaml
```

Operational notes:

- Replace `REPLACE_IMAGE` in `deployment.yaml` with the published controller image.
- The deployment injects `CONTROLLER_POD_NAME` so Kubernetes events include a stable reporting instance.
- The controller watches `CIDRPolicy` and managed `NetworkPolicy` objects cluster-wide, so the manifest uses a `ClusterRole` and `ClusterRoleBinding`.
- The binary entry point is `ipmaze-controller run`; `--requeue-seconds` controls the steady-state requeue interval.
- Required permissions include `cidrpolicies`, `cidrpolicies/status`, `networkpolicies`, and `events.events.k8s.io`.

Example manifests for upstream IP range feeds live under `config/examples/`.

- `google-cloud-us-central1-ipv4.yaml` shows a live Google Cloud allowlist query.
- `microsoft-service-tags-storage-westus.yaml` shows a regional Microsoft service tag query against a mirrored weekly JSON file.
- `config/examples/README.md` documents local smoke testing with the checked-in fixture payloads under `crates/ipmaze-controller/tests/fixtures/`.
