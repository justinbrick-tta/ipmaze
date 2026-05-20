use http::{Request, Response, StatusCode};
use ipmaze_controller::api::{
    CIDRPolicy, CIDRPolicySpec, CIDRPolicyStatus, Direction, LabelSelector, RuleSpec, SourceSpec,
    StringMap, TargetSpec,
};
use ipmaze_controller::controller::{handle_reconcile_failure, reconcile, ControllerContext};
use ipmaze_controller::extract::{IpFamily, NormalizedCidr};
use ipmaze_controller::netpol::build_managed_network_policy;
use ipmaze_controller::build_http_client;
use k8s_openapi::api::events::v1::Event as K8sEvent;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Status, Time};
use kube::client::Body;
use kube::runtime::events::{Recorder, Reporter};
use kube::{Client, ResourceExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::service_fn;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct FakeKubeState {
    policies: BTreeMap<(String, String), CIDRPolicy>,
    network_policies: BTreeMap<(String, String), NetworkPolicy>,
    events: BTreeMap<String, K8sEvent>,
    netpol_patch_count: usize,
    netpol_delete_count: usize,
    status_patch_count: usize,
}

#[tokio::test]
async fn reconcile_creates_policy_updates_status_and_emits_event() {
    let remote = json_server(json!({ "prefixes": ["10.0.0.0/24"] })).await;
    let policy = sample_policy(&format!("{}/allowlist.json", remote.uri()));
    let state = Arc::new(Mutex::new(FakeKubeState::default()));
    state
        .lock()
        .unwrap()
        .policies
        .insert(policy_key(&policy), policy.clone());
    let ctx = test_context(state.clone());

    reconcile(Arc::new(policy.clone()), ctx).await.unwrap();

    let state = state.lock().unwrap();
    let managed = state
        .network_policies
        .get(&(policy.namespace().unwrap(), policy.managed_network_policy_name()))
        .unwrap();
    let cidrs = rendered_ipblocks(managed);
    assert_eq!(cidrs, vec!["10.0.0.0/24"]);
    assert_eq!(state.netpol_patch_count, 1);

    let status = state
        .policies
        .get(&policy_key(&policy))
        .and_then(|policy| policy.status.clone())
        .unwrap();
    assert_eq!(status.last_observed_cidrs, vec!["10.0.0.0/24"]);
    assert!(status.last_successful_resolution_time.is_some());
    assert_eq!(status.last_reconciliation_error, None);

    let reasons = event_reasons(&state);
    assert_eq!(reasons, vec!["Reconciled"]);
}

#[tokio::test]
async fn reconcile_updates_existing_managed_policy() {
    let remote = json_server(json!({ "prefixes": ["192.0.2.0/24"] })).await;
    let policy = sample_policy(&format!("{}/allowlist.json", remote.uri()));
    let old_policy = build_managed_network_policy(
        &policy,
        &[NormalizedCidr {
            rendered: "10.0.0.0/24".to_owned(),
            family: IpFamily::V4,
        }],
    )
    .unwrap();
    let state = Arc::new(Mutex::new(FakeKubeState::default()));
    {
        let mut state = state.lock().unwrap();
        state.policies.insert(policy_key(&policy), policy.clone());
        state.network_policies.insert(
            (policy.namespace().unwrap(), policy.managed_network_policy_name()),
            old_policy,
        );
    }
    let ctx = test_context(state.clone());

    reconcile(Arc::new(policy.clone()), ctx).await.unwrap();

    let state = state.lock().unwrap();
    let managed = state
        .network_policies
        .get(&(policy.namespace().unwrap(), policy.managed_network_policy_name()))
        .unwrap();
    assert_eq!(rendered_ipblocks(managed), vec!["192.0.2.0/24"]);
    assert_eq!(state.netpol_patch_count, 1);
}

#[tokio::test]
async fn reconcile_deletes_managed_policy_when_resource_is_terminating() {
    let remote = json_server(json!({ "prefixes": ["10.0.0.0/24"] })).await;
    let mut policy = sample_policy(&format!("{}/allowlist.json", remote.uri()));
    policy.metadata.deletion_timestamp = Some(Time(chrono::Utc::now()));
    let managed_policy = build_managed_network_policy(
        &policy,
        &[NormalizedCidr {
            rendered: "10.0.0.0/24".to_owned(),
            family: IpFamily::V4,
        }],
    )
    .unwrap();
    let state = Arc::new(Mutex::new(FakeKubeState::default()));
    {
        let mut state = state.lock().unwrap();
        state.policies.insert(policy_key(&policy), policy.clone());
        state.network_policies.insert(
            (policy.namespace().unwrap(), policy.managed_network_policy_name()),
            managed_policy,
        );
    }
    let ctx = test_context(state.clone());

    reconcile(Arc::new(policy.clone()), ctx).await.unwrap();

    let state = state.lock().unwrap();
    assert!(!state
        .network_policies
        .contains_key(&(policy.namespace().unwrap(), policy.managed_network_policy_name())));
    assert_eq!(state.netpol_delete_count, 1);
    assert_eq!(event_reasons(&state), vec!["CleanupManagedPolicy"]);
}

#[tokio::test]
async fn failed_reconcile_preserves_last_good_policy_and_records_warning() {
    let remote = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/allowlist.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "prefixes": ["10.0.0.0/24"]
        })))
        .mount(&remote)
        .await;

    let policy = sample_policy(&format!("{}/allowlist.json", remote.uri()));
    let state = Arc::new(Mutex::new(FakeKubeState::default()));
    {
        let mut state = state.lock().unwrap();
        state.policies.insert(policy_key(&policy), policy.clone());
    }
    let ctx = test_context(state.clone());
    reconcile(Arc::new(policy.clone()), ctx.clone()).await.unwrap();

    let before_failure = state
        .lock()
        .unwrap()
        .network_policies
        .get(&(policy.namespace().unwrap(), policy.managed_network_policy_name()))
        .cloned()
        .unwrap();

    remote.reset().await;
    Mock::given(method("GET"))
        .and(path("/allowlist.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&remote)
        .await;

    let err = reconcile(Arc::new(policy.clone()), ctx.clone())
        .await
        .unwrap_err();
    handle_reconcile_failure(
        Arc::new(policy.clone()),
        err.stage(),
        err.to_string(),
        ctx,
    )
    .await
    .unwrap();

    let state = state.lock().unwrap();
    let after_failure = state
        .network_policies
        .get(&(policy.namespace().unwrap(), policy.managed_network_policy_name()))
        .unwrap();
    assert_eq!(after_failure.spec, before_failure.spec);

    let status = state
        .policies
        .get(&policy_key(&policy))
        .and_then(|policy| policy.status.clone())
        .unwrap();
    assert_eq!(status.last_observed_cidrs, vec!["10.0.0.0/24"]);
    assert!(status.last_successful_resolution_time.is_some());
    assert!(status
        .last_reconciliation_error
        .unwrap()
        .contains("transport:"));

    let reasons = event_reasons(&state);
    assert_eq!(reasons, vec!["ReconcileFailed", "Reconciled"]);
}

#[tokio::test]
async fn no_change_reconcile_avoids_network_policy_write() {
    let remote = json_server(json!({ "prefixes": ["10.0.0.0/24"] })).await;
    let policy = sample_policy(&format!("{}/allowlist.json", remote.uri()));
    let managed_policy = build_managed_network_policy(
        &policy,
        &[NormalizedCidr {
            rendered: "10.0.0.0/24".to_owned(),
            family: IpFamily::V4,
        }],
    )
    .unwrap();
    let state = Arc::new(Mutex::new(FakeKubeState::default()));
    {
        let mut state = state.lock().unwrap();
        state.policies.insert(policy_key(&policy), policy.clone());
        state.network_policies.insert(
            (policy.namespace().unwrap(), policy.managed_network_policy_name()),
            managed_policy,
        );
        state.netpol_patch_count = 0;
    }
    let ctx = test_context(state.clone());

    reconcile(Arc::new(policy.clone()), ctx).await.unwrap();

    let state = state.lock().unwrap();
    assert_eq!(state.netpol_patch_count, 0);
    assert_eq!(state.status_patch_count, 1);
    assert!(state.events.is_empty());
}

fn sample_policy(address: &str) -> CIDRPolicy {
    let mut policy = CIDRPolicy::new(
        "office-allowlist",
        CIDRPolicySpec {
            source: SourceSpec {
                address: address.to_owned(),
                jmes_path: "prefixes".to_owned(),
            },
            target: TargetSpec {
                pod_selector: LabelSelector {
                    match_labels: Some(StringMap::from([(
                        "app".to_owned(),
                        "api".to_owned(),
                    )])),
                    match_expressions: None,
                },
            },
            rules: vec![RuleSpec {
                directions: Some(vec![Direction::Ingress]),
                pod_selector: Some(LabelSelector {
                    match_labels: Some(StringMap::from([(
                        "access-tier".to_owned(),
                        "trusted".to_owned(),
                    )])),
                    match_expressions: None,
                }),
                namespace_selector: None,
            }],
        },
    );
    policy.metadata.namespace = Some("payments".to_owned());
    policy.metadata.uid = Some("12345".to_owned());
    policy
}

fn test_context(state: Arc<Mutex<FakeKubeState>>) -> Arc<ControllerContext> {
    let client = fake_client(state);
    Arc::new(ControllerContext {
        http_client: build_http_client().unwrap(),
        event_recorder: Recorder::new(
            client.clone(),
            Reporter {
                controller: "ipmaze-controller".to_owned(),
                instance: Some("test-instance".to_owned()),
            },
        ),
        client,
        requeue_after: Duration::from_secs(60),
    })
}

fn fake_client(state: Arc<Mutex<FakeKubeState>>) -> Client {
    let service = service_fn(move |request: Request<Body>| {
        let state = state.clone();
        async move { Ok::<_, Infallible>(handle_request(state, request).await) }
    });

    Client::new(service, "default")
}

async fn handle_request(
    state: Arc<Mutex<FakeKubeState>>,
    request: Request<Body>,
) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let (_, body) = request.into_parts();
    let body = body.collect_bytes().await.unwrap();
    let segments = path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    match (method.as_str(), segments.as_slice()) {
        (
            "GET",
            ["apis", "networking.k8s.io", "v1", "namespaces", namespace, "networkpolicies", name],
        ) => {
            let state = state.lock().unwrap();
            match state
                .network_policies
                .get(&(namespace.to_string(), name.to_string()))
            {
                Some(network_policy) => json_response(StatusCode::OK, network_policy),
                None => not_found_response("NetworkPolicy", name),
            }
        }
        (
            "PATCH",
            ["apis", "networking.k8s.io", "v1", "namespaces", namespace, "networkpolicies", name],
        ) => {
            let network_policy: NetworkPolicy = deserialize_body(body.as_ref());
            let mut state = state.lock().unwrap();
            state.netpol_patch_count += 1;
            state.network_policies.insert(
                (namespace.to_string(), name.to_string()),
                network_policy.clone(),
            );
            json_response(StatusCode::OK, &network_policy)
        }
        (
            "DELETE",
            ["apis", "networking.k8s.io", "v1", "namespaces", namespace, "networkpolicies", name],
        ) => {
            let mut state = state.lock().unwrap();
            state.netpol_delete_count += 1;
            state
                .network_policies
                .remove(&(namespace.to_string(), name.to_string()));
            json_response(StatusCode::OK, &success_status("Success", "deleted"))
        }
        (
            "GET",
            [
                "apis",
                "ipmaze.k8s.justin.directory",
                "v1alpha1",
                "namespaces",
                namespace,
                "cidrpolicies",
                name,
            ],
        ) => {
            let state = state.lock().unwrap();
            match state.policies.get(&(namespace.to_string(), name.to_string())) {
                Some(policy) => json_response(StatusCode::OK, policy),
                None => not_found_response("CIDRPolicy", name),
            }
        }
        (
            "PATCH",
            [
                "apis",
                "ipmaze.k8s.justin.directory",
                "v1alpha1",
                "namespaces",
                namespace,
                "cidrpolicies",
                name,
                "status",
            ],
        ) => {
            let patch: Value = deserialize_body(body.as_ref());
            let status_value = patch.get("status").cloned().unwrap_or(Value::Null);
            let status = serde_json::from_value::<CIDRPolicyStatus>(status_value).unwrap();

            let mut state = state.lock().unwrap();
            state.status_patch_count += 1;
            let policy = state
                .policies
                .get_mut(&(namespace.to_string(), name.to_string()))
                .unwrap();
            policy.status = Some(status);
            json_response(StatusCode::OK, policy)
        }
        (
            "POST",
            ["apis", "events.k8s.io", "v1", "namespaces", namespace, "events"],
        ) => {
            let event: K8sEvent = deserialize_body(body.as_ref());
            let name = event.metadata.name.clone().unwrap();
            let mut state = state.lock().unwrap();
            state.events.insert(format!("{namespace}/{name}"), event.clone());
            json_response(StatusCode::CREATED, &event)
        }
        (
            "PATCH",
            ["apis", "events.k8s.io", "v1", "namespaces", namespace, "events", name],
        ) => {
            let event: K8sEvent = deserialize_body(body.as_ref());
            let mut state = state.lock().unwrap();
            state.events.insert(format!("{namespace}/{name}"), event.clone());
            json_response(StatusCode::OK, &event)
        }
        _ => json_response(
            StatusCode::NOT_FOUND,
            &success_status("Failure", &format!("unhandled request: {method} {path}")),
        ),
    }
}

