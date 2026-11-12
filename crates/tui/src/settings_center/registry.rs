// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The pages a generic row cannot be (Codex's sub-pages: the theme with its
//! preview, the status line, the title, the keymap's capture, the presets,
//! the plugin browser, the hook review, the memory store, a Provider's
//! model, an MCP server's login): each is a command's views, shown in the
//! center while they are open and nowhere else. A page is declared beside
//! its command (`commands::theme::PAGE`), and the center asks the registry
//! for the one a row has, falling back to the generic row.

use hanafubuki_agent::SettingsGroup;

use crate::commands::{CommandContext, CommandSpec, SETTINGS_PAGES};

/// A page that is a command's views.
pub(crate) struct PageOverride {
    /// What `/config` opens it by.
    pub(crate) id: &'static str,
    pub(crate) attach: Attach,
    /// The command whose views they are: their replies and next steps
    /// reach them as its.
    pub(crate) command: &'static CommandSpec,
    /// Shows the views; a record's page is given its record.
    pub(crate) open: fn(&mut CommandContext<'_>, Option<&str>),
}

impl std::fmt::Debug for PageOverride {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "page {}", self.id)
    }
}

/// Where a page is in the center.
pub(crate) enum Attach {
    /// In place of the editor of this setting's row.
    Setting(&'static str),
    /// A row of its own in a tab.
    Row {
        group: SettingsGroup,
        label: &'static str,
        summary: &'static str,
    },
    /// An action of each record of the collection `collection`.
    Record {
        collection: &'static str,
        label: &'static str,
    },
}

/// The page of the setting `key`, when it has one.
pub(super) fn page_of_setting(key: &str) -> Option<&'static PageOverride> {
    SETTINGS_PAGES
        .into_iter()
        .find(|page| matches!(page.attach, Attach::Setting(attached) if attached == key))
}

/// The pages that are rows of their own in `group`.
pub(super) fn row_pages(group: SettingsGroup) -> impl Iterator<Item = &'static PageOverride> {
    SETTINGS_PAGES.into_iter().filter(
        move |page| matches!(page.attach, Attach::Row { group: attached, .. } if attached == group),
    )
}

/// The pages that are actions of the records of `collection`.
pub(super) fn record_pages(collection: &str) -> impl Iterator<Item = &'static PageOverride> + '_ {
    SETTINGS_PAGES.into_iter().filter(move |page| {
        matches!(page.attach, Attach::Record { collection: attached, .. } if attached == collection)
    })
}

/// The page `/config` names by `id`.
pub(super) fn page(id: &str) -> Option<&'static PageOverride> {
    SETTINGS_PAGES.into_iter().find(|page| page.id == id)
}
