# Example CIDRPolicy manifests

This directory contains focused allowlist examples that cover direct sources, pointer-resolved sources, and custom background resync schedules:

- `google-cloud-us-central1-ipv4.yaml` filters the live Google Cloud IP ranges feed down to IPv4 prefixes for the `us-central1` scope.
- `microsoft-service-tags-storage-westus.yaml` filters a Microsoft service tags payload down to the `Storage.WestUS` regional tag.
- `google-cloud-us-central1-ipv4-pointer.yaml` demonstrates resolving a final JSON endpoint through `spec.source.pointer.regex` before running the same JMESPath extraction.
- `google-cloud-us-central1-ipv4-quarter-hour.yaml` demonstrates a direct source with an explicit `resyncSchedule` of every 15 minutes.

The checked-in fixture payloads in `crates/ipmaze-controller/tests/fixtures/` are useful for local smoke tests because they avoid depending on a changing upstream response shape.

## Local smoke test

Serve the fixture directory over HTTP from the repository root:

```sh
python3 -m http.server 8080 --directory crates/ipmaze-controller/tests/fixtures
```

Then apply one of these manifests after changing `spec.source.address` to a reachable endpoint for your cluster, for example:

- `http://host.docker.internal:8080/google-cloud.sample.json`
- `http://host.docker.internal:8080/microsoft-service-tags.sample.json`
- `http://host.docker.internal:8080/source-pointer.txt` when testing a pointer example

For a local controller process running outside the cluster, `http://127.0.0.1:8080/...` is sufficient.

For pointer-based examples, the pointer endpoint may return plain text or HTML, but it must contain a substring that matches `spec.source.pointer.regex`, and the first capture group from the first match must be the final JSON endpoint.

## Query notes

The controller requires the JMESPath result to be an array of CIDR strings.

- The Google example filters to IPv4 only so records that only carry `ipv6Prefix` do not produce `null` elements.
- The Microsoft example projects `properties.addressPrefixes[]` from the selected service tag entry.
- The pointer example uses a regex with a single capture group for the final address and relies on the same direct-address normalization rules as non-pointer sources.
- `resyncSchedule` uses standard 5-field cron syntax in UTC, for example `*/15 * * * *` for every 15 minutes.
- Microsoft publishes downloadable service tag JSON files weekly, but the authenticated discovery API is not suitable for this controller because the implementation intentionally performs unauthenticated fetches only.
