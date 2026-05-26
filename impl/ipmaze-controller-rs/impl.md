---
spec: ../../spec/ipmaze-controller/spec.md
name: ipmaze-controller-rs
version: 0.1.0
location: crates/ipmaze-controller
references: []
title: Rust kube-rs Implementation for the Dynamic CIDR NetworkPolicy Controller
description: Planned Rust implementation of the ipmaze-controller specification using kube-rs, schemars-backed CRD generation, strict schema alignment, and controller-managed NetworkPolicy reconciliation.
tags:
- rust
- kube-rs
- kubernetes
- crd
- networkpolicy
- jmespath
dependencies: []
template_source:
  tier: EmbeddedDefault
  locator: embedded://impl
  cache_path: .specman/cache/templates/embedded-impl.md
---

# Implementation - Rust kube-rs Dynamic CIDR NetworkPolicy Controller

## Overview

This implementation realizes the Dynamic CIDR NetworkPolicy Controller defined in [Specification - Dynamic CIDR NetworkPolicy Controller](../../spec/ipmaze-controller/spec.md#specification---dynamic-cidr-networkpolicy-controller) as a Kubernetes controller written in modern Rust using kube-rs. The design centers on a typed `CIDRPolicy` custom resource, a reconcile loop that resolves either a direct JSON source or a pointer-discovered JSON source, evaluates a JMESPath expression, validates and normalizes CIDRs, and then renders exactly one controller-managed `NetworkPolicy` per custom resource.

The implementation is constrained by the behavior in [Concept: Remote CIDR Source](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source), [Concept: CIDR Extraction Query](../../spec/ipmaze-controller/spec.md#concept-cidr-extraction-query), [Concept: Policy Target Selection](../../spec/ipmaze-controller/spec.md#concept-policy-target-selection), [Concept: Policy Direction Configuration](../../spec/ipmaze-controller/spec.md#concept-policy-direction-configuration), [Entity: CIDRPolicy Custom Resource](../../spec/ipmaze-controller/spec.md#entity-cidrpolicy-custom-resource), [Entity: Managed NetworkPolicy](../../spec/ipmaze-controller/spec.md#entity-managed-networkpolicy), and [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome). The controller preserves the last known good `NetworkPolicy` on retrieval or parsing failure, removes stale CIDRs on successful updates, and defaults omitted rule directionality to both ingress and egress.

The implementation should generate its CRD from the Rust type definitions and verify that the generated schema stays aligned with [spec/ipmaze-controller/policy.schema.json](../../spec/ipmaze-controller/policy.schema.json). Because Kubernetes CRDs use OpenAPI v3 schema rather than full JSON Schema 2020-12, strict adherence should be enforced with contract tests that compare the generated CRD schema and the repository schema for the overlapping representable constraints. The transport profile is a normative requirement: bare DNS names must be fetched as HTTPS URLs, while bare IP addresses must be fetched as HTTP URLs. Periodic background reconciliation is driven per resource by `spec.resyncSchedule`, defaulting to UTC `0 0 * * *` when omitted.

## References

The front matter currently has no additional SpecMan artifact references beyond the governing specification link in `spec`. Implementation work should still treat [spec/ipmaze-controller/policy.schema.json](../../spec/ipmaze-controller/policy.schema.json) as a normative repository-local contract for field names, required fields, selector shapes, direction enums, and status field structure. If later artifacts are added for API conventions, deployment manifests, or operator packaging, they should be added to the `references` list and described here.

## Implementation Details

### Code Location

The planned code location is `crates/ipmaze-controller`. A practical repository layout is:

```text
crates/
  ipmaze-controller/
    src/
      api/
      controller/
      netpol/
      source/
      status/
      validation/
      main.rs
      lib.rs
config/
  crd/
tests/
  integration/
  fixtures/
```

`config/crd/cidrpolicies.networking.example.io.yaml` should be generated from the Rust CRD type rather than handwritten. The controller binary should run in-cluster or from a local kubeconfig, requiring Kubernetes RBAC for the custom resource, its status subresource, events, and `NetworkPolicy` objects.

### Libraries

The implementation should use these libraries:

- `kube` and `kube-runtime`: Kubernetes API access, controller runtime, watcher integration, finalizer handling, and CRD generation hooks. The selected version should match the cluster API version support policy used by the repo.
- `k8s-openapi`: Typed `NetworkPolicy`, `LabelSelector`, `ObjectMeta`, and status-related Kubernetes types. The feature set must align with the supported Kubernetes minor version.
- `serde`, `serde_json`: JSON decoding for remote payloads and CRD serialization.
- `schemars`: Schema derivation for the CRD and schema-based contract tests against the repository JSON schema.
- `reqwest`: Unauthenticated HTTP GET retrieval for remote JSON sources. The client configuration must explicitly avoid credential injection and cookie persistence.
- `regex`: Pointer response extraction using the first capture group from the first match.
- `jmespath`: Parsing and evaluating the configured query against the full payload.
- `cron`: Parsing 5-field cron expressions and deriving the next UTC background resync time.
- `cidr` or `ipnet`: Strict IPv4 and IPv6 CIDR parsing without coercing host addresses.
- `thiserror`: Structured reconcile and validation errors.
- `tracing` and `tracing-subscriber`: Structured controller logs with stage-specific failure metadata.
- `tokio`: Async runtime for controller, HTTP retrieval, and retry timing.
- `chrono`: Status timestamps such as `lastSuccessfulResolutionTime`.

If the implementation exposes a CLI for CRD generation, `clap` is appropriate for a `generate-crd` subcommand that emits YAML into `config/crd/`.

### Modules and Components

The controller should be split into narrowly owned modules so each normative stage maps cleanly to code:

- `api`: Rust definitions for `CIDRPolicySpec`, `CIDRPolicyStatus`, `Rule`, `Direction`, and CRD generation glue.
- `validation`: Pre-reconcile validation for address syntax, pointer regex compilation, JMESPath compilation, selector representability, direction normalization, and 5-field cron parsing.
- `source`: Deterministic remote address resolution, including the requirement that bare DNS names map to HTTPS and bare IP addresses map to HTTP, optional pointer-response fetch and regex extraction, final JSON retrieval, and stage-specific transport or extraction error classification.
- `extract`: JMESPath evaluation, array and element type checks, CIDR parsing, deduplication, and stable ordering.
- `netpol`: Rendering the exact `NetworkPolicy` object shape, ownership metadata, labels or annotations that mark controller management, and diff logic.
- `status`: Condition or status field calculation, error summarization, and status patch helpers.
- `controller`: `reconcile`, `error_policy`, event recording, per-resource cron requeue calculation, and finalizer or owner-reference behavior.
- `bin` or `main`: runtime bootstrap, config loading, and metrics or health endpoints if later added.

### Key Types and Interfaces

The core Rust-facing interfaces should look like this:

```rust
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(group = "networking.example.io", version = "v1alpha1", kind = "CIDRPolicy", namespaced)]
#[kube(status = "CIDRPolicyStatus")]
pub struct CIDRPolicySpec {
    pub source: SourceSpec,
  pub resync_schedule: Option<String>,
    pub target: TargetSpec,
    pub rules: Vec<RuleSpec>,
}

pub struct PointerSpec {
  pub regex: String,
}

pub async fn reconcile(policy: Arc<CIDRPolicy>, ctx: Arc<ContextData>) -> Result<Action, ReconcileError>;

pub trait RemoteFetcher {
    async fn fetch_json(&self, address: &RemoteAddress) -> Result<serde_json::Value, FetchError>;
}

pub trait PolicyRenderer {
    fn build_managed_network_policy(
        &self,
        policy: &CIDRPolicy,
        cidrs: &[NormalizedCidr],
    ) -> Result<NetworkPolicy, RenderError>;
}
```

`CIDRPolicySpec` remains the source of truth for CRD generation. Nested Rust types should use `serde(rename_all = "camelCase")` or explicit `#[serde(rename = "...")]` attributes so fields such as `jmesPath`, `source.pointer.regex`, `resyncSchedule`, `podSelector`, `namespaceSelector`, `lastSuccessfulResolutionTime`, `lastObservedCidrs`, and `lastReconciliationError` match the schema exactly. `RemoteFetcher` and `PolicyRenderer` are useful seams for unit tests. `NormalizedCidr` should be a validated wrapper type so later stages cannot receive invalid strings.

### Error Handling

Errors should be stage-specific and explicit so operators can distinguish failures required by [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome):

- `AddressValidation`: address is neither DNS name, IP literal, nor an accepted URL form.
- `PointerRetrieval`: pointer endpoint retrieval fails before a final JSON address is resolved.
- `PointerExtraction`: pointer regex does not match, captures an empty first group, or yields an invalid final address.
- `Transport`: DNS resolution, TLS, timeout, connection, or non-success HTTP failures.
- `JsonDecode`: response body is not valid JSON.
- `JmesPathCompile`: invalid query in the custom resource.
- `JmesPathEvaluate`: runtime query failure against the payload.
- `ResultShape`: query result is not an array of strings.
- `CidrValidation`: one or more strings are not valid IPv4 or IPv6 CIDRs.
- `SelectorTranslation`: selector fields cannot be rendered losslessly into `NetworkPolicy`.
- `Scheduling`: a validated cron schedule cannot be converted into a future requeue instant.
- `KubernetesApi`: create, patch, status patch, or event recording failure.

On any post-success failure, the controller must update status and events without deleting or blanking the previously reconciled managed `NetworkPolicy`.

### Data Flow

The steady-state reconcile flow is:

1. Read the `CIDRPolicy` instance and validate required fields, pointer regex, parseable JMESPath, and parseable cron schedule.
2. Resolve or normalize the configured base source address according to the required deterministic transport profile: bare DNS names map to `https://.../`, bare IP addresses map to `http://.../`, and explicit `http://` or `https://` URLs are preserved.
3. If `source.pointer` is present, perform an unauthenticated HTTP GET against the base address, apply the regex to the full response body, and use the first capture group from the first match as the final JSON address.
4. Normalize the resolved final address using the same direct-address transport rules.
5. Perform an unauthenticated HTTP GET with no cookies or auth material against the final JSON address.
6. Parse the body as JSON and fail the reconcile if parsing fails.
7. Evaluate the JMESPath expression against the full payload.
8. Assert the result is an array of strings, then parse every entry as an IPv4 or IPv6 CIDR.
9. Deduplicate while preserving the semantic set, then sort into a deterministic order for stable reconciliation.
10. Translate subject selectors and peer selectors into typed `NetworkPolicy` selectors.
11. Render ingress peers, egress peers, or both, based on each rule's directions or the default behavior.
12. Create or patch the managed `NetworkPolicy` only if the effective spec differs.
13. Patch status with last success time, last observed CIDRs, and cleared or updated error text, then compute the next cron-driven background resync.

### External Integrations

The implementation integrates with:

- Kubernetes API server through kube-rs for custom resources, `NetworkPolicy`, events, and status updates.
- Remote JSON endpoints over HTTP(S) through `reqwest`.
- JMESPath evaluation through the Rust library, compiled at validation or reconcile time.
- CRD generation from Rust type metadata into YAML artifacts checked into `config/crd/`.

The remote fetch client must not use ambient credentials, injected headers, or cookie jars. For source normalization, the implementation must treat bare DNS names as HTTPS targets and bare IP literals as HTTP targets to satisfy [Concept: Remote CIDR Source](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source).

## Deployment Packaging

The controller is distributed through repository-native Kustomize manifests and a Helm chart. A standalone generated `install.yaml` bundle is intentionally not part of the supported surface. The Kustomize entrypoint lives at `config/kustomization.yaml` and composes the CRD plus controller resources. The controller-specific Kustomize package under `config/controller/` owns the namespace, RBAC, deployment, and default image substitution. The Helm chart lives under `charts/ipmaze-controller/` and renders semantically equivalent controller resources with parameterized image, namespace, and runtime settings.

### Kustomize Distribution

`kubectl apply -k config` is the supported manifest-first installation path. The repository root `config/` Kustomization includes both the CRD and controller package so a fresh install renders the full resource set. `config/controller/kustomization.yaml` keeps `deployment.yaml` free of a hard-coded published image by replacing `REPLACE_IMAGE` with the default GHCR image reference during Kustomize rendering. This keeps the checked-in deployment manifest close to the controller runtime while making the install path directly usable.

### Helm Distribution

The Helm chart mirrors the controller deployment shape from `config/controller/` rather than introducing a separate operational model. The chart exposes values for image repository, image tag, pull policy, reconcile interval, log level, resource requests and limits, service account reuse, and target namespace selection. The `CIDRPolicy` CRD is sourced from `config/crd/` through the chart `crds/` directory so Helm installs the CRD before the controller resources and the repository keeps one authoritative CRD file.

### Container Image Build and Registry Publication

The repository `Dockerfile` builds the `ipmaze-controller` binary in a Rust builder stage and copies it into a minimal Debian runtime image with CA certificates. Release publication targets GHCR with the repository image name `ghcr.io/justinbrick-tta/ipmaze-controller`. Release automation is expected to publish a multi-architecture manifest for `linux/amd64` and `linux/arm64`, along with immutable release tags and moving compatibility tags such as the corresponding major-minor tag and `latest`.

### Namespace and Cluster Resource Layout

The controller runs in a dedicated namespace, `ipmaze-system`, by default. The Kustomize package includes an explicit `Namespace` object for clean installs. The Helm chart defaults to the Helm release namespace and can optionally render a `Namespace` object plus override the target namespace through values. Cluster-scoped permissions remain necessary because the controller watches `CIDRPolicy` resources and managed `NetworkPolicy` objects across namespaces.

### CI and Release Automation

CI is responsible for keeping the packaging surfaces executable, not just syntactically present. In addition to Rust formatting, lint, test, and CRD drift checks, the repository CI workflow should render the Kustomize install path, lint and template the Helm chart including CRDs, and smoke-test the multi-platform image build. Tag-driven release automation publishes the multi-arch container image to GHCR and packages then pushes the Helm chart as an OCI artifact to GHCR.

### Operational Installation and Upgrade Notes

Kustomize is the lowest-friction path for users who want repository-native manifests with a fixed default image location. Helm is the supported path for parameterized installs and upgrades. Both surfaces must stay semantically aligned with the deployment manifest and CRD checked into the repository. Any future deployment knobs added to the raw manifest should be reflected in the Helm values surface and validated through the same packaging checks so install drift is caught before release.

## Concept & Entity Breakdown

### Concept: Remote CIDR Source [Specification Link](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source)

This concept is realized by `source::RemoteAddress`, `source::ResolvedSource`, pointer-resolution helpers, and validation helpers. The controller accepts one configured base address, performs unauthenticated HTTP GET requests, refuses to attach credentials, and treats non-JSON payloads at the final endpoint as reconcile failures. When `source.pointer` is present, the base address may return plain text or HTML, the configured regex is applied to the full response body, and the first capture group from the first match becomes the final JSON address. Both direct and pointer-derived addresses are normalized through the same HTTPS-for-hostname and HTTP-for-IP rules. To preserve safety requirements from [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome), pointer or final fetch failures only update status or events and do not blank an existing managed policy.

#### API Signatures

```rust
pub enum RemoteAddress {
  Url(url::Url),
  Hostname(String),
  Ip(std::net::IpAddr),
}

pub async fn fetch_json(
  client: &reqwest::Client,
  address: &RemoteAddress,
) -> Result<serde_json::Value, FetchError>;
```

- Input is a validated address form. Output is the parsed JSON value. The function must synthesize HTTPS for bare DNS names, HTTP for bare IP literals, never attach credentials, surface transport-stage failures distinctly, and reject bodies that are not valid JSON.

#### Data Model

```rust
pub struct SourceSpec {
  pub address: String,
  pub pointer: Option<PointerSpec>,
  pub jmes_path: String,
}

pub struct PointerSpec {
  pub regex: String,
}
```

`SourceSpec` remains close to the CRD schema, while `RemoteAddress` is an internal validated form.

### Concept: CIDR Extraction Query [Specification Link](../../spec/ipmaze-controller/spec.md#concept-cidr-extraction-query)

This concept is implemented by compiling the configured JMESPath, evaluating it against the full JSON document, asserting that the result is an array of strings, and then parsing each string as a CIDR. The implementation must not coerce host addresses such as `10.0.0.1` into `/32`. It may deduplicate semantically identical CIDRs before rendering the managed policy.

#### API Signatures

```rust
pub fn compile_query(expr: &str) -> Result<jmespath::Expression<'static>, QueryError>;

pub fn extract_cidrs(
  expression: &jmespath::Expression<'_>,
  payload: &serde_json::Value,
) -> Result<Vec<NormalizedCidr>, ExtractionError>;
```

- `extract_cidrs` is responsible for result-shape checks, element validation, deduplication, and deterministic ordering.

#### Data Model

```rust
pub struct NormalizedCidr {
  pub rendered: String,
  pub family: IpFamily,
}

pub enum IpFamily {
  V4,
  V6,
}
```

### Concept: Policy Target Selection [Specification Link](../../spec/ipmaze-controller/spec.md#concept-policy-target-selection)

This concept is realized by keeping the CRD selector types structurally aligned with Kubernetes `LabelSelector` semantics and translating them losslessly into `NetworkPolicySpec.podSelector` and `NetworkPolicyPeer` selectors. The subject selector always targets pods in the custom resource namespace, while peer selectors are rendered independently per rule without reinterpretation.

#### API Signatures

```rust
pub fn render_subject_selector(selector: &LabelSelector) -> Result<LabelSelector, SelectorError>;

pub fn render_peer_selector(rule: &RuleSpec) -> Result<NetworkPolicyPeer, SelectorError>;
```

- These functions should reject any impossible translation rather than approximate Kubernetes semantics.

#### Data Model

```rust
pub struct TargetSpec {
  pub pod_selector: LabelSelector,
}

pub struct RuleSpec {
  pub directions: Option<Vec<Direction>>,
  pub pod_selector: Option<LabelSelector>,
  pub namespace_selector: Option<LabelSelector>,
}
```

### Concept: Policy Direction Configuration [Specification Link](../../spec/ipmaze-controller/spec.md#concept-policy-direction-configuration)

Directionality is modeled as an enum set per rule. If omitted, the implementation expands the rule to both ingress and egress during normalization, then renders IP blocks only into the enabled `NetworkPolicy` rule kinds. This behavior must stay stable across reconciles so a missing directions field never flips between defaults.

#### API Signatures

```rust
pub enum Direction {
  Ingress,
  Egress,
}

pub fn effective_directions(rule: &RuleSpec) -> std::collections::BTreeSet<Direction>;
```

- `effective_directions` should return both directions when the field is absent.

### Entity: CIDRPolicy Custom Resource [Specification Link](../../spec/ipmaze-controller/spec.md#entity-cidrpolicy-custom-resource)

This entity is implemented as the `CustomResource` derive input and is the source for both runtime deserialization and CRD generation. The Rust type definitions mirror the repository schema closely enough that generated CRD YAML and [spec/ipmaze-controller/policy.schema.json](../../spec/ipmaze-controller/policy.schema.json) stay contract-compatible, including the optional `source.pointer.regex` field and optional `resyncSchedule` field.

#### API Signatures

```rust
impl CIDRPolicy {
  pub fn validate_spec(&self) -> Result<ValidatedPolicy, ValidationError>;
  pub fn managed_network_policy_name(&self) -> String;
}
```

#### Data Model

```rust
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct CIDRPolicyStatus {
  pub last_successful_resolution_time: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
  pub last_observed_cidrs: Vec<String>,
  pub last_reconciliation_error: Option<String>,
}
```

### Entity: Managed NetworkPolicy [Specification Link](../../spec/ipmaze-controller/spec.md#entity-managed-networkpolicy)

This entity is produced by the `netpol` module and reconciled through server-side apply or patch semantics. It must be identifiable as controller-managed by owner references and a stable label or annotation, and its effective CIDR set must exactly match the validated resolved set. Successful reconciles remove stale CIDRs, while failed reconciles leave the prior policy intact.

#### API Signatures

```rust
pub fn build_managed_network_policy(
  policy: &CIDRPolicy,
  cidrs: &[NormalizedCidr],
) -> Result<NetworkPolicy, RenderError>;

pub async fn apply_managed_network_policy(
  api: &Api<NetworkPolicy>,
  network_policy: &NetworkPolicy,
) -> Result<(), kube::Error>;
```

#### Data Model

```rust
pub struct ManagedPolicyMetadata {
  pub name: String,
  pub namespace: String,
  pub owner_uid: String,
}
```

### Entity: Reconciliation Outcome [Specification Link](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome)

This entity is represented operationally through reconcile return values, status updates, Kubernetes events, and logs. The implementation should classify each reconcile as success, failure, or no-change, and must expose the failed stage in status or events so operators can distinguish source, transport, JSON, JMESPath, CIDR, and Kubernetes API problems.

#### API Signatures

```rust
pub enum ReconcileOutcome {
  Reconciled { observed_cidrs: Vec<String> },
  NoChange { observed_cidrs: Vec<String> },
  Failed { stage: ReconcileStage, message: String },
}

pub async fn patch_status_for_outcome(
  api: &Api<CIDRPolicy>,
  policy: &CIDRPolicy,
  outcome: &ReconcileOutcome,
) -> Result<(), kube::Error>;
```

## Implementation Status

All planned milestones are complete. The staged milestone checklist has been retired because the implementation now includes typed CRD generation and schema contract coverage, source validation and extraction tests, deterministic `NetworkPolicy` rendering, controller runtime and status behavior, repository CI for formatting, linting, tests, and CRD drift, example manifests with checked-in fixture payloads, and regression coverage that preserves the last known good managed policy across remote-source failures.

## Traceability

| Implementation Section | Governing Spec Heading | Coverage |
| --- | --- | --- |
| Overview | [Specification - Dynamic CIDR NetworkPolicy Controller](../../spec/ipmaze-controller/spec.md#specification---dynamic-cidr-networkpolicy-controller) | Scope, controller purpose, safety posture |
| Concept: Remote CIDR Source | [Concept: Remote CIDR Source](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source) | Source validation, transport, unauthenticated retrieval, JSON enforcement |
| Concept: CIDR Extraction Query | [Concept: CIDR Extraction Query](../../spec/ipmaze-controller/spec.md#concept-cidr-extraction-query) | JMESPath evaluation, array and string checks, CIDR validation, dedupe |
| Concept: Policy Target Selection | [Concept: Policy Target Selection](../../spec/ipmaze-controller/spec.md#concept-policy-target-selection) | Subject and peer selector mapping, lossless translation |
| Concept: Policy Direction Configuration | [Concept: Policy Direction Configuration](../../spec/ipmaze-controller/spec.md#concept-policy-direction-configuration) | Direction defaults and rule rendering |
| Entity: CIDRPolicy Custom Resource | [Entity: CIDRPolicy Custom Resource](../../spec/ipmaze-controller/spec.md#entity-cidrpolicy-custom-resource) | CRD schema, validation, status fields |
| Entity: Managed NetworkPolicy | [Entity: Managed NetworkPolicy](../../spec/ipmaze-controller/spec.md#entity-managed-networkpolicy) | Managed resource identity, exact-set rendering, idempotent reconciliation |
| Entity: Reconciliation Outcome | [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome) | Success, failure, no-change observability and safety |

## Operational Notes

The controller exposes structured logs for reconcile outcomes and emits Kubernetes events for successful reconciles, cleanup, and failure transitions. Runtime configuration currently includes the requeue interval through the CLI and the event reporting instance through `CONTROLLER_POD_NAME`. HTTP timeout and optional CA bundle configuration remain reasonable future runtime extensions, but authentication settings must stay absent because the specification forbids sending credential material.

For cron parsing, the implementation accepts standard 5-field expressions at the API boundary and normalizes them internally for the Rust `cron` crate, which expects a seconds-prefixed schedule representation. Background resync remains additive to watch-driven reconciles rather than replacing create or update event handling.

The implementation should also provide a repeatable CRD generation command, for example `cargo run --bin ipmaze-controller -- generate-crd > config/crd/cidrpolicies.ipmaze.k8s.justin.directory.yaml`, and CI should fail if the checked-in CRD differs from generated output.

