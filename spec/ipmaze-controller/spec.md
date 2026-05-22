---
name: ipmaze-controller
title: Network Policy Controller for Dynamic CIDR Sources
description: Specifies a Kubernetes controller and custom resource that resolve remote JSON through JMESPath into CIDR blocks and reconcile managed NetworkPolicy resources targeting selected pods.
tags:
- kubernetes
- networkpolicy
- controller
- crd
- jmespath
version: 1.0.0
dependencies: []
requires_implementation: true
---

<!-- AI TODO: Update the front matter fields to reflect the real specification metadata and retain a dependency on the SpecMan data model unless an official successor is adopted. -->

# Specification - Dynamic CIDR NetworkPolicy Controller

This specification defines a Kubernetes controller and its custom resource for resolving remote JSON data into CIDR blocks and reconciling controller-managed NetworkPolicy resources from that resolved set. The specification covers remote retrieval, optional pointer-address resolution, JMESPath evaluation, CIDR validation, selector mapping, periodic resynchronization, and reconciliation behavior.

## Terminology & References

This document uses the normative keywords defined in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).
Readers SHOULD also understand Kubernetes CustomResourceDefinitions, Kubernetes NetworkPolicy semantics, JSON, JMESPath, and CIDR notation for IPv4 and IPv6.

## Concepts

### Concept: Remote CIDR Source

The Remote CIDR Source is the externally reachable location from which the controller ultimately obtains the JSON document used for CIDR extraction. A resource may identify either a direct JSON endpoint or a pointer endpoint. When the resource does not declare pointer extraction behavior, the configured address is the JSON endpoint. When the resource declares pointer extraction behavior, the controller first retrieves the configured address as pointer content, applies the declared regular expression to that response body, and treats the extracted value as the JSON endpoint for the subsequent JSON retrieval and JMESPath evaluation stages.

!cidr.source.retrieval:

- The custom resource MUST declare exactly one base remote address.
- The base remote address MUST be either a DNS name or an IP address, with or without an explicit HTTP or HTTPS scheme.
- The controller MUST retrieve the remote document using an HTTP GET request.
- The controller MUST access the remote address without authentication.
- The controller MUST NOT attach authentication headers, cookies, client-identity assertions, or other credential material to the retrieval request.
- When pointer extraction behavior is omitted, the remote endpoint identified by the base remote address MUST return a JSON payload.
- When pointer extraction behavior is present, the pointer endpoint identified by the base remote address MAY return arbitrary text and the final resolved JSON endpoint MUST return a JSON payload.
- The controller MUST treat a response body from the final resolved JSON endpoint that is not valid JSON as a reconciliation failure for that resource instance.

!cidr.source.transport:

- The controller MUST support remote retrieval over a deterministic transport profile documented by the implementation.
- When a configured or extracted address is a bare DNS name with no scheme prefix, the controller MUST synthesize an HTTPS URL for retrieval.
- When a configured or extracted address is a bare IP address with no scheme prefix, the controller MUST synthesize an HTTP URL for retrieval.
- When a configured or extracted address already includes an explicit HTTP or HTTPS scheme, the controller MUST use the declared scheme as provided.
- The controller MUST record retrieval failures in resource status or events without deleting unrelated user-managed NetworkPolicy resources.

!cidr.source.pointer.resolution:

- The custom resource MAY declare optional pointer extraction behavior for the configured remote address.
- When pointer extraction behavior is omitted, the controller MUST treat the configured remote address as the final JSON endpoint.
- When pointer extraction behavior is present, the controller MUST first retrieve the configured remote address and treat the response body as pointer content.
- The controller MUST apply the declared regular expression to the full pointer-content response body.
- When the regular expression yields no match, the controller MUST treat that reconcile as a failure for that resource instance.
- When the regular expression yields a match, the controller MUST use the first capture group from the first match as the JSON endpoint for the subsequent retrieval stage.
- If the first capture group from the first match is empty, the controller MUST treat that reconcile as a failure for that resource instance.
- The controller MUST treat the extracted JSON endpoint exactly as it would treat a directly configured remote address for transport selection, JSON retrieval, JSON validation, and query evaluation.

!cidr.source.pointer.content:

- A pointer endpoint response body MUST NOT be required to be valid JSON.
- The final resolved JSON endpoint MUST return a valid JSON payload.
- The controller MUST NOT perform repeated pointer-chasing from one extracted address to another unless a future specification revision explicitly defines that behavior.