fn deserialize_body<T: DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).or_else(|_| serde_yaml::from_slice(body)).unwrap()
}

fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response<Body> {
    let payload = serde_json::to_vec(value).unwrap();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .unwrap()
}

fn not_found_response(kind: &str, name: &str) -> Response<Body> {
    json_response(
        StatusCode::NOT_FOUND,
        &Status {
            status: Some("Failure".to_owned()),
            code: Some(404),
            reason: Some("NotFound".to_owned()),
            message: Some(format!("{kind} {name} not found")),
            ..Status::default()
        },
    )
}

fn success_status(status: &str, message: &str) -> Status {
    Status {
        status: Some(status.to_owned()),
        code: Some(200),
        message: Some(message.to_owned()),
        ..Status::default()
    }
}

fn policy_key(policy: &CIDRPolicy) -> (String, String) {
    (policy.namespace().unwrap(), policy.name_any())
}

fn rendered_ipblocks(network_policy: &NetworkPolicy) -> Vec<String> {
    network_policy
        .spec
        .as_ref()
        .and_then(|spec| spec.ingress.as_ref())
        .into_iter()
        .flatten()
        .flat_map(|rule| rule.from.clone().unwrap_or_default())
        .filter_map(|peer| peer.ip_block.map(|ip_block| ip_block.cidr))
        .collect()
}

fn event_reasons(state: &FakeKubeState) -> Vec<&str> {
    let mut reasons = state
        .events
        .values()
        .filter_map(|event| event.reason.as_deref())
        .collect::<Vec<_>>();
    reasons.sort();
    reasons
}

async fn json_server(payload: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/allowlist.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(&server)
        .await;
    server
}