//! Deterministic metric-gate parsing and evaluation.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::errors::PolicyValidationError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GateOperator {
    Min,
    Max,
}

impl GateOperator {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PolicyGate {
    pub metric: String,
    pub operator: GateOperator,
    pub threshold: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReleasePolicy {
    pub gates: Vec<PolicyGate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GateEvaluation {
    pub metric: String,
    pub operator: GateOperator,
    pub threshold: f64,
    pub actual: Option<f64>,
    pub passed: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PolicyEvaluation {
    pub passed: bool,
    pub results: Vec<GateEvaluation>,
}

fn as_number(value: &Value, field_name: &str) -> Result<f64, PolicyValidationError> {
    let number = value
        .as_f64()
        .ok_or_else(|| PolicyValidationError(format!("{field_name} must be a number.")))?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(PolicyValidationError(format!(
            "{field_name} must be finite."
        )))
    }
}

/// # Errors
///
/// Returns [`PolicyValidationError`] when the policy does not have exactly one valid, non-empty
/// set of metric gates.
pub fn parse_policy(raw_policy: &Value) -> Result<ReleasePolicy, PolicyValidationError> {
    let policy = raw_policy
        .as_object()
        .ok_or_else(|| PolicyValidationError("Policy must be an object.".to_owned()))?;
    if policy.len() != 1 || !policy.contains_key("gates") {
        return Err(PolicyValidationError(
            "Policy must contain exactly one 'gates' field.".to_owned(),
        ));
    }
    let gates = policy["gates"]
        .as_object()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PolicyValidationError("Policy gates must be a non-empty object.".to_owned())
        })?;
    gates
        .iter()
        .map(|(metric, raw_gate)| {
            if metric.trim().is_empty() {
                return Err(PolicyValidationError(
                    "Policy gate names must be non-empty strings.".to_owned(),
                ));
            }
            let gate = raw_gate.as_object().ok_or_else(|| {
                PolicyValidationError(format!("Gate '{metric}' must be an object."))
            })?;
            let (operator, raw_threshold) = match (gate.get("min"), gate.get("max"), gate.len()) {
                (Some(value), None, 1) => (GateOperator::Min, value),
                (None, Some(value), 1) => (GateOperator::Max, value),
                _ => {
                    return Err(PolicyValidationError(format!(
                        "Gate '{metric}' must contain exactly one of 'min' or 'max'."
                    )));
                }
            };
            Ok(PolicyGate {
                metric: metric.clone(),
                operator,
                threshold: as_number(raw_threshold, &format!("Gate '{metric}' threshold"))?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|gates| ReleasePolicy { gates })
}

/// # Errors
///
/// Returns [`PolicyValidationError`] when a supplied metric value is not a finite number.
pub fn evaluate_policy(
    policy: &ReleasePolicy,
    metrics: &Map<String, Value>,
) -> Result<PolicyEvaluation, PolicyValidationError> {
    let results = policy
        .gates
        .iter()
        .map(|gate| match metrics.get(&gate.metric) {
            None => Ok(GateEvaluation {
                metric: gate.metric.clone(),
                operator: gate.operator,
                threshold: gate.threshold,
                actual: None,
                passed: false,
                reason: Some("Required metric was not supplied.".to_owned()),
            }),
            Some(value) => {
                let actual = as_number(value, &format!("Metric '{}'", gate.metric))?;
                let passed = match gate.operator {
                    GateOperator::Min => actual >= gate.threshold,
                    GateOperator::Max => actual <= gate.threshold,
                };
                Ok(GateEvaluation {
                    metric: gate.metric.clone(),
                    operator: gate.operator,
                    threshold: gate.threshold,
                    actual: Some(actual),
                    passed,
                    reason: None,
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PolicyEvaluation {
        passed: results.iter().all(|result| result.passed),
        results,
    })
}
