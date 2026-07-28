//! The All Music tab: every track the server knows about, flattened.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::widgets::ListItem;
use tca::Effect;
use tca::Reducer;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

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

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct State {
    pub tracks: list::State<Row>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum Action {
    Reload,
    Loaded(Vec<Track>),
    Failed(String),
    List(list::Action),
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop(Event),
    /// Play the whole visible list, starting at this index.
    Play { tracks: Vec<Track>, index: usize },
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
            |s: &mut State| &mut s.tracks,
            case!(Action::List),
            list::ListFeature::<Row>::default(),
        )
        .on_delegate(case!(list::Action::Delegated), |state: &mut State, d| match d {
            list::Delegated::Noop(e) => Effect::send(Action::Delegated(Delegated::Noop(e))),
            list::Delegated::Enter(index) => {
                let tracks: Vec<Track> = state.tracks.items.iter().map(|r| r.0.clone()).collect();
                if tracks.is_empty() {
                    return Effect::none();
                }
                Effect::send(Action::Delegated(Delegated::Play { tracks, index }))
            }
            list::Delegated::Toogle => Effect::none(),
        })
        .or_else(move |state: &mut State, action| match action {
            Action::Reload => {
                state.loading = true;
                state.error = None;
                let library = library.clone();
                Effect::run(move |send| async move {
                    // One page of albums' worth of tracks; enough to browse
                    // without pulling an entire library into memory.
                    match library.all_tracks(0, 40).await {
                        Ok(tracks) => send.send(Action::Loaded(tracks)),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }
            Action::Loaded(tracks) => {
                state.loading = false;
                state.tracks = list::State::new(tracks.into_iter().map(Row).collect());
                Effect::none()
            }
            Action::Failed(message) => {
                state.loading = false;
                state.error = Some(message);
                Effect::none()
            }
            Action::Event(e) => {
                if let Event::Key(key) = &e
                    && key.kind == KeyEventKind::Press
                    && key.code == KeyCode::Char('r')
                {
                    return Effect::send(Action::Reload);
                }
                Effect::send(Action::List(list::Action::Event(e)))
            }
            Action::Delegated(_) => Effect::none(),
            Action::List(_) => unreachable!("claimed by the scope"),
        })
        .reduce(state, action)
    }
}

pub fn ui(ctx: &mut crate::ui::DrawingContext, store: tca::Store<State, Action>) {
    let state = store.state();
    list::ui(ctx, &state.tracks);
}
