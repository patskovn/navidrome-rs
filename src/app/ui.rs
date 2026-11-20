//! Drawing for the root feature: active tab, player bar, visualiser.

use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use tca::Store;

use crate::app::entry::Action;
use crate::app::entry::State;
use crate::app::entry::Tab;
use crate::app::truncate;
use crate::app::{albums, artists, auth, radios, songs};
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

    // No separate tab row: the tabs ride the content block's own border, which
    // is a line the layout was spending anyway.
    let rows = Layout::vertical([Constraint::Fill(1), Constraint::Length(PLAYER_HEIGHT)])
        .split(ctx.area());

    draw_active_tab(&mut ctx.with_area(rows[0]), &store);
    draw_player(&mut ctx.with_area(rows[1]), &store, tap, spectrum);

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

/// The tab strip, as a title for the content block's top border.
///
/// The open album's name is deliberately absent: the detail panel on the right
/// already carries it, and repeating it in the border said the same thing twice.
fn tab_strip(active: Tab) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for tab in Tab::ALL {
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", Theme::dim()));
        }
        spans.push(Span::styled(
            tab.title(),
            if tab == active {
                Theme::accent().bold()
            } else {
                Theme::dim()
            },
        ));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
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
        .title(tab_strip(library.tab))
        // Bottom right, where it stays out of the way of the tabs but is still
        // on screen — this used to sit beside them in the old top bar.
        .title_bottom(
            Line::styled(" Tab switch · ↑↓ focus · ? help ", Theme::dim()).right_aligned(),
        );
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
        Tab::Artists => artists::ui(
            &mut ctx.with_area(inner),
            store.scope(
                |s: &State| &s.library().expect("signed in").artists,
                Action::Artists,
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
        Tab::Radios => radios::ui(
            &mut ctx.with_area(inner),
            store.scope(
                |s: &State| &s.library().expect("signed in").radios,
                Action::Radios,
            ),
            playing.as_deref(),
        ),
        Tab::Queue => draw_queue(
            ctx.frame(),
            inner,
            library.queue_cursor,
            &store.state().player,
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

    let (visualiser, transport) = player_columns(inner);

    let bars = visualiser.width as usize;
    spectrum.update(tap, bars);
    let levels = spectrum.levels().to_vec();
    let peaks = spectrum.peaks().to_vec();
    ctx.frame()
        .render_widget(SpectrumWidget::new(&levels, &peaks), visualiser);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(transport);

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
    draw_scrubber(
        ctx.frame(),
        rows[1],
        &elapsed,
        &total,
        player.progress(),
        player.buffered,
    );

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

/// Splits the player bar into the visualiser column and the transport column.
///
/// The two are separated by a column of air. The spectrum draws to the full
/// width of its area, so butted straight together a tall bar ends up flush
/// against the track title.
fn player_columns(inner: ratatui::layout::Rect) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
    let columns = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .split(inner);
    (columns[0], columns[2])
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
    buffered: Option<f64>,
) {
    // One space of breathing room on each side, matching what the old
    // `LineGauge` reserved for itself after its label.
    let columns = Layout::horizontal([
        Constraint::Length(elapsed.chars().count() as u16 + 1),
        Constraint::Fill(1),
        Constraint::Length(total.chars().count() as u16 + 1),
    ])
    .split(area);

    frame.render_widget(Paragraph::new(elapsed), columns[0]);
    frame.render_widget(bar(ratio, buffered, columns[1].width), columns[1]);
    frame.render_widget(
        Paragraph::new(Line::from(total).right_aligned()).style(Theme::dim()),
        columns[2],
    );
}

/// The bar itself: played, downloaded-but-unplayed, and not yet fetched.
///
/// Solid means the bytes are here and dotted means they are not, so a fully
/// cached track looks like an ordinary progress bar and a streaming one grows
/// its solid section towards the end. Hand-drawn rather than a `LineGauge`,
/// which only knows one ratio.
fn bar(ratio: f64, buffered: Option<f64>, width: u16) -> Line<'static> {
    const PLAYED: &str = "━";
    const HAVE: &str = "─";
    const MISSING: &str = "┄";

    let width = width as usize;
    let cells =
        |fraction: f64| ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);

    let played = cells(ratio);
    // Never behind the playhead: audio cannot come out of bytes we do not have,
    // and a bar saying otherwise would look like a glitch rather than a fact.
    // Without a content length there is nothing to be a fraction of, so the
    // whole bar is drawn as present rather than guessed at.
    let have = buffered.map_or(width, |b| cells(b).max(played));

    Line::from(vec![
        Span::styled(PLAYED.repeat(played), Theme::accent()),
        Span::styled(HAVE.repeat(have - played), Theme::dim()),
        Span::styled(MISSING.repeat(width - have), Theme::dim()),
    ])
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
        .highlight_symbol("▶ ")
        // Reserve the marker column even with no row selected, so rows do
        // not jump sideways the moment focus arrives.
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    let mut state = ListState::default();
    state.select(Some(picker.selected));
    StatefulWidget::render(list, inner, frame.buffer_mut(), &mut state);
}

