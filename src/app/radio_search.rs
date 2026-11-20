//! Finding a station to add: a search over radio-browser.info, inside the
//! Radios tab.
//!
//! Shaped like the Albums tab — a field on top, results below, Up and Down
//! between them, a debounced search per keystroke with stale answers dropped
//! by generation. Picking a result does not save it: it hands the station to
//! the add form, prefilled, with preview already on, so choosing is hearing.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use tca::Effect;
use tca::Reducer;

use crate::app::DEBOUNCE;
use crate::app::PaneFocus;
use crate::app::truncate;
use crate::directory::Found;
use crate::directory::SharedDirectory;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub query: String,
    pub focus: PaneFocus,
    pub results: Vec<Found>,
    pub selected: usize,
    pub loading: bool,
    /// Whether `results` answers a real query, so "nothing matched" is not
    /// shown before anything was asked.
    pub searched: bool,
    pub error: Option<String>,
    /// Bumped per keystroke; a search, or its timer, counts only if it still
    /// matches.
    pub generation: u64,
}

#[derive(Debug)]
pub enum Action {
    Event(Event),
    RunSearch {
        query: String,
        generation: u64,
    },
    Loaded {
        generation: u64,
        results: Vec<Found>,
    },
    Failed {
        generation: u64,
        message: String,
    },
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    /// Open the add form on this station.
    Pick(Box<Found>),
    /// Back to the station list.
    Close,
}

pub struct Feature {
    pub directory: SharedDirectory,
}

impl Reducer for Feature {
    type State = State;
    type Action = Action;

    fn reduce(&self, state: &mut State, action: Action) -> Effect<Action> {
        match action {
            Action::Event(event) => self.reduce_event(state, event),

            Action::RunSearch { query, generation } => {
                if generation != state.generation {
                    return Effect::none();
                }
                let directory = self.directory.clone();
                Effect::run(move |send| async move {
                    match directory.search(&query).await {
                        Ok(results) => send.send(Action::Loaded {
                            generation,
                            results,
                        }),
                        Err(err) => send.send(Action::Failed {
                            generation,
                            message: format!("{err:#}"),
                        }),
                    }
                })
            }

            Action::Loaded {
                generation,
                results,
            } => {
                if generation == state.generation {
                    state.loading = false;
                    state.searched = true;
                    state.error = None;
                    state.selected = 0;
                    state.results = results;
                }
                Effect::none()
            }

            Action::Failed {
                generation,
                message,
            } => {
                if generation == state.generation {
                    state.loading = false;
                    state.error = Some(message);
                }
                Effect::none()
            }

            Action::Delegated(_) => Effect::none(),
        }
    }
}

impl Feature {
    fn reduce_event(&self, state: &mut State, event: Event) -> Effect<Action> {
        if let Event::Paste(text) = &event {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            state.focus = PaneFocus::Search;
            state.query.push_str(&text);
            return query_changed(state);
        }

        let Event::Key(key) = &event else {
            return Effect::none();
        };
        if key.kind != KeyEventKind::Press {
            return Effect::none();
        }

        match (state.focus, key.code) {
            (_, KeyCode::Esc) => Effect::send(Action::Delegated(Delegated::Close)),

            (PaneFocus::Search, KeyCode::Down | KeyCode::Enter) => {
                if !state.results.is_empty() {
                    state.focus = PaneFocus::List;
                }
                Effect::none()
            }
            (PaneFocus::Search, KeyCode::Char(c)) => {
                state.query.push(c);
                query_changed(state)
            }
            (PaneFocus::Search, KeyCode::Backspace) => {
                state.query.pop();
                query_changed(state)
            }

            (PaneFocus::List, KeyCode::Up | KeyCode::Char('k')) if state.selected == 0 => {
                state.focus = PaneFocus::Search;
                Effect::none()
            }
            (PaneFocus::List, KeyCode::Up | KeyCode::Char('k')) => {
                state.selected -= 1;
                Effect::none()
            }
            (PaneFocus::List, KeyCode::Down | KeyCode::Char('j')) => {
                state.selected = (state.selected + 1).min(state.results.len().saturating_sub(1));
                Effect::none()
            }
            (PaneFocus::List, KeyCode::Enter) => {
                let Some(found) = state.results.get(state.selected).cloned() else {
                    return Effect::none();
                };
                // Picking starts the station playing, which is exactly what
                // the directory's click count is for. Fire and forget: a
                // missed count must never get in the way of listening.
                let directory = self.directory.clone();
                let uuid = found.uuid.clone();
                Effect::merge([
                    Effect::run(move |_send| async move {
                        if let Err(err) = directory.count_click(&uuid).await {
                            log::debug!("could not report a click: {err:#}");
                        }
                    }),
                    Effect::send(Action::Delegated(Delegated::Pick(Box::new(found)))),
                ])
            }
            // Typing in the list goes back to the field and carries on — the
            // obvious meaning of a letter here.
            (PaneFocus::List, KeyCode::Char(c)) => {
                state.focus = PaneFocus::Search;
                state.query.push(c);
                query_changed(state)
            }
            (PaneFocus::List, KeyCode::Backspace) => {
                state.focus = PaneFocus::Search;
                state.query.pop();
                query_changed(state)
            }
            _ => Effect::none(),
        }
    }
}

