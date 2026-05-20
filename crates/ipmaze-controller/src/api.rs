use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type StringMap = BTreeMap<String, String>;

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "ipmaze.k8s.justin.directory",
    version = "v1alpha1",
    kind = "CIDRPolicy",
    plural = "cidrpolicies",
    namespaced,
    status = "CIDRPolicyStatus"
)]
pub struct CIDRPolicySpec {
    pub source: SourceSpec,
    pub target: TargetSpec,
    pub rules: Vec<RuleSpec>,
}

impl CIDRPolicy {
    pub fn managed_network_policy_name(&self) -> String {
        let name = self.metadata.name.as_deref().unwrap_or("cidrpolicy");
        format!("{name}-managed")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CIDRPolicyStatus {
    pub last_successful_resolution_time: Option<String>,
    #[serde(default)]
    pub last_observed_cidrs: Vec<String>,
    pub last_reconciliation_error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpec {
    pub address: String,
    pub jmes_path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TargetSpec {
    pub pod_selector: LabelSelector,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directions: Option<Vec<Direction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod_selector: Option<LabelSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<LabelSelector>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Ingress,
    Egress,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_labels: Option<StringMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_expressions: Option<Vec<LabelSelectorRequirement>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: LabelSelectorOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub enum LabelSelectorOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}
