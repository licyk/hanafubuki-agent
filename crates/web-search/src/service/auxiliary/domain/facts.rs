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

use super::{
    date, path, source, DomainEngine as E, Items, WebSearchError, WebSearchResult, WebSearchService,
};
use chrono::Datelike;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;

impl WebSearchService {
    pub(super) async fn domain_facts(
        &self,
        engine: E,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok(match engine {
            E::Wikidata => {
                let d: Wikidata = self
                    .domain_json(
                        "https://www.wikidata.org/w/api.php",
                        &[
                            ("action", "wbsearchentities"),
                            ("search", query),
                            ("language", "zh-cn"),
                            ("format", "json"),
                            ("limit", "3"),
                        ],
                        cancel,
                    )
                    .await?;
                d.search
                    .0
                    .into_iter()
                    .map(|r| {
                        source(
                            "wikidata",
                            r.concepturi,
                            format!("{} ({})", r.label, r.id),
                            r.description.unwrap_or_default(),
                            None,
                        )
                    })
                    .collect()
            }
            E::Holidays => {
                let year = query
                    .split(|c: char| !c.is_ascii_digit())
                    .find(|s| s.len() == 4 && s.starts_with("20"))
                    .map(String::from)
                    .unwrap_or_else(|| chrono::Utc::now().year().to_string());
                let countries = [
                    ("中国", "CN"),
                    ("美国", "US"),
                    ("日本", "JP"),
                    ("德国", "DE"),
                    ("英国", "GB"),
                    ("法国", "FR"),
                    ("韩国", "KR"),
                    ("香港", "HK"),
                    ("台湾", "TW"),
                    ("新加坡", "SG"),
                    ("澳大利亚", "AU"),
                    ("加拿大", "CA"),
                    ("印度", "IN"),
                    ("俄罗斯", "RU"),
                ];
                let country = query
                    .split_whitespace()
                    .find(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()))
                    .map(str::to_uppercase)
                    .or_else(|| {
                        countries
                            .iter()
                            .find(|(name, _)| query.contains(name))
                            .map(|(_, cc)| cc.to_string())
                    })
                    .ok_or(WebSearchError::InvalidQuery(
                        "give a country name or two-letter country code",
                    ))?;
                let holidays: Items<Holiday> = self
                    .domain_json(
                        &path(
                            "https://date.nager.at",
                            &["api", "v3", "publicholidays", &year, &country],
                        )?,
                        &[],
                        cancel,
                    )
                    .await?;
                holidays
                    .0
                    .into_iter()
                    .take(3)
                    .map(|h| {
                        source(
                            "holidays",
                            "https://date.nager.at",
                            format!(
                                "{} ({})",
                                h.local_name.unwrap_or_else(|| h.name.clone()),
                                h.name
                            ),
                            format!("{} · {} · {country} {year}", h.date, h.types.join("/")),
                            Some(h.date),
                        )
                    })
                    .collect()
            }
            E::Worldclock => vec![worldclock(query)?],
            E::Openmeteo => {
                let report = self.weather(query, 1, cancel.clone()).await?;
                vec![source(
                    "openmeteo",
                    "https://open-meteo.com/",
                    format!("{} 天气", report.location),
                    format!(
                        "当前 {}°C · 风速 {} km/h · {}",
                        report
                            .current
                            .temperature
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "?".into()),
                        report
                            .current
                            .wind
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".into()),
                        report.current.text
                    ),
                    None,
                )]
            }
            E::Rdap => self.rdap(query, cancel).await?,
            E::Gleif => {
                let key = query.trim();
                let is_code = key.len() == 20
                    && key
                        .bytes()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
                let url = if is_code {
                    path("https://api.gleif.org", &["api", "v1", "lei-records", key])?
                } else {
                    "https://api.gleif.org/api/v1/lei-records".into()
                };
                let params = if is_code {
                    Vec::new()
                } else {
                    vec![("filter[entity.legalName]", key), ("page[size]", "3")]
                };
                let d: Gleif = self.domain_json(&url, &params, cancel).await?;
                let records = match d.data {
                    GleifData::Many(rows) => rows.0,
                    GleifData::One(row) => vec![row],
                };
                records
                    .into_iter()
                    .take(3)
                    .map(|r| {
                        let a = r.attributes;
                        source(
                            "gleif",
                            format!("https://api.gleif.org/api/v1/lei-records/{}", a.lei),
                            format!("{} ({})", a.entity.legal_name.name, a.lei),
                            format!(
                                "LEI status: {}",
                                a.entity
                                    .status
                                    .or(a.registration.and_then(|r| r.status))
                                    .unwrap_or_else(|| "?".into())
                            ),
                            None,
                        )
                    })
                    .collect()
            }
            E::Worldbank => self.worldbank(query, cancel).await?,
            _ => return Err(WebSearchError::InvalidQuery("not a facts engine")),
        })
    }
    async fn worldbank(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        let lower = query.to_lowercase();
        let indicators = [
            ("gdp growth", "NY.GDP.MKTP.KD.ZG"),
            ("增速", "NY.GDP.MKTP.KD.ZG"),
            ("gdp", "NY.GDP.MKTP.CD"),
            ("population", "SP.POP.TOTL"),
            ("人口", "SP.POP.TOTL"),
            ("inflation", "FP.CPI.TOTL.ZG"),
            ("通胀", "FP.CPI.TOTL.ZG"),
            ("unemployment", "SL.UEM.TOTL.ZS"),
            ("失业", "SL.UEM.TOTL.ZS"),
            ("co2", "EN.GHG.CO2.MT.CE.AR5"),
            ("碳排放", "EN.GHG.CO2.MT.CE.AR5"),
            ("energy", "EG.USE.PCAP.KG.OE"),
            ("trade", "NE.TRD.GNFS.ZS"),
        ];
        let countries = [
            ("中国", "CHN"),
            ("美国", "USA"),
            ("日本", "JPN"),
            ("印度", "IND"),
            ("德国", "DEU"),
            ("英国", "GBR"),
            ("法国", "FRA"),
            ("韩国", "KOR"),
            ("世界", "WLD"),
            ("巴西", "BRA"),
            ("俄罗斯", "RUS"),
            ("china", "CHN"),
            ("united states", "USA"),
            ("usa", "USA"),
            ("japan", "JPN"),
            ("india", "IND"),
            ("germany", "DEU"),
            ("uk", "GBR"),
            ("britain", "GBR"),
            ("france", "FRA"),
            ("korea", "KOR"),
            ("world", "WLD"),
            ("brazil", "BRA"),
            ("russia", "RUS"),
        ];
        let (_, indicator) = indicators
            .iter()
            .find(|(label, _)| lower.contains(label))
            .ok_or(WebSearchError::InvalidQuery(
                "name an indicator such as GDP, population, inflation or unemployment",
            ))?;
        let country = countries
            .iter()
            .find(|(label, _)| {
                if label.is_ascii() {
                    lower
                        .split(|c: char| !c.is_ascii_alphabetic())
                        .any(|word| word == *label)
                        || (label.contains(' ') && lower.contains(label))
                } else {
                    lower.contains(label)
                }
            })
            .map(|(_, cc)| cc.to_string())
            .or_else(|| {
                query
                    .split_whitespace()
                    .find(|s| {
                        s.len() == 3
                            && s.bytes().all(|c| c.is_ascii_alphabetic())
                            && !["gdp", "co2", "eps", "esg", "imf"]
                                .contains(&s.to_ascii_lowercase().as_str())
                    })
                    .map(str::to_uppercase)
            })
            .ok_or(WebSearchError::InvalidQuery(
                "give a country name or three-letter ISO country code",
            ))?;
        let (_, rows): (serde::de::IgnoredAny, Items<BankRow>) = self
            .domain_json(
                &path(
                    "https://api.worldbank.org",
                    &["v2", "country", &country, "indicator", indicator],
                )?,
                &[("format", "json"), ("per_page", "1"), ("mrnev", "1")],
                cancel,
            )
            .await?;
        Ok(rows
            .0
            .into_iter()
            .take(1)
            .map(|r| {
                source(
                    "worldbank",
                    format!("https://data.worldbank.org/indicator/{indicator}?locations={country}"),
                    format!(
                        "{} · {}: {} ({})",
                        r.country.value,
                        r.indicator.value,
                        r.value.map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                        r.date
                    ),
                    "World Bank WDI, most recent non-empty value",
                    Some(r.date),
                )
            })
            .collect())
    }
    async fn rdap(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok({
            let regex = regex::Regex::new(r"([a-z0-9-]+\.[a-z.]{2,})")
                .map_err(|_| WebSearchError::InvalidResponse)?;
            let lower = query.trim().to_lowercase();
            let domain = regex
                .find(&lower)
                .ok_or(WebSearchError::InvalidQuery(
                    "give a domain, for example example.com",
                ))?
                .as_str();
            let d: Rdap = self
                .inner
                .page_fetcher
                .public_json(&path("https://rdap.org", &["domain", domain])?, cancel)
                .await
                .map_err(|e| match e {
                    crate::WebFetchError::Cancelled => WebSearchError::Cancelled,
                    crate::WebFetchError::Timeout => WebSearchError::Timeout,
                    _ => WebSearchError::RequestFailed,
                })?;
            let registration = d
                .events
                .0
                .iter()
                .find(|e| e.event_action == "registration")
                .map(|e| e.event_date.clone());
            let expiry = date(
                d.events
                    .0
                    .iter()
                    .find(|e| e.event_action == "expiration")
                    .map(|e| e.event_date.clone()),
            );
            let registrar = d
                .entities
                .0
                .into_iter()
                .find(|e| e.roles.iter().any(|r| r == "registrar"))
                .and_then(|e| e.vcard_array)
                .and_then(|(_, cards)| cards.0.into_iter().find(|c| c.0 == "fn"))
                .map(|c| c.3);
            vec![source(
                "rdap",
                format!("https://rdap.org/?query={domain}"),
                format!(
                    "{domain}: registered {}, expires {}",
                    date(registration).as_deref().unwrap_or("?"),
                    expiry.as_deref().unwrap_or("?")
                ),
                format!(
                    "{}status {}",
                    registrar
                        .map(|r| format!("registrar: {r} · "))
                        .unwrap_or_default(),
                    d.status.into_iter().take(3).collect::<Vec<_>>().join(" ")
                ),
                expiry,
            )]
        })
    }
}

