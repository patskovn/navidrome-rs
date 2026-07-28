use ratatui::crossterm::event::Event;
use ratatui::widgets::Block;
use tca::Effect;
use tca::ReducerExt;
use tca::Scope;
use tca::case;

use crate::ui::textfield;

#[derive(Debug, Default, Eq, PartialEq, Clone)]
pub struct State<'a> {
    pub textarea: textfield::State<'a>,
}

impl<'a> State<'a> {
    pub fn new(block: Block<'a>) -> Self {
        Self {
            textarea: textfield::State::new(block),
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Event(Event),
    TextField(textfield::Action),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    Noop(Event),
    Enter,
    Exit,
}

#[derive(Default)]
pub struct Feature;

impl tca::Reducer for Feature {
    type State = State<'static>;
    type Action = Action;

    fn reduce(&self, state: &mut Self::State, action: Action) -> Effect<Action> {
        Scope::new(
            |s: &mut Self::State| &mut s.textarea,
            case!(Action::TextField),
            textfield::Feature,
        )
        .on_delegate(case!(textfield::Action::Delegated), |state: &mut Self::State, delegated| {
            match delegated {
                textfield::Delegated::Updated => {
                    if state.textarea.textarea.lines().len() > 1 {
                        state
                            .textarea
                            .textarea
                            .move_cursor(tui_textarea::CursorMove::Jump(0, u16::MAX));
                        state.textarea.textarea.delete_str(usize::MAX);
                    }
                    Effect::none()
                }
                textfield::Delegated::Quit => Effect::send(Action::Delegated(Delegated::Exit)),
                textfield::Delegated::Commit => Effect::send(Action::Delegated(Delegated::Enter)),
                textfield::Delegated::Noop(e) => {
                    Effect::send(Action::Delegated(Delegated::Noop(e)))
                }
            }
        })
        .or_else(|_state: &mut Self::State, action| match action {
            Action::Event(e) => Effect::send(Action::TextField(textfield::Action::Event(e))),
            // Delegates are this feature's output, not its input.
            Action::Delegated(_) => Effect::none(),
            Action::TextField(_) => unreachable!("claimed by the scope"),
        })
        .reduce(state, action)
    }
}

#[allow(dead_code)]
pub fn ui(ctx: &mut crate::ui::DrawingContext, state: &State) {
    let area = ctx.area();
    ctx.frame().render_widget(state.textarea.widget(), area);
}
