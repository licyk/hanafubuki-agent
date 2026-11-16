// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::path::PathBuf;

use ratatui::style::Style;
use ratatui::style::Styled;
use ratatui::text::Span;

const TAG_WIDTH: usize = "Plugin".len();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    File {
        path: PathBuf,
        absolute_path: PathBuf,
    },
    Tool {
        insert_text: String,
        path: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MentionType {
    Plugin,
    Skill,
    Task,
    File,
    Directory,
}

impl MentionType {
    pub(crate) fn is_filesystem(self) -> bool {
        matches!(self, Self::File | Self::Directory)
    }

    pub(crate) fn span(self, base_style: Style) -> Span<'static> {
        let style = match self {
            Self::Plugin => base_style.fg(crate::style::accent_color()),
            Self::Skill => base_style.dim(),
            Self::Task => base_style.cyan(),
            Self::File => base_style.cyan(),
            Self::Directory => base_style,
        };
        format!("{:<width$}", self.label(), width = TAG_WIDTH).set_style(style)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Plugin => "Plugin",
            Self::Skill => "Skill",
            Self::Task => "Task",
            Self::File => "File",
            Self::Directory => "Dir",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) display_name: String,
    pub(crate) description: Option<String>,
    pub(crate) search_terms: Vec<String>,
    pub(crate) mention_type: MentionType,
    pub(crate) selection: Selection,
}

#[derive(Clone, Debug)]
pub(crate) struct SearchResult {
    pub(crate) display_name: String,
    pub(crate) description: Option<String>,
    pub(crate) mention_type: MentionType,
    pub(crate) selection: Selection,
    pub(crate) match_indices: Option<Vec<usize>>,
    pub(crate) score: i32,
}

impl Candidate {
    pub(crate) fn to_result(&self, match_indices: Option<Vec<usize>>, score: i32) -> SearchResult {
        SearchResult {
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            mention_type: self.mention_type,
            selection: self.selection.clone(),
            match_indices,
            score,
        }
    }
}
