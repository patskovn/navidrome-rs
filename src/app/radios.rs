//! The Radios tab: the server's internet radio stations, and adding new ones.
//!
//! Stations live on the server (Navidrome's own Internet Radios), not in this
//! app, so every Subsonic client signed in to the same account sees them.
//!
//! Adding one is a small form — name, stream URL — with a **preview** toggle.
//! With preview on, the URL is probed (see `crate::probe`) and, if it is
//! audio, played straight away, so a station can be heard before it is saved.
//! The probe re-runs, debounced, as the URL is edited; and when it reads the
//! station's own `icy-name`, an empty name field is filled in from it.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use tca::Effect;
use tca::Reducer;

use crate::app::DEBOUNCE;
use crate::app::truncate;
use crate::probe::ProbeInfo;
use crate::probe::SharedProbe;
use crate::subsonic::SharedLibrary;
use crate::subsonic::Station;
use crate::subsonic::Track;

/// The id a previewed station plays under. The root uses it to stop only a
/// preview — never whatever the user has since moved on to.
pub const PREVIEW_ID: &str = "radio:preview";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormField {
    #[default]
    Name,
    Url,
    Preview,
}

impl FormField {
    fn down(self) -> Self {
        match self {
            Self::Name => Self::Url,
            Self::Url | Self::Preview => Self::Preview,
        }
    }

