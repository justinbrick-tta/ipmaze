use crate::api::{CIDRPolicy, Direction, LabelSelector, LabelSelectorOperator, RuleSpec};
use crate::extract::NormalizedCidr;
use k8s_openapi::api::networking::v1::{
    IPBlock, NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyIngressRule, NetworkPolicyPeer,
    NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{
    LabelSelector as KubeLabelSelector, ObjectMeta,
};
use kube::api::{Api, Patch, PatchParams};
use kube::Resource;
use kube::ResourceExt;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
const MANAGED_BY_VALUE: &str = "ipmaze-controller";
const SOURCE_POLICY_ANNOTATION: &str = "ipmaze.k8s.justin.directory/source-policy";
const FIELD_MANAGER: &str = "ipmaze-controller";

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("CIDRPolicy must have a namespace before rendering a managed NetworkPolicy")]
    MissingNamespace,
    #[error("selector expression `{0}` is missing values for operator `{1}`")]
    MissingSelectorValues(String, &'static str),
    #[error("selector expression `{0}` must not include values for operator `{1}`")]
    UnexpectedSelectorValues(String, &'static str),
}

pub fn effective_directions(rule: &RuleSpec) -> BTreeSet<Direction> {
    match &rule.directions {
        Some(directions) => directions.iter().cloned().collect(),
        None => BTreeSet::from([Direction::Ingress, Direction::Egress]),
    }
}

pub fn render_subject_selector(selector: &LabelSelector) -> Result<KubeLabelSelector, RenderError> {
    render_selector(selector)
}

pub fn render_peer_selector(rule: &RuleSpec) -> Result<NetworkPolicyPeer, RenderError> {
    Ok(NetworkPolicyPeer {
        ip_block: None,
        namespace_selector: rule
            .namespace_selector
            .as_ref()
            .map(render_selector)
            .transpose()?,
        pod_selector: rule
            .pod_selector
            .as_ref()
            .map(render_selector)
            .transpose()?,
    })
}

pub fn build_managed_network_policy(
    policy: &CIDRPolicy,
    cidrs: &[NormalizedCidr],
) -> Result<NetworkPolicy, RenderError> {
    let namespace = policy.namespace().ok_or(RenderError::MissingNamespace)?;
    let name = policy.managed_network_policy_name();
    let subject_selector = render_subject_selector(&policy.spec.target.pod_selector)?;

    let ingress = build_ingress_rules(&policy.spec.rules, cidrs)?;
    let egress = build_egress_rules(&policy.spec.rules, cidrs)?;
    let policy_types = build_policy_types(&policy.spec.rules);

    let mut labels = BTreeMap::new();
    labels.insert(MANAGED_BY_LABEL.to_owned(), MANAGED_BY_VALUE.to_owned());

    let mut annotations = BTreeMap::new();
    annotations.insert(SOURCE_POLICY_ANNOTATION.to_owned(), policy.name_any());

    Ok(NetworkPolicy {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace),
            labels: Some(labels),
            annotations: Some(annotations),
            owner_references: policy.controller_owner_ref(&()).map(|owner| vec![owner]),
            ..ObjectMeta::default()
        },
        spec: Some(NetworkPolicySpec {
            egress: Some(egress),
            ingress: Some(ingress),
            pod_selector: subject_selector,
            policy_types: Some(policy_types),
        }),
    })
}

pub async fn apply_managed_network_policy(
    api: &Api<NetworkPolicy>,
    network_policy: &NetworkPolicy,
) -> Result<(), kube::Error> {
    let name = network_policy.metadata.name.as_deref().unwrap_or_default();
    api.patch(
        name,
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(network_policy),
    )
    .await?;
    Ok(())
}

fn build_policy_types(rules: &[RuleSpec]) -> Vec<String> {
    let mut policy_types = BTreeSet::new();
    for rule in rules {
        for direction in effective_directions(rule) {
            policy_types.insert(match direction {
                Direction::Ingress => "Ingress".to_owned(),
                Direction::Egress => "Egress".to_owned(),
            });
        }
    }

    policy_types.into_iter().collect()
}

fn build_ingress_rules(
    rules: &[RuleSpec],
    cidrs: &[NormalizedCidr],
) -> Result<Vec<NetworkPolicyIngressRule>, RenderError> {
    let mut ingress_rules = Vec::new();

    for rule in rules {
        if effective_directions(rule).contains(&Direction::Ingress) {
            ingress_rules.push(NetworkPolicyIngressRule {
                from: Some(build_peers(rule, cidrs)?),
                ports: None,
            });
        }
    }

    Ok(ingress_rules)
}

fn build_egress_rules(
    rules: &[RuleSpec],
    cidrs: &[NormalizedCidr],
) -> Result<Vec<NetworkPolicyEgressRule>, RenderError> {
    let mut egress_rules = Vec::new();

    for rule in rules {
        if effective_directions(rule).contains(&Direction::Egress) {
            egress_rules.push(NetworkPolicyEgressRule {
                ports: None,
                to: Some(build_peers(rule, cidrs)?),
            });
        }
    }

    Ok(egress_rules)
}

