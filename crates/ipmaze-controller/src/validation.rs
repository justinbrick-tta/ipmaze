use crate::api::{
    CIDRPolicy, CIDRPolicySpec, Direction, LabelSelector, LabelSelectorOperator,
    LabelSelectorRequirement, RuleSpec,
};
use crate::extract::{compile_query, QueryError};
use crate::source::{normalize_source_address, NormalizedRemoteAddress, SourceAddressError};
use cron::Schedule;
use regex::Regex;
use std::str::FromStr;
use thiserror::Error;

pub const DEFAULT_RESYNC_SCHEDULE: &str = "0 0 * * *";

#[derive(Debug)]
pub struct ValidatedPolicy {
    pub source_address: NormalizedRemoteAddress,
    pub pointer_regex: Option<Regex>,
    pub resync_schedule: Schedule,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error(transparent)]
    InvalidSourceAddress(#[from] SourceAddressError),
    #[error(transparent)]
    InvalidJmesPath(#[from] QueryError),
    #[error("pointer regex is invalid: {0}")]
    InvalidPointerRegex(regex::Error),
    #[error("resync schedule must use standard 5-field cron syntax")]
    InvalidResyncScheduleShape,
    #[error("resync schedule is invalid: {0}")]
    InvalidResyncSchedule(String),
    #[error("rules must not be empty")]
    EmptyRules,
    #[error("each rule must declare at least one selector")]
    RuleMissingSelector,
    #[error("direction list, when present, must not be empty")]
    EmptyDirections,
    #[error("selector requirement `{0}` is missing a key")]
    MissingSelectorKey(String),
    #[error("selector requirement `{0}` with operator `{1}` requires non-empty values")]
    SelectorValuesRequired(String, &'static str),
    #[error("selector requirement `{0}` with operator `{1}` must not include values")]
    SelectorValuesForbidden(String, &'static str),
}

pub fn validate_policy(policy: &CIDRPolicy) -> Result<ValidatedPolicy, ValidationError> {
    validate_spec(&policy.spec)
}

pub fn validate_spec(spec: &CIDRPolicySpec) -> Result<ValidatedPolicy, ValidationError> {
    if spec.rules.is_empty() {
        return Err(ValidationError::EmptyRules);
    }

    let source_address = normalize_source_address(&spec.source.address)?;
    let pointer_regex = validate_pointer_regex(
        spec.source
            .pointer
            .as_ref()
            .map(|pointer| pointer.regex.as_str()),
    )?;
    compile_query(&spec.source.jmes_path)?;
    let resync_schedule = validate_resync_schedule(spec.resync_schedule.as_deref())?;

    validate_selector(&spec.target.pod_selector)?;

    for rule in &spec.rules {
        validate_rule(rule)?;
    }

    Ok(ValidatedPolicy {
        source_address,
        pointer_regex,
        resync_schedule,
    })
}

fn validate_rule(rule: &RuleSpec) -> Result<(), ValidationError> {
    if rule.pod_selector.is_none() && rule.namespace_selector.is_none() {
        return Err(ValidationError::RuleMissingSelector);
    }

    if let Some(directions) = &rule.directions {
        if directions.is_empty() {
            return Err(ValidationError::EmptyDirections);
        }

        for direction in directions {
            match direction {
                Direction::Ingress | Direction::Egress => {}
            }
        }
    }

    if let Some(selector) = &rule.pod_selector {
        validate_selector(selector)?;
    }

    if let Some(selector) = &rule.namespace_selector {
        validate_selector(selector)?;
    }

    Ok(())
}

fn validate_selector(selector: &LabelSelector) -> Result<(), ValidationError> {
    if let Some(requirements) = &selector.match_expressions {
        for requirement in requirements {
            validate_selector_requirement(requirement)?;
        }
    }

    Ok(())
}

fn validate_selector_requirement(
    requirement: &LabelSelectorRequirement,
) -> Result<(), ValidationError> {
    if requirement.key.trim().is_empty() {
        return Err(ValidationError::MissingSelectorKey(
            requirement.operator.as_str().to_owned(),
        ));
    }

    match requirement.operator {
        LabelSelectorOperator::In | LabelSelectorOperator::NotIn => {
            match requirement
                .values
                .as_ref()
                .filter(|values| !values.is_empty())
            {
                Some(_) => Ok(()),
                None => Err(ValidationError::SelectorValuesRequired(
                    requirement.key.clone(),
                    requirement.operator.as_str(),
                )),
            }
        }
        LabelSelectorOperator::Exists | LabelSelectorOperator::DoesNotExist => {
            if requirement.values.is_some() {
                return Err(ValidationError::SelectorValuesForbidden(
                    requirement.key.clone(),
                    requirement.operator.as_str(),
                ));
            }
            Ok(())
        }
    }
}

pub fn validate_pointer_regex(regex: Option<&str>) -> Result<Option<Regex>, ValidationError> {
    let Some(regex) = regex else {
        return Ok(None);
    };

    Regex::new(regex)
        .map(Some)
        .map_err(ValidationError::InvalidPointerRegex)
}

pub fn validate_resync_schedule(schedule: Option<&str>) -> Result<Schedule, ValidationError> {
    let schedule = schedule.unwrap_or(DEFAULT_RESYNC_SCHEDULE);

    if schedule.split_whitespace().count() != 5 {
        return Err(ValidationError::InvalidResyncScheduleShape);
    }

    let normalized_schedule = format!("0 {schedule}");

    Schedule::from_str(&normalized_schedule)
        .map_err(|error| ValidationError::InvalidResyncSchedule(error.to_string()))
}

impl LabelSelectorOperator {
    fn as_str(&self) -> &'static str {
        match self {
            LabelSelectorOperator::In => "In",
            LabelSelectorOperator::NotIn => "NotIn",
            LabelSelectorOperator::Exists => "Exists",
            LabelSelectorOperator::DoesNotExist => "DoesNotExist",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{
        CIDRPolicySpec, LabelSelector, LabelSelectorOperator, LabelSelectorRequirement,
        PointerSpec, RuleSpec, SourceSpec, TargetSpec,
    };
    use std::collections::BTreeMap;

    fn base_spec() -> CIDRPolicySpec {
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
                pod_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "access-tier".to_owned(),
                        "trusted".to_owned(),
                    )])),
                    match_expressions: None,
                }),
                namespace_selector: None,
            }],
        }
    }

    #[test]
    fn valid_spec_is_accepted() {
        let validated = validate_spec(&base_spec()).unwrap();
        assert_eq!(
            validated.source_address.request_url.as_str(),
            "https://example.invalid/"
        );
        assert!(validated.pointer_regex.is_none());
        assert_eq!(
            validated.resync_schedule.to_string(),
            format!("0 {DEFAULT_RESYNC_SCHEDULE}")
        );
    }

    #[test]
    fn invalid_jmespath_is_rejected() {
        let mut spec = base_spec();
        spec.source.jmes_path = "[".to_owned();
        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidJmesPath(_)));
    }

    #[test]
    fn rule_without_selectors_is_rejected() {
        let mut spec = base_spec();
        spec.rules[0].pod_selector = None;
        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(err, ValidationError::RuleMissingSelector));
    }

    #[test]
    fn selector_values_are_enforced_for_in_operator() {
        let mut spec = base_spec();
        spec.rules[0].pod_selector = Some(LabelSelector {
            match_labels: None,
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "environment".to_owned(),
                operator: LabelSelectorOperator::In,
                values: None,
            }]),
        });

        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::SelectorValuesRequired(_, "In")
        ));
    }

    #[test]
    fn selector_values_are_forbidden_for_exists_operator() {
        let mut spec = base_spec();
        spec.rules[0].pod_selector = Some(LabelSelector {
            match_labels: None,
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "environment".to_owned(),
                operator: LabelSelectorOperator::Exists,
                values: Some(vec!["prod".to_owned()]),
            }]),
        });

        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::SelectorValuesForbidden(_, "Exists")
        ));
    }

    #[test]
    fn invalid_pointer_regex_is_rejected() {
        let mut spec = base_spec();
        spec.source.pointer = Some(PointerSpec {
            regex: "(".to_owned(),
        });

        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidPointerRegex(_)));
    }

    #[test]
    fn invalid_resync_schedule_shape_is_rejected() {
        let mut spec = base_spec();
        spec.resync_schedule = Some("0 0 * * * *".to_owned());

        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidResyncScheduleShape));
    }

    #[test]
    fn invalid_resync_schedule_is_rejected() {
        let mut spec = base_spec();
        spec.resync_schedule = Some("0 0 32 * *".to_owned());

        let err = validate_spec(&spec).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidResyncSchedule(_)));
    }

    #[test]
    fn valid_pointer_regex_is_compiled() {
        let mut spec = base_spec();
        spec.source.pointer = Some(PointerSpec {
            regex: "endpoint=(https://[^\"\\s]+)".to_owned(),
        });

        let validated = validate_spec(&spec).unwrap();
        assert!(validated.pointer_regex.is_some());
    }
}
