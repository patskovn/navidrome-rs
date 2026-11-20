//! The Artists tab: a filter field over every album artist, drilling down
//! artist → albums → tracks.
//!
//! Unlike the Albums tab, the filter is local. `getArtists` returns the whole
//! roster in one response — there is no paging to work around — so after the
//! first load every keystroke filters in memory, with no debounce and no
//! request. Server-side search would be slower and would also match artists
//! only through their albums and songs.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::widgets::ListItem;
use tca::Effect;
use tca::Reducer;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

use crate::app::PaneFocus;
use crate::app::albums;
use crate::app::albums::Detail;
use crate::app::albums::DetailKey;
use crate::app::truncate;
use crate::subsonic::Album;
use crate::subsonic::Artist;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Track;
use crate::ui::cover::Cover;
use crate::ui::list;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row(pub Artist);

impl<'a> From<Row> for ListItem<'a> {
    fn from(row: Row) -> Self {
        match row.0.album_count {
            Some(1) => ListItem::from(format!("{} · 1 album", row.0.name)),
            Some(n) => ListItem::from(format!("{} · {n} albums", row.0.name)),
            None => ListItem::from(row.0.name),
        }
    }
}

/// One artist, opened: their albums, and possibly one of those albums.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArtistView {
    pub artist: Artist,
    pub albums: Vec<Album>,
    pub selected: usize,
    pub loading: bool,
    /// Set while one of the artist's albums is open. It takes over the tab,
    /// exactly as an album opened from the Albums tab does.
    pub album: Option<Detail>,
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct State {
    pub query: String,
    pub focus: PaneFocus,
    /// The full roster, as fetched. `items` is this filtered by `query`.
    pub all: Vec<Artist>,
    pub items: list::State<Row>,
    pub loading: bool,
    /// Whether `all` has been fetched, so an empty library is not mistaken for
    /// one that has yet to load and fetched again on every visit.
    pub loaded: bool,
    pub error: Option<String>,
    /// Set while an artist is open. Its presence takes over the tab.
    pub open: Option<ArtistView>,
}

impl State {
    /// Rebuilds the visible rows from the roster and the current query.
    fn refilter(&mut self) {
        let needle = self.query.trim().to_lowercase();
        let rows = self
            .all
            .iter()
            .filter(|a| needle.is_empty() || a.name.to_lowercase().contains(&needle))
            .cloned()
            .map(Row)
            .collect();
        self.items = list::State::new(rows);
    }
}

#[derive(Debug)]
pub enum Action {
    /// Fetch the roster.
    Reload,
    Loaded(Vec<Artist>),
    Failed(String),
    OpenArtist(Box<Artist>),
    AlbumsLoaded {
        artist_id: String,
        albums: Vec<Album>,
    },
    CloseArtist,
    OpenAlbum(Box<Album>),
    DetailLoaded {
        album: Box<Album>,
        tracks: Vec<Track>,
    },
    CoverLoaded {
        album_id: String,
        cover: Box<Cover>,
    },
    CloseAlbum,
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

