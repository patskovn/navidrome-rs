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
use ratatui::widgets::Gauge;
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
        .border_style(Style::new().fg(Theme::ACCENT));
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
            Style::new().fg(Theme::ACCENT).bold()
        } else {
            Theme::dim()
        };
        let _ = index;
        spans.push(Span::styled(format!(" {} ", tab.title()), style));
    }
    spans.push(Span::styled(
        "   Tab switch · ↑↓ focus · space play/pause · n/p · +/- · q quit",
        Theme::dim(),
    ));

    let area = ctx.area();
    ctx.frame().render_widget(Paragraph::new(Line::from(spans)), area);
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
    let gauge = Gauge::default()
        .ratio(player.progress())
        .label(format!("{elapsed} / {total}"))
        .gauge_style(Style::new().fg(Theme::ACCENT))
        .use_unicode(true);
    ctx.frame().render_widget(gauge, rows[1]);

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
        .border_style(Style::new().fg(Theme::ACCENT));
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
        .highlight_style(Style::new().fg(Theme::ACCENT).reversed())
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    state.select(Some(picker.selected));
    StatefulWidget::render(list, inner, frame.buffer_mut(), &mut state);
}
