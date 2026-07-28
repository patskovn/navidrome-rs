//! The Albums tab: a list of albums, Enter plays one.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::widgets::ListItem;
use tca::Effect;
use tca::Reducer;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

use crate::subsonic::Album;
use crate::subsonic::AlbumSort;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Track;
use crate::ui::list;

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
    pub albums: list::State<Row>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum Action {
    Reload,
    Loaded(Vec<Album>),
    Failed(String),
    /// Tracks fetched for an album the user chose.
    Play(Vec<Track>),
    List(list::Action),
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop(Event),
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
            |s: &mut State| &mut s.albums,
            case!(Action::List),
            list::ListFeature::<Row>::default(),
        )
        .on_delegate(case!(list::Action::Delegated), move |state: &mut State, d| {
            match d {
                list::Delegated::Noop(e) => Effect::send(Action::Delegated(Delegated::Noop(e))),
                list::Delegated::Enter(index) => {
                    let Some(Row(album)) = state.albums.items.get(index).cloned() else {
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
            Action::Reload => {
                state.loading = true;
                state.error = None;
                let library = library.clone();
                Effect::run(move |send| async move {
                    match library.albums(AlbumSort::Alphabetical, 0, 500).await {
                        Ok(albums) => send.send(Action::Loaded(albums)),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }
            Action::Loaded(albums) => {
                state.loading = false;
                state.albums = list::State::new(albums.into_iter().map(Row).collect());
                Effect::none()
            }
            Action::Failed(message) => {
                state.loading = false;
                state.error = Some(message);
                Effect::none()
            }
            Action::Play(tracks) => Effect::send(Action::Delegated(Delegated::Play(tracks))),
            Action::Event(e) => {
                // `r` reloads; everything else goes to the list.
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
    list::ui(ctx, &state.albums);
}