        Scope::new(
            |s: &mut State| &mut s.items,
            case!(Action::List),
            list::ListFeature::<Row>::default(),
        )
        .on_delegate(
            case!(list::Action::Delegated),
            |state: &mut State, d| match d {
                list::Delegated::Noop(_) => Effect::send(Action::Delegated(Delegated::Noop)),
                list::Delegated::Enter(index) => match state.items.items.get(index).cloned() {
                    Some(Row(artist)) => Effect::send(Action::OpenArtist(Box::new(artist))),
                    None => Effect::none(),
                },
                list::Delegated::Toogle => Effect::none(),
            },
        )
        .or_else(move |state: &mut State, action| match action {
            Action::Event(e) => reduce_event(state, e),

            Action::Reload => {
                state.loading = true;
                state.error = None;
                let library = library.clone();
                Effect::run(move |send| async move {
                    match library.artists().await {
                        Ok(artists) => send.send(Action::Loaded(artists)),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::Loaded(artists) => {
                state.loading = false;
                state.loaded = true;
                state.all = artists;
                state.refilter();
                Effect::none()
            }

            Action::Failed(message) => {
                state.loading = false;
                // Whatever was loading inside an open artist has stopped too,
                // or its "Loading…" line would stay up forever.
                if let Some(view) = &mut state.open {
                    view.loading = false;
                    if let Some(detail) = &mut view.album {
                        detail.loading = false;
                    }
                }
                state.error = Some(message);
                Effect::none()
            }

            Action::OpenArtist(artist) => {
                let artist = *artist;
                state.error = None;
                state.open = Some(ArtistView {
                    artist: artist.clone(),
                    loading: true,
                    ..Default::default()
                });
                let library = library.clone();
                Effect::run(move |send| async move {
                    match library.artist_albums(&artist.id).await {
                        Ok(albums) => send.send(Action::AlbumsLoaded {
                            artist_id: artist.id,
                            albums,
                        }),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::AlbumsLoaded { artist_id, albums } => {
                // The user may have backed out, or opened someone else, while
                // this was in flight.
                if let Some(view) = &mut state.open
                    && view.artist.id == artist_id
                {
                    view.albums = albums;
                    view.selected = 0;
                    view.loading = false;
                }
                Effect::none()
            }

            Action::CloseArtist => {
                state.open = None;
                state.error = None;
                Effect::none()
            }

            Action::OpenAlbum(album) => {
                let Some(view) = &mut state.open else {
                    return Effect::none();
                };
                let album = *album;
                state.error = None;
                view.album = Some(Detail::opening(album.clone()));
                albums::fetch_detail(
                    library.clone(),
                    album,
                    |album_id, cover| Action::CoverLoaded { album_id, cover },
                    |album, tracks| Action::DetailLoaded { album, tracks },
                    Action::Failed,
                )
            }

            Action::DetailLoaded { album, tracks } => {
                if let Some(view) = &mut state.open {
                    Detail::land_tracks(&mut view.album, *album, tracks);
                }
                Effect::none()
            }

            Action::CoverLoaded { album_id, cover } => {
                if let Some(view) = &mut state.open {
                    Detail::land_cover(&mut view.album, &album_id, *cover);
                }
                Effect::none()
            }

            Action::CloseAlbum => {
                if let Some(view) = &mut state.open {
                    view.album = None;
                }
                state.error = None;
                Effect::none()
            }

            Action::Delegated(_) => Effect::none(),
            Action::List(_) => unreachable!("claimed by the scope"),
        })
        .reduce(state, action)
    }
}

/// Routes a key to the innermost open level: an album, else an artist, else
/// the filterable roster.
fn reduce_event(state: &mut State, event: Event) -> Effect<Action> {
    let Event::Key(key) = &event else {
        return Effect::none();
    };
    if key.kind != KeyEventKind::Press {
        return Effect::none();
    }

    if let Some(view) = &mut state.open {
        if let Some(detail) = &mut view.album {
            // Back from an album lands on the artist's albums, not the roster.
            return match detail.key(key.code) {
                DetailKey::Back => Effect::send(Action::CloseAlbum),
                DetailKey::Play { tracks, index } => {
                    Effect::send(Action::Delegated(Delegated::Play { tracks, index }))
                }
                DetailKey::Handled => Effect::none(),
            };
        }
        return match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => Effect::send(Action::CloseArtist),
            KeyCode::Down | KeyCode::Char('j') => {
                view.selected = (view.selected + 1).min(view.albums.len().saturating_sub(1));
                Effect::none()
            }
            KeyCode::Up | KeyCode::Char('k') => {
                view.selected = view.selected.saturating_sub(1);
                Effect::none()
            }
            KeyCode::Enter | KeyCode::Right => match view.albums.get(view.selected).cloned() {
                Some(album) => Effect::send(Action::OpenAlbum(Box::new(album))),
                None => Effect::none(),
            },
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
            state.refilter();
            Effect::none()
        }
        (PaneFocus::Search, KeyCode::Backspace) => {
            state.query.pop();
            state.refilter();
            Effect::none()
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

    if let Some(view) = &state.open {
        match &view.album {
            Some(detail) => albums::draw_detail(ctx, detail, playing),
            None => draw_artist(ctx, view, &state.error),
        }
        return;
    }

    let rows = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).split(ctx.area());

    let focused = state.focus == PaneFocus::Search;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Filter ")
        .title_bottom(if focused {
            " ↓ to list "
        } else {
            " ↑ to filter "
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

    // Status renders where the list would be, never over the field, so the
    // field stays visible and editable.
    if let Some(line) = roster_status(&state) {
        ctx.frame().render_widget(Paragraph::new(line), rows[1]);
    } else {
        list::ui(&mut ctx.with_area(rows[1]), &state.items);
    }
}

/// Status shown in place of the roster: loading, a failure, or an empty result.
fn roster_status(state: &State) -> Option<ratatui::text::Line<'static>> {
    use ratatui::style::Style;
    use ratatui::text::Line;

    use crate::ui::theme::Theme;

    if let Some(error) = &state.error {
        return Some(Line::from(format!("  {error}")).style(Style::new().fg(Theme::ERROR)));
    }
    if state.loading {
        return Some(Line::from("  Loading artists…").style(Theme::dim()));
    }
    if state.loaded && state.items.items.is_empty() {
        let note = if state.all.is_empty() {
            "  No artists on this server"
        } else {
            "  No artist matches"
        };
        return Some(Line::from(note).style(Theme::dim()));
    }
    None
}

/// Columns a row spends besides the album name: the highlight symbol, the
/// year and its gap, and the gap plus right-aligned track count.
const YEAR: usize = 4;
const COUNT: usize = 9; // "99 tracks"
const FURNITURE: usize = 2 + YEAR + 2 + 1 + COUNT;

fn draw_artist(ctx: &mut crate::ui::DrawingContext, view: &ArtistView, error: &Option<String>) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::text::Span;
    use ratatui::widgets::{List, ListState, Paragraph, StatefulWidget};

    use crate::ui::theme::Theme;

    let rows = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)]).split(ctx.area());

    // The tab border no longer names what is open, so the artist is named here.
    let count = match view.albums.len() {
        0 if view.loading => String::new(),
        1 => " · 1 album".to_string(),
        n => format!(" · {n} albums"),
    };
    let header = Line::from(vec![
        Span::raw(" "),
        Span::styled(view.artist.name.clone(), Theme::accent().bold()),
        Span::styled(count, Theme::dim()),
        Span::styled("   ↵ open · Esc back", Theme::dim()),
    ]);
    ctx.frame().render_widget(Paragraph::new(header), rows[0]);

    if let Some(error) = error {
        ctx.frame().render_widget(
            Paragraph::new(Line::from(format!("  {error}")).style(Style::new().fg(Theme::ERROR))),
            rows[1],
        );
        return;
    }
    if view.loading {
        ctx.frame().render_widget(
            Paragraph::new(Line::from("  Loading albums…").style(Theme::dim())),
            rows[1],
        );
        return;
    }

    let width = (rows[1].width as usize).saturating_sub(FURNITURE);
    let items: Vec<ListItem> = view
        .albums
        .iter()
        .map(|album| {
            let year = album
                .year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "----".into());
            let tracks = album
                .song_count
                .map(|n| format!("{n} tracks"))
                .unwrap_or_default();
            ListItem::from(format!(
                "{year:>YEAR$}  {:<width$} {tracks:>COUNT$}",
                truncate(&album.name, width),
            ))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Theme::accent().reversed())
        .highlight_symbol("▶ ")
        // Reserve the marker column even with no row selected, so rows do
        // not jump sideways the moment focus arrives.
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    let mut list_state = ListState::default();
    if !view.albums.is_empty() {
        list_state.select(Some(view.selected));
    }
    StatefulWidget::render(list, rows[1], ctx.frame().buffer_mut(), &mut list_state);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::subsonic::fake::FakeLibrary;
    use std::sync::Arc;
    use tca::TestStore;

    fn artist(id: &str, name: &str) -> Artist {
        Artist {
            id: id.into(),
            name: name.into(),
            ..Default::default()
        }
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
            ..Default::default()
        }
    }

    fn roster() -> Vec<Artist> {
        vec![
            artist("1", "John Coltrane"),
            artist("2", "Miles Davis"),
            artist("3", "Charles Mingus"),
        ]
    }

    fn store_with(state: State) -> TestStore<Feature> {
        TestStore::new(
            Feature {
                library: Arc::new(FakeLibrary {
                    artists: roster(),
                    albums: vec![album("a"), album("b")],
                    tracks: vec![track("t1"), track("t2"), track("t3")],
                    ..Default::default()
                }),
            },
            state,
        )
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    fn names(store: &TestStore<Feature>) -> Vec<String> {
        store
            .state()
            .items
            .items
            .iter()
            .map(|Row(a)| a.name.clone())
            .collect()
    }

    fn loaded_store() -> TestStore<Feature> {
        let mut store = store_with(State::default());
        store.send(Action::Reload, |s| s.loading = true);
        store.settle();
        store
    }

    /// An artist open and still waiting for their albums.
    fn opening(id: &str, name: &str) -> ArtistView {
        ArtistView {
            artist: artist(id, name),
            loading: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn reload_fetches_the_whole_roster() {
        let store = loaded_store();
        assert!(store.state().loaded);
        assert!(!store.state().loading);
        assert_eq!(
            names(&store),
            ["John Coltrane", "Miles Davis", "Charles Mingus"]
        );
    }

    /// Filtering is local: it narrows on the keystroke itself and fetches
    /// nothing.
    #[tokio::test]
    async fn typing_filters_without_a_request() {
        let mut store = loaded_store();
        for c in "MI".chars() {
            store.send(Action::Event(key(KeyCode::Char(c))), |s| {
                s.query.push(c);
                s.refilter();
            });
            assert_eq!(store.pending_len(), 0, "a filter must not fetch");
        }
        assert_eq!(
            names(&store),
            ["Miles Davis", "Charles Mingus"],
            "case-insensitive, matching anywhere in the name"
        );

        store.send(Action::Event(key(KeyCode::Char('l'))), |s| {
            s.query.push('l');
            s.refilter();
        });
        assert_eq!(names(&store), ["Miles Davis"]);

        store.send(Action::Event(key(KeyCode::Backspace)), |s| {
            s.query.pop();
            s.refilter();
        });
        assert_eq!(names(&store).len(), 2, "backspace widens it again");
    }

    #[tokio::test]
    async fn enter_on_an_artist_lists_their_albums() {
        let mut store = loaded_store();
        store.send(Action::Event(key(KeyCode::Down)), |s| {
            s.focus = PaneFocus::List;
            s.items.select(Some(0));
        });
        // In the list, keys go to the list feature, which moves on settle.
        for code in [KeyCode::Down, KeyCode::Enter] {
            store.send(Action::Event(key(code)), |_| {});
            store.settle();
        }

        let view = store.state().open.as_ref().expect("an artist is open");
        assert_eq!(view.artist.name, "Miles Davis");
        assert!(!view.loading);
        assert_eq!(view.albums, vec![album("a"), album("b")]);
    }

    /// Albums for an artist the user has already left must not show up under
    /// the one they moved to.
    #[tokio::test]
    async fn albums_for_a_different_artist_are_ignored() {
        let mut store = store_with(State {
            open: Some(opening("2", "Miles Davis")),
            ..Default::default()
        });
        store.send(
            Action::AlbumsLoaded {
                artist_id: "1".into(),
                albums: vec![album("stale")],
            },
            |_| {},
        );
    }

    #[tokio::test]
    async fn a_late_album_list_is_dropped_after_backing_out() {
        let mut store = store_with(State {
            open: Some(opening("2", "Miles Davis")),
            ..Default::default()
        });
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.receive_next(|s| s.open = None);
        store.send(
            Action::AlbumsLoaded {
                artist_id: "2".into(),
                albums: vec![album("a")],
            },
            |_| {},
        );
        assert!(
            store.state().open.is_none(),
            "a late load must not reopen it"
        );
    }

    /// Artist → album → play, then back out one level at a time.
    #[tokio::test]
    async fn an_album_opens_plays_and_backs_out_level_by_level() {
        let mut store = loaded_store();
        store.send(
            Action::OpenArtist(Box::new(artist("2", "Miles Davis"))),
            |s| s.open = Some(opening("2", "Miles Davis")),
        );
        store.settle();

        // Second album, then open it.
        store.send(Action::Event(key(KeyCode::Char('j'))), |s| {
            s.open.as_mut().unwrap().selected = 1
        });
        store.send(Action::Event(key(KeyCode::Enter)), |_| {});
        store.settle();
        let detail = store.state().open.as_ref().unwrap().album.as_ref();
        let detail = detail.expect("the album should be open");
        assert_eq!(detail.album.id, "b");
        assert_eq!(detail.tracks.len(), 3);
        assert!(!detail.loading);

        // Play from the second track: one play is handed up to the root.
        store.send(Action::Event(key(KeyCode::Down)), |s| {
            s.open.as_mut().unwrap().album.as_mut().unwrap().selected = 1
        });
        store.send(Action::Event(key(KeyCode::Enter)), |_| {});
        assert_eq!(store.pending_len(), 1, "should have delegated a play");
        store.receive_next(|_| {});

        // Esc leaves the album for the artist, not the roster.
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.receive_next(|s| s.open.as_mut().unwrap().album = None);
        assert_eq!(
            store.state().open.as_ref().unwrap().selected,
            1,
            "the album cursor is where it was"
        );

        // And a second Esc leaves the artist.
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        store.receive_next(|s| s.open = None);
        store.finish();
    }

    /// What Enter on a track asks for: the whole album, from that track.
    #[test]
    fn enter_on_a_track_plays_the_album_from_there() {
        let mut detail = Detail {
            tracks: vec![track("t1"), track("t2"), track("t3")],
            selected: 1,
            ..Default::default()
        };
        assert_eq!(
            detail.key(KeyCode::Enter),
            DetailKey::Play {
                tracks: vec![track("t1"), track("t2"), track("t3")],
                index: 1,
            }
        );
    }

    /// A failed fetch inside an open artist must not leave it "loading" forever.
    #[tokio::test]
    async fn a_failure_clears_the_loading_state() {
        let mut store = store_with(State {
            open: Some(opening("2", "Miles Davis")),
            ..Default::default()
        });
        store.send(Action::Failed("boom".into()), |s| {
            s.open.as_mut().unwrap().loading = false;
            s.error = Some("boom".into());
        });
    }

    #[test]
    fn rows_count_albums_in_the_singular_and_plural() {
        let mut a = artist("1", "Nico");
        a.album_count = Some(1);
        let one = format!("{:?}", ListItem::from(Row(a.clone())));
        a.album_count = Some(4);
        let many = format!("{:?}", ListItem::from(Row(a)));
        assert!(
            one.contains("Nico · 1 album") && !one.contains("albums"),
            "{one}"
        );
        assert!(many.contains("Nico · 4 albums"), "{many}");
    }

    /// Renders an open artist at `width` and returns the screen as lines.
    fn artist_screen(width: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        use crate::ui::DrawingContext;
        use crate::ui::ModalQueue;

        let view = ArtistView {
            artist: artist("2", "Miles Davis"),
            albums: vec![
                Album {
                    name: "Kind of Blue".into(),
                    year: Some(1959),
                    song_count: Some(5),
                    ..Default::default()
                },
                Album {
                    name: "Bitches Brew, the Complete Columbia Sessions".into(),
                    year: Some(1970),
                    song_count: Some(12),
                    ..Default::default()
                },
            ],
            selected: 1,
            ..Default::default()
        };

        let mut terminal = Terminal::new(TestBackend::new(width, 5)).expect("test backend");
        terminal
            .draw(|frame| {
                let area = frame.area();
                let queue = ModalQueue::new(area);
                let mut ctx = DrawingContext::new(frame, &queue);
                draw_artist(&mut ctx, &view, &None);
            })
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

    /// The row budget must account for the highlight symbol, the year and the
    /// gaps, or a long title pushes the track count off the right edge — the
    /// "11:33" becoming "11:3" bug the queue once had.
    #[test]
    fn the_track_count_survives_a_long_title() {
        for width in [40, 60, 100] {
            let screen = artist_screen(width);
            assert!(screen[0].contains("Miles Davis"), "{screen:#?}");
            assert!(
                screen[3].trim_end().ends_with("12 tracks"),
                "count cut at {width}: {screen:#?}"
            );
            assert!(screen[3].contains("1970"), "{screen:#?}");
        }
    }
}
