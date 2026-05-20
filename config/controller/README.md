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
