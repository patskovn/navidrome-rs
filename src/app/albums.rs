//! The Albums tab: a live search field over a album list.
//!
//! The field is focused on arrival and takes every key except Up, Down and Tab.
//! Searching is debounced and runs on each keystroke; an empty query falls back
//! to the default listing.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::widgets::ListItem;
use tca::Effect;
use tca::Reducer;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

use crate::app::DEBOUNCE;
use crate::app::PaneFocus;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Track;
use crate::ui::cover::Cover;
use crate::ui::list;

use crate::subsonic::Album;
use crate::subsonic::AlbumSort;
use crate::subsonic::format_duration;

/// One album's tracks, shown after Enter on an album.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Detail {
    pub album: Album,
    pub tracks: Vec<Track>,
    pub selected: usize,
    pub loading: bool,
    /// Decoded once when the album opens; the widget resamples from it.
    pub cover: Option<Cover>,
}

impl State {
    /// Title for the enclosing tab block. An open album renames the tab rather
    /// than drawing a window of its own.
    pub fn tab_title(&self) -> String {
        match &self.detail {
            Some(detail) => format!("Albums - {}", detail.album.name),
            None => "Albums".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row(pub Album);

impl<'a> From<Row> for ListItem<'a> {
    fn from(row: Row) -> Self {
        let year = row.0.year.map(|y| format!(" ({y})")).unwrap_or_default();
        let artist = row.0.artist.clone().unwrap_or_else(|| "Unknown".into());
        ListItem::from(format!("{}{year} — {artist}", row.0.name))
    }
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct State {
    pub query: String,
    pub focus: PaneFocus,
    pub items: list::State<Row>,
    pub loading: bool,
    pub error: Option<String>,
    /// Bumped on every keystroke. A debounced search, and any result it
    /// produces, is discarded unless it still matches — which drops stale
    /// in-flight work without needing cancellation.
    pub generation: u64,
    /// Set while an album is open. Its presence takes over the tab.
    pub detail: Option<Detail>,
}

#[derive(Debug)]
pub enum Action {
    /// Load the default listing (empty query).
    Reload,
    /// The query text changed; schedule a debounced search.
    QueryChanged,
    /// Debounce elapsed. Ignored if the generation has moved on.
    RunQuery {
        query: String,
        generation: u64,
    },
    Loaded {
        items: Vec<Album>,
        generation: u64,
    },
    Failed(String),
    /// An album was chosen; its tracks are being fetched.
    OpenAlbum(Box<Album>),
    DetailLoaded {
        album: Box<Album>,
        tracks: Vec<Track>,
    },
    CloseDetail,
    CoverLoaded {
        album_id: String,
        cover: Box<Cover>,
    },
    List(list::Action),
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop,
    /// Play `tracks[index]`, with the rest of the album queued behind it.
    Play {
        tracks: Vec<Track>,
        index: usize,
    },
}

pub struct Feature {
    pub library: SharedLibrary,
}

impl Reducer for Feature {
    type State = State;
    type Action = Action;

    fn reduce(&self, state: &mut State, action: Action) -> Effect<Action> {
        let library = self.library.clone();
        let for_enter = self.library.clone();

        Scope::new(
            |s: &mut State| &mut s.items,
            case!(Action::List),
            list::ListFeature::<Row>::default(),
        )
        .on_delegate(
            case!(list::Action::Delegated),
            move |state: &mut State, d| match d {
                list::Delegated::Noop(_) => Effect::send(Action::Delegated(Delegated::Noop)),
                list::Delegated::Enter(index) => {
                    let _ = &for_enter;
                    let Some(Row(album)) = state.items.items.get(index).cloned() else {
                        return Effect::none();
                    };
                    let _ = &for_enter;
                    Effect::send(Action::OpenAlbum(Box::new(album)))
                }
                list::Delegated::Toogle => Effect::none(),
            },
        )
        .or_else(move |state: &mut State, action| match action {
            Action::Event(e) => reduce_event(state, e),

            Action::QueryChanged => {
                state.generation += 1;
                let generation = state.generation;
                let query = state.query.clone();
                Effect::run(move |send| async move {
                    tokio::time::sleep(DEBOUNCE).await;
                    send.send(Action::RunQuery { query, generation });
                })
            }

            Action::RunQuery { query, generation } => {
                // A newer keystroke has landed; this timer is stale.
                if generation != state.generation {
                    return Effect::none();
                }
                state.loading = true;
                state.error = None;
                let library = library.clone();
                Effect::run(move |send| async move {
                    let result = if query.trim().is_empty() {
                        library.albums(AlbumSort::Alphabetical, 0, 500).await
                    } else {
                        library.search(&query).await.map(|r| r.albums)
                    };
                    match result {
                        Ok(items) => send.send(Action::Loaded { items, generation }),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::Loaded { items, generation } => {
                // Responses can arrive out of order; only the newest counts.
                if generation != state.generation {
                    return Effect::none();
                }
                state.loading = false;
                state.items = list::State::new(items.into_iter().map(Row).collect());
                Effect::none()
            }

            Action::Reload => {
                state.generation += 1;
                Effect::send(Action::RunQuery {
                    query: state.query.clone(),
                    generation: state.generation,
                })
            }

            Action::Failed(message) => {
                state.loading = false;
                state.error = Some(message);
                Effect::none()
            }

            Action::OpenAlbum(album) => {
                let album = *album;
                state.detail = Some(Detail {
                    album: album.clone(),
                    loading: true,
                    ..Default::default()
                });
                let library = library.clone();
                Effect::run(move |send| async move {
                    // Artwork first: it is the slower of the two and the panel
                    // has a hole in it until it lands.
                    if let Some(art_id) = album.cover_art.clone() {
                        match library.cover_art(&art_id).await {
                            Ok(bytes) => match Cover::decode(&bytes) {
                                Ok(cover) => send.send(Action::CoverLoaded {
                                    album_id: album.id.clone(),
                                    cover: Box::new(cover),
                                }),
                                Err(err) => log::warn!("undecodable artwork: {err:#}"),
                            },
                            Err(err) => log::warn!("no artwork for {}: {err:#}", album.id),
                        }
                    }
                    match library.album_tracks(&album.id).await {
                        Ok(tracks) => send.send(Action::DetailLoaded {
                            album: Box::new(album),
                            tracks,
                        }),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::DetailLoaded { album, tracks } => {
                let album = *album;
                // The user may have backed out while this was in flight.
                if let Some(existing) = &state.detail
                    && existing.album.id == album.id
                {
                    let cover = existing.cover.clone();
                    state.detail = Some(Detail {
                        album,
                        tracks,
                        selected: 0,
                        loading: false,
                        cover,
                    });
                }
                Effect::none()
            }

            Action::CoverLoaded { album_id, cover } => {
                // Ignore artwork for an album the user has already left.
                if let Some(detail) = &mut state.detail
                    && detail.album.id == album_id
                {
                    detail.cover = Some(*cover);
                }
                Effect::none()
            }

            Action::CloseDetail => {
                state.detail = None;
                Effect::none()
            }

            Action::Delegated(_) => Effect::none(),
            Action::List(_) => unreachable!("claimed by the scope"),
        })
        .reduce(state, action)
    }
}

/// Routes a key according to focus. Only Up and Down are reserved here; Tab is
/// handled globally by `entry`, and everything else is text.
fn reduce_event(state: &mut State, event: Event) -> Effect<Action> {
    let Event::Key(key) = &event else {
        return Effect::none();
    };
    if key.kind != KeyEventKind::Press {
        return Effect::none();
    }

    // An open album takes the whole tab: its track list gets the keyboard.
    if let Some(detail) = &mut state.detail {
        return match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => Effect::send(Action::CloseDetail),
            KeyCode::Down | KeyCode::Char('j') => {
                detail.selected = (detail.selected + 1).min(detail.tracks.len().saturating_sub(1));
                Effect::none()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                detail.selected = detail.selected.saturating_sub(1);
                Effect::none()
            }
            KeyCode::Enter => {
                if detail.tracks.is_empty() {
                    return Effect::none();
                }
                // The whole album becomes the queue, starting here, so the rest
                // plays on after it.
                Effect::send(Action::Delegated(Delegated::Play {
                    tracks: detail.tracks.clone(),
                    index: detail.selected,
                }))
            }
            _ => Effect::none(),
        };
    }

    match (state.focus, key.code) {
        (PaneFocus::Search, KeyCode::Down) => {
            state.focus = PaneFocus::List;
            if state.items.selected().is_none() && !state.items.items.is_empty() {
                state.items.select(Some(0));
            }
            Effect::none()
        }
        // Up leaves the list only from its first row, so it still walks the
        // list everywhere else.
        (PaneFocus::List, KeyCode::Up) if state.items.selected().unwrap_or(0) == 0 => {
            state.focus = PaneFocus::Search;
            Effect::none()
        }
        (PaneFocus::List, _) => Effect::send(Action::List(list::Action::Event(event))),
        (PaneFocus::Search, KeyCode::Char(c)) => {
            state.query.push(c);
            Effect::send(Action::QueryChanged)
        }
        (PaneFocus::Search, KeyCode::Backspace) => {
            state.query.pop();
            Effect::send(Action::QueryChanged)
        }
        (PaneFocus::Search, _) => Effect::none(),
    }
}

pub fn ui(
    ctx: &mut crate::ui::DrawingContext,
    store: tca::Store<State, Action>,
    playing: Option<&str>,
) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

    use crate::ui::theme::Theme;

    let state = store.state();

    if let Some(detail) = &state.detail {
        draw_detail(ctx, detail, playing);
        return;
    }

    let rows = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).split(ctx.area());

    let focused = state.focus == PaneFocus::Search;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Search ")
        .title_bottom(if focused {
            " ↓ to list "
        } else {
            " ↑ to search "
        })
        .border_style(Theme::border(focused));
    // A block cursor marks the caret without needing a real text widget.
    let text = if focused {
        format!("{}█", state.query)
    } else {
        state.query.clone()
    };
    ctx.frame()
        .render_widget(Paragraph::new(text).block(block), rows[0]);

    // Loading and errors render where the list would be, never over the search
    // field: the field must stay visible and editable while a query is running.
    if let Some(line) = status_line(state.loading, &state.error) {
        ctx.frame().render_widget(Paragraph::new(line), rows[1]);
    } else {
        list::ui(&mut ctx.with_area(rows[1]), &state.items);
    }
}

/// Status shown in place of the list while a query runs or after it fails.
fn status_line(loading: bool, error: &Option<String>) -> Option<ratatui::text::Line<'_>> {
    use ratatui::style::Style;
    use ratatui::text::Line;

    use crate::ui::theme::Theme;

    if let Some(error) = error {
        return Some(Line::from(format!("  {error}")).style(Style::new().fg(Theme::ERROR)));
    }
    loading.then(|| Line::from("  Searching…").style(Theme::dim()))
}

/// Width of the info panel. Wide enough for a label/value pair without
/// squeezing the track titles.
const INFO_WIDTH: u16 = 34;

fn draw_detail(ctx: &mut crate::ui::DrawingContext, detail: &Detail, playing: Option<&str>) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;

    // No block of our own: the tab's border frames this, and its title carries
    // the album name.
    let columns =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(INFO_WIDTH)]).split(ctx.area());

    draw_tracks(ctx, detail, columns[0], playing);
    draw_info(ctx, detail, columns[1]);
}

fn draw_tracks(
    ctx: &mut crate::ui::DrawingContext,
    detail: &Detail,
    area: ratatui::layout::Rect,
    playing: Option<&str>,
) {
    use ratatui::style::Style;
    use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};

    use crate::ui::theme::Theme;

    let width = area.width.saturating_sub(14) as usize;
    let items: Vec<ListItem> = detail
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let number = track.track_no.unwrap_or(i as i32 + 1);
            let sounding = playing.is_some_and(|id| id == track.id);
            // Colour alone marks the sounding row: the track number stays put,
            // so nothing about the row changes shape as playback moves.
            let item = ListItem::from(format!(
                "{number:>3}. {:<width$} {}",
                truncate(&track.title, width),
                track.duration_display()
            ));
            if sounding {
                item.style(Style::new().fg(Theme::PLAYING))
            } else {
                item
            }
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Theme::accent().reversed())
        .highlight_symbol("▶ ");
    let mut list_state = ListState::default();
    if !detail.tracks.is_empty() {
        list_state.select(Some(detail.selected));
    }
    StatefulWidget::render(list, area, ctx.frame().buffer_mut(), &mut list_state);
}

fn draw_info(ctx: &mut crate::ui::DrawingContext, detail: &Detail, area: ratatui::layout::Rect) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::text::Line;
    use ratatui::text::Span;
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

    use crate::ui::theme::Theme;

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_type(BorderType::Plain)
        .border_style(Theme::dim());
    let inner = block.inner(area);
    ctx.frame().render_widget(block, area);

    // Square-ish cover area at the top, the rest for facts.
    let cover_height = inner.height.min(inner.width / 2).min(12);
    let rows =
        Layout::vertical([Constraint::Length(cover_height), Constraint::Fill(1)]).split(inner);

    draw_cover(ctx, detail, rows[0]);

    let album = &detail.album;
    let mut lines = vec![
        Line::from(Span::styled(album.name.clone(), Theme::accent().bold())),
        Line::from(
            album
                .artist
                .clone()
                .unwrap_or_else(|| "Unknown artist".into()),
        ),
        Line::from(""),
    ];

    let mut fact = |label: &str, value: Option<String>| {
        if let Some(value) = value {
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<9}"), Theme::dim()),
                Span::raw(value),
            ]));
        }
    };

    fact("Year", album.year.map(|y| y.to_string()));
    fact("Genre", album.genre.clone());
    fact(
        "Tracks",
        Some(if detail.tracks.is_empty() {
            album.song_count.map(|c| c.to_string()).unwrap_or_default()
        } else {
            detail.tracks.len().to_string()
        }),
    );
    fact(
        "Length",
        album
            .duration
            .or_else(|| {
                let total: i64 = detail.tracks.iter().filter_map(|t| t.duration).sum();
                (total > 0).then_some(total)
            })
            .map(|d| format_duration(d as u64)),
    );
    fact("Plays", album.play_count.map(|p| p.to_string()));

    if detail.loading {
        lines.push(Line::from(""));
        lines.push(Line::from("Loading tracks…").style(Theme::dim()));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("↵ play from here").style(Theme::dim()));
    lines.push(Line::from("Esc back").style(Theme::dim()));

    ctx.frame()
        .render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[1]);
}