fn worldclock(query: &str) -> Result<WebSearchResult, WebSearchError> {
    let key = query.trim();
    let zone = match key {
        "北京" | "上海" => "Asia/Shanghai",
        "东京" => "Asia/Tokyo",
        "首尔" => "Asia/Seoul",
        "纽约" => "America/New_York",
        "洛杉矶" => "America/Los_Angeles",
        "伦敦" => "Europe/London",
        "巴黎" => "Europe/Paris",
        "柏林" => "Europe/Berlin",
        "莫斯科" => "Europe/Moscow",
        "迪拜" => "Asia/Dubai",
        "悉尼" => "Australia/Sydney",
        "新加坡" => "Asia/Singapore",
        _ => key,
    };
    let timezone: chrono_tz::Tz = zone.parse().map_err(|_| {
        WebSearchError::InvalidQuery("give a city or IANA timezone such as America/New_York")
    })?;
    let now = chrono::Utc::now().with_timezone(&timezone);
    Ok(source(
        "worldclock",
        path("https://time.is", &[key])?,
        format!("{key} 当前时间"),
        format!("{} ({zone})", now.format("%Y-%m-%d %H:%M:%S")),
        None,
    ))
}
#[derive(Deserialize)]
struct Wikidata {
    #[serde(default)]
    search: Items<Entity>,
}
#[derive(Deserialize)]
struct Entity {
    concepturi: String,
    label: String,
    id: String,
    description: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Holiday {
    name: String,
    local_name: Option<String>,
    date: String,
    #[serde(default)]
    types: Vec<String>,
}
#[derive(Deserialize)]
struct Rdap {
    #[serde(default)]
    events: Items<Event>,
    #[serde(default)]
    entities: Items<RdapEntity>,
    #[serde(default)]
    status: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    event_action: String,
    event_date: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RdapEntity {
    #[serde(default)]
    roles: Vec<String>,
    vcard_array: Option<(String, Items<Card>)>,
}
#[derive(Deserialize)]
struct Card(String, serde::de::IgnoredAny, serde::de::IgnoredAny, String);
#[derive(Deserialize)]
struct Gleif {
    data: GleifData,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum GleifData {
    Many(Items<LeiRecord>),
    One(LeiRecord),
}
#[derive(Deserialize)]
struct LeiRecord {
    attributes: LeiAttributes,
}
#[derive(Deserialize)]
struct LeiAttributes {
    lei: String,
    entity: LeiEntity,
    registration: Option<LeiRegistration>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeiEntity {
    legal_name: LegalName,
    status: Option<String>,
}
#[derive(Deserialize)]
struct LegalName {
    name: String,
}
#[derive(Deserialize)]
struct LeiRegistration {
    status: Option<String>,
}
#[derive(Deserialize)]
struct BankRow {
    country: BankLabel,
    indicator: BankLabel,
    value: Option<f64>,
    date: String,
}
#[derive(Deserialize)]
struct BankLabel {
    value: String,
}
