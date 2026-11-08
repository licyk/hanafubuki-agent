use super::markdown::{normalize_blank_lines, truncate_utf8, MarkdownRenderer};
use crate::model::{WebFetchError, MAX_WEB_SEARCH_TITLE_BYTES};
use dom_query::Document;
use reqwest::Url;

pub(super) fn extract_html(
    body: &[u8],
    base_url: &Url,
    maximum_content_bytes: usize,
) -> Result<(Option<String>, String, bool), WebFetchError> {
    let html = String::from_utf8_lossy(body);
    let document = Document::from(html.as_ref());
    document
        .select("script,style,noscript,template,svg,canvas,form,nav,footer,aside")
        .remove();
    let title = document.select("title").first().text().trim().to_string();
    let title = (!title.is_empty()).then(|| truncate_utf8(&title, MAX_WEB_SEARCH_TITLE_BYTES).0);

    let mut best = None;
    for selector in ["article", "main", "[role=main]", "body"] {
        for node in document.select(selector).nodes() {
            let length = node.formatted_text().len();
            if best
                .as_ref()
                .is_none_or(|(_, best_length)| length > *best_length)
            {
                best = Some((*node, length));
            }
        }
        if best.as_ref().is_some_and(|(_, length)| *length >= 400) {
            break;
        }
    }
    let Some((root, _)) = best else {
        return Err(WebFetchError::EmptyContent);
    };
    let mut renderer = MarkdownRenderer::new(base_url, maximum_content_bytes);
    renderer.render(root, 0);
    let content = normalize_blank_lines(renderer.output.trim());
    if content.is_empty() {
        return Err(WebFetchError::EmptyContent);
    }
    Ok((title, content, renderer.truncated))
}