/// Draws the artwork, or a quiet note in its place.
///
/// No frame of its own: the image is the shape, and a border around it just
/// competes with the panel rule and the tab's own edge.
fn draw_cover(ctx: &mut crate::ui::DrawingContext, detail: &Detail, area: ratatui::layout::Rect) {
    use ratatui::layout::Alignment;
    use ratatui::widgets::Paragraph;

    use crate::ui::theme::Theme;

    if area.height == 0 || area.width == 0 {
        return;
    }

    if let Some(cover) = &detail.cover {
        ctx.frame()
            .render_widget(crate::ui::cover::CoverWidget::new(cover), area);
        return;
    }

    let label = if detail.album.cover_art.is_some() {
        "loading art…"
    } else {
        "no art"
    };
    let pad = (area.height.saturating_sub(1) / 2) as usize;
    ctx.frame().render_widget(
        Paragraph::new(format!("{}{label}", "\n".repeat(pad)))
            .alignment(Alignment::Center)
            .style(Theme::dim()),
        area,
    );
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return format!("{text:<width$}");
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::subsonic::Album;
    use crate::subsonic::fake::FakeLibrary;
    use std::sync::Arc;
    use tca::TestStore;

    fn store() -> TestStore<Feature> {
        TestStore::new(
            Feature {
                library: Arc::new(FakeLibrary::with_albums(vec![Album {
                    id: "1".into(),
                    name: "Kind of Blue".into(),
                    ..Default::default()
                }])),
            },
            State::default(),
        )
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    #[tokio::test]
    async fn typing_builds_the_query_and_schedules_a_search() {
        let mut store = store();
        for c in "blue".chars() {
            store.send(Action::Event(key(KeyCode::Char(c))), |s| s.query.push(c));
            store.settle();
        }
        assert_eq!(store.state().query, "blue");
        assert_eq!(
            store.state().generation,
            4,
            "each keystroke should invalidate the last"
        );
    }

    #[tokio::test]
    async fn backspace_edits_the_query() {
        let mut store = store();
        for c in "ab".chars() {
            store.send(Action::Event(key(KeyCode::Char(c))), |s| s.query.push(c));
            store.settle();
        }
        store.send(Action::Event(key(KeyCode::Backspace)), |s| {
            s.query.pop();
        });
        store.settle();
        assert_eq!(store.state().query, "a");
    }

    /// The debounce guard: a timer that fires after a newer keystroke must not
    /// run its search.
    #[tokio::test]
    async fn a_stale_debounce_timer_is_ignored() {
        let mut store = store();
        store.send(Action::Event(key(KeyCode::Char('a'))), |s| {
            s.query.push('a')
        });
        store.settle();
        let stale = store.state().generation;

        store.send(Action::Event(key(KeyCode::Char('b'))), |s| {
            s.query.push('b')
        });
        store.settle();

        // Replay the older timer; it should do nothing at all.
        store.send(
            Action::RunQuery {
                query: "a".into(),
                generation: stale,
            },
            |_| {},
        );
        assert!(
            !store.state().loading,
            "a stale timer must not start a search"
        );
        assert_eq!(store.pending_len(), 0, "and must emit nothing");
    }

    /// Results can come back out of order; only the newest may land.
    #[tokio::test]
    async fn a_stale_result_is_discarded() {
        let mut store = store();
        store.send(Action::Event(key(KeyCode::Char('x'))), |s| {
            s.query.push('x')
        });
        store.settle();

        store.send(
            Action::Loaded {
                items: vec![Album {
                    id: "old".into(),
                    name: "Stale".into(),
                    ..Default::default()
                }],
                generation: 0,
            },
            |_| {},
        );
        assert!(
            store.state().items.items.is_empty(),
            "a result from an older query must not be shown"
        );
    }

    fn album(id: &str) -> Album {
        Album {
            id: id.into(),
            name: format!("Album {id}"),
            ..Default::default()
        }
    }

    fn track(id: &str) -> Track {
        Track {
            id: id.into(),
            title: format!("Track {id}"),
            duration: Some(100),
            ..Default::default()
        }
    }

    /// Enter on an album must open it, not start playing.
    #[tokio::test]
    async fn enter_on_an_album_opens_it_rather_than_playing() {
        let mut store = store();
        store.send(Action::OpenAlbum(Box::new(album("1"))), |s| {
            s.detail = Some(Detail {
                album: album("1"),
                loading: true,
                ..Default::default()
            })
        });

        assert!(store.state().detail.is_some(), "the album should be open");
        assert!(
            store.state().detail.as_ref().unwrap().loading,
            "and its tracks should be loading"
        );
        store.settle();
        assert!(
            !store.state().detail.as_ref().unwrap().loading,
            "loading should clear once tracks arrive"
        );
    }

    #[tokio::test]
    async fn enter_on_a_track_queues_the_whole_album_from_that_point() {
        let mut store = store();
        let tracks = vec![track("a"), track("b"), track("c")];
        store.send(
            Action::DetailLoaded {
                album: Box::new(album("1")),
                tracks: tracks.clone(),
            },
            |_| {},
        );
        // Nothing is open, so a stray load is ignored.
        assert!(store.state().detail.is_none());

        store.send(Action::OpenAlbum(Box::new(album("1"))), |s| {
            s.detail = Some(Detail {
                album: album("1"),
                loading: true,
                ..Default::default()
            })
        });
        store.settle();
        store.send(
            Action::DetailLoaded {
                album: Box::new(album("1")),
                tracks: tracks.clone(),
            },
            |s| {
                s.detail = Some(Detail {
                    album: album("1"),
                    tracks: tracks.clone(),
                    selected: 0,
                    loading: false,
                    cover: None,
                })
            },
        );

        // Move to the second track and play from there.
        store.send(Action::Event(key(KeyCode::Down)), |s| {
            s.detail.as_mut().unwrap().selected = 1
        });
        store.send(Action::Event(key(KeyCode::Enter)), |_| {});

        assert_eq!(store.pending_len(), 1, "should have delegated a play");
    }

    /// Artwork for an album the user has already left must not appear on the
    /// one they moved to.
    #[tokio::test]
    async fn artwork_for_a_different_album_is_ignored() {
        use crate::ui::cover::Cover;

        let mut store = store();
        store.send(Action::OpenAlbum(Box::new(album("1"))), |s| {
            s.detail = Some(Detail {
                album: album("1"),
                loading: true,
                ..Default::default()
            })
        });
        store.settle();

        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 2))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let cover = Cover::decode(&png).unwrap();

        store.send(
            Action::CoverLoaded {
                album_id: "a-different-album".into(),
                cover: Box::new(cover),
            },
            |_| {},
        );
        assert!(
            store.state().detail.as_ref().unwrap().cover.is_none(),
            "artwork from another album must not land here"
        );
    }

    #[tokio::test]
    async fn escape_closes_an_open_album() {
        let mut store = store();
        store.send(Action::OpenAlbum(Box::new(album("1"))), |s| {
            s.detail = Some(Detail {
                album: album("1"),
                loading: true,
                ..Default::default()
            })
        });
        store.settle();
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.settle();
        assert!(store.state().detail.is_none(), "Esc should go back");
    }

    /// Backing out while the fetch is in flight must not reopen the album.
    #[tokio::test]
    async fn a_late_track_load_is_dropped_after_backing_out() {
        let mut store = store();
        store.send(Action::OpenAlbum(Box::new(album("1"))), |s| {
            s.detail = Some(Detail {
                album: album("1"),
                loading: true,
                ..Default::default()
            })
        });
        store.settle();
        store.send(Action::CloseDetail, |s| s.detail = None);
        store.send(
            Action::DetailLoaded {
                album: Box::new(album("1")),
                tracks: vec![track("a")],
            },
            |_| {},
        );
        assert!(
            store.state().detail.is_none(),
            "a late load must not reopen it"
        );
    }

    #[tokio::test]
    async fn down_moves_focus_into_the_list_and_selects_the_first_row() {
        let mut store = store();
        store.send(
            Action::Loaded {
                items: vec![Album {
                    id: "1".into(),
                    name: "A".into(),
                    ..Default::default()
                }],
                generation: 0,
            },
            |s| {
                s.items = list::State::new(vec![Row(Album {
                    id: "1".into(),
                    name: "A".into(),
                    ..Default::default()
                })]);
            },
        );
        store.send(Action::Event(key(KeyCode::Down)), |s| {
            s.focus = PaneFocus::List;
            s.items.select(Some(0));
        });
        store.settle();

        assert_eq!(store.state().focus, PaneFocus::List);
        assert_eq!(store.state().items.selected(), Some(0));
    }
}
