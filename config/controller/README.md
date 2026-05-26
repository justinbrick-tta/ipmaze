# ipmaze-controller deployment

Install the CRD, namespace, RBAC, and deployment through the repository Kustomize entrypoint.

```sh
kubectl apply -k config
```

To pin a specific image tag or override the install namespace, create a small local overlay and apply that instead.

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: ipmaze-custom-system
resources:
	- ../config
images:
	- name: ghcr.io/justinbrick-tta/ipmaze-controller
		newTag: 0.1.0
```

If you want to render the controller-only resources without the CRD, use:

```sh
kubectl kustomize config/controller
```

The Helm chart lives under `charts/ipmaze-controller` and provides the parameterized install path.

Operational notes:

- `config/controller/kustomization.yaml` replaces `REPLACE_IMAGE` with `ghcr.io/justinbrick-tta/ipmaze-controller:latest` by default.
- A parent Kustomize overlay can set `namespace:` to rewrite the controller namespace and the `ClusterRoleBinding` service account subject together.
- The deployment injects `CONTROLLER_POD_NAME` so Kubernetes events include a stable reporting instance.
- The controller watches `CIDRPolicy` and managed `NetworkPolicy` objects cluster-wide, so the manifest uses a `ClusterRole` and `ClusterRoleBinding`.
- The binary entry point is `ipmaze-controller run`; `--requeue-seconds` controls the steady-state requeue interval.
- Required permissions include `cidrpolicies`, `cidrpolicies/status`, `networkpolicies`, and `events.events.k8s.io`.
- Remove the install with `kubectl delete -k config`.

Example manifests for upstream IP range feeds live under `config/examples/`.

- `google-cloud-us-central1-ipv4.yaml` shows a live Google Cloud allowlist query.
- `microsoft-service-tags-storage-westus.yaml` shows a regional Microsoft service tag query against a mirrored weekly JSON file.
- `config/examples/README.md` documents local smoke testing with the checked-in fixture payloads under `crates/ipmaze-controller/tests/fixtures/`.
