//! The All Music tab: a live search field over a track list.
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
use crate::app::truncate;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Track;
use crate::ui::list;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row(pub Track);

impl<'a> From<Row> for ListItem<'a> {
    fn from(row: Row) -> Self {
        ListItem::from(format!(
            "{:<52} {}",
            truncate(&row.0.display(), 52),
            row.0.duration_display()
        ))
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
    RunQuery {
        query: String,
        generation: u64,
    },
    Loaded {
        items: Vec<Track>,
        generation: u64,
    },
    Failed(String),

    List(list::Action),
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop,
    /// Play the visible list, starting at this index.
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
                    let tracks: Vec<Track> =
                        state.items.items.iter().map(|r| r.0.clone()).collect();
                    if tracks.is_empty() {
                        return Effect::none();
                    }
                    Effect::send(Action::Delegated(Delegated::Play { tracks, index }))
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
                        library.all_tracks(0, 40).await
                    } else {
                        library.search(&query).await.map(|r| r.tracks)
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
        crate::ui::list::ui_marked(&mut ctx.with_area(rows[1]), &state.items, |row| {
            playing.is_some_and(|id| id == row.0.id)
        });
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
