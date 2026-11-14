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

// Derived from dsh-free-search 6ab5168 (MIT), lib/engine_meta.js.

use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum WebSearchProvider {
    #[default]
    Auto,
    Multi,
    #[serde(rename = "duckduckgo")]
    Duckduckgo,
    DdgLite,
    Bing,
    Searxng,
    Degoog,
    Anysearch,
    Exa,
    Tavily,
    Keenable,
    Firecrawl,
    Parallel,
    Perplexity,
    Serpbase,
    Serply,
    Outcrawl,
    Google,
    Metaso,
    Yandex,
    Yuanbao,
    DeepseekOfficial,
    DeepseekTools,
    You,
    Baidu,
    Kimi,
    Aliyun,
    Doubao,
    Tinyfish,
    Jina,
    #[serde(rename = "zhihu_global")]
    ZhihuGlobal,
    #[serde(rename = "zhihu_site")]
    ZhihuSite,
    Agy,
    Openai,
    Gemini,
    Claude,
    BaiduHtml,
    So360,
    Sogou,
    Wechat,
    Toutiao,
}

impl WebSearchProvider {
    pub const ALL: &'static [Self] = &[
        Self::Duckduckgo,
        Self::DdgLite,
        Self::Bing,
        Self::Searxng,
        Self::Degoog,
        Self::Anysearch,
        Self::Exa,
        Self::Tavily,
        Self::Keenable,
        Self::Firecrawl,
        Self::Parallel,
        Self::Perplexity,
        Self::Serpbase,
        Self::Serply,
        Self::Outcrawl,
        Self::Google,
        Self::Metaso,
        Self::Yandex,
        Self::Yuanbao,
        Self::DeepseekOfficial,
        Self::DeepseekTools,
        Self::You,
        Self::Baidu,
        Self::Kimi,
        Self::Aliyun,
        Self::Doubao,
        Self::Tinyfish,
        Self::Jina,
        Self::ZhihuGlobal,
        Self::ZhihuSite,
        Self::Agy,
        Self::Openai,
        Self::Gemini,
        Self::Claude,
        Self::BaiduHtml,
        Self::So360,
        Self::Sogou,
        Self::Wechat,
        Self::Toutiao,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Multi => "multi",
            Self::Duckduckgo => "duckduckgo",
            Self::DdgLite => "ddg-lite",
            Self::Bing => "bing",
            Self::Searxng => "searxng",
            Self::Degoog => "degoog",
            Self::Anysearch => "anysearch",
            Self::Exa => "exa",
            Self::Tavily => "tavily",
            Self::Keenable => "keenable",
            Self::Firecrawl => "firecrawl",
            Self::Parallel => "parallel",
            Self::Perplexity => "perplexity",
            Self::Serpbase => "serpbase",
            Self::Serply => "serply",
            Self::Outcrawl => "outcrawl",
            Self::Google => "google",
            Self::Metaso => "metaso",
            Self::Yandex => "yandex",
            Self::Yuanbao => "yuanbao",
            Self::DeepseekOfficial => "deepseek-official",
            Self::DeepseekTools => "deepseek-tools",
            Self::You => "you",
            Self::Baidu => "baidu",
            Self::Kimi => "kimi",
            Self::Aliyun => "aliyun",
            Self::Doubao => "doubao",
            Self::Tinyfish => "tinyfish",
            Self::Jina => "jina",
            Self::ZhihuGlobal => "zhihu_global",
            Self::ZhihuSite => "zhihu_site",
            Self::Agy => "agy",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::BaiduHtml => "baidu-html",
            Self::So360 => "so360",
            Self::Sogou => "sogou",
            Self::Wechat => "wechat",
            Self::Toutiao => "toutiao",
        }
    }

    pub const fn explicit_only(self) -> bool {
        matches!(
            self,
            Self::Metaso
                | Self::Yandex
                | Self::Yuanbao
                | Self::Openai
                | Self::Gemini
                | Self::Claude
        )
    }

    pub const fn serp(self) -> bool {
        matches!(
            self,
            Self::BaiduHtml | Self::So360 | Self::Sogou | Self::Wechat | Self::Toutiao
        )
    }

    pub const fn supports_time(self) -> bool {
        matches!(
            self,
            Self::Tavily
                | Self::Exa
                | Self::Keenable
                | Self::Firecrawl
                | Self::Google
                | Self::Parallel
                | Self::Searxng
                | Self::Degoog
                | Self::Duckduckgo
                | Self::DdgLite
                | Self::Baidu
                | Self::Yuanbao
                | Self::Doubao
                | Self::Tinyfish
                | Self::DeepseekTools
                | Self::ZhihuGlobal
                | Self::So360
                | Self::Sogou
        )
    }

    pub const fn keyless(self) -> bool {
        matches!(
            self,
            Self::Duckduckgo
                | Self::DdgLite
                | Self::Bing
                | Self::Searxng
                | Self::Degoog
                | Self::Anysearch
                | Self::Exa
                | Self::Tavily
                | Self::Keenable
                | Self::Firecrawl
                | Self::Parallel
                | Self::You
                | Self::BaiduHtml
                | Self::So360
                | Self::Sogou
                | Self::Wechat
                | Self::Toutiao
        )
    }

    pub const fn key_environment(self) -> &'static [&'static str] {
        match self {
            Self::Anysearch => &["ANYSEARCH_API_KEY"],
            Self::Exa => &["EXA_API_KEY"],
            Self::Tavily => &["TAVILY_API_KEY"],
            Self::Keenable => &["KEENABLE_API_KEY"],
            Self::Firecrawl => &["FIRECRAWL_API_KEY"],
            Self::Parallel => &["PARALLEL_API_KEY"],
            Self::Perplexity => &["PERPLEXITY_API_KEY"],
            Self::Serpbase => &["SERPBASE_API_KEY"],
            Self::Serply => &["SERPLY_API_KEY"],
            Self::Outcrawl => &["OUTCRAWL_API_KEY"],
            Self::Google => &["GOOGLE_CSE_API_KEY"],
            Self::Metaso => &["METASO_API_KEY"],
            Self::Yandex => &["YANDEX_API_KEY"],
            Self::Yuanbao => &["TENCENTCLOUD_WSA_APIKEY"],
            Self::DeepseekOfficial => &["DEEPSEEK_API_KEY"],
            Self::DeepseekTools => &["DEEPSEEK_TOOLS_API_KEY", "DEEPSEEK_API_KEY"],
            Self::You => &["YDC_API_KEY", "YOUCOM_API_KEY"],
            Self::Baidu => &["BAIDU_API_KEY"],
            Self::Kimi => &["MOONSHOT_API_KEY"],
            Self::Aliyun => &["DASHSCOPE_API_KEY"],
            Self::Doubao => &["DOUBAO_SEARCH_API_KEY", "WEB_SEARCH_API_KEY"],
            Self::Tinyfish => &["TINYFISH_API_KEY"],
            Self::Jina => &["JINA_API_KEY"],
            Self::ZhihuGlobal | Self::ZhihuSite => &["ZHIHU_API_KEY"],
            Self::Openai => &["OPENAI_API_KEY"],
            Self::Gemini => &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            Self::Claude => &["ANTHROPIC_API_KEY"],
            Self::Degoog => &["DEGOOG_API_KEY"],
            _ => &[],
        }
    }
}
