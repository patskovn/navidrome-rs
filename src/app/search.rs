//! The Search tab: a query box over `search3`, listing matching tracks.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use tca::Effect;
use tca::Reducer;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

use crate::app::songs::Row;
use crate::subsonic::SearchResults;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Track;
use crate::ui::list;
use crate::ui::single_line_input;

#[derive(Debug, PartialEq, Clone)]
pub struct State<'a> {
    pub query: single_line_input::State<'a>,
    pub results: list::State<Row>,
    pub album_count: usize,
    pub searching: bool,
    pub error: Option<String>,
    /// True while the query box has the keyboard.
    pub editing: bool,
}

impl Default for State<'_> {
    fn default() -> Self {
        Self {
            query: single_line_input::State::new(
                ratatui::widgets::Block::default()
                    .borders(ratatui::widgets::Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .title("Search")
                    .title_bottom("[/] edit  [Enter] search"),
            ),
            results: list::State::default(),
            album_count: 0,
            searching: false,
            error: None,
            editing: false,
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Submit(String),
    Loaded(SearchResults),
    Failed(String),
    Input(single_line_input::Action),
    List(list::Action),
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop(Event),
    Play { tracks: Vec<Track>, index: usize },
}

pub struct Feature {
    pub library: SharedLibrary,
}

impl Reducer for Feature {
    type State = State<'static>;
    type Action = Action;

    fn reduce(&self, state: &mut Self::State, action: Action) -> Effect<Action> {
        let library = self.library.clone();

        Scope::new(
            |s: &mut Self::State| &mut s.query,
            case!(Action::Input),
            single_line_input::Feature,
        )
        .on_delegate(
            case!(single_line_input::Action::Delegated),
            |state: &mut Self::State, d| match d {
                single_line_input::Delegated::Enter => {
                    let query = state
                        .query
                        .textarea
                        .textarea
                        .lines()
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    state.editing = false;
                    if query.trim().is_empty() {
                        return Effect::none();
                    }
                    Effect::send(Action::Submit(query))
                }
                // Leaving the field returns the keyboard to the results list.
                single_line_input::Delegated::Exit => {
                    state.editing = false;
                    Effect::none()
                }
                single_line_input::Delegated::Noop(e) => {
                    Effect::send(Action::Delegated(Delegated::Noop(e)))
                }
            },
        )
        .then(
            Scope::new(
                |s: &mut Self::State| &mut s.results,
                case!(Action::List),
                list::ListFeature::<Row>::default(),
            )
            .on_delegate(case!(list::Action::Delegated), |state: &mut Self::State, d| {
                match d {
                    list::Delegated::Noop(e) => Effect::send(Action::Delegated(Delegated::Noop(e))),
                    list::Delegated::Enter(index) => {
                        let tracks: Vec<Track> =
                            state.results.items.iter().map(|r| r.0.clone()).collect();
                        if tracks.is_empty() {
                            return Effect::none();
                        }
                        Effect::send(Action::Delegated(Delegated::Play { tracks, index }))
                    }
                    list::Delegated::Toogle => Effect::none(),
                }
            }),
        )
        .or_else(move |state: &mut Self::State, action| match action {
            Action::Submit(query) => {
                state.searching = true;
                state.error = None;
                let library = library.clone();
                Effect::run(move |send| async move {
                    match library.search(&query).await {
                        Ok(results) => send.send(Action::Loaded(results)),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }
            Action::Loaded(results) => {
                state.searching = false;
                state.album_count = results.albums.len();
                state.results = list::State::new(results.tracks.into_iter().map(Row).collect());
                Effect::none()
            }
            Action::Failed(message) => {
                state.searching = false;
                state.error = Some(message);
                Effect::none()
            }
            Action::Event(e) => {
                if state.editing {
                    return Effect::send(Action::Input(single_line_input::Action::Event(e)));
                }
                if let Event::Key(key) = &e
                    && key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Char('/')
                {
                    state.editing = true;
                    // The ported text field is a vim-modal editor that starts in
                    // Normal mode, where letters are commands rather than text.
                    // A search box should accept typing immediately, so drop it
                    // straight into Insert by replaying an `i` — that reuses the
                    // editor's own transition, including its block title and
                    // cursor style, instead of duplicating them here.
                    return Effect::send(Action::Input(single_line_input::Action::Event(
                        Event::Key(crossterm::event::KeyEvent::new(
                            KeyCode::Char('i'),
                            crossterm::event::KeyModifiers::NONE,
                        )),
                    )));
                }
                Effect::send(Action::List(list::Action::Event(e)))
            }
            Action::Delegated(_) => Effect::none(),
            Action::Input(_) | Action::List(_) => unreachable!("claimed by a scope"),
        })
        .reduce(state, action)
    }
}

pub fn ui(ctx: &mut crate::ui::DrawingContext, store: tca::Store<State<'static>, Action>) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;

    let state = store.state();
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).split(ctx.area());

    let area = rows[0];
    ctx.frame()
        .render_widget(state.query.textarea.widget(), area);
    list::ui(&mut ctx.with_area(rows[1]), &state.results);
}
