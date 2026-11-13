//! The settings vocabulary: what one user-configurable value is, the values
//! it takes and how a change of it applies. A plugin's options are a list
//! of these, and so is every section of the configuration that declares
//! its keys.
//!
//! A value is read tolerantly ([`SettingSpec::read`]): a renamed choice is
//! read as its current value, and a number out of range comes back with
//! its clamped value for the caller to use or refuse. A write is strict
//! ([`SettingSpec::check`]): it takes neither, nor a disabled choice.
//! Both are this vocabulary's only rules; a rule across settings is its
//! owner's.

mod read;

use serde::Serialize;
use serde_json::Value;

pub use read::SettingProblem;

/// One setting.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingSpec {
    /// Its key, camelCase, relative to where its owner keeps it: dotted
    /// from the file's root for the configuration (`memory.maxSourceAgeDays`),
    /// the key in its table for a plugin's option.
    pub key: String,
    /// What a row is labelled.
    pub label: String,
    /// One line, the row's description.
    pub summary: String,
    /// More, shown below the row when it is selected.
    pub help: Option<String>,
    pub kind: SettingKind,
    /// What it is when nothing usable sets it; `None` when it has none.
    pub default: Option<SettingValue>,
    pub apply: SettingApply,
    /// Shown only when advanced settings are.
    pub advanced: bool,
    /// Shown only while another setting of its owner has a value.
    pub visible_if: Option<SettingCondition>,
    /// The settings of one group are written together, so that a rule
    /// across them is checked once (a search provider and its URL).
    pub commit_group: Option<String>,
    /// Further words a search matches it by.
    pub keywords: Vec<String>,
}

impl SettingSpec {
    /// A setting with no summary, help or default, applied live; the rest
    /// is set with struct update syntax.
    pub fn new(key: impl Into<String>, label: impl Into<String>, kind: SettingKind) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            summary: String::new(),
            help: None,
            kind,
            default: None,
            apply: SettingApply::Live,
            advanced: false,
            visible_if: None,
            commit_group: None,
            keywords: Vec::new(),
        }
    }
}

/// What values a setting takes. Bounds are inclusive; `None` is unbounded.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum SettingKind {
    Boolean,
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Number {
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    Text {
        maximum_chars: Option<u32>,
        /// Drawn as dots, and never returned by a catalog.
        secret: bool,
        /// `null` is a value: the setting is unset.
        nullable: bool,
    },
    Path {
        must_be_absolute: bool,
    },
    Choice {
        values: Vec<SettingChoice>,
    },
    /// Its choices, a text typed in their place, and (`nullable`) unset.
    /// It is read as a text: the values are what is offered, not all it
    /// takes, and the setting's owner checks a typed one when it writes.
    ChoiceOrText {
        values: Vec<SettingChoice>,
        maximum_chars: Option<u32>,
        nullable: bool,
    },
    TextList {
        maximum_items: Option<u32>,
        maximum_item_chars: Option<u32>,
    },
}

/// One value of a choice.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingChoice {
    pub value: String,
    pub label: String,
    /// Former names of the value, read as it and never written.
    pub aliases: Vec<String>,
    /// Why it cannot be chosen now; a value already set stays readable.
    pub disabled: Option<String>,
}

impl SettingChoice {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            aliases: Vec::new(),
            disabled: None,
        }
    }
}

/// One setting's value, as its kind reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    /// A text, a path or a choice.
    Text(String),
    TextList(Vec<String>),
    /// A nullable text that is unset.
    Null,
}

impl SettingValue {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Boolean(value) => Value::Bool(*value),
            Self::Integer(value) => Value::from(*value),
            Self::Number(value) => Value::from(*value),
            Self::Text(value) => Value::String(value.clone()),
            Self::TextList(values) => Value::from(values.clone()),
            Self::Null => Value::Null,
        }
    }
}

/// When a change takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingApply {
    /// In the running Agent, its sessions included.
    Live,
    /// From the next session on.
    NextSession,
    /// When the Agent next starts.
    NextLaunch,
}

/// A setting shown only while the setting `key` of the same owner is
/// `equals`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingCondition {
    pub key: String,
    pub equals: SettingValue,
}
