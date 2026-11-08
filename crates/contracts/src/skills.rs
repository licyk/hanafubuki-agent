//! The Skills a turn may select, as the kernel reads them at admission.

/// The Skills a turn's user input may select, explicitly or by a `$mention`
/// of a Skill's name or folder. A turn admission contributor puts it in the
/// turn's extensions; a turn without it selects no Skill.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectableSkills {
    pub skills: Vec<SelectableSkill>,
    /// The names a plain `$mention` names instead of a Skill (ASCII
    /// case-insensitive): a tool the turn can call or a visible MCP server
    /// or tool. The kernel adds its own tools' names; an explicit reference
    /// or a `[$name](skill://<id>)` link still selects.
    pub mention_exclusions: Vec<String>,
}

/// One Skill a turn may select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectableSkill {
    /// The Skill id (`<scope>:<path>`).
    pub id: String,
    /// The display name a `$mention` may use.
    pub name: String,
    /// The instructions committed to the conversation when the Skill is
    /// selected.
    pub context: String,
}
