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

use super::super::plain;
use super::{
    date, last, path, source, DomainEngine as E, Items, WebSearchError, WebSearchResult,
    WebSearchService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::Deserialize;
use std::collections::BTreeMap;

impl WebSearchService {
    pub(super) async fn domain_finance(
        &self,
        engine: E,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok(match engine {
            E::Yahoofinance => {
                let symbol = last(query)?.to_uppercase();
                let d: Yahoo = self
                    .domain_json(
                        &path(
                            "https://query1.finance.yahoo.com",
                            &["v8", "finance", "chart", &symbol],
                        )?,
                        &[("range", "5d"), ("interval", "1d")],
                        cancel,
                    )
                    .await?;
                let m = d
                    .chart
                    .result
                    .0
                    .into_iter()
                    .next()
                    .ok_or(WebSearchError::NoResults)?
                    .meta;
                let change = m
                    .chart_previous_close
                    .filter(|p| *p != 0.0)
                    .map(|p| format!("{:+.2}%", (m.regular_market_price - p) / p * 100.0))
                    .unwrap_or_else(|| "?".into());
                vec![source(
                    "yahoofinance",
                    path("https://finance.yahoo.com", &["quote", &symbol])?,
                    format!(
                        "{symbol}: {} {} ({change})",
                        m.regular_market_price, m.currency
                    ),
                    format!(
                        "{} · prev close {}",
                        m.full_exchange_name.or(m.exchange_name).unwrap_or_default(),
                        m.chart_previous_close
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "?".into())
                    ),
                    None,
                )]
            }
            E::Coingecko => self.coingecko(query, cancel).await?,
            E::Frankfurter => {
                let codes: Vec<_> = query
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .filter(|s| s.len() == 3 && !s.eq_ignore_ascii_case("the"))
                    .map(str::to_uppercase)
                    .take(2)
                    .collect();
                let [from, to] = codes.as_slice() else {
                    return Err(WebSearchError::InvalidQuery(
                        "give two ISO currency codes, for example USD CNY",
                    ));
                };
                let d: Rates = self
                    .domain_json(
                        "https://api.frankfurter.dev/v1/latest",
                        &[("from", from), ("to", to)],
                        cancel,
                    )
                    .await?;
                let rate = d.rates.get(to).ok_or(WebSearchError::NoResults)?;
                vec![source(
                    "frankfurter",
                    "https://frankfurter.app",
                    format!("1 {from} = {rate} {to}"),
                    format!("ECB reference rate {}", d.date),
                    Some(d.date),
                )]
            }
            E::Cfets => {
                let d: Cfets = self
                    .domain_json(
                        "https://www.chinamoney.com.cn/r/cms/www/chinamoney/data/fx/ccpr.json",
                        &[],
                        cancel,
                    )
                    .await?;
                let upper = query.to_uppercase();
                let wanted = upper
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .find(|s| s.len() == 3)
                    .unwrap_or_else(|| match query.trim() {
                        "欧元" => "EUR",
                        "日元" => "JPY",
                        "英镑" => "GBP",
                        "港币" => "HKD",
                        "澳元" => "AUD",
                        "加元" => "CAD",
                        "新加坡元" => "SGD",
                        "韩元" => "KRW",
                        _ => "USD",
                    });
                d.records.0.into_iter().find(|r|r.vrt_ename.trim_start_matches("100").starts_with(wanted)).and_then(|r| { let rate = r.price.replace(',',"").parse::<f64>().ok()? / if r.vrt_ename.starts_with("100") {100.0} else {1.0}; Some(source("cfets","https://www.chinamoney.com.cn/chinese/bkccpr/",format!("{wanted}/CNY 中间价 {rate}"),format!("1 {wanted} = {rate} CNY · 中国外汇交易中心每交易日 9:15 公布，非实时汇率"),date(d.data.last_date))) }).into_iter().collect()
            }
            E::Cninfo => {
                let bytes = self
                    .send_bounded(
                        self.inner
                            .client
                            .post("https://www.cninfo.com.cn/new/hisAnnouncement/query")
                            .form(&[
                                ("pageNum", "1"),
                                ("pageSize", "5"),
                                ("column", "szse"),
                                ("tabName", "fulltext"),
                                ("searchkey", query),
                                ("isHLtitle", "true"),
                            ]),
                        cancel,
                    )
                    .await?;
                let d: Cninfo = super::decode(&bytes)?;
                d.announcements
                    .0
                    .into_iter()
                    .take(5)
                    .map(|a| {
                        let stamp = a
                            .announcement_time
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|t| {
                                (t + chrono::Duration::hours(8))
                                    .format("%Y-%m-%d")
                                    .to_string()
                            });
                        source(
                            "cninfo",
                            format!(
                                "https://static.cninfo.com.cn/{}",
                                a.adjunct_url.trim_start_matches('/')
                            ),
                            format!(
                                "{}({}) {}",
                                a.sec_name,
                                a.sec_code,
                                plain(&a.announcement_title)
                            ),
                            format!(
                                "披露 {}{}",
                                stamp.as_deref().unwrap_or("?"),
                                a.adjunct_type
                                    .map(|s| format!(" · {s}"))
                                    .unwrap_or_default()
                            ),
                            stamp,
                        )
                    })
                    .collect()
            }
            E::SecEdgar => self.edgar(query, cancel).await?,
            E::Imf => self.imf(query, cancel).await?,
            _ => return Err(WebSearchError::InvalidQuery("not a finance engine")),
        })
    }
    async fn edgar(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        let regex = regex::Regex::new(
            r"(?i)\b(10-K(?:/A)?|10-Q(?:/A)?|8-K(?:/A)?|20-F|S-1|S-4|13F(?:-HR)?|DEF\s?14A)\b",
        )
        .map_err(|_| WebSearchError::InvalidResponse)?;
        let form = regex
            .find(query)
            .map(|m| m.as_str().to_uppercase().replace(' ', ""))
            .unwrap_or_default();
        let text = regex.replace(query, " ");
        let stops = [
            "A", "I", "AND", "OR", "THE", "IN", "OF", "TO", "AI", "ML", "US", "USA", "UK", "EU",
            "CEO", "CFO", "IPO", "ESG", "GDP", "SEC", "EPS", "ETF", "PDF", "API", "INC", "LTD",
            "IT", "ALL", "NEW", "ON", "SO", "GO", "NOW", "BY", "FOR",
        ];
        let ticker_pattern = regex::Regex::new(r"^[A-Z]{1,5}(\.[A-Z]{1,2})?$")
            .map_err(|_| WebSearchError::InvalidResponse)?;
        let ticker = text
            .split_whitespace()
            .find(|s| ticker_pattern.is_match(s) && !stops.contains(s));
        let mut params = vec![("q", text.trim())];
        if !form.is_empty() {
            params.push(("forms", form.as_str()));
        }
        if let Some(ticker) = ticker {
            params.push(("entityName", ticker));
        }
        let bytes = self
            .send_bounded(
                self.inner
                    .client
                    .get("https://efts.sec.gov/LATEST/search-index")
                    .header("user-agent", "hanafubuki-agent web plugin")
                    .query(&params),
                cancel,
            )
            .await?;
        let d: Edgar = super::decode(&bytes)?;
        Ok(d.hits
            .hits
            .0
            .into_iter()
            .take(3)
            .map(|h| {
                let s = h.source;
                let url =
                    h.id.split_once(':')
                        .filter(|_| !s.cik.is_empty())
                        .map(|(a, doc)| {
                            format!(
                                "https://www.sec.gov/Archives/edgar/data/{}/{}/{doc}",
                                s.cik.trim_start_matches('0'),
                                a.replace('-', "")
                            )
                        })
                        .unwrap_or_else(|| {
                            "https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany".into()
                        });
                source(
                    "sec_edgar",
                    url,
                    format!(
                        "{} {}",
                        s.display_names
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| "SEC filing".into()),
                        s.file_type.unwrap_or_else(|| form.clone())
                    ),
                    format!(
                        "filed {}{}",
                        s.file_date.as_deref().unwrap_or("?"),
                        s.file_description
                            .map(|v| format!(" · {v}"))
                            .unwrap_or_default()
                    ),
                    date(s.file_date),
                )
            })
            .collect())
    }
    async fn coingecko(
        &self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<WebSearchResult>, WebSearchError> {
        Ok({
            let d: Coins = self
                .domain_json(
                    "https://api.coingecko.com/api/v3/search",
                    &[("query", query)],
                    cancel,
                )
                .await?;
            let coins: Vec<_> = d.coins.0.into_iter().take(3).collect();
            if coins.is_empty() {
                return Ok(Vec::new());
            }
            let ids = coins
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let prices: BTreeMap<String, Price> = self
                .domain_json(
                    "https://api.coingecko.com/api/v3/simple/price",
                    &[("ids", &ids), ("vs_currencies", "usd")],
                    cancel,
                )
                .await?;
            coins
                .into_iter()
                .map(|c| {
                    source(
                        "coingecko",
                        format!("https://www.coingecko.com/en/coins/{}", c.id),
                        format!("{} ({})", c.name, c.symbol.to_uppercase()),
                        format!(
                            "rank {} · ${}",
                            c.market_cap_rank
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| "?".into()),
                            prices
                                .get(&c.id)
                                .and_then(|p| p.usd)
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| "?".into())
                        ),
                        None,
                    )
                })
                .collect()
        })
    }
}
#[derive(Deserialize)]
struct Yahoo {
    chart: Chart,
}
#[derive(Deserialize)]
struct Chart {
    #[serde(default)]
    result: Items<ChartResult>,
}
#[derive(Deserialize)]
struct ChartResult {
    meta: Quote,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Quote {
    regular_market_price: f64,
    currency: String,
    chart_previous_close: Option<f64>,
    full_exchange_name: Option<String>,
    exchange_name: Option<String>,
}
#[derive(Deserialize)]
struct Coins {
    #[serde(default)]
    coins: Items<Coin>,
}
#[derive(Deserialize)]
struct Coin {
    id: String,
    name: String,
    symbol: String,
    market_cap_rank: Option<u64>,
}
#[derive(Deserialize)]
struct Price {
    usd: Option<f64>,
}
#[derive(Deserialize)]
struct Rates {
    date: String,
    #[serde(default)]
    rates: BTreeMap<String, f64>,
}
#[derive(Deserialize)]
struct Cfets {
    data: CfetsData,
    #[serde(default)]
    records: Items<CfetsRate>,
}
#[derive(Deserialize)]
struct CfetsData {
    #[serde(rename = "lastDate")]
    last_date: Option<String>,
}
#[derive(Deserialize)]
struct CfetsRate {
    #[serde(rename = "vrtEName")]
    vrt_ename: String,
    price: String,
}
#[derive(Deserialize)]
struct Cninfo {
    #[serde(default)]
    announcements: Items<Announcement>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Announcement {
    adjunct_url: String,
    #[serde(default)]
    sec_name: String,
    #[serde(default)]
    sec_code: String,
    announcement_title: String,
    announcement_time: Option<i64>,
    adjunct_type: Option<String>,
}
#[derive(Deserialize)]
struct Edgar {
    hits: EdgarHits,
}
#[derive(Deserialize)]
struct EdgarHits {
    #[serde(default)]
    hits: Items<FilingHit>,
}
#[derive(Deserialize)]
struct FilingHit {
    #[serde(rename = "_id")]
    id: String,
    #[serde(rename = "_source")]
    source: Filing,
}
#[derive(Deserialize)]
struct Filing {
    #[serde(default)]
    cik: String,
    #[serde(default)]
    display_names: Vec<String>,
    file_type: Option<String>,
    file_date: Option<String>,
    file_description: Option<String>,
}
