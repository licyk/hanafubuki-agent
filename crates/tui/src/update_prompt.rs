// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The update prompt (site/content/docs/terminal-interface.en.mdx,
//! "Updates"): the screen the command line shows at launch, before the
//! Agent starts, when a release newer than this build is known. Codex's
//! screen and keys: update now, skip, or skip until the next version.
//!
//! It owns the terminal while it is shown: the modes are set, the keys
//! typed before it appeared are dropped, and once a choice is made the
//! screen is cleared and the terminal restored, however it returns. It is
//! drawn in the inline viewport at the cursor, the screen's height, before
//! the interface chooses its presentation. The command line words every
//! text; the interface reads nothing of the update itself.

use std::io::{self, Write};
use std::sync::mpsc::{channel, Receiver};

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget, Wrap};

use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::render_menu_surface;
use crate::key_hint;
use crate::render::renderable::{Renderable, RenderableExt, RenderableItem};
use crate::render::{Insets, RectExt};
use crate::style::accent_color;
use crate::terminal_hyperlinks::{mark_hyperlink_cells, web_destination};
use crate::tui::{self, InputReader, RestoreOnDrop, Tui};
use crate::wrapping::{word_wrap_lines, RtOptions};

/// What the prompt shows, worded by the command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiUpdatePrompt {
    /// This build's version.
    pub current: String,
    /// The newer release's version.
    pub latest: String,
    /// The address of its release notes.
    pub release_notes: String,
    /// The first option, saying what it does: `Update now (…)`.
    pub update_now: String,
}

/// What the person chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiUpdateChoice {
    UpdateNow,
    /// Also Esc, Ctrl-C and Ctrl-D: ask again at the next launch.
    Skip,
    /// Do not ask again until a release newer than this one.
    SkipVersion,
}

const CHOICES: [TuiUpdateChoice; 3] = [
    TuiUpdateChoice::UpdateNow,
    TuiUpdateChoice::Skip,
    TuiUpdateChoice::SkipVersion,
];

/// Shows the prompt until a choice is made. Standard input and output must
/// be the terminal; the process's panic hook must be installed already.
/// The terminal is restored and the screen cleared on return.
pub fn update_prompt(prompt: TuiUpdatePrompt) -> Result<TuiUpdateChoice, String> {
    let terminal = tui::init().map_err(|error| format!("the terminal: {error}"))?;
    let restore = RestoreOnDrop;
    let mut tui = Tui::new(terminal);
    // An Enter typed while the program started is not a choice.
    tui::discard_input();
    let (sender, events) = channel();
    tui.set_input(
        InputReader::start(AppEventSender::new(sender))
            .map_err(|error| format!("the terminal's input: {error}"))?,
    );
    let chosen = choose(&mut tui, &events, &mut UpdatePromptScreen::new(prompt));
    tui.stop_input();
    if chosen.is_err() {
        let _ = tui.leave();
    }
    drop(restore);
    chosen.map_err(|error| format!("the terminal: {error}"))
}

/// The prompt on `tui`, read from `events`, for the tests of what follows
/// it.
#[cfg(test)]
pub(crate) fn choose_on<B: Backend<Error = io::Error> + Write>(
    tui: &mut Tui<B>,
    events: &Receiver<AppEvent>,
    prompt: TuiUpdatePrompt,
) -> io::Result<TuiUpdateChoice> {
    choose(tui, events, &mut UpdatePromptScreen::new(prompt))
}

/// Draws the screen and reads `events` until a choice, the input's end
/// being Skip; then clears the screen's rows.
fn choose<B: Backend<Error = io::Error> + Write>(
    tui: &mut Tui<B>,
    events: &Receiver<AppEvent>,
    screen: &mut UpdatePromptScreen,
) -> io::Result<TuiUpdateChoice> {
    screen.draw(tui)?;
    // What the input thread read before the screen was drawn was typed
    // before it appeared.
    while events.try_recv().is_ok() {}
    let mut choice = TuiUpdateChoice::Skip;
    while let Ok(event) = events.recv() {
        match event {
            AppEvent::Key(key) => match screen.handle_key(key) {
                Some(chosen) => {
                    choice = chosen;
                    break;
                }
                None => screen.draw(tui)?,
            },
            AppEvent::Redraw => screen.draw(tui)?,
            _ => {}
        }
    }
    tui.leave()?;
    Ok(choice)
}