Example: a resource points to `https://example.invalid/allowlist.json`, the endpoint returns a JSON object, and the controller proceeds to query evaluation.

Example: if a resource declares `example.invalid` as a bare DNS name, the controller retrieves `https://example.invalid/`. If a resource declares `203.0.113.10` as a bare IP address, the controller retrieves `http://203.0.113.10/`.

Example: if a resource declares `https://example.invalid/current.txt` as the base remote address and a pointer regex whose first capture group extracts `cdn.example.invalid/allowlist.json`, the controller retrieves the pointer content from `https://example.invalid/current.txt`, interprets the captured address using the same address rules as a directly configured address, and then retrieves `https://cdn.example.invalid/allowlist.json` for JSON parsing and query evaluation.

Edge case: if the address resolves successfully but the endpoint returns HTML or plain text, the controller MUST reject that payload for reconciliation because the payload is not JSON.

Edge case: if the pointer endpoint returns content that does not produce a match, or the first capture group from the first match is empty, the controller MUST fail reconciliation for that resource instance.

### Concept: CIDR Extraction Query

The CIDR Extraction Query is the JMESPath expression applied to the retrieved JSON payload.

!cidr.extraction.query.evaluation:

- The custom resource MUST declare exactly one JMESPath query.
- The controller MUST evaluate the query against the full JSON payload returned by the remote source.
- The query result MUST be a JSON array.
- Every element of the resulting array MUST be a string.
- Every resulting string MUST be a syntactically valid IPv4 CIDR block or IPv6 CIDR block.
- The controller MUST treat any non-array result, any non-string element, or any invalid CIDR string as a reconciliation failure for that resource instance.

!cidr.extraction.query.normalization:

- The controller SHOULD preserve the semantic CIDR set while removing duplicate entries before reconciling the managed NetworkPolicy.
- The controller MUST NOT silently coerce host addresses into CIDR notation.
- The controller MUST treat an empty array as a successful query evaluation.

Example: given JSON `{ "prefixes": ["10.0.0.0/24", "2001:db8::/32"] }` and query `prefixes`, the resolved value is valid.

Edge case: if the query resolves to `["10.0.0.1", "2001:db8::/32"]`, reconciliation MUST fail because `10.0.0.1` is not CIDR notation.

### Concept: Policy Target Selection

Policy Target Selection identifies which pods and namespaces are selected by the controller-managed NetworkPolicy structures adopted by this specification.

!policy.target.mapping:

- The custom resource MUST include selector fields that map 1:1 to the Kubernetes selector structures adopted by this specification.
- The custom resource MUST support a pod selector with the same structure and semantics as `NetworkPolicySpec.podSelector`.
- The custom resource MUST support a namespace selector with the same structure and semantics as the `namespaceSelector` used in Kubernetes NetworkPolicy peers.
- When the specification adopts an upstream Kubernetes selector structure, the custom resource MUST preserve the same field names, value domains, and matching semantics for that structure.
- The controller MUST reject a resource whose selector fields cannot be translated losslessly into the target NetworkPolicy representation.

!policy.target.scope:

- The resource MUST distinguish selectors that target the managed policy's subject pods from selectors that constrain ingress or egress peers.
- The subject pod selector MUST be resolved within the namespace of the custom resource.
- A namespace selector, when present for peer selection, MUST preserve the cross-namespace matching semantics defined by Kubernetes NetworkPolicy peers.
- The controller MUST NOT reinterpret a subject pod selector as a peer selector, or a peer selector as a subject pod selector.

!policy.target.directionality:

- A selector-bearing rule in the custom resource MUST be able to declare whether it applies to ingress, egress, or both.
- If a selector-bearing rule omits directionality, the controller MUST treat that rule as applying to both ingress and egress.
- The controller MUST preserve the declared directionality when rendering the managed NetworkPolicy.

Example: a custom resource in namespace `payments` with a subject pod selector matching `app=api` targets pods in namespace `payments` labeled `app=api`, and a peer namespace selector can separately constrain which namespaces are allowed to communicate with those pods.

Edge case: if a resource attempts to express a selector shape that has no exact NetworkPolicy equivalent in the adopted upstream model, the controller MUST reject that resource instead of approximating the match.

### Concept: Policy Direction Configuration

Policy Direction Configuration determines whether resolved CIDR blocks are rendered into ingress rules, egress rules, or both.

!policy.direction.rendering:

