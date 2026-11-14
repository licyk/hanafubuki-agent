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

// Derived from dsh-free-search 6ab5168 (MIT), WMO labels in lib/weather.js.

pub(super) fn description(code: Option<u8>, language: &str) -> String {
    let labels = match code {
        Some(0) => ["晴", "Clear sky", "快晴"],
        Some(1) => ["大部晴朗", "Mainly clear", "晴れ"],
        Some(2) => ["多云", "Partly cloudy", "晴れ時々曇り"],
        Some(3) => ["阴", "Overcast", "曇り"],
        Some(45) => ["雾", "Fog", "霧"],
        Some(48) => ["雾凇", "Depositing rime fog", "霧（霧氷を伴う）"],
        Some(51) => ["毛毛雨（弱）", "Light drizzle", "弱い霧雨"],
        Some(53) => ["毛毛雨", "Moderate drizzle", "霧雨"],
        Some(55) => ["毛毛雨（强）", "Dense drizzle", "強い霧雨"],
        Some(56) => [
            "冻毛毛雨（弱）",
            "Light freezing drizzle",
            "弱い着氷性の霧雨",
        ],
        Some(57) => ["冻毛毛雨（强）", "Dense freezing drizzle", "着氷性の霧雨"],
        Some(61) => ["小雨", "Slight rain", "弱い雨"],
        Some(63) => ["中雨", "Moderate rain", "雨"],
        Some(65) => ["大雨", "Heavy rain", "強い雨"],
        Some(66) => ["冻雨（弱）", "Light freezing rain", "弱い着氷性の雨"],
        Some(67) => ["冻雨（强）", "Heavy freezing rain", "着氷性の雨"],
        Some(71) => ["小雪", "Slight snow", "弱い雪"],
        Some(73) => ["中雪", "Moderate snow", "雪"],
        Some(75) => ["大雪", "Heavy snow", "強い雪"],
        Some(77) => ["雪粒", "Snow grains", "霧雪"],
        Some(80) => ["阵雨（弱）", "Slight rain showers", "弱いにわか雨"],
        Some(81) => ["阵雨", "Moderate rain showers", "にわか雨"],
        Some(82) => ["阵雨（强）", "Violent rain showers", "激しいにわか雨"],
        Some(85) => ["阵雪（弱）", "Slight snow showers", "弱いにわか雪"],
        Some(86) => ["阵雪（强）", "Heavy snow showers", "強いにわか雪"],
        Some(95) => ["雷暴", "Thunderstorm", "雷雨"],
        Some(96) => [
            "雷暴伴小冰雹",
            "Thunderstorm with slight hail",
            "雷雨（ひょうを伴う）",
        ],
        Some(99) => [
            "雷暴伴大冰雹",
            "Thunderstorm with heavy hail",
            "激しい雷雨（ひょうを伴う）",
        ],
        _ => ["未知", "Unknown", "不明"],
    };
    let text = labels[match language {
        "zh" => 0,
        "ja" => 2,
        _ => 1,
    }];
    code.map(|c| format!("{text} (WMO {c})"))
        .unwrap_or_else(|| text.into())
}
