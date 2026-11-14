// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The footer's notice that a newer release exists (Codex's update
//! banner, on the footer's row here). The command line finds the release,
//! words the action for its installation and hands both over as text,
//! from its cache at launch ([`crate::TuiOptions::update_notice`]) or from
//! its check when that ends ([`TuiUpdateNotifier`]); the interface only
//! draws it, for the rest of the run.

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};

use crate::app_event::{AppEvent, AppEventSender};
use crate::appearance::AppearanceEvent;
use crate::style::accent_color;

/// A newer release, and what installs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiUpdateNotice {
    /// The release's version, as `0.3.0`.
    pub version: String,
    /// The command that installs it here, as `hanafubuki-agent update`.
    pub action: String,
}

/// Tells the running interface of a newer release; sent before the
/// interface's loop runs, it is shown at its first frame.
#[derive(Clone, Debug)]
pub struct TuiUpdateNotifier {
    sender: AppEventSender,
}

impl TuiUpdateNotifier {
    pub(crate) fn new(sender: AppEventSender) -> Self {
        Self { sender }
    }

    pub fn notify(&self, notice: TuiUpdateNotice) {
        self.sender
            .send(AppEvent::Appearance(AppearanceEvent::UpdateNotice(notice)));
    }
}

/// The notice in at most `room` columns: in full, else without its action,
/// else `None`.
pub(crate) fn notice_line(notice: &TuiUpdateNotice, room: usize) -> Option<Line<'static>> {
    let short = Line::from(vec![
        "Update available: ".dim(),
        Span::styled(notice.version.clone(), Style::default().fg(accent_color())),
    ]);
    let mut full = short.clone();
    full.push_span(format!(" · {}", notice.action).dim());
    [full, short].into_iter().find(|line| line.width() <= room)
}
