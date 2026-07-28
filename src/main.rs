//! A terminal Navidrome client, built on `tca-rs`.

mod app;
mod audio;
mod config;
mod subsonic;
mod ui;

use std::io;
use std::sync::Arc;

use anyhow::Context;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::Event;
use futures::FutureExt;
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::EnterAlternateScreen;
use ratatui::crossterm::terminal::LeaveAlternateScreen;
use ratatui::crossterm::terminal::disable_raw_mode;
use ratatui::crossterm::terminal::enable_raw_mode;
use ratatui::prelude::Backend;
use simplelog::CombinedLogger;
use simplelog::WriteLogger;

use crate::app::entry::Action;
use crate::app::entry::Feature;
use crate::app::entry::State;
use crate::audio::AudioEngine;
use crate::audio::TrackCache;
use crate::config::ServerConfig;
use crate::subsonic::Library;
use crate::subsonic::OpenSubsonicLibrary;
use crate::subsonic::SharedLibrary;
use crate::ui::visualizer::Spectrum;

fn configure_logger() -> anyhow::Result<()> {
    let dir = dirs::state_dir()
        .or_else(dirs::cache_dir)
        .context("no state directory")?
        .join("navidrome-rs");
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::File::create(dir.join("latest.log"))?;
    CombinedLogger::init(vec![WriteLogger::new(
        log::LevelFilter::Debug,
        simplelog::Config::default(),
        file,
    )])
    .context("configuring logging")
}

/// Whether an event is worth waking the store for.
///
/// The store notifies observers once per processed action, so anything the app
/// does not reduce still costs a redraw. Mouse motion in particular arrives
/// continuously while the pointer is over the terminal.
///
/// Exhaustive on purpose: adding mouse support should fail to compile here
/// rather than silently drop events.
fn is_actionable(event: &Event) -> bool {
    match event {
        Event::Key(_) | Event::Paste(_) | Event::Resize(..) => true,
        Event::Mouse(_) | Event::FocusGained | Event::FocusLost => false,
    }
}

/// Builds a library client from saved credentials.
///
/// A missing or rejected keychain entry is not fatal: the app falls back to the
/// sign-in screen, which is what a first run does anyway.
fn restore_library(config: Option<&ServerConfig>) -> Option<SharedLibrary> {
    let config = config?;
    let password = match config.password() {
        Ok(password) => password,
        Err(err) => {
            log::warn!("no stored password for {}: {err:#}", config.username);
            return None;
        }
    };
    match OpenSubsonicLibrary::new(&config.url, &config.username, &password) {
        Ok(library) => Some(Arc::new(library) as SharedLibrary),
        Err(err) => {
            log::warn!("could not build a client for {}: {err:#}", config.url);
            None
        }
    }
}

/// Stand-in used before sign-in, so the root feature always has a library to
/// hold. Every call fails, which the UI reports.
struct NotSignedIn;

#[async_trait::async_trait]
impl Library for NotSignedIn {
    async fn ping(&self) -> anyhow::Result<()> {
        anyhow::bail!("not signed in")
    }
    async fn albums(
        &self,
        _: subsonic::AlbumSort,
        _: u32,
        _: u32,
    ) -> anyhow::Result<Vec<subsonic::Album>> {
        anyhow::bail!("not signed in")
    }
    async fn album_tracks(&self, _: &str) -> anyhow::Result<Vec<subsonic::Track>> {
        anyhow::bail!("not signed in")
    }
    async fn all_tracks(&self, _: u32, _: u32) -> anyhow::Result<Vec<subsonic::Track>> {
        anyhow::bail!("not signed in")
    }
    async fn search(&self, _: &str) -> anyhow::Result<subsonic::SearchResults> {
        anyhow::bail!("not signed in")
    }
    fn stream_url(&self, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("not signed in")
    }
}

async fn event_loop<B>(terminal: &mut Terminal<B>) -> anyhow::Result<()>
where
    B: Backend,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let saved = ServerConfig::load();
    let restored = restore_library(saved.as_ref());
    let signed_in = restored.is_some();
    let library = restored.unwrap_or_else(|| Arc::new(NotSignedIn) as SharedLibrary);

    let cache = Arc::new(TrackCache::new(
        config::cache_dir()?,
        config::CACHE_BUDGET_BYTES,
    )?);
    log::info!("cache holds {} bytes", cache.total_bytes());

    let (audio, mut audio_events) = AudioEngine::start(cache);
    let tap = audio.tap().clone();

    let store = tca::Store::new(
        Feature {
            library,
            audio: audio.clone(),
        },
        State::restored(signed_in.then_some(()).and(saved)),
    );

    // Audio runs on its own thread; its events re-enter the store as actions.
    {
        let store = store.clone();
        tokio::spawn(async move {
            while let Some(event) = audio_events.recv().await {
                store.send(Action::Player(app::player::Action::Engine(event)));
            }
        });
    }

    if signed_in {
        store.send(Action::Albums(app::albums::Action::Reload));
    }

    let mut spectrum = Spectrum::new();
    let mut redraws = store.observe();
    let mut terminal_events = crossterm::event::EventStream::new();

    // Draw once up front. The loop below only paints on a store notification,
    // and a signed-out app sends no actions until the user presses a key —
    // which would otherwise leave a blank terminal on launch.
    terminal.draw(|f| app::ui::draw(f, store.clone(), &tap, &mut spectrum))?;

    loop {
        let terminal_event = terminal_events.next().fuse();
        let redraw = redraws.changed().fuse();
        tokio::select! {
            changed = redraw => {
                // `watch` coalesces and never lags, so the only other outcome
                // is the store having quit.
                if changed.is_err() {
                    break;
                }
                terminal.draw(|f| app::ui::draw(f, store.clone(), &tap, &mut spectrum))?;
            }
            maybe_event = terminal_event => {
                match maybe_event {
                    Some(Ok(event)) if is_actionable(&event) => {
                        store.send(Action::Event(event));
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(err)) => return Err(err.into()),
                    None => continue,
                }
            }
        }
    }

    audio.send(audio::Command::Shutdown);
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    configure_logger()?;
    enable_raw_mode()?;

    let mut stderr = io::stderr();
    // Mouse capture is deliberately not enabled: nothing here reduces mouse
    // events, and enabling it would break the terminal's own text selection.
    execute!(stderr, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = event_loop(&mut terminal).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    result
}
