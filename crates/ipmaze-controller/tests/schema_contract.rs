use ipmaze_controller::{
    generate_crd_yaml, CIDRPolicy, CIDRPolicySpec, CIDRPolicyStatus, RuleSpec, SourceSpec,
    TargetSpec,
};
use kube::core::CustomResourceExt;
use schemars::schema_for;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[test]
fn generated_crd_contains_expected_gvk() {
    let crd = CIDRPolicy::crd();
    assert_eq!(crd.spec.group, "ipmaze.k8s.justin.directory");
    assert_eq!(crd.spec.names.kind, "CIDRPolicy");
    assert_eq!(crd.spec.names.plural, "cidrpolicies");
    assert_eq!(crd.spec.versions[0].name, "v1alpha1");
}

#[test]
fn generated_crd_yaml_is_serializable() {
    let yaml = generate_crd_yaml().unwrap();
    assert!(yaml.contains("CustomResourceDefinition"));
    assert!(yaml.contains("cidrpolicies.ipmaze.k8s.justin.directory"));
}

#[test]
fn generated_schema_matches_repository_contract() {
    let repo_schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/ipmaze-controller/policy.schema.json");
    let repo_schema: Value =
        serde_json::from_str(&fs::read_to_string(repo_schema_path).unwrap()).unwrap();

    let crd_schema = serde_json::to_value(CIDRPolicy::crd()).unwrap();
    let root_schema = crd_schema
        .pointer("/spec/versions/0/schema/openAPIV3Schema")
        .expect("generated CRD must contain an OpenAPI schema");
    let spec_schema = serde_json::to_value(schema_for!(CIDRPolicySpec)).unwrap();
    let source_schema = serde_json::to_value(schema_for!(SourceSpec)).unwrap();
    let target_schema = serde_json::to_value(schema_for!(TargetSpec)).unwrap();
    let rule_schema = serde_json::to_value(schema_for!(RuleSpec)).unwrap();
    let status_schema = serde_json::to_value(schema_for!(CIDRPolicyStatus)).unwrap();

    assert_eq!(
        property_names(root_schema),
        vec!["spec".to_owned(), "status".to_owned()]
    );
    assert_eq!(required_names(root_schema), vec!["spec".to_owned()]);
    assert_eq!(
        required_names(&spec_schema),
        required_names(repo_schema.pointer("/$defs/spec").unwrap())
    );
    assert_eq!(
        required_names(&source_schema),
        required_names(repo_schema.pointer("/$defs/source").unwrap())
    );
    assert_eq!(
        property_names(&source_schema),
        property_names(repo_schema.pointer("/$defs/source").unwrap())
    );
    assert_eq!(
        required_names(&target_schema),
        required_names(repo_schema.pointer("/$defs/target").unwrap())
    );
    assert_eq!(
        property_names(&target_schema),
        property_names(repo_schema.pointer("/$defs/target").unwrap())
    );
    assert_eq!(
        property_names(&rule_schema),
        property_names(repo_schema.pointer("/$defs/rule").unwrap())
    );
    assert_eq!(
        property_names(&status_schema),
        property_names(repo_schema.pointer("/$defs/status").unwrap())
    );
}

fn required_names(schema: &Value) -> Vec<String> {
    let mut required = schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    required.sort();
    required
}

fn property_names(schema: &Value) -> Vec<String> {
    let mut names = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    names.sort();
    names
}
