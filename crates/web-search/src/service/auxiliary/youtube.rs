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

//! Traverse YouTube's changing wrapper tree while decoding only video records.
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use std::fmt;

pub(super) struct VideoTree(pub Vec<Video>);
impl<'de> Deserialize<'de> for VideoTree {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        struct Walk;
        impl<'de> Visitor<'de> for Walk {
            type Value = VideoTree;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a video result tree")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut videos = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if key == "videoRenderer" {
                        #[derive(Deserialize)]
                        #[serde(untagged)]
                        enum Candidate {
                            Video(Box<Video>),
                            Other(de::IgnoredAny),
                        }
                        if let Candidate::Video(video) = map.next_value()? {
                            videos.push(*video);
                        }
                    } else {
                        videos.extend(map.next_value::<VideoTree>()?.0);
                    }
                }
                Ok(VideoTree(videos))
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut sequence: S,
            ) -> Result<Self::Value, S::Error> {
                let mut videos = Vec::new();
                while let Some(tree) = sequence.next_element::<VideoTree>()? {
                    videos.extend(tree.0);
                }
                Ok(VideoTree(videos))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(VideoTree(Vec::new()))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(VideoTree(Vec::new()))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(VideoTree(Vec::new()))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(VideoTree(Vec::new()))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(VideoTree(Vec::new()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(VideoTree(Vec::new()))
            }
        }
        decoder.deserialize_any(Walk)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Video {
    pub video_id: String,
    #[serde(default)]
    pub title: Text,
    #[serde(default)]
    pub owner_text: Text,
    #[serde(default)]
    pub long_byline_text: Text,
    #[serde(default)]
    pub length_text: Text,
    #[serde(default)]
    pub view_count_text: Text,
    #[serde(default)]
    pub published_time_text: Text,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Text {
    simple_text: Option<String>,
    #[serde(default)]
    runs: super::wire::Items<TextRun>,
}
#[derive(Deserialize)]
struct TextRun {
    text: String,
}
impl Text {
    pub fn text(&self) -> String {
        self.simple_text
            .clone()
            .unwrap_or_else(|| self.runs.0.iter().map(|r| r.text.as_str()).collect())
    }
}