/// The query changed: drop what is in flight and, after a pause, ask again.
fn query_changed(state: &mut State) -> Effect<Action> {
    state.generation += 1;
    state.error = None;
    let query = state.query.trim().to_string();
    if query.is_empty() {
        // Nothing to ask; an empty search is a fresh start, not "no results".
        state.loading = false;
        state.searched = false;
        state.results.clear();
        state.selected = 0;
        return Effect::none();
    }
    state.loading = true;
    let generation = state.generation;
    Effect::run(move |send| async move {
        tokio::time::sleep(DEBOUNCE).await;
        send.send(Action::RunSearch { query, generation });
    })
}

/// Columns the right-hand summary gets; the name has the rest.
const SUMMARY: usize = 34;

pub fn ui(ctx: &mut crate::ui::DrawingContext, state: &State) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::text::Span;
    use ratatui::widgets::StatefulWidget;
    use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph};

    use crate::ui::theme::Theme;

    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .split(ctx.area());

    let focused = state.focus == PaneFocus::Search;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Find a station ")
        .title_bottom(Line::styled(" radio-browser.info ", Theme::dim()).right_aligned())
        .border_style(Theme::border(focused));
    let text = if focused {
        format!("{}█", state.query)
    } else {
        state.query.clone()
    };
    ctx.frame()
        .render_widget(Paragraph::new(text).block(block), rows[0]);

    let hints = if focused {
        " type a name or a genre · ↓ results · Esc back"
    } else {
        " ↵ listen and add · ↑↓ move · type to refine · Esc back"
    };
    ctx.frame().render_widget(
        Paragraph::new(Line::from(hints).style(Theme::dim())),
        rows[2],
    );

    let status = if let Some(error) = &state.error {
        Some(Line::from(format!("  {error}")).style(Style::new().fg(Theme::ERROR)))
    } else if state.loading && state.results.is_empty() {
        Some(Line::from("  Searching…").style(Theme::dim()))
    } else if state.searched && state.results.is_empty() {
        Some(Line::from("  No stations match").style(Theme::dim()))
    } else if !state.searched && state.results.is_empty() {
        Some(Line::from(vec![
            Span::styled("  Try ", Theme::dim()),
            Span::raw("lofi"),
            Span::styled(", ", Theme::dim()),
            Span::raw("synthwave"),
            Span::styled(", ", Theme::dim()),
            Span::raw("chillhop"),
            Span::styled(" or a station's name", Theme::dim()),
        ]))
    } else {
        None
    };
    if let Some(line) = status {
        ctx.frame().render_widget(Paragraph::new(line), rows[1]);
        return;
    }

    // Highlight symbol, the name, a gap, and the summary — every column the
    // row spends, so the summary is never the part that gets cut.
    let width = rows[1].width as usize;
    let summary_width = SUMMARY.min(width / 2);
    let name_width = width.saturating_sub(2 + 1 + summary_width);
    let items: Vec<ListItem> = state
        .results
        .iter()
        .map(|found| {
            ListItem::from(Line::from(vec![
                Span::raw(format!(
                    "{:<name_width$} ",
                    truncate(&found.name, name_width)
                )),
                Span::styled(
                    format!(
                        "{:>summary_width$}",
                        truncate(&found.summary(), summary_width)
                    ),
                    Theme::dim(),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Theme::accent().reversed())
        .highlight_symbol("▶ ");
    let mut list_state = ListState::default();
    if state.focus == PaneFocus::List && !state.results.is_empty() {
        list_state.select(Some(state.selected));
    }
    StatefulWidget::render(list, rows[1], ctx.frame().buffer_mut(), &mut list_state);
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::directory::fake::FakeDirectory;
    use std::sync::Arc;
    use tca::TestStore;

    fn found(uuid: &str, name: &str) -> Found {
        Found {
            uuid: uuid.into(),
            name: name.into(),
            stream_url: format!("https://{uuid}.radio.invalid/"),
            ..Default::default()
        }
    }

    fn store(state: State) -> (TestStore<Feature>, Arc<FakeDirectory>) {
        let directory = Arc::new(FakeDirectory {
            results: vec![found("a", "Lofi 24/7"), found("b", "Chillhop")],
            ..Default::default()
        });
        (
            TestStore::new(
                Feature {
                    directory: directory.clone(),
                },
                state,
            ),
            directory,
        )
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    fn with_results() -> State {
        State {
            query: "lofi".into(),
            focus: PaneFocus::List,
            results: vec![found("a", "Lofi 24/7"), found("b", "Chillhop")],
            searched: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn typing_schedules_a_search_and_marks_it_loading() {
        let (mut store, _) = store(State::default());
        store.send(Action::Event(key(KeyCode::Char('l'))), |s| {
            s.query = "l".into();
            s.generation = 1;
            s.loading = true;
        });
    }

    /// The debounce guard: a timer outlived by a newer keystroke does nothing.
    #[tokio::test]
    async fn a_stale_timer_does_not_search() {
        let (mut store, directory) = store(State {
            generation: 2,
            ..Default::default()
        });
        store.send(
            Action::RunSearch {
                query: "lo".into(),
                generation: 1,
            },
            |_| {},
        );
        assert_eq!(store.pending_len(), 0);
        assert!(directory.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_current_search_lands() {
        let (mut store, directory) = store(State {
            query: "lofi".into(),
            generation: 1,
            loading: true,
            ..Default::default()
        });
        store.send(
            Action::RunSearch {
                query: "lofi".into(),
                generation: 1,
            },
            |_| {},
        );
        store.settle();
        assert_eq!(store.state().results.len(), 2);
        assert!(store.state().searched && !store.state().loading);
        assert_eq!(*directory.calls.lock().unwrap(), ["search lofi"]);
    }

    /// Answers can come back out of order; only the newest may land.
    #[tokio::test]
    async fn a_stale_answer_is_dropped() {
        let (mut store, _) = store(State {
            generation: 3,
            loading: true,
            ..Default::default()
        });
        store.send(
            Action::Loaded {
                generation: 2,
                results: vec![found("old", "Old")],
            },
            |_| {},
        );
        store.send(
            Action::Failed {
                generation: 2,
                message: "late".into(),
            },
            |_| {},
        );
    }

    #[tokio::test]
    async fn clearing_the_query_clears_the_results_without_asking() {
        let mut state = with_results();
        state.query = "l".into();
        state.focus = PaneFocus::Search;
        let (mut store, _) = store(state);
        store.send(Action::Event(key(KeyCode::Backspace)), |s| {
            s.query.clear();
            s.generation = 1;
            s.results.clear();
            s.searched = false;
        });
        assert_eq!(store.pending_len(), 0);
    }

    /// Enter on a result hands it to the form and reports the click.
    #[tokio::test]
    async fn picking_hands_the_station_over_and_counts_a_click() {
        let (mut store, directory) = store(with_results());
        store.send(Action::Event(key(KeyCode::Down)), |s| s.selected = 1);
        store.send(Action::Event(key(KeyCode::Enter)), |_| {});
        assert_eq!(store.pending_len(), 1, "a pick should be delegated");
        store.receive_next(|_| {});
        assert_eq!(*directory.calls.lock().unwrap(), ["click b"]);
    }

    #[tokio::test]
    async fn up_from_the_first_result_returns_to_the_field() {
        let (mut store, _) = store(with_results());
        store.send(Action::Event(key(KeyCode::Up)), |s| {
            s.focus = PaneFocus::Search
        });
    }

    /// A letter in the list means "refine", not a command.
    #[tokio::test]
    async fn typing_in_the_list_refines_the_query() {
        let (mut store, _) = store(with_results());
        store.send(Action::Event(key(KeyCode::Char('x'))), |s| {
            s.focus = PaneFocus::Search;
            s.query = "lofix".into();
            s.generation = 1;
            s.loading = true;
        });
    }

    #[tokio::test]
    async fn a_paste_lands_in_the_query() {
        let (mut store, _) = store(State::default());
        store.send(Action::Event(Event::Paste("synthwave\n".into())), |s| {
            s.query = "synthwave".into();
            s.generation = 1;
            s.loading = true;
        });
    }

    #[tokio::test]
    async fn escape_asks_to_close() {
        let (mut store, _) = store(with_results());
        store.send(Action::Event(key(KeyCode::Esc)), |_| {});
        assert_eq!(store.pending_len(), 1);
        store.receive_next(|_| {});
    }

    /// The summary column is budgeted: a long name gives way, the summary
    /// survives — the rule `AGENTS.md` sets for list rows.
    #[test]
    fn a_long_name_gives_way_to_the_summary() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        use crate::ui::DrawingContext;
        use crate::ui::ModalQueue;

        let mut station = found(
            "a",
            "An extremely long station name that goes on and on and on",
        );
        station.codec = Some("MP3".into());
        station.bitrate = Some(128);
        station.country = Some("US".into());
        let state = State {
            results: vec![station],
            searched: true,
            focus: PaneFocus::List,
            ..Default::default()
        };

        let mut terminal = Terminal::new(TestBackend::new(70, 8)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let queue = ModalQueue::new(area);
                let mut ctx = DrawingContext::new(frame, &queue);
                ui(&mut ctx, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row: String = (0..70).map(|x| buffer[(x, 3)].symbol()).collect();
        assert!(row.trim_end().ends_with("MP3 128k · US"), "{row:?}");
        assert!(
            row.contains('…'),
            "the name should be the part cut: {row:?}"
        );
    }
}
