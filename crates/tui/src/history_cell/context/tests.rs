use hanafubuki_agent::{
    AgentContextSection, AgentContextTools, AgentSentRequest, AgentUsageSource,
};

use super::*;

fn render(cell: &ContextHistoryCell, width: u16) -> String {
    let lines: Vec<String> = cell
        .display_lines(width)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();
    for line in &lines {
        assert!(
            line.width() <= usize::from(width),
            "wider than {width}: {line:?}"
        );
    }
    lines.join("\n")
}

/// A request with every group: instructions, tools of the kernel and of an
/// MCP server, two sections, the conversation, a summary and an image.
fn composition() -> AgentContextComposition {
    AgentContextComposition {
        instructions_tokens: 6_200,
        tools: vec![
            AgentContextTools {
                provider_id: Some("builtin".to_string()),
                count: 14,
                tokens: 9_800,
            },
            AgentContextTools {
                provider_id: Some("mcp.github".to_string()),
                count: 3,
                tokens: 2_100,
            },
            AgentContextTools {
                provider_id: None,
                count: 1,
                tokens: 300,
            },
        ],
        sections: vec![
            AgentContextSection {
                kind: "environment_context".to_string(),
                tokens: 450,
            },
            AgentContextSection {
                kind: "workspace_instructions_of_a_long_named_plugin".to_string(),
                tokens: 3_900,
            },
        ],
        user_messages_tokens: 4_100,
        assistant_messages_tokens: 12_500,
        reasoning_tokens: 2_300,
        tool_calls_tokens: 31_000,
        continuation_tokens: 120,
        compaction_summaries_tokens: 1_800,
        images: 2,
        image_tokens: 850,
        other_tokens: 60,
        total_tokens: 75_480,
    }
}

fn usage(cache: bool) -> AgentModelUsage {
    AgentModelUsage {
        last_response_input_tokens: 74_000,
        last_response_output_tokens: 900,
        cumulative_input_tokens: 410_000,
        cumulative_output_tokens: 5_200,
        context_window_tokens: Some(200_000),
        context_consumed_tokens: Some(74_000),
        source: AgentUsageSource::ProviderReported,
        last_response_cache_read_tokens: cache.then_some(61_000),
        last_response_cache_write_tokens: cache.then_some(2_000),
        cumulative_cache_read_tokens: if cache { 300_000 } else { 0 },
        cumulative_cache_write_tokens: if cache { 9_000 } else { 0 },
        cache_reporting_responses: if cache { 6 } else { 0 },
    }
}

fn sent(cache: bool) -> AgentSessionContext {
    AgentSessionContext {
        request: AgentContextRequest::Sent(AgentSentRequest {
            composition: composition(),
            captured_at_ms: 1,
            reported: Some(AgentReportedPrompt {
                input_tokens: 74_000,
                cache_read_tokens: cache.then_some(61_000),
                cache_write_tokens: cache.then_some(2_000),
            }),
        }),
        context_window_tokens: Some(200_000),
        usage: Some(usage(cache)),
    }
}

#[test]
fn context_with_the_cache_at_100_columns() {
    insta::assert_snapshot!(render(&new_context_output(&sent(true)), 100));
}

#[test]
fn context_with_the_cache_at_50_columns() {
    insta::assert_snapshot!(render(&new_context_output(&sent(true)), 50));
}

/// A Provider that reports no cache figure: "not reported", never 0%.
#[test]
fn context_without_cache_reporting_at_100_columns() {
    insta::assert_snapshot!(render(&new_context_output(&sent(false)), 100));
}

#[test]
fn context_without_cache_reporting_at_50_columns() {
    insta::assert_snapshot!(render(&new_context_output(&sent(false)), 50));
}

/// Before any request: the next one's estimate, labelled as such, with no
/// window known (each share is of the request, no free row) and no usage.
#[test]
fn context_before_any_request_with_no_window() {
    let context = AgentSessionContext {
        request: AgentContextRequest::Next(AgentContextComposition {
            instructions_tokens: 6_200,
            tools: vec![AgentContextTools {
                provider_id: Some("builtin".to_string()),
                count: 14,
                tokens: 9_800,
            }],
            other_tokens: 40,
            total_tokens: 16_040,
            ..AgentContextComposition::default()
        }),
        context_window_tokens: None,
        usage: None,
    };
    insta::assert_snapshot!(render(&new_context_output(&context), 80));
}

/// While the response streams the Provider has not counted the request;
/// a reported zero read is a 0% rate, not "not reported".
#[test]
fn context_during_a_response_and_with_a_cache_miss() {
    let with_reported = |reported| {
        let mut context = sent(true);
        let AgentContextRequest::Sent(request) = &mut context.request else {
            unreachable!()
        };
        request.reported = reported;
        render(&new_context_output(&context), 100)
    };
    let during = with_reported(None);
    assert!(during.contains("no count from the Provider"), "{during}");
    assert!(
        during.contains("Cached tokens: last not reported · session 300k"),
        "{during}"
    );

    let miss = with_reported(Some(AgentReportedPrompt {
        input_tokens: 74_000,
        cache_read_tokens: Some(0),
        cache_write_tokens: None,
    }));
    assert!(
        miss.contains("Cache hit rate: last 0.0% · session 73.2%"),
        "{miss}"
    );
}

/// No request, and none could be estimated: the reason, and the cache.
#[test]
fn context_when_the_next_request_cannot_be_built() {
    let context = AgentSessionContext {
        request: AgentContextRequest::Unavailable("no model is selected".to_string()),
        context_window_tokens: Some(200_000),
        usage: None,
    };
    let rendered = render(&new_context_output(&context), 100);
    assert!(
        rendered.contains("could not be estimated: no model"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Cache hit rate: last (no request yet)"),
        "{rendered}"
    );
    assert!(!rendered.contains('█'), "{rendered}");
}

/// Too narrow for a bar: the rows keep their label, percent and tokens.
#[test]
fn context_at_a_width_too_narrow_for_bars() {
    let rendered = render(&new_context_output(&sent(true)), 28);
    assert!(!rendered.contains('░'), "{rendered}");
    assert!(rendered.contains("Free"), "{rendered}");
}
