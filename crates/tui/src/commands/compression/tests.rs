use hanafubuki_agent::{AgentCompressionStats, AgentContentStoreStats};

use super::*;

fn text(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

fn status(content_store: Option<AgentContentStoreStats>) -> AgentCompressionStatus {
    AgentCompressionStatus {
        panel: "Blocks: 1 active / 1 total (12k tokens compressed, cumulative)\n  [b1] T1 12k→40: Setup"
            .to_string(),
        cache_report: "Cache report\n  1 request".to_string(),
        stats: AgentCompressionStats {
            tokens_compressed: 12_000,
            compression_count: 1,
            absorbed_tokens: 300,
            stored_count: 2,
            retrieval_count: 1,
        },
        content_store,
    }
}

#[test]
fn the_panel_ends_with_the_counters_and_the_content_stores_size() {
    let cell = CompressionCell::panel(&status(Some(AgentContentStoreStats {
        entries: 2,
        unique_contents: 1,
        total_chars: 9_000,
    })));

    assert_eq!(
        text(&cell.raw_lines()),
        [
            "Compression",
            "Blocks: 1 active / 1 total (12k tokens compressed, cumulative)",
            "  [b1] T1 12k→40: Setup",
            "",
            "Stats: 1 compress call(s) applied · 12K tokens compressed · 300 tokens absorbed · 2 result(s) stored · 1 retrieval(s)",
            "Content store: 2 entries (1 unique, 9000 characters)",
        ]
    );
    let shown = text(&cell.display_lines(60));
    assert_eq!(shown[0], "  Compression");
    assert!(
        shown.contains(&"    [b1] T1 12k→40: Setup".to_string()),
        "{shown:#?}"
    );
    assert!(
        shown.iter().all(|line| line.chars().count() <= 60),
        "{shown:#?}"
    );
}

#[test]
fn without_a_content_store_the_panel_has_no_line_for_it() {
    let cell = CompressionCell::panel(&status(None));

    assert!(!text(&cell.raw_lines())
        .iter()
        .any(|line| line.starts_with("Content store")));
}

#[test]
fn the_cache_report_shows_under_its_title() {
    let cell = CompressionCell::cache(&status(None));

    assert_eq!(
        text(&cell.raw_lines()),
        ["Compression cache", "Cache report", "  1 request"]
    );
}