struct UpdatePromptScreen {
    prompt: TuiUpdatePrompt,
    /// The index in [`CHOICES`] of the highlighted row.
    highlighted: usize,
}

impl UpdatePromptScreen {
    fn new(prompt: TuiUpdatePrompt) -> Self {
        Self {
            prompt,
            highlighted: 0,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<TuiUpdateChoice> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            return Some(TuiUpdateChoice::Skip);
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.highlighted = (self.highlighted + CHOICES.len() - 1) % CHOICES.len();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.highlighted = (self.highlighted + 1) % CHOICES.len();
            }
            KeyCode::Char('1') => return Some(TuiUpdateChoice::UpdateNow),
            KeyCode::Char('2') => return Some(TuiUpdateChoice::Skip),
            KeyCode::Char('3') => return Some(TuiUpdateChoice::SkipVersion),
            KeyCode::Enter => return Some(CHOICES[self.highlighted]),
            KeyCode::Esc => return Some(TuiUpdateChoice::Skip),
            _ => {}
        }
        None
    }

    fn draw<B: Backend<Error = io::Error> + Write>(&self, tui: &mut Tui<B>) -> io::Result<()> {
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            self.render(area, frame.buffer_mut());
        })
    }

    /// Codex's column: a blank row, the heading, the release notes, the
    /// options, the hint and a blank row. On a screen too short for all of
    /// it the blank rows give way first, then the heading, the release
    /// notes and the options share what the hint leaves: the options take
    /// at least a row, the highlighted option's first.
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        // The insets need four columns.
        if area.width < 5 {
            return;
        }
        let prompt = &self.prompt;
        let heading = padded(Line::from(vec![
            "Update available".bold(),
            " · ".dim(),
            format!("{} → {}", prompt.current, prompt.latest).dim(),
        ]));
        let notes = padded(Line::from(vec![
            "Release notes: ".dim(),
            prompt.release_notes.clone().dim().underlined(),
        ]));
        let options = OptionList {
            labels: [
                prompt.update_now.as_str(),
                "Skip",
                "Skip until next version",
            ],
            highlighted: self.highlighted,
        };
        let hint = padded(Line::from(vec![
            key_hint::plain(KeyCode::Enter).into(),
            " continue · ".dim(),
            key_hint::plain(KeyCode::Esc).into(),
            " skip".dim(),
        ]));
        let column: [(bool, &dyn Renderable); 4] = [
            (true, &heading),
            (true, &notes),
            (true, &options),
            (false, &hint),
        ];
        let desired = column.map(|(flexible, part)| (flexible, part.desired_height(area.width)));
        let content = desired.iter().map(|(_, height)| *height).sum::<u16>();
        let panel = Rect {
            height: content.saturating_add(2).min(area.height),
            ..area
        };
        render_menu_surface(panel, buf);
        let heights = allocate(&desired, panel.height);
        // The blank rows take what is left, the one above first.
        let spare = panel.height - heights.iter().sum::<u16>();
        let mut y = panel.y + spare.min(1);
        for ((_, part), height) in column.iter().zip(heights) {
            part.render(Rect::new(panel.x, y, panel.width, height), buf);
            y += height;
        }
        if let Some(destination) = web_destination(&prompt.release_notes) {
            link_underlined(buf, panel, &destination);
        }
    }
}

