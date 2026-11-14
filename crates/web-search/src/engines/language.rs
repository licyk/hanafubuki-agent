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

use super::SearchOptions;
impl SearchOptions {
    pub(crate) fn market(&self) -> &str {
        if !self.bing_market.is_empty() {
            return &self.bing_market;
        }
        match self.language.as_str() {
            "zh" => "zh-CN",
            "ru" => "ru-RU",
            "ja" => "ja-JP",
            "de" => "de-DE",
            "fr" => "fr-FR",
            "es" => "es-ES",
            "ko" => "ko-KR",
            _ => "en-US",
        }
    }
    pub(crate) fn accept_language(&self) -> String {
        let market = self.market();
        let language = market.split('-').next().unwrap_or("en");
        format!("{market},{language};q=0.9,en;q=0.8")
    }
}
