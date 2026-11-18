use super::*;

#[test]
fn provisional_title_normalizes_and_bounds_unicode_by_characters() {
    assert_eq!(
        provisional_agent_title("  分析\n\n任务中心  的接口  "),
        Some("分析 任务中心 的接口".to_string())
    );
    assert_eq!(
        provisional_agent_title(&"界".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS + 4)),
        Some("界".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS))
    );
    let prefix = "界".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS - 1);
    assert_eq!(
        provisional_agent_title(&format!("{prefix} next")),
        Some(prefix)
    );
    assert_eq!(provisional_agent_title(" \n\t "), None);
}
