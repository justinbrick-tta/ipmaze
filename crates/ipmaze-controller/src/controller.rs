use crate::api::CIDRPolicy;
use crate::extract::compile_query;
use crate::extract::ExtractionError;
use crate::netpol::{
    apply_managed_network_policy, build_managed_network_policy, is_managed_network_policy_for,
    RenderError,
};
use crate::source::{build_http_client, fetch_json, resolve_final_source, FetchError};
use crate::status::{patch_status_for_outcome, ReconcileOutcome, ReconcileStage};
use crate::validation::{validate_policy, ValidationError};
use chrono::{DateTime, Utc};
use cron::Schedule;
use futures::StreamExt;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, DeleteParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::events::{Event, EventType, Recorder, Reporter};
use kube::runtime::watcher;
use kube::{Client, Resource, ResourceExt};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{error, info};

const CONTROLLER_NAME: &str = "ipmaze-controller";
const CONTROLLER_INSTANCE_ENV: &str = "CONTROLLER_POD_NAME";

#[derive(Clone, Debug)]
pub struct ControllerConfig {
    pub requeue_after: Duration,
}

#[derive(Clone)]
pub struct ControllerContext {
    pub client: Client,
    pub http_client: reqwest::Client,
    pub event_recorder: Recorder,
    pub requeue_after: Duration,
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("managed NetworkPolicy collision for {0}")]
    ManagedPolicyCollision(String),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Extract(#[from] ExtractionError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("unable to calculate next resync: {0}")]
    Scheduling(String),
    #[error(transparent)]
    KubernetesApi(#[from] kube::Error),
}

impl ReconcileError {
    pub fn stage(&self) -> ReconcileStage {
        match self {
            Self::Validation(ValidationError::InvalidJmesPath(_)) => {
                ReconcileStage::JmesPathCompile
            }
            Self::Validation(_) => ReconcileStage::Validation,
            Self::ManagedPolicyCollision(_) => ReconcileStage::ManagedPolicyCollision,
            Self::Fetch(FetchError::PointerRetrieval(_)) => ReconcileStage::PointerRetrieval,
            Self::Fetch(FetchError::PointerExtractionNoMatch)
            | Self::Fetch(FetchError::PointerExtractionEmptyCapture)
            | Self::Fetch(FetchError::PointerResolvedAddress(_)) => {
                ReconcileStage::PointerExtraction
            }
            Self::Fetch(FetchError::FinalRetrieval(_)) => ReconcileStage::Transport,
            Self::Fetch(FetchError::InvalidJson(_)) => ReconcileStage::JsonDecode,
            Self::Extract(ExtractionError::Evaluate(_)) => ReconcileStage::JmesPathEvaluate,
            Self::Extract(ExtractionError::ResultNotArray)
            | Self::Extract(ExtractionError::ResultElementNotString) => ReconcileStage::ResultShape,
            Self::Extract(ExtractionError::InvalidCidr(_)) => ReconcileStage::CidrValidation,
            Self::Extract(_) => ReconcileStage::JmesPathEvaluate,
            Self::Render(_) => ReconcileStage::SelectorTranslation,
            Self::Scheduling(_) => ReconcileStage::Scheduling,
            Self::KubernetesApi(_) => ReconcileStage::KubernetesApi,
        }
    }
}

pub async fn run_controller(config: ControllerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::try_default().await?;
    let http_client = build_http_client()?;
    let event_recorder = Recorder::new(client.clone(), reporter_from_env());
    let context = Arc::new(ControllerContext {
        client: client.clone(),
        http_client,
        event_recorder,
        requeue_after: config.requeue_after,
    });

    Controller::new(
        Api::<CIDRPolicy>::all(client.clone()),
        watcher::Config::default(),
    )
    .owns(
        Api::<NetworkPolicy>::all(client),
        watcher::Config::default(),
    )
    .run(reconcile, error_policy, context)
    .for_each(|result| async move {
        match result {
            Ok((object_ref, action)) => {
                info!(?object_ref, ?action, "reconciled CIDRPolicy");
            }
            Err(error) => {
                error!(error = %error, "controller reconcile failed");
            }
        }
    })
    .await;

    Ok(())
}

pub async fn reconcile(
    policy: Arc<CIDRPolicy>,
    ctx: Arc<ControllerContext>,
) -> Result<Action, ReconcileError> {
    if policy.meta().deletion_timestamp.is_some() {
        cleanup(policy, ctx).await?;
        return Ok(Action::await_change());
    }

    let validated = validate_policy(&policy)?;
    let resolved_source = resolve_final_source(
        &ctx.http_client,
        &validated.source_address,
        validated.pointer_regex.as_ref(),
    )
    .await?;
    let payload = fetch_json(&ctx.http_client, &resolved_source.final_address).await?;
    let (rendered, observed_cidrs) = {
        let query = compile_query(&policy.spec.source.jmes_path).map_err(ValidationError::from)?;
        let cidrs = crate::extract::extract_cidrs(&query, &payload)?;
        let rendered = build_managed_network_policy(&policy, &cidrs)?;
        let observed_cidrs = cidrs
            .iter()
            .map(|cidr| cidr.rendered.clone())
            .collect::<Vec<_>>();

        (rendered, observed_cidrs)
    };
    let namespace = policy.namespace().ok_or(RenderError::MissingNamespace)?;
    let policies_api: Api<CIDRPolicy> = Api::namespaced(ctx.client.clone(), &namespace);
    let netpol_api: Api<NetworkPolicy> = Api::namespaced(ctx.client.clone(), &namespace);
    let existing = get_managed_network_policy(&netpol_api, &policy, &ctx, "Reconcile").await?;
    let outcome = classify_outcome(existing.as_ref(), &rendered, observed_cidrs.clone());

    if matches!(outcome, ReconcileOutcome::Reconciled { .. }) {
        apply_managed_network_policy(&netpol_api, &rendered).await?;
    }

    patch_status_for_outcome(&policies_api, &policy, &outcome).await?;
    publish_outcome_event(&policy, Some(&rendered), &outcome, &ctx).await?;
    Ok(Action::requeue(schedule_requeue_after(
        &validated.resync_schedule,
    )?))
}

pub async fn cleanup(
    policy: Arc<CIDRPolicy>,
    ctx: Arc<ControllerContext>,
) -> Result<(), ReconcileError> {
    let namespace = policy.namespace().ok_or(RenderError::MissingNamespace)?;
    let netpol_api: Api<NetworkPolicy> = Api::namespaced(ctx.client.clone(), &namespace);
    let existing = get_managed_network_policy(&netpol_api, &policy, &ctx, "Cleanup").await?;
    let deleted = delete_managed_network_policy(&netpol_api, &policy, existing.is_some()).await?;
    let note = if deleted {
        format!(
            "Deleted managed NetworkPolicy {} during CIDRPolicy cleanup",
            policy.managed_network_policy_name()
        )
    } else {
        format!(
            "Managed NetworkPolicy {} was already absent during CIDRPolicy cleanup",
            policy.managed_network_policy_name()
        )
    };
    publish_policy_event(
        &ctx,
        &policy,
        EventType::Normal,
        "CleanupManagedPolicy",
        "Cleanup",
        Some(note),
        None,
    )
    .await?;

    Ok(())
}

pub fn error_policy(
    policy: Arc<CIDRPolicy>,
    error: &ReconcileError,
    ctx: Arc<ControllerContext>,
) -> Action {
    let stage = error.stage();
    let message = error.to_string();
    let policy = policy.clone();
    let spawned_ctx = ctx.clone();

    tokio::spawn(async move {
        if let Err(status_error) =
            handle_reconcile_failure(policy.clone(), stage, message, spawned_ctx).await
        {
            error!(error = %status_error, policy = %policy.name_any(), "failed to publish reconcile failure state");
        }
    });

    Action::requeue(ctx.requeue_after)
}

fn schedule_requeue_after(schedule: &Schedule) -> Result<Duration, ReconcileError> {
    next_resync_after(schedule, Utc::now()).map_err(ReconcileError::Scheduling)
}

pub fn next_resync_after(schedule: &Schedule, now: DateTime<Utc>) -> Result<Duration, String> {
    let next = schedule
        .after(&now)
        .next()
        .ok_or_else(|| "schedule did not produce a future execution".to_owned())?;
    let delay = next
        .signed_duration_since(now)
        .to_std()
        .map_err(|error| error.to_string())?;

    if delay.is_zero() {
        Ok(Duration::from_secs(1))
    } else {
        Ok(delay)
    }
}

pub async fn handle_reconcile_failure(
    policy: Arc<CIDRPolicy>,
    stage: ReconcileStage,
    message: String,
    ctx: Arc<ControllerContext>,
) -> Result<(), kube::Error> {
    let Some(namespace) = policy.namespace() else {
        return Ok(());
    };

    let policies_api: Api<CIDRPolicy> = Api::namespaced(ctx.client.clone(), &namespace);
    let outcome = ReconcileOutcome::Failed {
        stage: stage.clone(),
        message: message.clone(),
    };

    patch_status_for_outcome(&policies_api, &policy, &outcome).await?;
    publish_policy_event(
        &ctx,
        &policy,
        EventType::Warning,
        "ReconcileFailed",
        "Reconcile",
        Some(format!("{}: {}", stage.as_str(), message)),
        None,
    )
    .await?;

    Ok(())
}

async fn get_managed_network_policy(
    api: &Api<NetworkPolicy>,
    policy: &CIDRPolicy,
    ctx: &ControllerContext,
    action: &str,
) -> Result<Option<NetworkPolicy>, ReconcileError> {
    match api.get_opt(&policy.managed_network_policy_name()).await? {
        Some(network_policy) => {
            if is_managed_network_policy_for(policy, &network_policy) {
                Ok(Some(network_policy))
            } else {
                report_managed_policy_collision(ctx, policy, &network_policy, action).await?;
                Err(ReconcileError::ManagedPolicyCollision(
                    policy.managed_network_policy_name(),
                ))
            }
        }
        None => Ok(None),
    }
}

pub async fn delete_managed_network_policy(
    api: &Api<NetworkPolicy>,
    policy: &CIDRPolicy,
    exists: bool,
) -> Result<bool, ReconcileError> {
    if !exists {
        return Ok(false);
    }

    match api
        .delete(
            &policy.managed_network_policy_name(),
            &DeleteParams::default(),
        )
        .await
    {
        Ok(_) => Ok(true),
        Err(kube::Error::Api(error)) if error.code == 404 => Ok(false),
        Err(error) => Err(error.into()),
    }
}

async fn report_managed_policy_collision(
    ctx: &ControllerContext,
    policy: &CIDRPolicy,
    network_policy: &NetworkPolicy,
    action: &str,
) -> Result<(), kube::Error> {
    error!(
        policy = %policy.name_any(),
        namespace = %policy.namespace().unwrap_or_default(),
        network_policy = %policy.managed_network_policy_name(),
        action,
        "managed NetworkPolicy name collision detected"
    );

    publish_policy_event(
        ctx,
        policy,
        EventType::Warning,
        "ManagedPolicyCollision",
        action,
        Some(format!(
            "Refusing to {action_lower} NetworkPolicy {} because an existing resource with that name is not owned by this CIDRPolicy",
            policy.managed_network_policy_name(),
            action_lower = action.to_lowercase(),
        )),
        Some(network_policy),
    )
    .await
}

fn classify_outcome(
    existing: Option<&NetworkPolicy>,
    desired: &NetworkPolicy,
    observed_cidrs: Vec<String>,
) -> ReconcileOutcome {
    match existing {
        Some(current) if managed_policy_matches(current, desired) => {
            ReconcileOutcome::NoChange { observed_cidrs }
        }
        _ => ReconcileOutcome::Reconciled { observed_cidrs },
    }
}

fn managed_policy_matches(current: &NetworkPolicy, desired: &NetworkPolicy) -> bool {
    current.spec == desired.spec
        && current.metadata.labels == desired.metadata.labels
        && current.metadata.annotations == desired.metadata.annotations
        && current.metadata.owner_references == desired.metadata.owner_references
}

async fn publish_outcome_event(
    policy: &CIDRPolicy,
    managed_policy: Option<&NetworkPolicy>,
    outcome: &ReconcileOutcome,
    ctx: &ControllerContext,
) -> Result<(), kube::Error> {
    match outcome {
        ReconcileOutcome::Reconciled { observed_cidrs } => {
            publish_policy_event(
                ctx,
                policy,
                EventType::Normal,
                "Reconciled",
                "Reconcile",
                Some(format!(
                    "Reconciled managed NetworkPolicy with {} CIDR entries",
                    observed_cidrs.len()
                )),
                managed_policy,
            )
            .await
        }
        ReconcileOutcome::NoChange { .. } | ReconcileOutcome::Failed { .. } => Ok(()),
    }
}

async fn publish_policy_event(
    ctx: &ControllerContext,
    policy: &CIDRPolicy,
    event_type: EventType,
    reason: &str,
    action: &str,
    note: Option<String>,
    secondary: Option<&NetworkPolicy>,
) -> Result<(), kube::Error> {
    let event = Event {
        type_: event_type,
        reason: reason.to_owned(),
        note,
        action: action.to_owned(),
        secondary: secondary.map(|network_policy| network_policy.object_ref(&())),
    };

    ctx.event_recorder
        .publish(&event, &policy.object_ref(&()))
        .await
}

fn reporter_from_env() -> Reporter {
    Reporter {
        controller: CONTROLLER_NAME.to_owned(),
        instance: std::env::var(CONTROLLER_INSTANCE_ENV).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CIDRPolicySpec, Direction, LabelSelector, RuleSpec, SourceSpec, TargetSpec};
    use crate::extract::{IpFamily, NormalizedCidr};
    use crate::validation::validate_resync_schedule;
    use chrono::TimeZone;

    fn sample_policy() -> CIDRPolicy {
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
                    directions: Some(vec![Direction::Ingress]),
                    pod_selector: Some(LabelSelector::default()),
                    namespace_selector: None,
                }],
            },
        );
        policy.metadata.namespace = Some("payments".to_owned());
        policy.metadata.uid = Some("12345".to_owned());
        policy
    }

    fn sample_rendered_policy(cidr: &str) -> NetworkPolicy {
        build_managed_network_policy(
            &sample_policy(),
            &[NormalizedCidr {
                rendered: cidr.to_owned(),
                family: IpFamily::V4,
            }],
        )
        .unwrap()
    }

    #[test]
    fn classify_outcome_detects_no_change() {
        let desired = sample_rendered_policy("10.0.0.0/24");
        let outcome = classify_outcome(Some(&desired), &desired, vec!["10.0.0.0/24".to_owned()]);

        assert_eq!(
            outcome,
            ReconcileOutcome::NoChange {
                observed_cidrs: vec!["10.0.0.0/24".to_owned()]
            }
        );
    }

    #[test]
    fn classify_outcome_detects_material_spec_change() {
        let current = sample_rendered_policy("10.0.0.0/24");
        let desired = sample_rendered_policy("192.0.2.0/24");
        let outcome = classify_outcome(Some(&current), &desired, vec!["192.0.2.0/24".to_owned()]);

        assert_eq!(
            outcome,
            ReconcileOutcome::Reconciled {
                observed_cidrs: vec!["192.0.2.0/24".to_owned()]
            }
        );
    }

    #[test]
    fn reconcile_error_maps_result_shape_failures() {
        let error = ReconcileError::Extract(ExtractionError::ResultNotArray);
        assert_eq!(error.stage(), ReconcileStage::ResultShape);
    }

    #[test]
    fn reconcile_error_maps_pointer_extraction_failures() {
        let error = ReconcileError::Fetch(FetchError::PointerExtractionNoMatch);
        assert_eq!(error.stage(), ReconcileStage::PointerExtraction);
    }

    #[test]
    fn reconcile_error_maps_managed_policy_collisions() {
        let error = ReconcileError::ManagedPolicyCollision("office-allowlist-managed".to_owned());
        assert_eq!(error.stage(), ReconcileStage::ManagedPolicyCollision);
    }

    #[test]
    fn next_resync_after_returns_non_zero_delay_for_minutely_schedule() {
        let schedule = validate_resync_schedule(Some("* * * * *")).unwrap();
        let now = Utc
            .with_ymd_and_hms(2026, 5, 22, 12, 34, 30)
            .single()
            .unwrap();

        let delay = next_resync_after(&schedule, now).unwrap();

        assert!(delay > Duration::ZERO);
        assert!(delay < Duration::from_secs(60));
    }
}