    fn up(self) -> Self {
        match self {
            Self::Name | Self::Url => Self::Name,
            Self::Preview => Self::Url,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProbeStatus {
    /// Preview is off, or nothing has been asked yet.
    #[default]
    Idle,
    Probing,
    Ready(ProbeInfo),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Form {
    pub name: String,
    pub url: String,
    pub field: FormField,
    pub preview: bool,
    pub probe: ProbeStatus,
    /// Bumped whenever the URL or the toggle changes. A probe — or its debounce
    /// timer — is acted on only if it still matches, which drops stale work
    /// without cancelling it, as the Albums search does.
    pub generation: u64,
    /// Whether this form has started something playing, so that turning the
    /// preview off, or leaving, knows there is something to stop.
    pub previewing: bool,
    pub saving: bool,
    pub error: Option<String>,
}

impl Form {
    /// What to save: the probed address when there is one — it may have been
    /// resolved through a playlist — else what was typed.
    fn stream_url(&self) -> String {
        match &self.probe {
            ProbeStatus::Ready(info) => info.stream_url.clone(),
            _ => self.url.trim().to_string(),
        }
    }

    fn preview_track(&self, info: &ProbeInfo) -> Track {
        let name = match self.name.trim() {
            "" => "Preview".to_string(),
            name => name.to_string(),
        };
        let mut track = Station {
            id: "preview".into(),
            name,
            stream_url: info.stream_url.clone(),
            home_page_url: None,
        }
        .as_track();
        track.id = PREVIEW_ID.into();
        track
    }
}

#[derive(Debug, Default, PartialEq, Clone)]
pub struct State {
    pub stations: Vec<Station>,
    pub selected: usize,
    pub loading: bool,
    /// Whether the list has been fetched, so an empty one is not fetched
    /// again on every visit.
    pub loaded: bool,
    pub error: Option<String>,
    /// Set while a station is being added. Takes over the tab and keyboard.
    pub form: Option<Form>,
    /// Set while asking whether to delete this station.
    pub confirm_delete: Option<Station>,
}

impl State {
    /// Whether the tab wants every key as text or an answer, so that global
    /// single-letter shortcuts must stand aside.
    pub fn captures_keys(&self) -> bool {
        self.form.is_some() || self.confirm_delete.is_some()
    }
}

#[derive(Debug)]
pub enum Action {
    Reload,
    Loaded(Vec<Station>),
    Failed(String),
    OpenForm,
    /// Debounce elapsed, or the toggle was just switched on.
    RunProbe {
        generation: u64,
    },
    Probed {
        generation: u64,
        result: Result<ProbeInfo, String>,
    },
    Save,
    Saved,
    SaveFailed(String),
    Delete(String),
    Deleted,
    Event(Event),
    Delegated(Delegated),
}

#[derive(Debug)]
pub enum Delegated {
    /// Play this, live, in place of whatever is playing.
    Play(Box<Track>),
    /// Stop the preview — if it is still what is playing.
    StopPreview,
}

pub struct Feature {
    pub library: SharedLibrary,
    pub probe: SharedProbe,
}

impl Reducer for Feature {
    type State = State;
    type Action = Action;

    fn reduce(&self, state: &mut State, action: Action) -> Effect<Action> {
        match action {
            Action::Event(event) => reduce_event(state, event),

            Action::Reload => {
                state.loading = true;
                state.error = None;
                let library = self.library.clone();
                Effect::run(move |send| async move {
                    match library.radios().await {
                        Ok(stations) => send.send(Action::Loaded(stations)),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::Loaded(stations) => {
                state.loading = false;
                state.loaded = true;
                state.selected = state.selected.min(stations.len().saturating_sub(1));
                state.stations = stations;
                Effect::none()
            }

            Action::Failed(message) => {
                state.loading = false;
                state.error = Some(message);
                Effect::none()
            }

            Action::OpenForm => {
                state.form = Some(Form::default());
                Effect::none()
            }

            Action::RunProbe { generation } => {
                let Some(form) = &mut state.form else {
                    return Effect::none();
                };
                if !form.preview || form.generation != generation {
                    return Effect::none();
                }
                let url = form.url.trim().to_string();
                if url.is_empty() {
                    form.probe = ProbeStatus::Failed("type or paste a stream URL first".into());
                    return Effect::none();
                }
                form.probe = ProbeStatus::Probing;
                let probe = self.probe.clone();
                Effect::run(move |send| async move {
                    let result = probe.probe(&url).await.map_err(|err| format!("{err:#}"));
                    send.send(Action::Probed { generation, result });
                })
            }

            Action::Probed { generation, result } => {
                let Some(form) = &mut state.form else {
                    return Effect::none();
                };
                // The URL changed, or preview went off, while this was out.
                if !form.preview || form.generation != generation {
                    return Effect::none();
                }
                match result {
                    Ok(info) => {
                        if form.name.trim().is_empty()
                            && let Some(name) = &info.name
                        {
                            form.name = name.clone();
                        }
                        let track = form.preview_track(&info);
                        form.probe = ProbeStatus::Ready(info);
                        form.previewing = true;
                        Effect::send(Action::Delegated(Delegated::Play(Box::new(track))))
                    }
                    Err(message) => {
                        form.probe = ProbeStatus::Failed(message);
                        // Silence beats the previous URL playing on under a
                        // form that now says this one is broken.
                        stop_preview(form)
                    }
                }
            }

            Action::Save => {
                let Some(form) = &mut state.form else {
                    return Effect::none();
                };
                if form.saving {
                    return Effect::none();
                }
                let name = form.name.trim().to_string();
                let url = form.stream_url();
                if name.is_empty() {
                    form.error = Some("give the station a name".into());
                    form.field = FormField::Name;
                    return Effect::none();
                }
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    form.error = Some("the stream URL must start with http:// or https://".into());
                    form.field = FormField::Url;
                    return Effect::none();
                }
                form.saving = true;
                form.error = None;
                let library = self.library.clone();
                Effect::run(move |send| async move {
                    match library.create_radio(&name, &url).await {
                        Ok(()) => send.send(Action::Saved),
                        Err(err) => send.send(Action::SaveFailed(format!("{err:#}"))),
                    }
                })
            }

            Action::Saved => {
                // A preview keeps playing: it is the station that was just
                // saved, and cutting it off would be a strange reward.
                state.form = None;
                Effect::send(Action::Reload)
            }

            Action::SaveFailed(message) => {
                if let Some(form) = &mut state.form {
                    form.saving = false;
                    form.error = Some(message);
                }
                Effect::none()
            }

            Action::Delete(id) => {
                let library = self.library.clone();
                Effect::run(move |send| async move {
                    match library.delete_radio(&id).await {
                        Ok(()) => send.send(Action::Deleted),
                        Err(err) => send.send(Action::Failed(format!("{err:#}"))),
                    }
                })
            }

            Action::Deleted => Effect::send(Action::Reload),

            Action::Delegated(_) => Effect::none(),
        }
    }
}

/// Stops a preview this form started, if it did.
fn stop_preview(form: &mut Form) -> Effect<Action> {
    if !std::mem::take(&mut form.previewing) {
        return Effect::none();
    }
    Effect::send(Action::Delegated(Delegated::StopPreview))
}

/// The URL changed: forget what was learned about the old one, and, with
/// preview on, look again once typing pauses.
fn url_changed(form: &mut Form) -> Effect<Action> {
    form.error = None;
    form.generation += 1;
    if !form.preview {
        form.probe = ProbeStatus::Idle;
        return Effect::none();
    }
    form.probe = ProbeStatus::Probing;
    let generation = form.generation;
    Effect::run(move |send| async move {
        tokio::time::sleep(DEBOUNCE).await;
        send.send(Action::RunProbe { generation });
    })
}

fn toggle_preview(form: &mut Form) -> Effect<Action> {
    form.preview = !form.preview;
    form.generation += 1;
    if form.preview {
        // No debounce: the toggle is a deliberate "try it now".
        Effect::send(Action::RunProbe {
            generation: form.generation,
        })
    } else {
        form.probe = ProbeStatus::Idle;
        stop_preview(form)
    }
}

fn reduce_event(state: &mut State, event: Event) -> Effect<Action> {
    if let Some(form) = &mut state.form {
        return match reduce_form_event(form, event) {
            Ok(effect) => effect,
            // Closing: drop the form, then run what it left to do.
            Err(close) => {
                state.form = None;
                close
            }
        };
    }

    let Event::Key(key) = &event else {
        return Effect::none();
    };
    if key.kind != KeyEventKind::Press {
        return Effect::none();
    }

    // A pending delete takes the next key as its answer, whatever it is.
    if let Some(station) = state.confirm_delete.take() {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Effect::send(Action::Delete(station.id)),
            _ => Effect::none(),
        };
    }

    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(state.stations.len().saturating_sub(1));
            Effect::none()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
            Effect::none()
        }
        KeyCode::Enter => match state.stations.get(state.selected) {
            Some(station) => Effect::send(Action::Delegated(Delegated::Play(Box::new(
                station.as_track(),
            )))),
            None => Effect::none(),
        },
        KeyCode::Char('a') => Effect::send(Action::OpenForm),
        KeyCode::Char('d') | KeyCode::Delete => {
            state.confirm_delete = state.stations.get(state.selected).cloned();
            Effect::none()
        }
        KeyCode::Char('r') => Effect::send(Action::Reload),
        _ => Effect::none(),
    }
}

/// Keys and pastes while the form is open. `Err` carries the effect to run
/// after closing the form.
fn reduce_form_event(form: &mut Form, event: Event) -> Result<Effect<Action>, Effect<Action>> {
    // A pasted URL arrives whole, as one event. Line breaks would only ever be
    // a copying accident in a one-line field.
    if let Event::Paste(text) = &event {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        return Ok(match form.field {
            FormField::Name => {
                form.name.push_str(&text);
                Effect::none()
            }
            FormField::Url => {
                form.url.push_str(text.trim());
                url_changed(form)
            }
            FormField::Preview => Effect::none(),
        });
    }

    let Event::Key(key) = &event else {
        return Ok(Effect::none());
    };
    if key.kind != KeyEventKind::Press {
        return Ok(Effect::none());
    }

    Ok(match (form.field, key.code) {
        (_, KeyCode::Esc) => return Err(stop_preview(form)),
        (_, KeyCode::Down) => {
            form.field = form.field.down();
            Effect::none()
        }
        (_, KeyCode::Up) => {
            form.field = form.field.up();
            Effect::none()
        }
        (FormField::Preview, KeyCode::Char(' ') | KeyCode::Enter) => toggle_preview(form),
        (_, KeyCode::Enter) => Effect::send(Action::Save),
        (FormField::Name, KeyCode::Char(c)) => {
            form.name.push(c);
            form.error = None;
            Effect::none()
        }
        (FormField::Name, KeyCode::Backspace) => {
            form.name.pop();
            Effect::none()
        }
        (FormField::Url, KeyCode::Char(c)) => {
            form.url.push(c);
            url_changed(form)
        }
        (FormField::Url, KeyCode::Backspace) => {
            form.url.pop();
            url_changed(form)
        }
        _ => Effect::none(),
    })
}

pub fn ui(
    ctx: &mut crate::ui::DrawingContext,
    store: tca::Store<State, Action>,
    playing: Option<&str>,
) {
    let state = store.state();
    match &state.form {
        Some(form) => draw_form(ctx, form),
        None => draw_list(ctx, &state, playing),
    }
}

fn draw_list(ctx: &mut crate::ui::DrawingContext, state: &State, playing: Option<&str>) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::widgets::{List, ListItem, ListState, Paragraph, StatefulWidget};

    use crate::ui::theme::Theme;

    let rows = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(ctx.area());

    let footer = match &state.confirm_delete {
        Some(station) => Line::from(format!(
            " Delete “{}” from the server? y to delete, any other key to keep",
            station.name
        ))
        .style(Style::new().fg(Theme::ERROR)),
        None => Line::from(" ↵ play · a add · d delete · r refresh").style(Theme::dim()),
    };
    ctx.frame().render_widget(Paragraph::new(footer), rows[1]);

    let status = if let Some(error) = &state.error {
        Some(Line::from(format!("  {error}")).style(Style::new().fg(Theme::ERROR)))
    } else if state.loading && state.stations.is_empty() {
        Some(Line::from("  Loading stations…").style(Theme::dim()))
    } else if state.loaded && state.stations.is_empty() {
        Some(Line::from("  No stations yet — press a to add one").style(Theme::dim()))
    } else {
        None
    };
    if let Some(line) = status {
        ctx.frame().render_widget(Paragraph::new(line), rows[0]);
        return;
    }

    // Highlight symbol, the name, a gap, and the host on the right.
    let width = rows[0].width as usize;
    let host_width = (width / 3).min(32);
    let name_width = width.saturating_sub(2 + 1 + host_width);
    let items: Vec<ListItem> = state
        .stations
        .iter()
        .map(|station| {
            let host = host_of(&station.stream_url);
            let item = ListItem::from(format!(
                "{:<name_width$} {:>host_width$}",
                truncate(&station.name, name_width),
                truncate(&host, host_width),
            ));
            if playing == Some(station.as_track().id.as_str()) {
                item.style(Style::new().fg(Theme::PLAYING))
            } else {
                item
            }
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Theme::accent().reversed())
        .highlight_symbol("▶ ");
    let mut list_state = ListState::default();
    if !state.stations.is_empty() {
        list_state.select(Some(state.selected));
    }
    StatefulWidget::render(list, rows[0], ctx.frame().buffer_mut(), &mut list_state);
}

/// `radio.example:8000` from `https://radio.example:8000/live.mp3`, for a
/// glance at where a station comes from.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?']).next().unwrap_or(rest).to_string()
}

/// The end of `text` when it is too long — for a URL being typed, the end is
/// where the cursor is and the part worth seeing.
fn tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out = String::from("…");
    out.extend(text.chars().skip(count - keep));
    out
}

fn draw_form(ctx: &mut crate::ui::DrawingContext, form: &Form) {
    use ratatui::layout::Constraint;
    use ratatui::layout::Layout;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::text::Span;
    use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

    use crate::ui::theme::Theme;

    let rows = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Length(3), // name
        Constraint::Length(3), // url
        Constraint::Length(1), // toggle
        Constraint::Fill(1),   // probe result, error
        Constraint::Length(1), // hints
    ])
    .split(ctx.area());

