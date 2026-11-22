//! Keyboard and pointer reference for the sidebar's chooser, tabs and readers.

use ratatui::{style::Stylize, text::Line};

use crate::keymap::{self, Action, Binding};
use crate::pager_overlay::Overlay;

pub(super) fn overlay() -> Overlay {
    let keymap = keymap::current();
    let bound = |action: Action, description| {
        let mut labels = Vec::new();
        for binding in keymap.bindings(action) {
            // The composer handles submission and queuing before its text editor sees the key.
            if action == Action::InsertNewline
                && [Action::Submit, Action::Queue]
                    .iter()
                    .any(|action| keymap.bindings(*action).contains(binding))
            {
                continue;
            }
            match binding {
                Binding::Leader(key) => {
                    for leader in keymap.bindings(Action::Leader) {
                        labels.push(format!("{} {}", leader.spec(), key.spec()));
                    }
                }
                binding => labels.push(binding.spec()),
            }
        }
        (!labels.is_empty()).then(|| shortcut(labels.join(" or "), description))
    };
    let mut lines = vec![
        Line::from("Open in sidebar").bold(),
        shortcut("/sidebar or +", "Open the chooser."),
        shortcut("↑/↓ or k/j", "Select a row. Enter or a click opens it."),
        shortcut(
            "PgUp/PgDn",
            "Move a page. Home/End selects the first or last row.",
        ),
        shortcut(
            "Wheel",
            "Scroll the list. Click a visible entry to open it.",
        ),
        shortcut(
            "p",
            "Show the selected file’s full path. Scroll to read it. Esc, p or Enter returns.",
        ),
        shortcut(
            "Esc",
            "Return to the last visited tab, or hide the chooser when no tab was visited.",
        ),
        Line::default(),
        Line::from("Tabs and panes").bold(),
        shortcut(
            "Click a tab",
            "Switch to it. Use ‹ and › for tabs beyond the strip, or scroll the wheel over a tab.",
        ),
        shortcut(
            "←/→",
            "Switch tabs from the chooser or an idle file reader.",
        ),
        shortcut(
            "Click a pane",
            "Give it keyboard focus. Drag the divider to resize the panes.",
        ),
    ];
    lines.extend(
        [
            bound(
                Action::ShowShortcuts,
                "Show this help from the chooser, a reader or an empty docked composer.",
            ),
            bound(Action::Close, "Close this help and return to the sidebar."),
            bound(
                Action::FocusSidebarTabs,
                "Open the chooser from a conversation.",
            ),
            bound(
                Action::SidebarUp,
                "Switch to the previous tab while the sidebar has focus.",
            ),
            bound(
                Action::SidebarDown,
                "Switch to the next tab while the sidebar has focus.",
            ),
            bound(Action::FocusPaneLeft, "Focus the main pane."),
            bound(Action::FocusPaneRight, "Focus the sidebar."),
            bound(
                Action::ToggleSideConversation,
                "Switch pane focus, or open the chooser when the sidebar is hidden.",
            ),
            bound(Action::MoveDividerLeft, "Widen the sidebar."),
            bound(Action::MoveDividerRight, "Narrow the sidebar."),
            bound(
                Action::EqualizePaneWidths,
                "Make the panes as close to equal width as they fit.",
            ),
            bound(
                Action::TogglePaneZoom,
                "Zoom the focused pane or restore the split.",
            ),
            bound(
                Action::HidePane,
                "Hide a docked sidebar and return to the main conversation.",
            ),
        ]
        .into_iter()
        .flatten(),
    );
    lines.extend([
        Line::from("Below 100 columns the focused pane fills the screen."),
        Line::default(),
        Line::from("Side chats").bold(),
        shortcut("New side chat", "Start a saved chat after the first message in the main conversation."),
        shortcut("Side chats", "Reopen a chat belonging to this parent. Each open chat keeps its draft and running turn when you switch tabs."),
        shortcut("/side [message]", "Open a temporary side conversation from the main chat. An optional message starts its turn."),
        shortcut("Esc", "Return to the chooser from an empty, idle docked chat."),
    ]);
    lines.extend([
        bound(Action::Submit, "Send the draft, or use it to steer a running turn."),
        bound(Action::Queue, "Queue the draft for after the running turn. In an idle chat this sends it immediately."),
        bound(Action::InsertNewline, "Insert a line break in the draft."),
        bound(Action::HistorySearch, "Search sent messages. Repeat this shortcut or press ↑ for an older match. ↓ or Ctrl+S finds a newer one. Enter keeps the match in the composer. Esc restores your draft."),
    ].into_iter().flatten());
    lines.extend([
        Line::default(),
        Line::from("Subagents").bold(),
        Line::from("Choose an agent to watch its live transcript. Scroll, search or select text there. Esc returns to the chooser when the reader is idle."),
        Line::default(),
        Line::from("Files").bold(),
        shortcut("Browse", "Open a workspace folder. Choose ../ to go up, or open a file mentioned in the main conversation."),
        Line::from("Open a text file to read it, or a PNG, JPEG, GIF or WebP file to preview the image."),
        shortcut("j / k", "Scroll down or up one row. Home and End jump to the start and end of the file."),
        shortcut("Tab / Shift+Tab", "Select the next or previous link in a text file. Enter or a click opens the link."),
        shortcut("Esc", "Clear a selected link or line highlight, then return to the chooser."),
        Line::default(),
        Line::from("Search and selection").bold(),
        shortcut("↑/↓ or wheel", "Scroll a text file or a watched subagent."),
        shortcut("/", "Find text. Type or paste a query and press Enter to read the match."),
        shortcut("Ctrl+N / Ctrl+P", "While searching, move to the next newer or older match."),
        shortcut("n / Shift+N", "After a search in keyboard selection, move to the next older or newer match."),
    ]);
    lines.extend(
        [
            bound(Action::FindTranscript, "Find text in the reader."),
            bound(Action::TranscriptPageUp, "Read the previous page."),
            bound(Action::TranscriptPageDown, "Read the next page."),
            bound(
                Action::StartSelection,
                "Start a selection at the top of the visible text. Extend it with the arrows.",
            ),
            bound(Action::SelectTranscript, "Enter keyboard selection."),
        ]
        .into_iter()
        .flatten(),
    );
    lines.extend([
        shortcut(
            "v or Space",
            "Mark text in keyboard selection, then extend with the arrows. V marks whole lines.",
        ),
        shortcut(
            "y or Ctrl+C",
            "Copy the marked text. Drag with the mouse and use Ctrl+C to copy a pointer selection.",
        ),
        shortcut(
            "Esc",
            "Clear the current search or selection before leaving the reader.",
        ),
        Line::default(),
        Line::from("Close and archive").bold(),
        shortcut(
            "× or middle-click a tab",
            "Close that tab. A closed saved chat can be reopened from Side chats.",
        ),
    ]);
    lines.extend(bound(Action::CloseSidebarTab, "Close the selected tab."));
    lines.extend([
        shortcut("Delete", "Archive the highlighted chat in the chooser, removing it from Side chats."),
        Line::from("The confirmation lists what closing or archiving will discard. Cancel is selected first."),
        shortcut("←/→ or Tab", "Choose Cancel or Close/Archive. Enter activates the selection. Esc cancels. You can also click a button."),
    ]);
    Overlay::new_static_with_lines(lines, "Sidebar help".to_owned())
}

fn shortcut(keys: impl Into<String>, description: &'static str) -> Line<'static> {
    Line::from(vec![keys.into().bold(), "  ".into(), description.into()])
}