/// The Queue tab: the play order, with the sounding track marked.
///
/// A plain pane rather than a modal — it draws into the tab's own frame and so
/// has no border of its own.
fn draw_queue(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    cursor: usize,
    player: &crate::app::player::State,
) {
    use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};

    if player.queue.is_empty() {
        frame.render_widget(
            Paragraph::new(" Nothing queued. Play an album or a track to fill it.")
                .style(Theme::dim()),
            area,
        );
        return;
    }

    let rows = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(area);

    // Everything the row spends besides the title: the highlight symbol the
    // list reserves, "999. ", a gap, a right-aligned duration, and a margin.
    const FURNITURE: u16 = 2 + 5 + 1 + 6 + 1;
    let room = rows[0].width.saturating_sub(FURNITURE) as usize;
    let items: Vec<ListItem> = player
        .queue
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let item = ListItem::from(format!(
                "{:>3}. {:<room$} {:>6} ",
                index + 1,
                truncate(&track.display(), room),
                track.duration_display()
            ));
            // Colour alone marks the sounding row, as the track lists do.
            if player.current == Some(index) {
                item.style(Style::new().fg(Theme::PLAYING))
            } else {
                item
            }
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Theme::selection())
        .highlight_symbol("▶ ")
        // Reserve the marker column even with no row selected, so rows do
        // not jump sideways the moment focus arrives.
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    let mut state = ListState::default();
    state.select(Some(cursor.min(player.queue.len() - 1)));
    StatefulWidget::render(list, rows[0], frame.buffer_mut(), &mut state);

    frame.render_widget(
        Paragraph::new(" ↵ play · d remove").style(Theme::dim()),
        rows[1],
    );
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
        ("↵", "open an artist or album, or play a track"),
        ("Esc ←", "back out one level"),
        ("d", "drop a track, in the Queue tab"),
        ("s /", "search radio-browser.info, in the Radios tab"),
        ("a d", "add by URL / delete a station, in the Radios tab"),
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

    /// Renders the queue modal over the app and returns it as rows.
    fn queue_rows(width: u16, titles: &[(&str, i64)]) -> Vec<String> {
        use std::sync::Arc;

        use crate::app::entry::{Feature, Library, Session, Tab};
        use crate::audio::{AudioEngine, TrackCache};
        use crate::subsonic::{SwappableLibrary, Track, fake::FakeLibrary};

        let dir = std::env::temp_dir().join("navidrome-queue-render");
        let cache = Arc::new(TrackCache::new(&dir, 1024).expect("cache"));
        let (audio, _events) = AudioEngine::start(cache);
        let tap = audio.tap().clone();

        let store = tca::Store::new(
            Feature {
                library: Arc::new(SwappableLibrary::new(Arc::new(FakeLibrary::default()))),
                audio,
                probe: Arc::new(crate::probe::fake::FakeProbe::failing("offline")),
                directory: Arc::new(crate::directory::fake::FakeDirectory::default()),
            },
            State {
                session: Session::SignedIn(Box::new(Library {
                    tab: Tab::Queue,
                    ..Default::default()
                })),
                player: crate::app::player::State {
                    queue: titles
                        .iter()
                        .map(|(title, seconds)| Track {
                            title: (*title).into(),
                            duration: Some(*seconds),
                            ..Default::default()
                        })
                        .collect(),
                    current: Some(0),
                    ..crate::app::player::State::new()
                },
                ..Default::default()
            },
        );

        let mut spectrum = Spectrum::new();
        let mut terminal = Terminal::new(TestBackend::new(width, 24)).expect("test backend");
        terminal
            .draw(|frame| draw(frame, store.clone(), &tap, &mut spectrum))
            .expect("draw");

        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    /// Regression: the row reserved too few columns for its furniture, so a
    /// long track lost the tail of its duration — "11:33" rendered as "11:3".
    #[tokio::test]
    async fn queue_durations_are_never_cut_off() {
        // A title long enough to be truncated at every width tested.
        let long = "A Love Supreme, Part One: Acknowledgement (Alternate Take)";

        for width in [60, 76, 100, 140] {
            let rows = queue_rows(width, &[(long, 693), (long, 45)]);
            for wanted in ["11:33", "0:45"] {
                assert!(
                    rows.iter().any(|row| row.contains(wanted)),
                    "{wanted} missing at width {width}:\n{}",
                    rows.join("\n")
                );
            }
        }
    }

    /// `cargo test -- --ignored --nocapture preview_queue`
    #[tokio::test]
    #[ignore = "visual check only"]
    async fn preview_queue() {
        use std::sync::Arc;

        use crate::app::entry::{Feature, Library, Session, Tab};
        use crate::audio::{AudioEngine, TrackCache};
        use crate::subsonic::{SwappableLibrary, Track, fake::FakeLibrary};

        let dir = std::env::temp_dir().join("navidrome-preview-queue");
        let cache = Arc::new(TrackCache::new(&dir, 1024).expect("cache"));
        let (audio, _events) = AudioEngine::start(cache);
        let tap = audio.tap().clone();

        let track = |title: &str, artist: &str, seconds| Track {
            title: title.into(),
            artist: Some(artist.into()),
            duration: Some(seconds),
            ..Default::default()
        };

        let store = tca::Store::new(
            Feature {
                library: Arc::new(SwappableLibrary::new(Arc::new(FakeLibrary::default()))),
                audio,
                probe: Arc::new(crate::probe::fake::FakeProbe::failing("offline")),
                directory: Arc::new(crate::directory::fake::FakeDirectory::default()),
            },
            State {
                session: Session::SignedIn(Box::new(Library {
                    tab: Tab::Queue,
                    queue_cursor: 2,
                    ..Default::default()
                })),
                player: crate::app::player::State {
                    queue: vec![
                        track("So What", "Miles Davis", 545),
                        track("Freddie Freeloader", "Miles Davis", 586),
                        track("Blue in Green", "Miles Davis", 337),
                        track("All Blues", "Miles Davis", 693),
                        track(
                            "A Love Supreme, Pt. I: Acknowledgement",
                            "John Coltrane",
                            472,
                        ),
                    ],
                    current: Some(1),
                    is_playing: true,
                    ..crate::app::player::State::new()
                },
                ..Default::default()
            },
        );

        let mut spectrum = Spectrum::new();
        let mut terminal = Terminal::new(TestBackend::new(76, 20)).expect("test backend");
        terminal
            .draw(|frame| draw(frame, store.clone(), &tap, &mut spectrum))
            .expect("draw");

        let buffer = terminal.backend().buffer();
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            println!("{row}");
        }
    }

    /// The whole signed-in screen, for eyeballing layout changes.
    ///
    /// `cargo test -- --ignored --nocapture preview_screen`
    #[tokio::test]
    #[ignore = "visual check only"]
    async fn preview_screen() {
        use std::sync::Arc;

        use crate::app::albums::Detail;
        use crate::app::entry::{Feature, Library, Session};
        use crate::audio::{AudioEngine, TrackCache};
        use crate::subsonic::{Album, SwappableLibrary, Track, fake::FakeLibrary};
        use crate::ui::list;

        let dir = std::env::temp_dir().join("navidrome-preview");
        let cache = Arc::new(TrackCache::new(&dir, 1024).expect("cache"));
        let (audio, _events) = AudioEngine::start(cache);
        let tap = audio.tap().clone();
        let audio = Arc::new(audio);

        let album = |name: &str, artist: &str, year| Album {
            name: name.into(),
            artist: Some(artist.into()),
            year: Some(year),
            song_count: Some(5),
            duration: Some(2793),
            genre: Some("Jazz".into()),
            ..Default::default()
        };
        let track = |title: &str, seconds| Track {
            title: title.into(),
            duration: Some(seconds),
            ..Default::default()
        };

        let mut spectrum = Spectrum::new();

        let mut shot = |label: &str, library: Library, size: (u16, u16)| {
            let mut terminal =
                Terminal::new(TestBackend::new(size.0, size.1)).expect("test backend");
            let store = tca::Store::new(
                Feature {
                    library: Arc::new(SwappableLibrary::new(Arc::new(FakeLibrary::default()))),
                    audio: (*audio).clone(),
                    probe: Arc::new(crate::probe::fake::FakeProbe::failing("offline")),
                    directory: Arc::new(crate::directory::fake::FakeDirectory::default()),
                },
                State {
                    session: Session::SignedIn(Box::new(library)),
                    ..Default::default()
                },
            );
            terminal
                .draw(|frame| draw(frame, store.clone(), &tap, &mut spectrum))
                .expect("draw");
            let buffer = terminal.backend().buffer();
            println!("\n{label}");
            for y in 0..buffer.area.height {
                let row: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                println!("{row}");
            }
        };

        let listing = || {
            let mut library = Library::default();
            library.albums.items = list::State::new(vec![
                albums::Row(album("Kind of Blue", "Miles Davis", 1959)),
                albums::Row(album("A Love Supreme", "John Coltrane", 1965)),
                albums::Row(album("Mingus Ah Um", "Charles Mingus", 1959)),
            ]);
            library.albums.items.select(Some(1));
            library
        };

        shot("── album list ──", listing(), (76, 20));

        // With an album open the name lives in the panel on the right, which is
        // why the border no longer repeats it.
        let mut open = listing();
        open.albums.detail = Some(Detail {
            album: album("Kind of Blue", "Miles Davis", 1959),
            tracks: vec![
                track("So What", 545),
                track("Freddie Freeloader", 586),
                track("Blue in Green", 337),
            ],
            ..Default::default()
        });
        // Twice, because the panel's split depends on the height available: a
        // short terminal must still show the name, a tall one gets real artwork.
        shot("── album open, short ──", open.clone(), (76, 20));
        shot("── album open, tall ──", open, (96, 34));
    }

    /// Renders one scrubber row and returns it as text.
    fn row(width: u16, elapsed: &str, total: &str, ratio: f64) -> String {
        buffered_row(width, elapsed, total, ratio, None)
    }

    /// The same, with a known download extent.
    fn buffered_row(
        width: u16,
        elapsed: &str,
        total: &str,
        ratio: f64,
        buffered: Option<f64>,
    ) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("test backend");
        terminal
            .draw(|frame| draw_scrubber(frame, frame.area(), elapsed, total, ratio, buffered))
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
            .draw(|frame| draw_scrubber(frame, frame.area(), "1:23", "4:56", ratio, None))
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

    /// `cargo test -- --ignored --nocapture preview_buffer`
    #[test]
    #[ignore = "visual check only"]
    fn preview_buffer() {
        for (label, ratio, buffered) in [
            ("just started, barely any downloaded", 0.02, Some(0.10)),
            ("playing, download running ahead", 0.30, Some(0.65)),
            ("download finished, still playing", 0.30, None::<f64>),
            ("cache hit — everything is here", 0.55, Some(1.0)),
            ("stalled — playhead caught the buffer", 0.40, Some(0.40)),
        ] {
            println!(
                "{}  {label}",
                buffered_row(46, "1:23", "4:56", ratio, buffered)
            );
        }
    }

    #[test]
    fn the_undownloaded_tail_is_dotted() {
        let line = buffered_row(30, "1:23", "4:56", 0.25, Some(0.5));
        let bar = &line[5..line.len() - 5];

        assert_eq!(bar.matches('━').count(), 5, "played: {bar:?}");
        assert_eq!(bar.matches('─').count(), 5, "downloaded, unplayed: {bar:?}");
        assert_eq!(bar.matches('┄').count(), 10, "not downloaded: {bar:?}");
    }

    /// Solid throughout, so a cached track looks like an ordinary progress bar.
    #[test]
    fn a_fully_downloaded_track_has_no_dots() {
        for buffered in [Some(1.0), None] {
            let line = buffered_row(30, "1:23", "4:56", 0.5, buffered);
            assert!(
                !line.contains('┄'),
                "{buffered:?} should be solid: {line:?}"
            );
            assert_eq!(line[5..line.len() - 5].matches('━').count(), 10);
        }
    }

    /// Audio cannot come out of bytes we do not have, so a buffer reading that
    /// trails the playhead is a stale sample, not something to draw.
    #[test]
    fn the_buffer_never_renders_behind_the_playhead() {
        let line = buffered_row(30, "1:23", "4:56", 0.75, Some(0.25));
        let bar = &line[5..line.len() - 5];

        assert_eq!(bar.matches('━').count(), 15, "played wins: {bar:?}");
        assert_eq!(bar.matches('─').count(), 0, "no backwards gap: {bar:?}");
        assert_eq!(bar.matches('┄').count(), 5);
    }

    /// The gap between the spectrum and the transport, which is easy to lose to
    /// a future layout tweak because nothing about the code demands it.
    #[test]
    fn a_column_of_air_separates_the_visualiser_from_the_transport() {
        use ratatui::layout::Rect;

        for width in [40, 76, 120, 200] {
            let inner = Rect::new(0, 0, width, 4);
            let (visualiser, transport) = player_columns(inner);

            assert_eq!(
                transport.x - (visualiser.x + visualiser.width),
                1,
                "expected one blank column at width {width}"
            );
            assert!(visualiser.width > 0 && transport.width > 0, "width {width}");
        }
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
