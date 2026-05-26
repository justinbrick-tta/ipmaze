# ipmaze

ipmaze is a Kubernetes controller that turns remotely published IP range data into controller-managed `NetworkPolicy` resources.

It introduces a namespaced `CIDRPolicy` custom resource that:

- fetches JSON from an unauthenticated HTTP or HTTPS endpoint,
- optionally resolves the final JSON endpoint from a pointer document using a regex capture group,
- extracts CIDR blocks with JMESPath,
- validates and normalizes the resulting IPv4 and IPv6 CIDRs, and
- reconciles a managed `NetworkPolicy` for the selected workloads.

The controller is implemented in Rust with `kube-rs` and ships with both Kustomize and Helm installation paths.

## How It Works

Each `CIDRPolicy` declares:

- `spec.source.address`: the remote endpoint to read,
- `spec.source.pointer.regex`: an optional regex for pointer-style source discovery,
- `spec.source.jmesPath`: the JMESPath expression used to extract CIDR strings,
- `spec.target.podSelector`: the pods protected by the managed policy,
- `spec.rules`: ingress and egress rule definitions that receive the resolved CIDRs,
- `spec.resyncSchedule`: an optional 5-field UTC cron expression for background refresh.

Address handling is deterministic:

- bare DNS names are fetched as `https://...`,
- bare IP addresses are fetched as `http://...`,
- explicit `http://` and `https://` URLs are preserved.

The controller only performs unauthenticated fetches. It does not attach cookies or credentials to remote requests.

## Quick Start

Install with Kustomize:

```sh
kubectl apply -k config
```

Install with Helm:

```sh
helm upgrade --install ipmaze-controller ./charts/ipmaze-controller \
  --namespace ipmaze-system \
  --create-namespace
```

Install from the published OCI chart without cloning the repository:

```sh
helm install ipmaze-controller oci://ghcr.io/justinbrick-tta/charts/ipmaze-controller \
  --version 0.1.0 \
  --namespace ipmaze-system \
  --create-namespace
```

If the chart package is public on GHCR, anonymous pulls are sufficient. If the package is private, log in first:

```sh
echo "$GITHUB_TOKEN" | helm registry login ghcr.io --username YOUR_GITHUB_USERNAME --password-stdin
```

Apply an example policy:

```sh
kubectl apply -f config/examples/google-cloud-us-central1-ipv4.yaml
```

Inspect the generated policy:

```sh
kubectl get cidrpolicy
kubectl get networkpolicy
```

## Example

```yaml
apiVersion: ipmaze.k8s.justin.directory/v1alpha1
kind: CIDRPolicy
metadata:
  name: google-cloud-us-central1-ipv4
  namespace: default
spec:
  source:
    address: https://www.gstatic.com/ipranges/cloud.json
    jmesPath: prefixes[?service=='Google Cloud' && scope=='us-central1' && ipv4Prefix].ipv4Prefix[]
  target:
    podSelector:
      matchLabels:
        app: api
  rules:
    - directions:
        - ingress
      namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: ingress
```

This creates a managed `NetworkPolicy` named `<cidrpolicy-name>-managed` in the same namespace as the `CIDRPolicy`.

## Repository Layout

- `crates/ipmaze-controller`: Rust controller crate, binary entrypoint, API types, reconcile logic, and tests.
- `config/`: Kustomize install path, CRD, controller manifests, and example `CIDRPolicy` resources.
- `charts/ipmaze-controller`: Helm chart for parameterized installation.
- `spec/ipmaze-controller`: specification and repository schema contract.
- `impl/ipmaze-controller-rs`: implementation notes mapped to the spec.

## Local Development

Build the controller:

```sh
cargo build -p ipmaze-controller
```

Run the test suite:

```sh
cargo test -p ipmaze-controller
```

Run formatting and lint checks:

```sh
cargo fmt --all --check
cargo clippy -p ipmaze-controller --all-targets -- -D warnings
```

Generate the CRD and compare it with the checked-in manifest:

```sh
cargo run -p ipmaze-controller --bin ipmaze-controller -- generate-crd > /tmp/cidrpolicies.ipmaze.k8s.justin.directory.yaml
diff -u config/crd/cidrpolicies.ipmaze.k8s.justin.directory.yaml /tmp/cidrpolicies.ipmaze.k8s.justin.directory.yaml
```

Run the controller locally against your current kubeconfig:

```sh
cargo run -p ipmaze-controller --bin ipmaze-controller -- run --requeue-seconds 300
```

Build the container image:

```sh
docker build --tag ipmaze-controller:test .
```

Validate packaging locally:

```sh
kubectl kustomize config
kubectl kustomize config/controller
helm lint charts/ipmaze-controller
helm template ipmaze-controller charts/ipmaze-controller --namespace ipmaze-system --include-crds
```

## Publishing For Remote Installs

Tagged releases already publish both the controller image and the Helm chart through GitHub Actions.

- Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yaml`.
- The controller image is pushed to `ghcr.io/justinbrick-tta/ipmaze-controller`.
- The Helm chart is packaged from `charts/ipmaze-controller` and pushed to `oci://ghcr.io/justinbrick-tta/charts`.
- Users then install it with `helm install ... oci://ghcr.io/justinbrick-tta/charts/ipmaze-controller --version <chart-version>`.

This means users do not need the repository checkout at install time.

## Smoke Testing

The repository includes a GitHub Actions smoke workflow that exercises install, upgrade, and uninstall flows for both Kustomize and Helm on a kind cluster using a locally built image.

For local source testing without depending on live upstream payloads, serve the checked-in fixture data:

```sh
python3 -m http.server 8080 --directory crates/ipmaze-controller/tests/fixtures
```

Then adapt one of the examples under `config/examples/` to point at your reachable fixture URL.

## Additional Documentation

- [Specification](spec/ipmaze-controller/spec.md)
- [Implementation notes](impl/ipmaze-controller-rs/impl.md)
- [Kustomize deployment notes](config/controller/README.md)
- [Helm chart notes](charts/ipmaze-controller/README.md)
- [Example manifest notes](config/examples/README.md)
- [CRD schema](spec/ipmaze-controller/policy.schema.json)

## License

This repository is licensed under the terms in [LICENSE.md](LICENSE.md).