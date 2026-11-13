//! Reading one setting's value, tolerantly for what a file holds and
//! strictly for a write.

use std::path::Path;

use serde_json::Value;

use super::{SettingChoice, SettingKind, SettingSpec, SettingValue};

/// Why a value is not used as it is.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingProblem {
    /// Out of its bounds (`bounds`, "between 1 and 16"): `clamped` is the
    /// nearest value in them.
    OutOfRange {
        clamped: SettingValue,
        bounds: String,
    },
    /// Not a value of the setting, and why ("is not an integer").
    Unusable(String),
}

impl SettingSpec {
    /// `raw` as this setting's value, as a file holding it is read: an alias
    /// of a choice is its value, a disabled choice is still one, and a
    /// number out of range is a problem that carries its clamped value.
    pub fn read(&self, raw: &Value) -> Result<SettingValue, SettingProblem> {
        let unusable = |reason: &str| Err(SettingProblem::Unusable(reason.to_string()));
        match &self.kind {
            SettingKind::Boolean => match raw.as_bool() {
                Some(value) => Ok(SettingValue::Boolean(value)),
                None => unusable("is not true or false"),
            },
            SettingKind::Integer { minimum, maximum } => {
                let Some(value) = integer(raw) else {
                    return unusable("is not an integer");
                };
                let low = minimum.map_or(i128::from(i64::MIN), i128::from);
                let high = maximum.map_or(i128::from(i64::MAX), i128::from);
                let clamped = i64::try_from(value.clamp(low, high)).unwrap_or(i64::MAX);
                if i128::from(clamped) == value {
                    Ok(SettingValue::Integer(clamped))
                } else {
                    // Past an absent bound, the value is past what an i64 holds.
                    Err(SettingProblem::OutOfRange {
                        clamped: SettingValue::Integer(clamped),
                        bounds: bounds(
                            minimum.or((value < low).then_some(i64::MIN)),
                            maximum.or((value > high).then_some(i64::MAX)),
                        ),
                    })
                }
            }
            SettingKind::Number { minimum, maximum } => {
                let Some(value) = raw.as_f64() else {
                    return unusable("is not a number");
                };
                let clamped = value
                    .max(minimum.unwrap_or(f64::MIN))
                    .min(maximum.unwrap_or(f64::MAX));
                if clamped == value {
                    Ok(SettingValue::Number(value))
                } else {
                    Err(SettingProblem::OutOfRange {
                        clamped: SettingValue::Number(clamped),
                        bounds: bounds(*minimum, *maximum),
                    })
                }
            }
            SettingKind::Text {
                maximum_chars,
                nullable,
                ..
            }
            | SettingKind::ChoiceOrText {
                maximum_chars,
                nullable,
                ..
            } => match raw {
                Value::Null if *nullable => Ok(SettingValue::Null),
                Value::String(text) if fits(text, *maximum_chars) => {
                    Ok(SettingValue::Text(text.clone()))
                }
                Value::String(_) => Err(SettingProblem::Unusable(format!(
                    "is longer than {} characters",
                    maximum_chars.unwrap_or_default()
                ))),
                _ => unusable("is not a string"),
            },
            SettingKind::Path { must_be_absolute } => match raw.as_str() {
                Some(path) if *must_be_absolute && !Path::new(path).is_absolute() => {
                    unusable("is not an absolute path")
                }
                Some(path) => Ok(SettingValue::Text(path.to_string())),
                None => unusable("is not a path"),
            },
            SettingKind::Choice { values } => match raw.as_str().and_then(|text| {
                values.iter().find(|choice| {
                    choice.value == text || choice.aliases.iter().any(|alias| alias == text)
                })
            }) {
                Some(choice) => Ok(SettingValue::Text(choice.value.clone())),
                None => Err(SettingProblem::Unusable(format!(
                    "is not one of {}",
                    choice_values(values)
                ))),
            },
            SettingKind::TextList {
                maximum_items,
                maximum_item_chars,
            } => {
                let Some(items) = raw.as_array() else {
                    return unusable("is not a list of strings");
                };
                if let Some(maximum) =
                    maximum_items.filter(|maximum| items.len() > *maximum as usize)
                {
                    return Err(SettingProblem::Unusable(format!(
                        "has more than {maximum} items"
                    )));
                }
                let mut texts = Vec::with_capacity(items.len());
                for item in items {
                    match item.as_str() {
                        Some(text) if fits(text, *maximum_item_chars) => {
                            texts.push(text.to_string())
                        }
                        Some(_) => {
                            return Err(SettingProblem::Unusable(format!(
                                "has an item longer than {} characters",
                                maximum_item_chars.unwrap_or_default()
                            )))
                        }
                        None => return unusable("is not a list of strings"),
                    }
                }
                Ok(SettingValue::TextList(texts))
            }
        }
    }

    /// `raw` as this setting's value, or why a write cannot set it to it:
    /// a value out of range, a former name of a choice and a disabled
    /// choice are refused.
    pub fn check(&self, raw: &Value) -> Result<SettingValue, String> {
        let value = self.read(raw).map_err(|problem| match problem {
            SettingProblem::OutOfRange { bounds, .. } => format!("`{}` is {bounds}", self.key),
            SettingProblem::Unusable(reason) => format!("`{}` {reason}", self.key),
        })?;
        let SettingKind::Choice { values } = &self.kind else {
            return Ok(value);
        };
        match values
            .iter()
            .find(|choice| Some(choice.value.as_str()) == raw.as_str())
        {
            Some(SettingChoice {
                disabled: Some(reason),
                value,
                ..
            }) => Err(format!("`{}` cannot be {value}: {reason}", self.key)),
            Some(_) => Ok(value),
            None => Err(format!(
                "`{}` is not one of {}; {raw} is a former name of {}",
                self.key,
                choice_values(values),
                value.to_json()
            )),
        }
    }
}

/// "between 1 and 16", "at least 1" or "at most 16".
fn bounds<T: std::fmt::Display>(minimum: Option<T>, maximum: Option<T>) -> String {
    match (minimum, maximum) {
        (Some(minimum), Some(maximum)) => format!("between {minimum} and {maximum}"),
        (Some(minimum), None) => format!("at least {minimum}"),
        (None, Some(maximum)) => format!("at most {maximum}"),
        (None, None) => "unbounded".to_string(),
    }
}

fn fits(text: &str, maximum_chars: Option<u32>) -> bool {
    maximum_chars.is_none_or(|maximum| text.chars().count() <= maximum as usize)
}

fn choice_values(values: &[SettingChoice]) -> String {
    values
        .iter()
        .map(|choice| choice.value.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A JSON integer, or a number with no fraction (`1.0`), widened so no
/// value overflows before it is clamped.
fn integer(raw: &Value) -> Option<i128> {
    if let Some(value) = raw.as_i64() {
        return Some(i128::from(value));
    }
    if let Some(value) = raw.as_u64() {
        return Some(i128::from(value));
    }
    let value = raw.as_f64()?;
    (value.fract() == 0.0 && value.abs() < 1e30).then_some(value as i128)
}
