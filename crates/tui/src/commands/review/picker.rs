// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The review picker, one view through its pages (Codex's preset menu,
//! branch and commit pickers and custom prompt): the presets; the base
//! branches, the current one marked; the latest commits; the instructions.
//! Picking a target starts the review; Esc on a page goes back to the
//! presets, and on the presets closes the picker.

use hanafubuki_agent::{AgentReviewTarget, AgentReviewTargets};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;

use super::start;
use crate::app_event::AppEventSender;
use crate::bottom_pane::{
    BottomPaneView, CancellationEvent, ListSelectionView, SelectionItem, SelectionViewParams,
};
use crate::commands::prompt::PromptView;
use crate::render::renderable::Renderable;

/// The presets that open a page, by their place (uncommitted changes
/// come first).
const BASE_BRANCH: usize = 1;
const COMMIT: usize = 2;
const CUSTOM: usize = 3;

pub(super) struct ReviewPicker {
    tx: AppEventSender,
    /// What the base branch and the commit are chosen from, or why they
    /// cannot be.
    targets: Result<AgentReviewTargets, String>,
    page: Page,
    done: bool,
}

enum Page {
    Presets(ListSelectionView),
    Choice(ListSelectionView),
    Custom(PromptView),
}

impl ReviewPicker {
    pub(super) fn new(tx: AppEventSender, targets: Result<AgentReviewTargets, String>) -> Self {
        let page = Page::Presets(presets(&tx, &targets));
        Self {
            tx,
            targets,
            page,
            done: false,
        }
    }

    fn branches(&self, targets: &AgentReviewTargets) -> ListSelectionView {
        let current = targets.current_branch.as_deref();
        let items = targets
            .branches
            .iter()
            .map(|branch| {
                let target = AgentReviewTarget::BaseBranch {
                    branch: branch.clone(),
                };
                SelectionItem {
                    name: branch.clone(),
                    is_current: Some(branch.as_str()) == current,
                    search_value: Some(branch.clone()),
                    actions: vec![Box::new(move |tx| start(tx, target.clone()))],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();
        ListSelectionView::new(
            SelectionViewParams {
                title: Some("Select a base branch".to_string()),
                subtitle: Some(format!(
                    "Current branch: {}",
                    current.unwrap_or("(detached HEAD)")
                )),
                items,
                is_searchable: true,
                search_placeholder: Some("Type to search branches".to_string()),
                ..Default::default()
            },
            self.tx.clone(),
        )
    }

    fn commits(&self, targets: &AgentReviewTargets) -> ListSelectionView {
        let items = targets
            .commits
            .iter()
            .map(|commit| {
                let target = AgentReviewTarget::Commit {
                    sha: commit.sha.clone(),
                    title: Some(commit.subject.clone()),
                };
                let short_sha: String = commit.sha.chars().take(7).collect();
                SelectionItem {
                    name: commit.subject.clone(),
                    description: Some(short_sha),
                    search_value: Some(format!("{} {}", commit.subject, commit.sha)),
                    actions: vec![Box::new(move |tx| start(tx, target.clone()))],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();
        ListSelectionView::new(
            SelectionViewParams {
                title: Some("Select a commit to review".to_string()),
                items,
                is_searchable: true,
                search_placeholder: Some("Type to search commits".to_string()),
                ..Default::default()
            },
            self.tx.clone(),
        )
    }

    fn custom(&self) -> PromptView {
        PromptView::new(
            "Custom review instructions",
            "Type instructions and press Enter",
            self.tx.clone(),
            Box::new(|tx, instructions| start(tx, AgentReviewTarget::Custom { instructions })),
        )
    }

    /// The page a preset opens; `None` when it started the review.
    fn open(&self, preset: usize) -> Option<Page> {
        let targets = self.targets.as_ref().ok();
        match preset {
            BASE_BRANCH => targets.map(|targets| Page::Choice(self.branches(targets))),
            COMMIT => targets.map(|targets| Page::Choice(self.commits(targets))),
            CUSTOM => Some(Page::Custom(self.custom())),
            _ => None,
        }
    }

    fn view(&self) -> &dyn BottomPaneView {
        match &self.page {
            Page::Presets(list) | Page::Choice(list) => list,
            Page::Custom(prompt) => prompt,
        }
    }

    fn view_mut(&mut self) -> &mut dyn BottomPaneView {
        match &mut self.page {
            Page::Presets(list) | Page::Choice(list) => list,
            Page::Custom(prompt) => prompt,
        }
    }
}

impl BottomPaneView for ReviewPicker {
    fn handle_key_event(&mut self, key: KeyEvent) {
        let esc = key.code == KeyCode::Esc && key.kind != KeyEventKind::Release;
        if esc && !matches!(self.page, Page::Presets(_)) {
            self.page = Page::Presets(presets(&self.tx, &self.targets));
            return;
        }
        self.view_mut().handle_key_event(key);
        let picked = match &mut self.page {
            Page::Presets(list) => list.take_last_selected_index(),
            _ => None,
        };
        if let Some(page) = picked.and_then(|preset| self.open(preset)) {
            self.page = page;
            return;
        }
        self.done = self.view().is_complete();
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        self.view_mut().handle_paste(pasted)
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.done = true;
        CancellationEvent::Handled
    }
}

impl Renderable for ReviewPicker {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.view().render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.view().desired_height(width)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.view().cursor_pos(area)
    }
}

/// The presets: a base branch and a commit are offered only when the
/// repository's targets were read.
fn presets(tx: &AppEventSender, targets: &Result<AgentReviewTargets, String>) -> ListSelectionView {
    let unavailable = targets.as_ref().err().cloned();
    let target_item = |name: &str| SelectionItem {
        name: name.to_string(),
        disabled_reason: unavailable.clone(),
        ..Default::default()
    };
    let items = vec![
        SelectionItem {
            name: "Review uncommitted changes".to_string(),
            description: Some("staged, unstaged and untracked".to_string()),
            actions: vec![Box::new(|tx| {
                start(tx, AgentReviewTarget::UncommittedChanges)
            })],
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            description: Some("(PR style)".to_string()),
            ..target_item("Review against a base branch")
        },
        target_item("Review a commit"),
        SelectionItem {
            name: "Custom review instructions".to_string(),
            ..Default::default()
        },
    ];
    ListSelectionView::new(
        SelectionViewParams {
            title: Some("Select a review preset".to_string()),
            subtitle: Some(
                "The review runs read-only in a child session and reports back here.".to_string(),
            ),
            items,
            ..Default::default()
        },
        tx.clone(),
    )
}