- The custom resource MUST allow configuration of whether the managed NetworkPolicy permits ingress, egress, or both.
- If the resource does not explicitly constrain directionality, the controller MUST configure both ingress and egress behavior by default.
- When ingress is enabled, the controller MUST render resolved CIDR blocks into ingress peers using Kubernetes NetworkPolicy ingress semantics.
- When egress is enabled, the controller MUST render resolved CIDR blocks into egress peers using Kubernetes NetworkPolicy egress semantics.
- The controller MUST NOT render ingress rules when only egress is configured, and MUST NOT render egress rules when only ingress is configured.

Example: if a rule is marked `ingress` only, the controller renders the CIDR blocks only under ingress peers; if directionality is omitted, the controller renders equivalent ingress and egress peers.

## Key Entities

### Entity: CIDRPolicy Custom Resource

The CIDRPolicy Custom Resource is the declarative input consumed by the controller.

!policy.crd.schema:

- The resource MUST contain a field for the base remote address.
- The resource MUST contain a field for the JMESPath query.
- The resource MUST allow an optional nested pointer-configuration object for pointer-based JSON endpoint extraction.
- The resource MUST allow an optional field for periodic resynchronization schedule.
- The resource MUST contain a field set for subject pod selection.
- The resource MUST contain a field set for selector-bearing peer constraints, including pod selector and namespace selector support where used by the resource model.
- The resource MUST allow each selector-bearing rule to declare ingress, egress, or both.
- The resource MUST preserve the Kubernetes-native field names and semantics for each adopted selector structure.
- The resource SHOULD expose status fields for last successful resolution time, last observed CIDR set, and most recent reconciliation error.

!policy.crd.source.pointer:

- The resource MUST allow source pointer extraction behavior to be omitted.
- The resource MUST allow an optional `pointer` object under the source configuration.
- The `pointer` object MUST allow a regular-expression field used to derive the actual JSON endpoint from the base remote address response body.
- When pointer extraction behavior is present, the implementation MUST apply the regex to the pointer response body and use the first capture group from the first match as the resolved JSON endpoint.
- If the first capture group from the first match is empty, the implementation MUST treat pointer extraction as failed.
- Validation MUST reject pointer configuration whose regex does not define the first capture group used to derive the final address.
- Validation or reconciliation MUST reject a pointer regex match result whose first capture group is empty or cannot be interpreted using the same address rules as a directly configured remote address.
- Validation MUST NOT require pointer extraction configuration when the base remote address already identifies the JSON endpoint.

!policy.crd.resync.schedule:

- The resource MUST allow an optional cron schedule field that controls periodic resynchronization.
- When the cron schedule field is omitted, the controller MUST behave as though the resource declared the UTC schedule `0 0 * * *`.
- The cron schedule field MUST use standard 5-field cron syntax.
- The periodic resynchronization schedule MUST govern background resync cadence only.
- The controller MUST continue to reconcile immediately on create and update events regardless of the periodic resynchronization schedule.
- The resource MUST NOT require the user to specify a cron schedule in order to obtain periodic reconciliation.

!policy.crd.validation:

- Admission or reconciliation validation MUST reject a resource missing any required field.
- Validation MUST reject a remote address that cannot be interpreted as a DNS name or an IP address with optional HTTP or HTTPS scheme.
- Validation MUST reject a query that cannot be parsed as JMESPath.
- Validation MUST reject a pointer regex that cannot be parsed as a valid regular expression when that field is present.
- Validation MUST reject a cron schedule that cannot be parsed as a standard 5-field cron expression when that field is present.
- Validation MUST reject any configuration that requests a direction other than ingress, egress, or both.
- Validation MUST reject selector content that is not representable by the adopted Kubernetes selector structures.

Example:

```yaml
apiVersion: networking.example.io/v1alpha1
kind: CIDRPolicy
metadata:
  name: office-allowlist
  namespace: payments
spec:
  source:
    address: https://example.invalid/current.txt
    pointer:
      regex: '(https://cdn\.example\.invalid/allowlists/[A-Za-z0-9._/-]+\.json)'
    jmesPath: prefixes
  resyncSchedule: "0 0 * * *"
  target:
    podSelector:
      matchLabels:
        app: api
  rules:
    - directions:
        - ingress
      namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: shared-services
    - podSelector:
        matchLabels:
          access-tier: trusted
```

### Entity: Managed NetworkPolicy

The Managed NetworkPolicy is the Kubernetes NetworkPolicy resource created or updated by the controller from a CIDRPolicy Custom Resource.

