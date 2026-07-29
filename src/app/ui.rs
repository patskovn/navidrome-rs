//! Drawing for the root feature: tab bar, active tab, player bar, visualiser.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::LineGauge;
use ratatui::widgets::Paragraph;
use tca::Store;

use crate::app::entry::Action;
use crate::app::entry::State;
use crate::app::entry::Tab;
use crate::app::{albums, auth, songs};
use crate::audio::SampleTap;
use crate::subsonic::format_duration;
use crate::ui::DrawingContext;
use crate::ui::ModalQueue;
use crate::ui::theme::Theme;
use crate::ui::visualizer::{Spectrum, SpectrumWidget};

/// Height of the transport area at the bottom of the screen.
const PLAYER_HEIGHT: u16 = 6;

pub fn draw(
    frame: &mut Frame,
    store: Store<State, Action>,
    tap: &SampleTap,
    spectrum: &mut Spectrum,
) {
    // Published before anything draws, including the sign-in screen below.
    Theme::set_focused(store.state().focused);

    let queue = ModalQueue::new(frame.area());
    let mut ctx = DrawingContext::new(frame, &queue);

    if store.state().auth().is_some() {
        draw_sign_in(&mut ctx, &store);
        ctx.flush_modals();
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(PLAYER_HEIGHT),
    ])
    .split(ctx.area());

    draw_tabs(&mut ctx.with_area(rows[0]), &store);
    draw_active_tab(&mut ctx.with_area(rows[1]), &store);
    draw_player(&mut ctx.with_area(rows[2]), &store, tap, spectrum);

    // Drawn last so it sits above the tab; this is what the ported modal queue
    // was for.
    if store.state().help {
        ctx.queue_modal(draw_help);
    }

    if let Some(picker) = store.state().picker.clone() {
        let player = store.state().player.clone();
        ctx.queue_modal(move |frame, area| draw_picker(frame, area, &picker, &player));
    }

    ctx.flush_modals();
}

fn draw_sign_in(ctx: &mut DrawingContext, store: &Store<State, Action>) {
    use crate::ui::layout::centered_pct;

    let area = ctx.area();
    let centered = centered_pct(area, ratatui::layout::Direction::Horizontal, 60);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Sign in to Navidrome ")
        .border_style(Theme::accent());
    let inner = block.inner(centered);
    ctx.frame().render_widget(block, centered);

    auth::ui(
        &mut ctx.with_area(inner),
        store.scope(
            |s: &State| s.auth().expect("checked by the caller"),
            Action::Auth,
        ),
    );
}

fn draw_tabs(ctx: &mut DrawingContext, store: &Store<State, Action>) {
    let state = store.state();
    let active = state.library().map(|l| l.tab).unwrap_or_default();

    let mut spans = Vec::new();
    for (index, tab) in Tab::ALL.iter().enumerate() {
        let style = if *tab == active {
            Theme::accent().bold()
        } else {
            Theme::dim()
        };
        let _ = index;
        spans.push(Span::styled(format!(" {} ", tab.title()), style));
    }
    spans.push(Span::styled(
        "   Tab switch · ↑↓ focus · ? help",
        Theme::dim(),
    ));

    let area = ctx.area();
    ctx.frame()
        .render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_active_tab(ctx: &mut DrawingContext, store: &Store<State, Action>) {
    let Some(library) = store.state().library().cloned() else {
        return;
    };
    // Which track is sounding right now, so lists can mark it.
    let playing = store
        .state()
        .player
        .current_track()
        .map(|track| track.id.clone());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(
            " {} ",
            match library.tab {
                Tab::Albums => library.albums.tab_title(),
                Tab::AllMusic => library.tab.title().to_string(),
            }
        ));
    let inner = block.inner(ctx.area());
    let area = ctx.area();
    ctx.frame().render_widget(block, area);

    match library.tab {
        Tab::Albums => albums::ui(
            &mut ctx.with_area(inner),
            store.scope(
                |s: &State| &s.library().expect("signed in").albums,
                Action::Albums,
            ),
            playing.as_deref(),
        ),
        Tab::AllMusic => songs::ui(
            &mut ctx.with_area(inner),
            store.scope(
                |s: &State| &s.library().expect("signed in").songs,
                Action::Songs,
            ),
            playing.as_deref(),
        ),
    }
}