    ctx.frame().render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Add a station",
            Theme::accent().bold(),
        ))),
        rows[0],
    );

    let field = |title: &'static str, value: &str, focused: bool, area: ratatui::layout::Rect| {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title)
            .border_style(Theme::border(focused));
        // Room for the caret, so the end of the text is never hidden under it.
        let width = block.inner(area).width.saturating_sub(1) as usize;
        let mut text = tail(value, width);
        if focused {
            text.push('█');
        }
        (Paragraph::new(text).block(block), area)
    };
    let (name, area) = field(" Name ", &form.name, form.field == FormField::Name, rows[1]);
    ctx.frame().render_widget(name, area);
    let (url, area) = field(
        " Stream URL ",
        &form.url,
        form.field == FormField::Url,
        rows[2],
    );
    ctx.frame().render_widget(url, area);

    let toggle_focused = form.field == FormField::Preview;
    let toggle = Line::from(vec![
        Span::styled(
            if form.preview { " [x] " } else { " [ ] " },
            if toggle_focused {
                Theme::accent().reversed()
            } else {
                Theme::accent()
            },
        ),
        Span::raw(" Preview"),
        Span::styled(" — check the URL and play it now", Theme::dim()),
    ]);
    ctx.frame().render_widget(Paragraph::new(toggle), rows[3]);

    let mut lines = vec![Line::from("")];
    match &form.probe {
        ProbeStatus::Idle => {}
        ProbeStatus::Probing => lines.push(Line::from("   Checking…").style(Theme::dim())),
        ProbeStatus::Ready(info) => {
            let mut facts = vec![info.content_type.clone()];
            if let Some(br) = info.bitrate {
                facts.push(format!("{br} kbps"));
            }
            if let Some(name) = &info.name {
                facts.push(name.clone());
            }
            if let Some(genre) = &info.genre {
                facts.push(genre.clone());
            }
            lines.push(Line::from(vec![
                Span::styled("   ✓ ", Style::new().fg(Theme::PLAYING)),
                Span::raw(format!("Playing · {}", facts.join(" · "))),
            ]));
            if info.stream_url != form.url.trim() {
                lines.push(
                    Line::from(format!("     via playlist → {}", info.stream_url))
                        .style(Theme::dim()),
                );
            }
        }
        ProbeStatus::Failed(message) => {
            lines.push(Line::from(format!("   ✗ {message}")).style(Style::new().fg(Theme::ERROR)))
        }
    }
    if form.saving {
        lines.push(Line::from("   Saving…").style(Theme::dim()));
    }
    if let Some(error) = &form.error {
        lines.push(Line::from(format!("   {error}")).style(Style::new().fg(Theme::ERROR)));
    }
    ctx.frame()
        .render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[4]);

    let hints = if toggle_focused {
        " space toggle preview · ↑ fields · Esc cancel"
    } else {
        " ↵ save · ↑↓ field · Esc cancel"
    };
    ctx.frame().render_widget(
        Paragraph::new(Line::from(hints).style(Theme::dim())),
        rows[5],
    );
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::probe::fake::FakeProbe;
    use crate::subsonic::fake::FakeLibrary;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tca::TestStore;

    fn station(id: &str, name: &str) -> Station {
        Station {
            id: id.into(),
            name: name.into(),
            // `.invalid` never resolves: nothing here can reach a real server.
            stream_url: format!("https://{id}.radio.invalid/live.mp3"),
            home_page_url: None,
        }
    }

    fn info(url: &str) -> ProbeInfo {
        ProbeInfo {
            stream_url: url.into(),
            content_type: "audio/mpeg".into(),
            name: Some("Chillhop FM".into()),
            bitrate: Some(192),
            genre: None,
        }
    }

    struct Harness {
        store: TestStore<Feature>,
        library: Arc<FakeLibrary>,
        probe: Arc<FakeProbe>,
    }

    fn harness(probe: FakeProbe, state: State) -> Harness {
        let library = Arc::new(FakeLibrary {
            stations: Mutex::new(vec![station("1", "Lofi 24/7"), station("2", "Synthwave")]),
            ..Default::default()
        });
        let probe = Arc::new(probe);
        let store = TestStore::new(
            Feature {
                library: library.clone(),
                probe: probe.clone(),
            },
            state,
        );
        Harness {
            store,
            library,
            probe,
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    /// A form on the toggle row, with a URL typed and nothing probed yet.
    fn form_on_toggle(url: &str) -> State {
        State {
            form: Some(Form {
                url: url.into(),
                field: FormField::Preview,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn form(store: &TestStore<Feature>) -> &Form {
        store.state().form.as_ref().expect("the form is open")
    }

    #[tokio::test]
    async fn reload_lists_the_servers_stations() {
        let mut h = harness(FakeProbe::failing("unused"), State::default());
        h.store.send(Action::Reload, |s| s.loading = true);
        h.store.settle();
        let names: Vec<_> = h.store.state().stations.iter().map(|s| &s.name).collect();
        assert_eq!(names, ["Lofi 24/7", "Synthwave"]);
        assert!(h.store.state().loaded);
    }

    /// The whole feature in one pass: switching preview on probes the URL,
    /// fills the empty name from the station's own, and plays it.
    #[tokio::test]
    async fn preview_probes_names_and_plays_the_url() {
        let url = "https://chill.radio.invalid/192";
        let mut h = harness(FakeProbe::ok(info(url)), form_on_toggle(url));

        h.store.send(Action::Event(key(KeyCode::Char(' '))), |s| {
            let form = s.form.as_mut().unwrap();
            form.preview = true;
            form.generation = 1;
        });
        h.store.settle();

        let form = form(&h.store);
        assert_eq!(form.probe, ProbeStatus::Ready(info(url)));
        assert_eq!(form.name, "Chillhop FM", "an empty name takes the icy-name");
        assert!(form.previewing);
        assert_eq!(*h.probe.calls.lock().unwrap(), [url]);
    }

    /// What the preview hands up to be played: the probed address, live.
    #[test]
    fn the_preview_plays_the_probed_address_live() {
        let form = Form {
            name: "Mine".into(),
            ..Default::default()
        };
        let track = form.preview_track(&info("https://x.radio.invalid/real"));
        assert_eq!(track.id, PREVIEW_ID);
        assert_eq!(track.title, "Mine");
        assert_eq!(
            track.live_url.as_deref(),
            Some("https://x.radio.invalid/real")
        );
    }

    #[tokio::test]
    async fn a_name_already_typed_is_kept() {
        let url = "https://chill.radio.invalid/192";
        let mut state = form_on_toggle(url);
        state.form.as_mut().unwrap().name = "My lofi".into();
        let mut h = harness(FakeProbe::ok(info(url)), state);

        h.store.send(Action::Event(key(KeyCode::Char(' '))), |s| {
            let form = s.form.as_mut().unwrap();
            form.preview = true;
            form.generation = 1;
        });
        h.store.settle();
        assert_eq!(form(&h.store).name, "My lofi");
    }

    /// A probe answering for a URL that has since been edited must not land —
    /// it would play the old address under the new one.
    #[tokio::test]
    async fn a_stale_probe_is_dropped() {
        let mut state = form_on_toggle("https://new.radio.invalid/");
        let form_state = state.form.as_mut().unwrap();
        form_state.preview = true;
        form_state.generation = 2;
        form_state.probe = ProbeStatus::Probing;
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(
            Action::Probed {
                generation: 1,
                result: Ok(info("https://old.radio.invalid/")),
            },
            |_| {},
        );
        assert_eq!(h.store.pending_len(), 0, "nothing should play");
    }

    #[tokio::test]
    async fn editing_the_url_reprobes_while_previewing() {
        let mut state = form_on_toggle("https://a.radio.invalid/");
        let form_state = state.form.as_mut().unwrap();
        form_state.preview = true;
        form_state.field = FormField::Url;
        form_state.probe = ProbeStatus::Ready(info("https://a.radio.invalid/"));
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(Action::Event(key(KeyCode::Char('x'))), |s| {
            let form = s.form.as_mut().unwrap();
            form.url.push('x');
            form.generation += 1;
            form.probe = ProbeStatus::Probing;
        });
    }

    #[tokio::test]
    async fn without_preview_editing_probes_nothing() {
        let mut state = form_on_toggle("https://a.radio.invalid/");
        state.form.as_mut().unwrap().field = FormField::Url;
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(Action::Event(key(KeyCode::Char('x'))), |s| {
            let form = s.form.as_mut().unwrap();
            form.url.push('x');
            form.generation += 1;
        });
        assert_eq!(h.store.pending_len(), 0);
        assert!(h.probe.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn switching_preview_off_stops_it() {
        let mut state = form_on_toggle("https://a.radio.invalid/");
        let form_state = state.form.as_mut().unwrap();
        form_state.preview = true;
        form_state.previewing = true;
        form_state.probe = ProbeStatus::Ready(info("https://a.radio.invalid/"));
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(Action::Event(key(KeyCode::Enter)), |s| {
            let form = s.form.as_mut().unwrap();
            form.preview = false;
            form.previewing = false;
            form.generation += 1;
            form.probe = ProbeStatus::Idle;
        });
        assert_eq!(h.store.pending_len(), 1, "should ask to stop the preview");
    }

    /// A broken URL must not leave the last good one playing under it.
    #[tokio::test]
    async fn a_failed_probe_silences_the_previous_preview() {
        let mut state = form_on_toggle("https://broken.radio.invalid/");
        let form_state = state.form.as_mut().unwrap();
        form_state.preview = true;
        form_state.previewing = true;
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(
            Action::Probed {
                generation: 0,
                result: Err("the server answered 404 Not Found".into()),
            },
            |s| {
                let form = s.form.as_mut().unwrap();
                form.previewing = false;
                form.probe = ProbeStatus::Failed("the server answered 404 Not Found".into());
            },
        );
        assert_eq!(h.store.pending_len(), 1, "should ask to stop the preview");
    }

    #[tokio::test]
    async fn saving_needs_a_name_and_a_web_url() {
        let mut state = form_on_toggle("radio");
        state.form.as_mut().unwrap().field = FormField::Url;
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(Action::Save, |s| {
            let form = s.form.as_mut().unwrap();
            form.error = Some("give the station a name".into());
            form.field = FormField::Name;
        });

        assert!(h.library.calls.lock().unwrap().is_empty(), "nothing saved");

        let mut state = form_on_toggle("radio");
        state.form.as_mut().unwrap().name = "Mine".into();
        let mut h = harness(FakeProbe::failing("unused"), state);
        h.store.send(Action::Save, |s| {
            let form = s.form.as_mut().unwrap();
            form.error = Some("the stream URL must start with http:// or https://".into());
            form.field = FormField::Url;
        });
        assert!(h.library.calls.lock().unwrap().is_empty(), "nothing saved");
    }

    /// A playlist URL resolves to its stream; that address is what is saved,
    /// since the player streams it directly.
    #[tokio::test]
    async fn saving_stores_the_probed_address_and_closes() {
        let mut state = form_on_toggle("https://x.radio.invalid/station.pls");
        let form_state = state.form.as_mut().unwrap();
        form_state.name = "Chill".into();
        form_state.preview = true;
        form_state.previewing = true;
        form_state.probe = ProbeStatus::Ready(info("https://x.radio.invalid/stream"));
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store.send(Action::Save, |s| {
            let form = s.form.as_mut().unwrap();
            form.saving = true;
        });
        h.store.settle();

        assert!(h.store.state().form.is_none(), "the form closes");
        let saved = h.store.state().stations.last().unwrap().clone();
        assert_eq!(saved.name, "Chill");
        assert_eq!(saved.stream_url, "https://x.radio.invalid/stream");
    }

    #[tokio::test]
    async fn escape_closes_the_form_and_stops_its_preview() {
        let mut state = form_on_toggle("https://a.radio.invalid/");
        state.form.as_mut().unwrap().previewing = true;
        let mut h = harness(FakeProbe::failing("unused"), state);

        h.store
            .send(Action::Event(key(KeyCode::Esc)), |s| s.form = None);
        assert_eq!(h.store.pending_len(), 1, "should ask to stop the preview");
    }

    #[tokio::test]
    async fn delete_asks_first_and_any_other_key_keeps_it() {
        let mut h = harness(FakeProbe::failing("unused"), State::default());
        h.store.send(Action::Reload, |s| s.loading = true);
        h.store.settle();

        h.store.send(Action::Event(key(KeyCode::Char('d'))), |s| {
            s.confirm_delete = Some(station("1", "Lofi 24/7"))
        });
        h.store.send(Action::Event(key(KeyCode::Char('n'))), |s| {
            s.confirm_delete = None
        });
        assert_eq!(h.store.pending_len(), 0);

        h.store.send(Action::Event(key(KeyCode::Char('d'))), |s| {
            s.confirm_delete = Some(station("1", "Lofi 24/7"))
        });
        h.store.send(Action::Event(key(KeyCode::Char('y'))), |s| {
            s.confirm_delete = None
        });
        h.store.settle();
        let names: Vec<_> = h.store.state().stations.iter().map(|s| &s.name).collect();
        assert_eq!(names, ["Synthwave"]);
    }

    #[test]
    fn long_urls_show_their_end() {
        assert_eq!(
            tail("https://a.example/stream", 50),
            "https://a.example/stream"
        );
        assert_eq!(tail("https://a.example/stream", 8), "…/stream");
    }

    #[test]
    fn hosts_are_read_from_urls() {
        assert_eq!(
            host_of("https://radio.example:8000/live.mp3"),
            "radio.example:8000"
        );
        assert_eq!(host_of("http://x.example/proxy/jamz?mp=/1"), "x.example");
    }
}
