// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Search time filters, including the extension's custom durations and dates.
use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WebSearchTimeRange {
    Day,
    Week,
    Month,
    Year,
    DurationSeconds(i64),
    After(NaiveDate),
}
impl WebSearchTimeRange {
    pub(crate) fn days(self) -> i64 {
        match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
            Self::Year => 365,
            Self::DurationSeconds(s) => s.div_euclid(86400) + i64::from(s.rem_euclid(86400) != 0),
            Self::After(date) => (Utc::now().date_naive() - date).num_days().max(1),
        }
    }
    pub(crate) fn start_timestamp(self) -> i64 {
        match self {
            Self::After(date) => date
                .and_hms_opt(0, 0, 0)
                .map(|d| d.and_utc().timestamp())
                .unwrap_or(0),
            Self::DurationSeconds(seconds) => Utc::now().timestamp().saturating_sub(seconds),
            _ => Utc::now().timestamp().saturating_sub(self.days() * 86400),
        }
    }
    pub(crate) fn start_date(self) -> String {
        match self {
            Self::After(date) => date.to_string(),
            _ => chrono::DateTime::from_timestamp(self.start_timestamp(), 0)
                .map(|d| format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
                .unwrap_or_default(),
        }
    }
    pub(crate) fn as_searxng(self) -> &'static str {
        match self.days() {
            0..=2 => "day",
            3..=14 => "week",
            15..=90 => "month",
            _ => "year",
        }
    }
    pub(crate) fn as_duckduckgo(self) -> &'static str {
        match self.as_searxng() {
            "day" => "d",
            "week" => "w",
            "month" => "m",
            _ => "y",
        }
    }
}
impl std::str::FromStr for WebSearchTimeRange {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let input = value.trim().to_lowercase();
        match input.as_str() {
            "day" => return Ok(Self::Day),
            "week" => return Ok(Self::Week),
            "month" => return Ok(Self::Month),
            "year" => return Ok(Self::Year),
            _ => {}
        }
        if let Ok(date) = NaiveDate::parse_from_str(&input, "%Y-%m-%d") {
            return Ok(Self::After(date));
        }
        let split = input
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or("time range needs a duration unit or YYYY-MM-DD")?;
        let amount: f64 = input[..split]
            .parse()
            .map_err(|_| "invalid time duration")?;
        let factor = match input[split..].trim() {
            "h" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            "w" | "week" | "weeks" => 604800.0,
            "mo" | "month" | "months" => 2592000.0,
            "y" | "year" | "years" => 31536000.0,
            _ => return Err("unknown time duration unit"),
        };
        let seconds = (amount * factor).round();
        if !seconds.is_finite() || seconds < 1.0 || seconds > i64::MAX as f64 {
            return Err("invalid time duration");
        }
        let seconds = seconds as i64;
        if chrono::DateTime::from_timestamp(Utc::now().timestamp().saturating_sub(seconds), 0)
            .is_none()
        {
            return Err("time duration is outside the supported date range");
        }
        Ok(Self::DurationSeconds(seconds))
    }
}
impl Serialize for WebSearchTimeRange {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Day => serializer.serialize_str("day"),
            Self::Week => serializer.serialize_str("week"),
            Self::Month => serializer.serialize_str("month"),
            Self::Year => serializer.serialize_str("year"),
            Self::After(date) => serializer.serialize_str(&date.to_string()),
            Self::DurationSeconds(s) => {
                serializer.serialize_str(&format!("{}h", *s as f64 / 3600.0))
            }
        }
    }
}
impl<'de> Deserialize<'de> for WebSearchTimeRange {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
