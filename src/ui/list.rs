use std::cmp::max;
use std::cmp::min;

use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::style::Style;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::StatefulWidget;
use tca::Effect;

use crate::ui::theme::Theme;

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

/// Shared by both entry points, so the selection cannot drift between them.
///
/// The highlight comes from `Theme::selection()` rather than a colour of its
/// own: it is the accent, and so follows the window's focus with everything
/// else. This widget was ported with a hard-coded blue, which is why the album
/// and track lists used to stay bright while the rest of the UI greyed out.
fn list_widget(items: Vec<ListItem<'_>>) -> List<'_> {
    List::new(items)
        .highlight_style(Theme::selection())
        .highlight_symbol(">")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
}

pub fn ui<T>(ctx: &mut crate::ui::DrawingContext, state: &State<T>)
where
    T: for<'a> Into<ListItem<'a>>,
    T: Clone,
{
    let items: Vec<ListItem> = state.items.iter().map(|i| i.clone().into()).collect();
    let mut list_state = state.list_state;
    StatefulWidget::render(
        list_widget(items),
        ctx.area(),
        ctx.frame().buffer_mut(),
        &mut list_state,
    );
}

/// Like [`ui`], but flags rows matching `mark` — used to show which track is
/// currently sounding.
pub fn ui_marked<T, F>(ctx: &mut crate::ui::DrawingContext, state: &State<T>, mark: F)
where
    T: for<'a> Into<ListItem<'a>>,
    T: Clone,
    F: Fn(&T) -> bool,
{
    let items: Vec<ListItem> = state
        .items
        .iter()
        .map(|item| {
            let marked = mark(item);
            let list_item = item.clone().into();
            if marked {
                // Not the accent: this says which track is sounding, which is
                // still worth knowing about a window you are not looking at.
                list_item.style(Style::default().fg(Theme::PLAYING))
            } else {
                list_item
            }
        })
        .collect();
    let mut list_state = state.list_state;
    StatefulWidget::render(
        list_widget(items),
        ctx.area(),
        ctx.frame().buffer_mut(),
        &mut list_state,
    );
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

#[cfg(test)]
mod test {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::ui::DrawingContext;
    use crate::ui::ModalQueue;

    #[derive(Debug, Clone, PartialEq)]
    struct Row(&'static str);

    impl<'a> From<Row> for ListItem<'a> {
        fn from(row: Row) -> Self {
            ListItem::from(row.0)
        }
    }

    /// Draws a two-row list with the first selected, and returns the buffer.
    ///
    /// `mark` flags the sounding row, as `ui_marked` does in the app.
    fn draw(mark: Option<&'static str>) -> ratatui::buffer::Buffer {
        let mut state = State::new(vec![Row("first"), Row("second")]);
        state.select(Some(0));

        let mut terminal = Terminal::new(TestBackend::new(20, 2)).expect("test backend");
        terminal
            .draw(|frame| {
                let queue = ModalQueue::new(frame.area());
                let mut ctx = DrawingContext::new(frame, &queue);
                match mark {
                    Some(name) => ui_marked(&mut ctx, &state, |row: &Row| row.0 == name),
                    None => ui(&mut ctx, &state),
                }
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// Regression: this widget was ported from tgpt with a hard-coded blue
    /// highlight, so the album and track lists stayed bright while the rest of
    /// the UI greyed out on losing window focus.
    ///
    /// Asserted as "either accent" rather than by setting the focus flag: that
    /// flag is process-global, and a test that wrote it would race the one in
    /// `app::ui` that does.
    #[test]
    fn the_selected_row_uses_the_accent() {
        // Column 2 is past the ">" highlight symbol, onto the row text.
        let fg = draw(None)[(2, 0)].style().fg;
        assert!(
            fg == Some(Theme::ACCENT) || fg == Some(Theme::ACCENT_UNFOCUSED),
            "the highlight should follow the theme, not a colour of its own: {fg:?}"
        );
        assert_eq!(fg, Theme::selection().fg);
    }

    /// The sounding track keeps its own colour: it says what is playing, which
    /// is worth knowing about a window you are not looking at.
    #[test]
    fn the_marked_row_is_not_the_accent() {
        // Row 1, since the selection wins on row 0 where the two overlap.
        let fg = draw(Some("second"))[(2, 1)].style().fg;
        assert_eq!(fg, Some(Theme::PLAYING));
    }
}
