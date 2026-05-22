# ipmaze-controller Helm Chart

Install the published OCI chart without cloning the repository:

```sh
helm install ipmaze-controller oci://ghcr.io/justinbrick-tta/charts/ipmaze-controller \
	--version 0.1.0 \
	--namespace ipmaze-system \
	--create-namespace
```

Upgrade an existing installation from the published OCI chart:

```sh
helm upgrade --install ipmaze-controller oci://ghcr.io/justinbrick-tta/charts/ipmaze-controller \
	--version 0.1.0 \
	--namespace ipmaze-system \
	--create-namespace
```

Install the chart into the controller namespace:

```sh
helm install ipmaze-controller ./charts/ipmaze-controller --namespace ipmaze-system --create-namespace
```

Upgrade an existing release in place:

```sh
helm upgrade --install ipmaze-controller ./charts/ipmaze-controller --namespace ipmaze-system --create-namespace
```

Render the chart without installing it:

```sh
helm template ipmaze-controller ./charts/ipmaze-controller --namespace ipmaze-system --include-crds
```

Uninstall the release:

```sh
helm uninstall ipmaze-controller --namespace ipmaze-system
```

Common values:

- `image.repository`, `image.tag`, `image.pullPolicy`: controller image settings.
- `controller.requeueSeconds`: steady-state reconcile interval in seconds.
- `controller.logLevel`: `RUST_LOG` value for the controller container.
- `namespace.name`: advanced override for the rendered workload namespace. Leave it empty to follow the Helm release namespace.
- `namespace.create`: render a `Namespace` object for the target namespace. This is enabled by default so the chart manages namespace labels and annotations consistently.
- `serviceAccount.create`, `serviceAccount.name`: control service account creation or reuse.

The chart packages the `CIDRPolicy` CRD from the repository so Helm installs it before the controller resources.
Published chart artifacts are intended to be pushed to GHCR as an OCI chart alongside the controller image.

For a tagged image install, override the image tag explicitly:

```sh
helm upgrade --install ipmaze-controller ./charts/ipmaze-controller \
	--namespace ipmaze-system \
	--create-namespace \
	--set image.tag=0.1.0
```

If you set `namespace.name`, keep it aligned with the Helm release namespace unless you intentionally want Helm state and managed resources separated.