fn build_peers(
    rule: &RuleSpec,
    cidrs: &[NormalizedCidr],
) -> Result<Vec<NetworkPolicyPeer>, RenderError> {
    let mut peers = Vec::new();
    if rule.namespace_selector.is_some() || rule.pod_selector.is_some() {
        peers.push(render_peer_selector(rule)?);
    }

    peers.extend(cidrs.iter().map(|cidr| NetworkPolicyPeer {
        ip_block: Some(IPBlock {
            cidr: cidr.rendered.clone(),
            except: None,
        }),
        namespace_selector: None,
        pod_selector: None,
    }));

    Ok(peers)
}

fn render_selector(selector: &LabelSelector) -> Result<KubeLabelSelector, RenderError> {
    let match_expressions =
        selector
            .match_expressions
            .as_ref()
            .map(|requirements| {
                requirements
                .iter()
                .map(|requirement| {
                    let operator = match requirement.operator {
                        LabelSelectorOperator::In => {
                            if requirement.values.as_ref().is_none_or(Vec::is_empty) {
                                return Err(RenderError::MissingSelectorValues(
                                    requirement.key.clone(),
                                    "In",
                                ));
                            }
                            "In"
                        }
                        LabelSelectorOperator::NotIn => {
                            if requirement.values.as_ref().is_none_or(Vec::is_empty) {
                                return Err(RenderError::MissingSelectorValues(
                                    requirement.key.clone(),
                                    "NotIn",
                                ));
                            }
                            "NotIn"
                        }
                        LabelSelectorOperator::Exists => {
                            if requirement.values.is_some() {
                                return Err(RenderError::UnexpectedSelectorValues(
                                    requirement.key.clone(),
                                    "Exists",
                                ));
                            }
                            "Exists"
                        }
                        LabelSelectorOperator::DoesNotExist => {
                            if requirement.values.is_some() {
                                return Err(RenderError::UnexpectedSelectorValues(
                                    requirement.key.clone(),
                                    "DoesNotExist",
                                ));
                            }
                            "DoesNotExist"
                        }
                    };

                    Ok(k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement {
                        key: requirement.key.clone(),
                        operator: operator.to_owned(),
                        values: requirement.values.clone(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;

    Ok(KubeLabelSelector {
        match_labels: selector.match_labels.clone(),
        match_expressions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CIDRPolicySpec, LabelSelector, RuleSpec, SourceSpec, StringMap, TargetSpec};
    use pretty_assertions::assert_eq;

    fn policy_with_rule(rule: RuleSpec) -> CIDRPolicy {
        let mut policy = CIDRPolicy::new(
            "office-allowlist",
            CIDRPolicySpec {
                source: SourceSpec {
                    address: "example.invalid".to_owned(),
                    jmes_path: "prefixes".to_owned(),
                },
                target: TargetSpec {
                    pod_selector: LabelSelector {
                        match_labels: Some(StringMap::from([("app".to_owned(), "api".to_owned())])),
                        match_expressions: None,
                    },
                },
                rules: vec![rule],
            },
        );
        policy.metadata.namespace = Some("payments".to_owned());
        policy.metadata.uid = Some("12345".to_owned());
        policy
    }

    fn sample_cidrs() -> Vec<NormalizedCidr> {
        vec![
            NormalizedCidr {
                rendered: "10.0.0.0/24".to_owned(),
                family: crate::extract::IpFamily::V4,
            },
            NormalizedCidr {
                rendered: "2001:db8::/32".to_owned(),
                family: crate::extract::IpFamily::V6,
            },
        ]
    }

    #[test]
    fn ingress_only_rule_renders_only_ingress() {
        let policy = policy_with_rule(RuleSpec {
            directions: Some(vec![Direction::Ingress]),
            pod_selector: Some(LabelSelector {
                match_labels: Some(StringMap::from([(
                    "access-tier".to_owned(),
                    "trusted".to_owned(),
                )])),
                match_expressions: None,
            }),
            namespace_selector: None,
        });

        let rendered = build_managed_network_policy(&policy, &sample_cidrs()).unwrap();
        let spec = rendered.spec.unwrap();

        assert_eq!(spec.policy_types.unwrap(), vec!["Ingress"]);
        assert_eq!(spec.ingress.unwrap().len(), 1);
        assert!(spec.egress.unwrap().is_empty());
    }

    #[test]
    fn omitted_directions_render_both_policy_types() {
        let policy = policy_with_rule(RuleSpec {
            directions: None,
            pod_selector: Some(LabelSelector {
                match_labels: Some(StringMap::from([(
                    "access-tier".to_owned(),
                    "trusted".to_owned(),
                )])),
                match_expressions: None,
            }),
            namespace_selector: None,
        });

        let rendered = build_managed_network_policy(&policy, &sample_cidrs()).unwrap();
        let spec = rendered.spec.unwrap();

        assert_eq!(spec.policy_types.unwrap(), vec!["Egress", "Ingress"]);
        assert_eq!(spec.ingress.unwrap().len(), 1);
        assert_eq!(spec.egress.unwrap().len(), 1);
    }

    #[test]
    fn mixed_family_cidrs_are_rendered_as_ipblocks() {
        let policy = policy_with_rule(RuleSpec {
            directions: Some(vec![Direction::Ingress]),
            pod_selector: None,
            namespace_selector: Some(LabelSelector {
                match_labels: Some(StringMap::from([(
                    "kubernetes.io/metadata.name".to_owned(),
                    "shared-services".to_owned(),
                )])),
                match_expressions: None,
            }),
        });

        let rendered = build_managed_network_policy(&policy, &sample_cidrs()).unwrap();
        let peers = rendered
            .spec
            .unwrap()
            .ingress
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .from
            .unwrap();

        let ipblocks = peers
            .into_iter()
            .filter_map(|peer| peer.ip_block.map(|ip_block| ip_block.cidr))
            .collect::<Vec<_>>();
        assert_eq!(ipblocks, vec!["10.0.0.0/24", "2001:db8::/32"]);
    }

    #[test]
    fn empty_cidr_sets_keep_selector_peer_without_ipblocks() {
        let policy = policy_with_rule(RuleSpec {
            directions: Some(vec![Direction::Egress]),
            pod_selector: Some(LabelSelector {
                match_labels: Some(StringMap::from([(
                    "access-tier".to_owned(),
                    "trusted".to_owned(),
                )])),
                match_expressions: None,
            }),
            namespace_selector: None,
        });

        let rendered = build_managed_network_policy(&policy, &[]).unwrap();
        let peers = rendered
            .spec
            .unwrap()
            .egress
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .to
            .unwrap();

        assert_eq!(peers.len(), 1);
        assert!(peers[0].ip_block.is_none());
        assert!(peers[0].pod_selector.is_some());
    }

    #[test]
    fn stale_cidrs_are_removed_when_re_rendering() {
        let policy = policy_with_rule(RuleSpec {
            directions: Some(vec![Direction::Ingress]),
            pod_selector: Some(LabelSelector {
                match_labels: Some(StringMap::from([(
                    "access-tier".to_owned(),
                    "trusted".to_owned(),
                )])),
                match_expressions: None,
            }),
            namespace_selector: None,
        });

        let first = build_managed_network_policy(
            &policy,
            &[NormalizedCidr {
                rendered: "10.0.0.0/24".to_owned(),
                family: crate::extract::IpFamily::V4,
            }],
        )
        .unwrap();
        let second = build_managed_network_policy(
            &policy,
            &[NormalizedCidr {
                rendered: "192.0.2.0/24".to_owned(),
                family: crate::extract::IpFamily::V4,
            }],
        )
        .unwrap();

        let first_peers = first.spec.unwrap().ingress.unwrap()[0]
            .from
            .clone()
            .unwrap();
        let second_peers = second.spec.unwrap().ingress.unwrap()[0]
            .from
            .clone()
            .unwrap();

        let first_ipblocks = first_peers
            .into_iter()
            .filter_map(|peer| peer.ip_block.map(|ip_block| ip_block.cidr))
            .collect::<Vec<_>>();
        let second_ipblocks = second_peers
            .into_iter()
            .filter_map(|peer| peer.ip_block.map(|ip_block| ip_block.cidr))
            .collect::<Vec<_>>();

        assert_eq!(first_ipblocks, vec!["10.0.0.0/24"]);
        assert_eq!(second_ipblocks, vec!["192.0.2.0/24"]);
    }

    #[test]
    fn rendered_policy_is_marked_as_controller_managed() {
        let policy = policy_with_rule(RuleSpec {
            directions: Some(vec![Direction::Ingress]),
            pod_selector: Some(LabelSelector {
                match_labels: Some(StringMap::from([(
                    "access-tier".to_owned(),
                    "trusted".to_owned(),
                )])),
                match_expressions: None,
            }),
            namespace_selector: None,
        });

        let rendered = build_managed_network_policy(&policy, &sample_cidrs()).unwrap();

        assert_eq!(
            rendered.metadata.labels.unwrap().get(MANAGED_BY_LABEL),
            Some(&MANAGED_BY_VALUE.to_owned())
        );
        assert_eq!(
            rendered
                .metadata
                .annotations
                .unwrap()
                .get(SOURCE_POLICY_ANNOTATION),
            Some(&"office-allowlist".to_owned())
        );
        assert_eq!(rendered.metadata.owner_references.unwrap().len(), 1);
    }
}
