use ipnet::IpNet;
use jmespath::{Expression, Variable};
use serde_json::Value;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NormalizedCidr {
    pub rendered: String,
    pub family: IpFamily,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IpFamily {
    V4,
    V6,
}

#[derive(Debug, Error)]
pub enum QueryError {
    #[error(transparent)]
    Compile(#[from] jmespath::JmespathError),
}

#[derive(Debug, Error)]
pub enum ExtractionError {
    #[error("unable to serialize JSON payload for query evaluation")]
    SerializePayload(#[source] serde_json::Error),
    #[error("unable to convert payload into JMESPath variable")]
    VariableFromJson(String),
    #[error("JMESPath evaluation failed")]
    Evaluate(#[source] jmespath::JmespathError),
    #[error("query result must be an array")]
    ResultNotArray,
    #[error("query result elements must all be strings")]
    ResultElementNotString,
    #[error("invalid CIDR `{0}`")]
    InvalidCidr(String),
}

pub fn compile_query(expr: &str) -> Result<Expression<'static>, QueryError> {
    Ok(jmespath::compile(expr)?)
}

pub fn extract_cidrs(
    expression: &Expression<'_>,
    payload: &Value,
) -> Result<Vec<NormalizedCidr>, ExtractionError> {
    let payload_json = serde_json::to_string(payload).map_err(ExtractionError::SerializePayload)?;
    let variable = Variable::from_json(&payload_json).map_err(ExtractionError::VariableFromJson)?;
    let result = expression
        .search(variable)
        .map_err(ExtractionError::Evaluate)?;
    let values = result.as_array().ok_or(ExtractionError::ResultNotArray)?;

    let mut seen = BTreeSet::new();
    let mut cidrs = Vec::new();

    for value in values {
        let Some(cidr) = value.as_string() else {
            return Err(ExtractionError::ResultElementNotString);
        };

        let parsed: IpNet = cidr
            .parse()
            .map_err(|_| ExtractionError::InvalidCidr(cidr.clone()))?;
        let normalized = NormalizedCidr::from(parsed);
        if seen.insert(normalized.rendered.clone()) {
            cidrs.push(normalized);
        }
    }

    cidrs.sort();
    Ok(cidrs)
}

impl From<IpNet> for NormalizedCidr {
    fn from(value: IpNet) -> Self {
        let family = match value {
            IpNet::V4(_) => IpFamily::V4,
            IpNet::V6(_) => IpFamily::V6,
        };

        Self {
            rendered: value.to_string(),
            family,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn deduplicates_and_sorts_valid_cidrs() {
        let expression = compile_query("prefixes").unwrap();
        let payload = json!({
            "prefixes": [
                "2001:db8::/32",
                "10.0.0.0/24",
                "10.0.0.0/24"
            ]
        });

        let actual = extract_cidrs(&expression, &payload).unwrap();
        let rendered: Vec<_> = actual.into_iter().map(|item| item.rendered).collect();

        assert_eq!(rendered, vec!["10.0.0.0/24", "2001:db8::/32"]);
    }

    #[test]
    fn empty_array_is_valid() {
        let expression = compile_query("prefixes").unwrap();
        let payload = json!({ "prefixes": [] });
        let actual = extract_cidrs(&expression, &payload).unwrap();
        assert!(actual.is_empty());
    }

    #[test]
    fn non_array_results_fail() {
        let expression = compile_query("prefixes").unwrap();
        let payload = json!({ "prefixes": "10.0.0.0/24" });
        let err = extract_cidrs(&expression, &payload).unwrap_err();
        assert!(matches!(err, ExtractionError::ResultNotArray));
    }

    #[test]
    fn non_string_elements_fail() {
        let expression = compile_query("prefixes").unwrap();
        let payload = json!({ "prefixes": [42] });
        let err = extract_cidrs(&expression, &payload).unwrap_err();
        assert!(matches!(err, ExtractionError::ResultElementNotString));
    }

    #[test]
    fn invalid_cidrs_fail() {
        let expression = compile_query("prefixes").unwrap();
        let payload = json!({ "prefixes": ["10.0.0.1"] });
        let err = extract_cidrs(&expression, &payload).unwrap_err();
        assert!(matches!(err, ExtractionError::InvalidCidr(value) if value == "10.0.0.1"));
    }
}
