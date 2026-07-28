use std::cmp::max;
use std::cmp::min;

use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::StatefulWidget;
use tca::Effect;


#[derive(Debug, Default, PartialEq, Clone)]
pub struct State<T>
where
    T: for<'a> Into<ListItem<'a>>,
    T: Clone,
{
    list_state: ListState,
    pub items: Vec<T>,
}

impl<T> State<T>
where
    T: for<'a> Into<ListItem<'a>>,
    T: Clone,
{
    pub fn selected(&self) -> Option<usize> {
        self.list_state.selected()
    }

    pub fn select(&mut self, index: Option<usize>) {
        self.list_state.select(index);
    }

    pub fn new(items: Vec<T>) -> Self {
        State {
            list_state: ListState::default(),
            items,
        }
    }
}

pub fn ui<T>(ctx: &mut crate::ui::DrawingContext, state: &State<T>)
where
    T: for<'a> Into<ListItem<'a>>,
    T: Clone,
{
    let items: Vec<ListItem> = state.items.iter().map(|i| i.clone().into()).collect();
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
                .fg(ratatui::style::Color::Blue),
        )
        .highlight_symbol(">")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    let mut list_state = state.list_state;
    StatefulWidget::render(list, ctx.area(), ctx.frame().buffer_mut(), &mut list_state);
}

#[derive(Debug)]
pub enum Action {
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop(Event),
    Toogle,
    Enter(usize),
}

/// Reducers now carry their own type parameters, since `State` is an
/// associated type rather than a trait parameter.
pub struct ListFeature<T> {
    _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<T> Default for ListFeature<T> {
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> tca::Reducer for ListFeature<T>
where
    T: for<'a> Into<ListItem<'a>>,
    T: Clone,
    T: PartialEq,
{
    type State = State<T>;
    type Action = Action;

    fn reduce(&self, state: &mut State<T>, action: Action) -> Effect<Action> {
        match action {
            Action::Event(e) => match e {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        state.list_state.select(
                            state
                                .list_state
                                .selected()
                                .map(|selected| {
                                    min(selected + 1, state.items.len().saturating_sub(1))
                                })
                                .or(Some(0)),
                        );

                        Effect::none()
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        state.list_state.select(
                            state
                                .list_state
                                .selected()
                                .map(|selected| max(selected, 1) - 1)
                                .or(Some(0)),
                        );
                        Effect::none()
                    }
                    KeyCode::Char(' ') => {
                        state.list_state.selected().map_or(Effect::none(), |_s| {
                            Effect::send(Action::Delegated(Delegated::Toogle))
                        })
                    }
                    KeyCode::Enter => state.list_state.selected().map_or(Effect::none(), |s| {
                        Effect::send(Action::Delegated(Delegated::Enter(s)))
                    }),
                    _ => Effect::send(Action::Delegated(Delegated::Noop(e))),
                },
                _ => Effect::send(Action::Delegated(Delegated::Noop(e))),
            },
            Action::Delegated(_) => Effect::none(),
        }
    }
}