fn padded(line: Line<'static>) -> RenderableItem<'static> {
    Paragraph::new(line)
        .wrap(Wrap { trim: false })
        .inset(Insets::vh(0, 2))
}

/// Codex's flex layout: the rows of the parts that do not flex first, then
/// those that do, each needing no more than an equal share of what is left
/// taking what it needs, and the rest shared equally, the last taking the
/// remainder. The heights, in `parts`' order.
fn allocate(parts: &[(bool, u16)], height: u16) -> Vec<u16> {
    let mut sizes = vec![0; parts.len()];
    let mut remaining = height;
    let mut flexible = Vec::new();
    for (index, &(flex, desired)) in parts.iter().enumerate() {
        if flex {
            flexible.push((index, desired));
        } else {
            sizes[index] = desired.min(remaining);
            remaining -= sizes[index];
        }
    }
    loop {
        let count = flexible.len() as u16;
        let before = flexible.len();
        flexible.retain(|&(index, desired)| {
            let fits = count > 0 && desired <= remaining / count;
            if fits {
                sizes[index] = desired;
                remaining -= desired;
            }
            !fits
        });
        if flexible.is_empty() || flexible.len() == before {
            break;
        }
    }
    let count = flexible.len() as u16;
    let mut given = 0;
    for (position, &(index, desired)) in flexible.iter().enumerate() {
        let share = if position + 1 == flexible.len() {
            remaining - given
        } else {
            remaining / count
        };
        sizes[index] = desired.min(share);
        given += sizes[index];
    }
    sizes
}

/// The numbered options, wrapped under their labels, between a blank row
/// above and one below that show `↑` and `↓` when options are out of view.
struct OptionList<'a> {
    labels: [&'a str; 3],
    highlighted: usize,
}

impl OptionList<'_> {
    fn rows(&self, width: u16) -> Vec<Vec<Line<'static>>> {
        let highlighted = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        self.labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let selected = index == self.highlighted;
                let marker = if selected { '›' } else { ' ' };
                let prefix = format!("{marker} {}. ", index + 1);
                let indent = " ".repeat(prefix.chars().count());
                let text = format!("{prefix}{label}");
                let line = if selected {
                    Line::from(Span::styled(text, highlighted))
                } else {
                    Line::from(text)
                };
                let options = RtOptions::new(usize::from(width))
                    .initial_indent(Line::from(""))
                    .subsequent_indent(Line::from(indent));
                word_wrap_lines([line], options)
            })
            .collect()
    }
}

impl Renderable for OptionList<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let spaced = area.height >= 3;
        let body = area.inset(Insets::vh(u16::from(spaced), 0));
        let rows = self.rows(body.width);
        // The first option shown: as early as keeps the highlighted one in
        // view.
        let mut first = 0;
        while first < self.highlighted
            && rows[first..=self.highlighted]
                .iter()
                .map(Vec::len)
                .sum::<usize>()
                > usize::from(body.height)
        {
            first += 1;
        }
        let lines: Vec<&Line> = rows[first..].iter().flatten().collect();
        let shown = lines.len().min(usize::from(body.height));
        for (y, line) in (body.y..).zip(&lines[..shown]) {
            buf.set_line(body.x, y, line, body.width);
        }
        let hidden_below = shown < lines.len();
        if spaced {
            let arrow = Style::default().fg(accent_color());
            if first > 0 {
                buf.set_string(area.x, area.y, "↑", arrow);
            }
            if hidden_below {
                buf.set_string(area.x, area.bottom() - 1, "↓", arrow);
            }
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        let lines = self.rows(width).iter().map(Vec::len).sum::<usize>();
        lines as u16 + 2
    }
}

/// Links each run of underlined cells in `area` (the release notes'
/// address, on as many rows as it wraps onto) to `destination`.
fn link_underlined(buf: &mut Buffer, area: Rect, destination: &str) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        let mut x = area.left();
        while x < area.right() {
            let start = x;
            while x < area.right() && is_linked(buf, x, y) {
                x += 1;
            }
            if x > start {
                mark_hyperlink_cells(buf, Rect::new(start, y, x - start, 1), destination);
            } else {
                x += 1;
            }
        }
    }
}

fn is_linked(buf: &Buffer, x: u16, y: u16) -> bool {
    let cell = &buf[(x, y)];
    cell.modifier.contains(Modifier::UNDERLINED) && !cell.symbol().trim().is_empty()
}

#[cfg(test)]
mod tests;