fn draw_player(
    ctx: &mut DrawingContext,
    store: &Store<State, Action>,
    tap: &SampleTap,
    spectrum: &mut Spectrum,
) {
    let state = store.state();
    let player = &state.player;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if player.is_playing {
            Theme::PLAYING
        } else {
            Theme::DIM
        }));
    let inner = block.inner(ctx.area());
    let area = ctx.area();
    ctx.frame().render_widget(block, area);

    // Visualiser on the left, transport on the right.
    let columns =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Fill(1)]).split(inner);

    let bars = columns[0].width as usize;
    spectrum.update(tap, bars);
    let levels = spectrum.levels().to_vec();
    let peaks = spectrum.peaks().to_vec();
    ctx.frame()
        .render_widget(SpectrumWidget::new(&levels, &peaks), columns[0]);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(columns[1]);

    let title = match player.current_track() {
        Some(track) => Line::from(vec![
            Span::styled(
                if player.is_playing { "▶ " } else { "⏸ " },
                Style::new().fg(Theme::PLAYING),
            ),
            Span::raw(track.display()),
        ]),
        None => Line::from("Nothing playing").style(Theme::dim()),
    };
    ctx.frame().render_widget(Paragraph::new(title), rows[0]);

    let elapsed = format_duration(player.elapsed.as_secs());
    let total = player
        .current_track()
        .map(|t| t.duration_display())
        .unwrap_or_else(|| "--:--".into());
    draw_scrubber(ctx.frame(), rows[1], &elapsed, &total, player.progress());

    let footer = match &player.error {
        Some(error) => Line::from(error.as_str()).style(Style::new().fg(Theme::ERROR)),
        None => Line::from(format!(
            "vol {:>3}%   {} in queue",
            (player.volume * 100.0).round() as i32,
            player.queue.len()
        ))
        .style(Theme::dim()),
    };
    ctx.frame().render_widget(Paragraph::new(footer), rows[2]);
}

/// One row: elapsed, a bar, and the track length.
///
/// The times sit either side of the bar rather than on top of it. A `Gauge`
/// centres its label over the filled/unfilled boundary, so the reading inverts
/// mid-word — and it does so exactly while the number is changing, which is
/// when it most wants reading.
fn draw_scrubber(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    elapsed: &str,
    total: &str,
    ratio: f64,
) {
    // The elapsed column is sized exactly: `LineGauge` always reserves one cell
    // after its label — even an empty one — which supplies the gap on that side.
    let columns = Layout::horizontal([
        Constraint::Length(elapsed.chars().count() as u16),
        Constraint::Fill(1),
        Constraint::Length(total.chars().count() as u16 + 1),
    ])
    .split(area);

    frame.render_widget(Paragraph::new(elapsed), columns[0]);

    let gauge = LineGauge::default()
        .ratio(ratio.clamp(0.0, 1.0))
        // Suppressed: an unset label falls back to a percentage readout.
        .label("")
        .filled_symbol("━")
        .unfilled_symbol("─")
        .filled_style(Theme::accent())
        .unfilled_style(Theme::dim());
    frame.render_widget(gauge, columns[1]);

    frame.render_widget(
        Paragraph::new(Line::from(total).right_aligned()).style(Theme::dim()),
        columns[2],
    );
}

/// The output picker: system default plus every device the host offers.
fn draw_picker(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    picker: &crate::app::entry::DevicePicker,
    player: &crate::app::player::State,
) {
    use ratatui::layout::Constraint;
    use ratatui::widgets::{Clear, List, ListItem, ListState, StatefulWidget, Widget};

    use crate::ui::layout::centered_constraint;
    use crate::ui::layout::centered_pct;

    let rows = (player.devices.len() + 1).min(12) as u16 + 2;
    let vertical = centered_constraint(
        area,
        Constraint::Length(rows),
        ratatui::layout::Direction::Vertical,
    );
    let modal = centered_pct(vertical, ratatui::layout::Direction::Horizontal, 60);

    Clear.render(modal, frame.buffer_mut());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Output ")
        .title_bottom(" ↵ select · Esc close ")
        .border_style(Theme::accent());
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    // Index 0 is always the system default, so there is a way back.
    let mut items = vec![ListItem::from(match &player.active_device {
        None => "System default  ·  in use".to_string(),
        Some(_) => "System default".to_string(),
    })];
    items.extend(player.devices.iter().map(|(id, label)| {
        let in_use = player.active_device.as_deref() == Some(id.as_str());
        ListItem::from(if in_use {
            format!("{label}  ·  in use")
        } else {
            label.clone()
        })
    }));

    let list = List::new(items)
        .highlight_style(Theme::accent().reversed())
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    state.select(Some(picker.selected));
    StatefulWidget::render(list, inner, frame.buffer_mut(), &mut state);
}

