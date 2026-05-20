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

This specification defines a Kubernetes controller and its custom resource for resolving remote JSON data into CIDR blocks and reconciling controller-managed NetworkPolicy resources from that resolved set. The specification covers remote retrieval, JMESPath evaluation, CIDR validation, selector mapping, and reconciliation behavior.

## Terminology & References

This document uses the normative keywords defined in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).
Readers SHOULD also understand Kubernetes CustomResourceDefinitions, Kubernetes NetworkPolicy semantics, JSON, JMESPath, and CIDR notation for IPv4 and IPv6.

## Concepts

### Concept: Remote CIDR Source

The Remote CIDR Source is the externally hosted JSON document from which the controller derives allowed CIDR blocks.

!cidr.source.retrieval:

- The custom resource MUST declare exactly one remote address.
- The remote address MUST be either a DNS name or an IP address.
- The controller MUST retrieve the remote document using an HTTP GET request.
- The controller MUST access the remote address without authentication.
- The controller MUST NOT attach authentication headers, cookies, client-identity assertions, or other credential material to the retrieval request.
- The remote endpoint MUST return a JSON payload.
- The controller MUST treat a response body that is not valid JSON as a reconciliation failure for that resource instance.

!cidr.source.transport:

- The controller MUST support remote retrieval over a deterministic transport profile documented by the implementation.
- When the remote address is a bare DNS name with no scheme prefix, the controller MUST synthesize an HTTPS URL for retrieval.
- When the remote address is a bare IP address with no scheme prefix, the controller MUST synthesize an HTTP URL for retrieval.
- When the remote address already includes an explicit HTTP or HTTPS scheme, the controller MUST use the declared scheme as provided.
- The controller MUST record retrieval failures in resource status or events without deleting unrelated user-managed NetworkPolicy resources.

Example: a resource points to `https://example.invalid/allowlist.json`, the endpoint returns a JSON object, and the controller proceeds to query evaluation.

Example: if a resource declares `example.invalid` as a bare DNS name, the controller retrieves `https://example.invalid/`. If a resource declares `203.0.113.10` as a bare IP address, the controller retrieves `http://203.0.113.10/`.

Edge case: if the address resolves successfully but the endpoint returns HTML or plain text, the controller MUST reject that payload for reconciliation because the payload is not JSON.

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

- The resource MUST contain a field for the remote address.
- The resource MUST contain a field for the JMESPath query.
- The resource MUST contain a field set for subject pod selection.
- The resource MUST contain a field set for selector-bearing peer constraints, including pod selector and namespace selector support where used by the resource model.
- The resource MUST allow each selector-bearing rule to declare ingress, egress, or both.
- The resource MUST preserve the Kubernetes-native field names and semantics for each adopted selector structure.
- The resource SHOULD expose status fields for last successful resolution time, last observed CIDR set, and most recent reconciliation error.

!policy.crd.validation:

- Admission or reconciliation validation MUST reject a resource missing any required field.
- Validation MUST reject a remote address that is neither a DNS name nor an IP address.
- Validation MUST reject a query that cannot be parsed as JMESPath.
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
    address: https://example.invalid/allowlist.json
    jmesPath: prefixes
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
- When the custom resource is deleted, the controller SHOULD delete the managed NetworkPolicy if ownership semantics permit it.

Example: if the resolved set changes from `["10.0.0.0/24"]` to `["10.0.0.0/24", "2001:db8::/32"]`, the controller updates the managed NetworkPolicy so both CIDR blocks are represented and no stale CIDR blocks remain.

### Entity: Reconciliation Outcome

The Reconciliation Outcome captures whether a single reconcile cycle succeeded, failed, or produced no material change.

!controller.reconciliation.observability:

- A successful reconcile MUST indicate that remote retrieval, query evaluation, CIDR validation, and managed NetworkPolicy reconciliation all completed without error.
- A failed reconcile MUST identify the failed stage with enough detail for an operator to distinguish address errors, transport errors, JSON parsing errors, JMESPath errors, CIDR validation errors, and Kubernetes API errors.
- A no-change reconcile SHOULD avoid unnecessary writes to the Kubernetes API.

!controller.reconciliation.safety:

- On failure after a previously successful reconcile, the controller MUST preserve the last successfully reconciled managed NetworkPolicy until a later successful reconcile or explicit deletion event.
- The controller MUST NOT replace the managed NetworkPolicy with an empty or partial policy solely because remote retrieval failed.
- If the resolved CIDR set is an empty array, the controller MUST reconcile a managed NetworkPolicy whose CIDR-derived peer set is empty, rather than treating the empty result as an error.

