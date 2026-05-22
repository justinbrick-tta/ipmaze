pub mod api;
pub mod controller;
pub mod extract;
pub mod netpol;
pub mod source;
pub mod status;
pub mod validation;

pub use api::{
    CIDRPolicy, CIDRPolicySpec, CIDRPolicyStatus, Direction, LabelSelector, LabelSelectorOperator,
    LabelSelectorRequirement, RuleSpec, SourceSpec, StringMap, TargetSpec,
};
pub use controller::{run_controller, ControllerConfig, ControllerContext, ReconcileError};
pub use extract::{
    compile_query, extract_cidrs, ExtractionError, IpFamily, NormalizedCidr, QueryError,
};
pub use netpol::{
    apply_managed_network_policy, build_managed_network_policy, effective_directions,
    render_peer_selector, render_subject_selector, RenderError,
};
pub use source::{
    build_http_client, fetch_json, normalize_source_address, FetchError, NormalizedRemoteAddress,
    RemoteAddress, SourceAddressError,
};
pub use status::{patch_status_for_outcome, status_for_outcome, ReconcileOutcome, ReconcileStage};
pub use validation::{validate_policy, validate_spec, ValidatedPolicy, ValidationError};

use kube::core::CustomResourceExt;

pub fn generate_crd_yaml() -> Result<String, serde_yaml::Error> {
    serde_yaml::to_string(&CIDRPolicy::crd())
}