/// Every binding, including the ones the footer no longer has room for.
fn draw_help(frame: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::layout::Constraint;
    use ratatui::text::Span;
    use ratatui::widgets::{Clear, Paragraph, Widget};

    use crate::ui::layout::centered_constraint;
    use crate::ui::layout::centered_pct;

    const BINDINGS: &[(&str, &str)] = &[
        ("Tab", "switch tab"),
        ("↑ ↓", "move between the search field and the list"),
        ("j k", "move in a list"),
        ("↵", "open an album, or play a track"),
        ("Esc ←", "back out of an open album"),
        ("", ""),
        ("space", "play / pause"),
        ("n p", "next / previous track"),
        ("+ -", "volume"),
        ("", ""),
        ("Ctrl-O", "choose an audio output"),
        ("?", "this help"),
        ("q", "quit"),
        ("Ctrl-C", "quit, even while typing"),
    ];

    let rows = BINDINGS.len() as u16 + 4;
    let vertical = centered_constraint(
        area,
        Constraint::Length(rows.min(area.height)),
        ratatui::layout::Direction::Vertical,
    );
    let modal = centered_pct(vertical, ratatui::layout::Direction::Horizontal, 60);

    Clear.render(modal, frame.buffer_mut());

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Keys ")
        .title_bottom(" any key to close ")
        .border_style(Theme::accent());
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let lines: Vec<Line> = BINDINGS
        .iter()
        .map(|(keys, what)| {
            if keys.is_empty() {
                return Line::from("");
            }
            Line::from(vec![
                Span::styled(format!(" {keys:<8}"), Theme::accent()),
                Span::raw(*what),
            ])
        })
        .collect();

    // A note the list itself cannot express.
    let mut lines = lines;
    lines.push(Line::from(""));
    lines.push(
        Line::from(" Letters type into the search field; shortcuts need the list focused.")
            .style(Theme::dim()),
    );

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    /// Renders one scrubber row and returns it as text.
    fn row(width: u16, elapsed: &str, total: &str, ratio: f64) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test backend");
        terminal
            .draw(|frame| draw_scrubber(frame, frame.area(), elapsed, total, ratio))
            .expect("draw");
        terminal.backend().buffer().content()[..width as usize]
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The same row as a buffer, for asserting on style rather than symbols.
    fn buffer(width: u16, ratio: f64) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test backend");
        terminal
            .draw(|frame| draw_scrubber(frame, frame.area(), "1:23", "4:56", ratio))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// End to end: losing window focus reaches the accent a widget draws with.
    ///
    /// The only test that touches the focus flag — it is process-global, so a
    /// second one would race this under the default parallel runner.
    #[test]
    fn losing_window_focus_greys_the_bar() {
        // The played part of the bar is the accent; cell 5 is its first.
        let played = |focused| {
            Theme::set_focused(focused);
            buffer(30, 1.0)[(5, 0)].style()
        };

        let focused = played(true);
        let unfocused = played(false);
        Theme::set_focused(true);

        assert_eq!(focused.fg, Some(Theme::ACCENT));
        assert_eq!(
            unfocused.fg,
            Some(Theme::ACCENT_UNFOCUSED),
            "an unfocused window should draw the accent grey"
        );
    }

    #[test]
    fn times_sit_outside_the_bar() {
        let line = row(30, "1:23", "4:56", 0.5);

        assert!(line.starts_with("1:23 "), "elapsed leads the row: {line:?}");
        assert!(line.ends_with(" 4:56"), "total trails the row: {line:?}");

        // The regression this replaced: the gauge painted its label over the
        // bar, so a time could only be read against a half-inverted background.
        let bar = &line[5..line.len() - 5];
        assert!(
            bar.chars().all(|c| c == '━' || c == '─'),
            "the bar must carry nothing but bar: {bar:?}"
        );
    }

    #[test]
    fn the_bar_fills_in_proportion() {
        let filled = |ratio| row(30, "0:00", "4:56", ratio).matches('━').count();

        assert_eq!(filled(0.0), 0);
        assert_eq!(filled(1.0), 20, "the full width between the two times");
        assert_eq!(filled(0.5), 10);
    }

    /// `cargo test -- --ignored --nocapture preview_scrubber`
    #[test]
    #[ignore = "visual check only"]
    fn preview_scrubber() {
        for ratio in [0.0, 0.35, 1.0] {
            println!("{}", row(46, "1:23", "4:56", ratio));
        }
    }

    #[test]
    fn an_unknown_length_still_renders() {
        let line = row(30, "0:00", "--:--", 0.0);
        assert!(line.ends_with(" --:--"), "{line:?}");
    }
}