!controller.networkpolicy.reconciliation:

- The controller MUST create a managed NetworkPolicy when a valid CIDRPolicy Custom Resource exists and no corresponding managed NetworkPolicy exists.
- The controller MUST update the managed NetworkPolicy when the resolved CIDR set changes or when the target selector changes.
- The controller MUST ensure that the effective set of CIDR blocks represented in the managed NetworkPolicy exactly matches the validated resolved CIDR set.
- The controller MUST render resolved CIDR blocks into ingress `from.ipBlock` peers when ingress is enabled.
- The controller MUST render resolved CIDR blocks into egress `to.ipBlock` peers when egress is enabled.
- If directionality is omitted in the custom resource, the controller MUST configure both ingress and egress in the managed NetworkPolicy.
- The controller MUST preserve exact set matching for the validated resolved CIDR blocks and MUST remove stale CIDR entries from the managed NetworkPolicy.
- The controller MUST reconcile the managed NetworkPolicy idempotently.

!controller.networkpolicy.ownership:

- The managed NetworkPolicy MUST be identifiable as controller-managed.
- The controller MUST set an ownership relationship or equivalent management marker that lets it distinguish managed resources from unrelated NetworkPolicy resources.
- The controller MUST NOT modify unrelated user-managed NetworkPolicy resources.
- If the deterministic managed NetworkPolicy name resolves to an existing NetworkPolicy that is not provably owned by the reconciling custom resource, the controller MUST treat that condition as a naming collision and MUST fail reconciliation without modifying the existing NetworkPolicy.
- If the deterministic managed NetworkPolicy name resolves to an existing NetworkPolicy that is not provably owned by the reconciling custom resource, the controller MUST fail cleanup without deleting the existing NetworkPolicy.
- Proof of ownership for a managed NetworkPolicy MUST include controller management metadata sufficient to distinguish the resource from unrelated NetworkPolicy objects created by users or other orchestration software.
- When a naming collision is detected, the controller MUST emit a warning event for the custom resource and MUST write an operator-visible warning or error log entry describing the collision.
- Implementations targeting a specific orchestration environment MUST treat collisions with that environment's reserved or implementation-defined managed NetworkPolicy naming conventions as reconciliation failures rather than attempting adoption by name alone.
- When the custom resource is deleted, the controller SHOULD delete the managed NetworkPolicy if ownership semantics permit it.

Example: if the resolved set changes from `["10.0.0.0/24"]` to `["10.0.0.0/24", "2001:db8::/32"]`, the controller updates the managed NetworkPolicy so both CIDR blocks are represented and no stale CIDR blocks remain.

### Entity: Reconciliation Outcome

The Reconciliation Outcome captures whether a single reconcile cycle succeeded, failed, or produced no material change.

!controller.reconciliation.observability:

- A successful reconcile MUST indicate that remote retrieval, query evaluation, CIDR validation, and managed NetworkPolicy reconciliation all completed without error.
- A failed reconcile MUST identify the failed stage with enough detail for an operator to distinguish address errors, transport errors, pointer retrieval errors, pointer extraction errors, JSON parsing errors, JMESPath errors, CIDR validation errors, and Kubernetes API errors.
- A failed reconcile MUST distinguish managed-resource naming collisions from ordinary Kubernetes API failures.
- A no-change reconcile SHOULD avoid unnecessary writes to the Kubernetes API.

!controller.reconciliation.pointer.failures:

- A failed reconcile MUST distinguish failure to retrieve the base remote address from failure to extract a JSON endpoint from pointer content.
- A failed reconcile MUST distinguish pointer extraction failure from failure to retrieve or parse the final resolved JSON endpoint.
- If pointer extraction yields no match, the reconcile outcome MUST be failed rather than no-change.
- If the first capture group from the first regex match is empty, the reconcile outcome MUST be failed rather than no-change.

!controller.reconciliation.safety:

- On failure after a previously successful reconcile, the controller MUST preserve the last successfully reconciled managed NetworkPolicy until a later successful reconcile or explicit deletion event.
- The controller MUST NOT replace the managed NetworkPolicy with an empty or partial policy solely because remote retrieval failed.
- The controller MUST NOT replace the managed NetworkPolicy with an empty or partial policy solely because pointer extraction failed.
- If the resolved CIDR set is an empty array, the controller MUST reconcile a managed NetworkPolicy whose CIDR-derived peer set is empty, rather than treating the empty result as an error.

