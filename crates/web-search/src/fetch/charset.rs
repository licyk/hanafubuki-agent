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

//! Decode declared HTML/text encodings before projecting text. Providers in
//! Chinese still return GBK pages; treating them as UTF-8 destroys evidence.
use encoding_rs::{Encoding, UTF_8};
pub(super) fn decode(body: &[u8], content_type: &str) -> Vec<u8> {
    let declared = content_type.split(';').find_map(|part| {
        part.trim()
            .split_once('=')
            .filter(|(key, _)| key.eq_ignore_ascii_case("charset"))
            .map(|(_, v)| v.trim().trim_matches(['\'', '"']))
    });
    let header = String::from_utf8_lossy(&body[..body.len().min(8192)]);
    let meta = regex::Regex::new(r#"(?i)charset\s*=\s*["']?([a-z0-9_-]+)"#)
        .ok()
        .and_then(|r| {
            r.captures(&header)
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        });
    let encoding = declared
        .or(meta.as_deref())
        .and_then(|name| Encoding::for_label(name.as_bytes()))
        .unwrap_or(UTF_8);
    encoding.decode(body).0.into_owned().into_bytes()
}

#[cfg(test)]
mod tests;
