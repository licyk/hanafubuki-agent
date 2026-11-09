use super::*;

#[test]
fn every_command_is_found_by_its_name() {
    for command in SlashCommand::ALL {
        assert_eq!(SlashCommand::find(command.command()), Some(command));
    }
    assert_eq!(SlashCommand::find("mod"), None);
}

#[test]
fn a_prefix_is_a_case_insensitive_subsequence() {
    assert!(SlashCommand::has_prefix("mo"));
    assert!(SlashCommand::has_prefix("MCP"));
    assert!(SlashCommand::has_prefix("rsm"));
    assert!(!SlashCommand::has_prefix("zz"));
}
