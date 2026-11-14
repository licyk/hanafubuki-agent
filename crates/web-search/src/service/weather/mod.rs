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

// Derived from dsh-free-search 6ab5168 (MIT), lib/weather.js.

use super::WebSearchService;
use crate::WebSearchError;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherReport {
    pub location: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
    pub current: CurrentWeather,
    pub daily: Vec<DailyWeather>,
    pub sources: [String; 1],
    pub untrusted_external_content: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentWeather {
    pub temperature: Option<f64>,
    pub apparent: Option<f64>,
    pub humidity: Option<f64>,
    pub wind: Option<f64>,
    pub code: Option<u8>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyWeather {
    pub date: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub precipitation_probability: Option<f64>,
    pub code: Option<u8>,
    pub text: String,
}
#[derive(Deserialize)]
struct Geocoding {
    #[serde(default)]
    results: Vec<Location>,
}
#[derive(Deserialize)]
struct Location {
    name: String,
    #[serde(default)]
    admin1: String,
    #[serde(default)]
    country: String,
    latitude: f64,
    longitude: f64,
}
#[derive(Deserialize)]
struct Forecast {
    #[serde(default)]
    current: Current,
    #[serde(default)]
    daily: Daily,
    #[serde(default)]
    timezone: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Current {
    temperature_2m: Option<f64>,
    apparent_temperature: Option<f64>,
    relative_humidity_2m: Option<f64>,
    wind_speed_10m: Option<f64>,
    weather_code: Option<u8>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Daily {
    time: Vec<String>,
    temperature_2m_min: Vec<Option<f64>>,
    temperature_2m_max: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<f64>>,
    weather_code: Vec<Option<u8>>,
}

impl WebSearchService {
    pub async fn weather(
        &self,
        location: &str,
        days: usize,
        cancellation: CancellationToken,
    ) -> Result<WeatherReport, WebSearchError> {
        let language = &self.inner.config.search.language;
        let body = self
            .send_bounded(
                self.inner
                    .client
                    .get("https://geocoding-api.open-meteo.com/v1/search")
                    .query(&[
                        ("name", location),
                        ("count", "1"),
                        ("language", language),
                        ("format", "json"),
                    ]),
                &cancellation,
            )
            .await?;
        let locations: Geocoding =
            serde_json::from_slice(&body).map_err(|_| WebSearchError::InvalidResponse)?;
        let geo =
            locations
                .results
                .into_iter()
                .next()
                .ok_or(WebSearchError::InvalidConfiguration(
                    "location was not found; try a nearby city",
                ))?;
        let body = self.send_bounded(self.inner.client.get("https://api.open-meteo.com/v1/forecast").query(&[
            ("latitude", geo.latitude.to_string()), ("longitude", geo.longitude.to_string()),
            ("current", "temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m".into()),
            ("daily", "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max".into()),
            ("timezone", "auto".into()), ("forecast_days", days.clamp(1,7).to_string()),
        ]), &cancellation).await?;
        let data: Forecast =
            serde_json::from_slice(&body).map_err(|_| WebSearchError::InvalidResponse)?;
        let current = data.current;
        let daily = data
            .daily
            .time
            .iter()
            .enumerate()
            .map(|(i, date)| {
                let code = data.daily.weather_code.get(i).copied().flatten();
                DailyWeather {
                    date: date.clone(),
                    min: data.daily.temperature_2m_min.get(i).copied().flatten(),
                    max: data.daily.temperature_2m_max.get(i).copied().flatten(),
                    precipitation_probability: data
                        .daily
                        .precipitation_probability_max
                        .get(i)
                        .copied()
                        .flatten(),
                    code,
                    text: codes::description(code, language),
                }
            })
            .collect();
        Ok(WeatherReport {
            location: [geo.name, geo.admin1, geo.country]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            latitude: geo.latitude,
            longitude: geo.longitude,
            timezone: data.timezone,
            current: CurrentWeather {
                temperature: current.temperature_2m,
                apparent: current.apparent_temperature,
                humidity: current.relative_humidity_2m,
                wind: current.wind_speed_10m,
                code: current.weather_code,
                text: codes::description(current.weather_code, language),
            },
            daily,
            sources: ["https://open-meteo.com/".into()],
            untrusted_external_content: true,
        })
    }
}
mod codes;
