use crate::api::{CIDRPolicy, CIDRPolicyStatus};
use chrono::Utc;
use kube::api::{Api, Patch, PatchParams};
use kube::ResourceExt;
use serde_json::json;

const STATUS_FIELD_MANAGER: &str = "ipmaze-controller-status";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileStage {
    Validation,
    PointerRetrieval,
    PointerExtraction,
    Transport,
    JsonDecode,
    JmesPathCompile,
    JmesPathEvaluate,
    ResultShape,
    CidrValidation,
    SelectorTranslation,
    Scheduling,
    KubernetesApi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    Reconciled {
        observed_cidrs: Vec<String>,
    },
    NoChange {
        observed_cidrs: Vec<String>,
    },
    Failed {
        stage: ReconcileStage,
        message: String,
    },
}

impl ReconcileStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::PointerRetrieval => "pointer-retrieval",
            Self::PointerExtraction => "pointer-extraction",
            Self::Transport => "transport",
            Self::JsonDecode => "json-decode",
            Self::JmesPathCompile => "jmespath-compile",
            Self::JmesPathEvaluate => "jmespath-evaluate",
            Self::ResultShape => "result-shape",
            Self::CidrValidation => "cidr-validation",
            Self::SelectorTranslation => "selector-translation",
            Self::Scheduling => "scheduling",
            Self::KubernetesApi => "kubernetes-api",
        }
    }
}

pub fn status_for_outcome(policy: &CIDRPolicy, outcome: &ReconcileOutcome) -> CIDRPolicyStatus {
    let current = policy.status.clone().unwrap_or_default();

    match outcome {
        ReconcileOutcome::Reconciled { observed_cidrs }
        | ReconcileOutcome::NoChange { observed_cidrs } => CIDRPolicyStatus {
            last_successful_resolution_time: Some(Utc::now().to_rfc3339()),
            last_observed_cidrs: observed_cidrs.clone(),
            last_reconciliation_error: None,
        },
        ReconcileOutcome::Failed { stage, message } => CIDRPolicyStatus {
            last_successful_resolution_time: current.last_successful_resolution_time,
            last_observed_cidrs: current.last_observed_cidrs,
            last_reconciliation_error: Some(format!("{}: {}", stage.as_str(), message)),
        },
    }
}

pub async fn patch_status_for_outcome(
    api: &Api<CIDRPolicy>,
    policy: &CIDRPolicy,
    outcome: &ReconcileOutcome,
) -> Result<(), kube::Error> {
    let name = policy.name_any();
    let status_source = match outcome {
        ReconcileOutcome::Failed { .. } => api.get(&name).await?,
        ReconcileOutcome::Reconciled { .. } | ReconcileOutcome::NoChange { .. } => policy.clone(),
    };
    let status = status_for_outcome(&status_source, outcome);
    let patch = json!({
        "status": {
            "lastSuccessfulResolutionTime": status.last_successful_resolution_time,
            "lastObservedCidrs": status.last_observed_cidrs,
            "lastReconciliationError": status.last_reconciliation_error,
        }
    });

    api.patch_status(
        &name,
        &PatchParams::apply(STATUS_FIELD_MANAGER),
        &Patch::Merge(&patch),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CIDRPolicySpec, SourceSpec, TargetSpec};
    use crate::api::{LabelSelector, RuleSpec};

    fn policy_with_status(status: Option<CIDRPolicyStatus>) -> CIDRPolicy {
        let mut policy = CIDRPolicy::new(
            "office-allowlist",
            CIDRPolicySpec {
                source: SourceSpec {
                    address: "example.invalid".to_owned(),
                    pointer: None,
                    jmes_path: "prefixes".to_owned(),
                },
                resync_schedule: None,
                target: TargetSpec {
                    pod_selector: LabelSelector::default(),
                },
                rules: vec![RuleSpec {
                    directions: None,
                    pod_selector: Some(LabelSelector::default()),
                    namespace_selector: None,
                }],
            },
        );
        policy.status = status;
        policy
    }

    #[test]
    fn successful_outcome_clears_error_and_updates_observed_cidrs() {
        let policy = policy_with_status(Some(CIDRPolicyStatus {
            last_successful_resolution_time: Some("2026-01-01T00:00:00+00:00".to_owned()),
            last_observed_cidrs: vec!["10.0.0.0/24".to_owned()],
            last_reconciliation_error: Some("transport: timeout".to_owned()),
        }));

        let status = status_for_outcome(
            &policy,
            &ReconcileOutcome::Reconciled {
                observed_cidrs: vec!["192.0.2.0/24".to_owned()],
            },
        );

        assert_eq!(status.last_observed_cidrs, vec!["192.0.2.0/24"]);
        assert!(status.last_successful_resolution_time.is_some());
        assert_eq!(status.last_reconciliation_error, None);
    }

    #[test]
    fn failed_outcome_preserves_last_good_status() {
        let policy = policy_with_status(Some(CIDRPolicyStatus {
            last_successful_resolution_time: Some("2026-01-01T00:00:00+00:00".to_owned()),
            last_observed_cidrs: vec!["10.0.0.0/24".to_owned()],
            last_reconciliation_error: None,
        }));

        let status = status_for_outcome(
            &policy,
            &ReconcileOutcome::Failed {
                stage: ReconcileStage::Transport,
                message: "request timed out".to_owned(),
            },
        );

        assert_eq!(
            status.last_successful_resolution_time,
            Some("2026-01-01T00:00:00+00:00".to_owned())
        );
        assert_eq!(status.last_observed_cidrs, vec!["10.0.0.0/24"]);
        assert_eq!(
            status.last_reconciliation_error,
            Some("transport: request timed out".to_owned())
        );
    }
}
