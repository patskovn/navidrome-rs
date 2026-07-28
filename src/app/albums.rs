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
use crate::ui::list;

use crate::subsonic::Album;
use crate::subsonic::AlbumSort;

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
}

#[derive(Debug)]
pub enum Action {
    /// Load the default listing (empty query).
    Reload,
    /// The query text changed; schedule a debounced search.
    QueryChanged,
    /// Debounce elapsed. Ignored if the generation has moved on.
    RunQuery { query: String, generation: u64 },
    Loaded { items: Vec<Album>, generation: u64 },
    Failed(String),
    /// Tracks fetched for an album the user chose.
    Play(Vec<Track>),
    List(list::Action),
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop,
    /// Hand a whole album to the player.
    Play(Vec<Track>),
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
        .on_delegate(case!(list::Action::Delegated), move |state: &mut State, d| {
            match d {
                list::Delegated::Noop(_) => Effect::send(Action::Delegated(Delegated::Noop)),
                list::Delegated::Enter(index) => { let _ = &for_enter; 
                    let Some(Row(album)) = state.items.items.get(index).cloned() else {
                        return Effect::none();
                    };
                    let library = for_enter.clone();
                    Effect::run(move |send| async move {
                        match library.album_tracks(&album.id).await {
                            Ok(tracks) => send.send(Action::Play(tracks)),
                            Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                        }
                    })
                 }
                list::Delegated::Toogle => Effect::none(),
            }
        })
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

            Action::Play(tracks) => Effect::send(Action::Delegated(Delegated::Play(tracks))),

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

pub fn ui(ctx: &mut crate::ui::DrawingContext, store: tca::Store<State, Action>) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

    use crate::ui::theme::Theme;

    let state = store.state();
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).split(ctx.area());

    let focused = state.focus == PaneFocus::Search;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Search ")
        .title_bottom(if focused { " ↓ to list " } else { " ↑ to search " })
        .border_style(Theme::border(focused));
    // A block cursor marks the caret without needing a real text widget.
    let text = if focused {
        format!("{}█", state.query)
    } else {
        state.query.clone()
    };
    ctx.frame()
        .render_widget(Paragraph::new(text).block(block), rows[0]);

    list::ui(&mut ctx.with_area(rows[1]), &state.items);
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
            store.state().generation, 4,
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
        store.send(Action::Event(key(KeyCode::Char('a'))), |s| s.query.push('a'));
        store.settle();
        let stale = store.state().generation;

        store.send(Action::Event(key(KeyCode::Char('b'))), |s| s.query.push('b'));
        store.settle();

        // Replay the older timer; it should do nothing at all.
        store.send(
            Action::RunQuery {
                query: "a".into(),
                generation: stale,
            },
            |_| {},
        );
        assert!(!store.state().loading, "a stale timer must not start a search");
        assert_eq!(store.pending_len(), 0, "and must emit nothing");
    }

    /// Results can come back out of order; only the newest may land.
    #[tokio::test]
    async fn a_stale_result_is_discarded() {
        let mut store = store();
        store.send(Action::Event(key(KeyCode::Char('x'))), |s| s.query.push('x'));
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
