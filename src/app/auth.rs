//! Sign-in: server URL, username, password.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use tca::Effect;
use tca::Reducer;

use crate::config::ServerConfig;
use crate::ui::single_line_input;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Field {
    #[default]
    Url,
    Username,
    Password,
}

impl Field {
    fn next(self) -> Self {
        match self {
            Self::Url => Self::Username,
            Self::Username => Self::Password,
            Self::Password => Self::Url,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct State<'a> {
    pub url: single_line_input::State<'a>,
    pub username: single_line_input::State<'a>,
    pub password: single_line_input::State<'a>,
    pub focused: Field,
    pub checking: bool,
    pub error: Option<String>,
}

fn field_block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title.to_string())
}

impl Default for State<'_> {
    fn default() -> Self {
        Self {
            url: single_line_input::State::new(field_block("Server URL")),
            username: single_line_input::State::new(field_block("Username")),
            password: single_line_input::State::new(field_block("Password")),
            focused: Field::default(),
            checking: false,
            error: None,
        }
    }
}

impl<'a> State<'a> {
    fn value(input: &single_line_input::State<'_>) -> String {
        input.textarea.textarea.lines().first().cloned().unwrap_or_default()
    }

    pub fn credentials(&self) -> (String, String, String) {
        (
            Self::value(&self.url).trim().to_string(),
            Self::value(&self.username).trim().to_string(),
            Self::value(&self.password),
        )
    }

    fn focused_input(&mut self) -> &mut single_line_input::State<'a> {
        let field = self.focused;
        match field {
            Field::Url => &mut self.url,
            Field::Username => &mut self.username,
            Field::Password => &mut self.password,
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Event(Event),
    Submit,
    Succeeded(ServerConfig),
    Failed(String),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    /// Credentials verified and persisted.
    SignedIn(ServerConfig),
    Noop(Event),
}

pub struct Feature;

impl Reducer for Feature {
    type State = State<'static>;
    type Action = Action;

    fn reduce(&self, state: &mut Self::State, action: Action) -> Effect<Action> {
        match action {
            Action::Event(e) => {
                if let Event::Key(key) = &e
                    && key.kind == KeyEventKind::Press
                {
                    match key.code {
                        KeyCode::Tab => {
                            state.focused = state.focused.next();
                            return Effect::none();
                        }
                        KeyCode::Enter => return Effect::send(Action::Submit),
                        KeyCode::Backspace => {
                            state.focused_input().textarea.textarea.delete_char();
                            return Effect::none();
                        }
                        KeyCode::Char(c) => {
                            state.focused_input().textarea.textarea.insert_char(c);
                            return Effect::none();
                        }
                        _ => {}
                    }
                }
                Effect::send(Action::Delegated(Delegated::Noop(e)))
            }

            Action::Submit => {
                let (url, username, password) = state.credentials();
                if url.is_empty() || username.is_empty() {
                    state.error = Some("Server URL and username are required".into());
                    return Effect::none();
                }
                state.checking = true;
                state.error = None;

                Effect::run(move |send| async move {
                    let config = ServerConfig { url, username };
                    // Verify before persisting, so a typo never becomes the
                    // saved configuration.
                    match crate::subsonic::OpenSubsonicLibrary::new(
                        &config.url,
                        &config.username,
                        &password,
                    ) {
                        Ok(library) => {
                            use crate::subsonic::Library as _;
                            match library.ping().await {
                                Ok(()) => {
                                    if let Err(err) = config.store_password(&password) {
                                        send.send(Action::Failed(format!(
                                            "signed in, but could not save the password: {err:#}"
                                        )));
                                        return;
                                    }
                                    if let Err(err) = config.save() {
                                        send.send(Action::Failed(format!("{err:#}")));
                                        return;
                                    }
                                    send.send(Action::Succeeded(config));
                                }
                                Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                            }
                        }
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::Succeeded(config) => {
                state.checking = false;
                Effect::send(Action::Delegated(Delegated::SignedIn(config)))
            }

            Action::Failed(message) => {
                state.checking = false;
                state.error = Some(message);
                Effect::none()
            }

            Action::Delegated(_) => Effect::none(),
        }
    }
}

pub fn ui(ctx: &mut crate::ui::DrawingContext, store: tca::Store<State<'static>, Action>) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::widgets::Paragraph;

    use crate::ui::theme::Theme;

    let state = store.state();
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .split(ctx.area());

    for (index, (field, input)) in [
        (Field::Url, &state.url),
        (Field::Username, &state.username),
        (Field::Password, &state.password),
    ]
    .into_iter()
    .enumerate()
    {
        let mut widget = input.textarea.clone();
        if let Some(block) = widget.textarea.block() {
            let focused = state.focused == field;
            widget
                .textarea
                .set_block(block.clone().border_style(Theme::border(focused)));
        }
        // The password is masked; everything else shows as typed.
        if field == Field::Password {
            widget.textarea.set_mask_char('•');
        }
        ctx.frame().render_widget(widget.widget(), rows[index]);
    }

    let status = match (&state.error, state.checking) {
        (Some(error), _) => Line::from(error.as_str()).style(Style::new().fg(Theme::ERROR)),
        (None, true) => Line::from("Signing in…").style(Theme::dim()),
        (None, false) => {
            Line::from("[Tab] next field   [Enter] sign in").style(Theme::dim())
        }
    };
    let area = rows[3];
    ctx.frame().render_widget(Paragraph::new(status), area);
}
