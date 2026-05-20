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

This implementation realizes the Dynamic CIDR NetworkPolicy Controller defined in [Specification - Dynamic CIDR NetworkPolicy Controller](../../spec/ipmaze-controller/spec.md#specification---dynamic-cidr-networkpolicy-controller) as a Kubernetes controller written in modern Rust using kube-rs. The design centers on a typed `CIDRPolicy` custom resource, a reconcile loop that fetches remote JSON, evaluates a JMESPath expression, validates and normalizes CIDRs, and then renders exactly one controller-managed `NetworkPolicy` per custom resource.

The implementation is constrained by the behavior in [Concept: Remote CIDR Source](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source), [Concept: CIDR Extraction Query](../../spec/ipmaze-controller/spec.md#concept-cidr-extraction-query), [Concept: Policy Target Selection](../../spec/ipmaze-controller/spec.md#concept-policy-target-selection), [Concept: Policy Direction Configuration](../../spec/ipmaze-controller/spec.md#concept-policy-direction-configuration), [Entity: CIDRPolicy Custom Resource](../../spec/ipmaze-controller/spec.md#entity-cidrpolicy-custom-resource), [Entity: Managed NetworkPolicy](../../spec/ipmaze-controller/spec.md#entity-managed-networkpolicy), and [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome). The controller preserves the last known good `NetworkPolicy` on retrieval or parsing failure, removes stale CIDRs on successful updates, and defaults omitted rule directionality to both ingress and egress.

The implementation should generate its CRD from the Rust type definitions and verify that the generated schema stays aligned with [spec/ipmaze-controller/policy.schema.json](../../spec/ipmaze-controller/policy.schema.json). Because Kubernetes CRDs use OpenAPI v3 schema rather than full JSON Schema 2020-12, strict adherence should be enforced with contract tests that compare the generated CRD schema and the repository schema for the overlapping representable constraints. The transport profile is a normative requirement: bare DNS names must be fetched as HTTPS URLs, while bare IP addresses must be fetched as HTTP URLs.

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
- `jmespath`: Parsing and evaluating the configured query against the full payload.
- `cidr` or `ipnet`: Strict IPv4 and IPv6 CIDR parsing without coercing host addresses.
- `thiserror`: Structured reconcile and validation errors.
- `tracing` and `tracing-subscriber`: Structured controller logs with stage-specific failure metadata.
- `tokio`: Async runtime for controller, HTTP retrieval, and retry timing.
- `chrono`: Status timestamps such as `lastSuccessfulResolutionTime`.

If the implementation exposes a CLI for CRD generation, `clap` is appropriate for a `generate-crd` subcommand that emits YAML into `config/crd/`.

### Modules and Components

The controller should be split into narrowly owned modules so each normative stage maps cleanly to code:

- `api`: Rust definitions for `CIDRPolicySpec`, `CIDRPolicyStatus`, `Rule`, `Direction`, and CRD generation glue.
- `validation`: Pre-reconcile validation for address syntax, JMESPath compilation, selector representability, and direction normalization.
- `source`: Deterministic remote address resolution, including the requirement that bare DNS names map to HTTPS and bare IP addresses map to HTTP, plus unauthenticated retrieval, content-type agnostic JSON parsing, and transport error classification.
- `extract`: JMESPath evaluation, array and element type checks, CIDR parsing, deduplication, and stable ordering.
- `netpol`: Rendering the exact `NetworkPolicy` object shape, ownership metadata, labels or annotations that mark controller management, and diff logic.
- `status`: Condition or status field calculation, error summarization, and status patch helpers.
- `controller`: `reconcile`, `error_policy`, event recording, and finalizer or owner-reference behavior.
- `bin` or `main`: runtime bootstrap, config loading, and metrics or health endpoints if later added.

### Key Types and Interfaces

The core Rust-facing interfaces should look like this:

```rust
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[kube(group = "networking.example.io", version = "v1alpha1", kind = "CIDRPolicy", namespaced)]
#[kube(status = "CIDRPolicyStatus")]
pub struct CIDRPolicySpec {
    pub source: SourceSpec,
    pub target: TargetSpec,
    pub rules: Vec<RuleSpec>,
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

`CIDRPolicySpec` remains the source of truth for CRD generation. Nested Rust types should use `serde(rename_all = "camelCase")` or explicit `#[serde(rename = "...")]` attributes so fields such as `jmesPath`, `podSelector`, `namespaceSelector`, `lastSuccessfulResolutionTime`, `lastObservedCidrs`, and `lastReconciliationError` match the schema exactly. `RemoteFetcher` and `PolicyRenderer` are useful seams for unit tests. `NormalizedCidr` should be a validated wrapper type so later stages cannot receive invalid strings.

### Error Handling

Errors should be stage-specific and explicit so operators can distinguish failures required by [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome):

- `AddressValidation`: address is neither DNS name, IP literal, nor an accepted URL form.
- `Transport`: DNS resolution, TLS, timeout, connection, or non-success HTTP failures.
- `JsonDecode`: response body is not valid JSON.
- `JmesPathCompile`: invalid query in the custom resource.
- `JmesPathEvaluate`: runtime query failure against the payload.
- `ResultShape`: query result is not an array of strings.
- `CidrValidation`: one or more strings are not valid IPv4 or IPv6 CIDRs.
- `SelectorTranslation`: selector fields cannot be rendered losslessly into `NetworkPolicy`.
- `KubernetesApi`: create, patch, status patch, or event recording failure.

On any post-success failure, the controller must update status and events without deleting or blanking the previously reconciled managed `NetworkPolicy`.

### Data Flow

The steady-state reconcile flow is:

1. Read the `CIDRPolicy` instance and validate required fields plus parseable JMESPath.
2. Resolve or normalize the configured source address according to the required deterministic transport profile: bare DNS names map to `https://.../`, bare IP addresses map to `http://.../`, and explicit `http://` or `https://` URLs are preserved.
3. Perform an unauthenticated HTTP GET with no cookies or auth material.
4. Parse the body as JSON and fail the reconcile if parsing fails.
5. Evaluate the JMESPath expression against the full payload.
6. Assert the result is an array of strings, then parse every entry as an IPv4 or IPv6 CIDR.
7. Deduplicate while preserving the semantic set, then sort into a deterministic order for stable reconciliation.
8. Translate subject selectors and peer selectors into typed `NetworkPolicy` selectors.
9. Render ingress peers, egress peers, or both, based on each rule's directions or the default behavior.
10. Create or patch the managed `NetworkPolicy` only if the effective spec differs.
11. Patch status with last success time, last observed CIDRs, and cleared or updated error text.

### External Integrations

The implementation integrates with:

- Kubernetes API server through kube-rs for custom resources, `NetworkPolicy`, events, and status updates.
- Remote JSON endpoints over HTTP(S) through `reqwest`.
- JMESPath evaluation through the Rust library, compiled at validation or reconcile time.
- CRD generation from Rust type metadata into YAML artifacts checked into `config/crd/`.

The remote fetch client must not use ambient credentials, injected headers, or cookie jars. For source normalization, the implementation must treat bare DNS names as HTTPS targets and bare IP literals as HTTP targets to satisfy [Concept: Remote CIDR Source](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source).

## Concept & Entity Breakdown

### Concept: Remote CIDR Source [Specification Link](../../spec/ipmaze-controller/spec.md#concept-remote-cidr-source)

This concept is realized by `source::RemoteAddress`, `source::ReqwestFetcher`, and validation helpers. The controller must accept exactly one configured address, perform an unauthenticated HTTP GET, refuse to attach credentials, and treat non-JSON payloads as reconcile failures. It must also normalize bare DNS names to HTTPS URLs and bare IP literals to HTTP URLs before retrieval, while preserving explicit HTTP or HTTPS schemes already present in the resource. To preserve safety requirements from [Entity: Reconciliation Outcome](../../spec/ipmaze-controller/spec.md#entity-reconciliation-outcome), fetch failures only update status or events and do not blank an existing managed policy.

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
  pub jmes_path: String,
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

This entity is implemented as the `CustomResource` derive input and is the source for both runtime deserialization and CRD generation. The Rust type definitions should mirror the repository schema closely enough that generated CRD YAML and [spec/ipmaze-controller/policy.schema.json](../../spec/ipmaze-controller/policy.schema.json) stay contract-compatible.

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

## Staged Implementation Plan

### Milestone 1 - API Types and CRD Generation

Status: Complete

- Define `CIDRPolicy`, nested spec types, status types, and direction enums in Rust.
- Generate the CRD YAML from the Rust types and place it under `config/crd/`.
- Add schema contract tests comparing generated CRD structure against [spec/ipmaze-controller/policy.schema.json](../../spec/ipmaze-controller/policy.schema.json) for required fields, enum values, selector shapes, and status structure.
- Add documentation describing how to regenerate the CRD.

Implemented in [crates/ipmaze-controller/src/api.rs](../../crates/ipmaze-controller/src/api.rs), [crates/ipmaze-controller/src/lib.rs](../../crates/ipmaze-controller/src/lib.rs), [crates/ipmaze-controller/src/main.rs](../../crates/ipmaze-controller/src/main.rs), [crates/ipmaze-controller/tests/schema_contract.rs](../../crates/ipmaze-controller/tests/schema_contract.rs), and the generated CRD artifact [config/crd/cidrpolicies.ipmaze.k8s.justin.directory.yaml](../../config/crd/cidrpolicies.ipmaze.k8s.justin.directory.yaml).

### Milestone 2 - Validation and Extraction Pipeline

Status: Complete

- Implement address validation, transport normalization with DNS-to-HTTPS and IP-to-HTTP defaults, JSON retrieval, JMESPath compilation, and strict CIDR extraction.
- Add unit tests for invalid addresses, invalid JSON, invalid JMESPath, non-array results, non-string elements, invalid CIDRs, duplicate CIDRs, and empty-array success.
- Add unit tests covering bare DNS name normalization, bare IP normalization, and explicit scheme preservation.

Implemented in [crates/ipmaze-controller/src/source.rs](../../crates/ipmaze-controller/src/source.rs), [crates/ipmaze-controller/src/extract.rs](../../crates/ipmaze-controller/src/extract.rs), and [crates/ipmaze-controller/src/validation.rs](../../crates/ipmaze-controller/src/validation.rs). The current crate test suite covers the source normalization, JSON parsing, JMESPath validation, result-shape checks, CIDR validation, duplicate removal, and empty-result behavior required by this milestone.

### Milestone 3 - NetworkPolicy Rendering

Status: Complete

- Implement selector translation and deterministic `NetworkPolicy` rendering.
- Add unit tests for ingress-only, egress-only, omitted directionality, mixed IPv4 and IPv6 CIDRs, empty CIDR sets, and stale CIDR removal.
- Document the managed policy naming and ownership scheme.

Implemented in [crates/ipmaze-controller/src/netpol.rs](../../crates/ipmaze-controller/src/netpol.rs) with unit coverage for ingress-only and default-both directionality, mixed-family IPBlock rendering, empty CIDR handling, stale CIDR replacement, and controller-managed metadata markers. The managed policy name remains `<cidrpolicy-name>-managed`, and the rendered `NetworkPolicy` is marked with the `app.kubernetes.io/managed-by=ipmaze-controller` label, the `ipmaze.k8s.justin.directory/source-policy` annotation, and a controller owner reference when available.

### Milestone 4 - Controller Runtime and Status Reporting

Status: Complete

- Implement the kube-rs reconcile loop, status updates, events, retry policy, and ownership or finalizer behavior.
- Add integration tests against a test cluster or envtest-like harness covering create, update, delete, transient fetch failure after success, and no-op reconcile behavior.
- Add operator-facing docs for required RBAC and deployment configuration.

Implemented in [crates/ipmaze-controller/src/controller.rs](../../crates/ipmaze-controller/src/controller.rs), [crates/ipmaze-controller/src/status.rs](../../crates/ipmaze-controller/src/status.rs), and [crates/ipmaze-controller/src/main.rs](../../crates/ipmaze-controller/src/main.rs). The controller now runs a kube-rs reconcile loop, patches status for success, no-change, and failure outcomes, emits Kubernetes events for successful reconciles, cleanup, and failures, and deletes the managed `NetworkPolicy` when a `CIDRPolicy` enters deletion. Integration-style coverage for create, update, delete, transient fetch failure after success, and no-change behavior lives in [crates/ipmaze-controller/tests/controller_runtime.rs](../../crates/ipmaze-controller/tests/controller_runtime.rs) using a fake Kubernetes API harness plus real HTTP fixtures for the remote source. Operator-facing RBAC and deployment configuration are provided in [config/controller/rbac.yaml](../../config/controller/rbac.yaml), [config/controller/deployment.yaml](../../config/controller/deployment.yaml), and [config/controller/README.md](../../config/controller/README.md).

### Milestone 5 - Release Hardening

Status: Not started

- Add CI for CRD generation drift, formatting, clippy, unit tests, and integration tests.
- Add example manifests and end-to-end fixture payloads.
- Verify that previously successful policies remain intact across remote-source failures.

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

The implementation should also provide a repeatable CRD generation command, for example `cargo run --bin ipmaze-controller -- generate-crd > config/crd/cidrpolicies.ipmaze.k8s.justin.directory.yaml`, and CI should fail if the checked-in CRD differs from generated output.

