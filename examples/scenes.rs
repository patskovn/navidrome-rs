//! Look at the pixel-art scenes without the rest of the app — no server, no
//! sign-in, no audio.
//!
//! ```sh
//! cargo run --example scenes                 # fullscreen; ← → to switch
//! cargo run --example scenes -- train        # start on a scene, by name or number
//! cargo run --example scenes -- --png out    # write frames as PNGs instead
//! cargo run --example scenes -- --png out 300x70
//! cargo run --example scenes -- --png out 160x90 --zoom 10 --frames 0,40,80
//! ```
//!
//! Resize the terminal to see a scene recompose itself for the new shape.
//! Keys: ← → (or h l, Tab) switch scenes, space pauses, `.` steps a paused
//! frame, q or Esc quits.

// The scenes are included straight from the app's source, so the viewer
// shows exactly what the app draws.
#[allow(dead_code)]
#[path = "../src/ui/scene/mod.rs"]
mod scene;

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use scene::{FRAMES, Scene, SceneWidget};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--png") {
        let dir = args.get(1).map(String::as_str).unwrap_or("scenes");
        let size = args.get(2).and_then(|s| parse_size(s)).unwrap_or((180, 88));
        let option = |name: &str| {
            let at = args.iter().position(|a| a == name)?;
            args.get(at + 1).cloned()
        };
        let zoom = option("--zoom").and_then(|z| z.parse().ok());
        let frames = option("--frames")
            .map(|f| f.split(',').filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_else(|| vec![0, FRAMES / 3, FRAMES * 2 / 3]);
        return write_pngs(dir, size, zoom, &frames);
    }
    let start = args.first().map_or(0, |wanted| find(wanted));
    run(start)
}

/// A scene by its position (1-based) or a word of its name.
fn find(wanted: &str) -> usize {
    if let Ok(n) = wanted.parse::<usize>() {
        return n.saturating_sub(1).min(Scene::ALL.len() - 1);
    }
    let wanted = wanted.to_lowercase();
    Scene::ALL
        .iter()
        .position(|s| s.name().to_lowercase().contains(&wanted))
        .unwrap_or(0)
}

fn parse_size(text: &str) -> Option<(usize, usize)> {
    let (w, h) = text.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn run(start: usize) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut index = start;
    let mut paused: Option<u64> = None;

    let result = loop {
        let frame = paused.unwrap_or_else(scene::current_frame);
        let scene = Scene::ALL[index];
        let drawn = terminal.draw(|f| {
            let rows =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(f.area());
            f.render_widget(SceneWidget::new(scene, frame), rows[0]);
            let (scale, w, h) = scene::fit(rows[0].width, rows[0].height);
            let state = if paused.is_some() {
                format!("paused at frame {}", frame % FRAMES)
            } else {
                "playing".to_string()
            };
            let status = format!(
                " ‹ {} › {}/{}  ·  {w}×{h} px at {scale}×  ·  {state}  ·  ← → scene · space pause · . step · q quit",
                scene.name(),
                index + 1,
                Scene::ALL.len(),
            );
            f.render_widget(
                Paragraph::new(Line::from(status)).style(Style::new().fg(Color::Gray)),
                rows[1],
            );
        });
        if let Err(err) = drawn {
            break Err(err);
        }

        // 50ms is the scene's own frame time, so the animation runs at speed.
        match event::poll(Duration::from_millis(50)) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(err) => break Err(err),
        }
        let Ok(Event::Key(key)) = event::read() else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let n = Scene::ALL.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => break Ok(()),
            KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => index = (index + 1) % n,
            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => index = (index + n - 1) % n,
            KeyCode::Char(' ') => {
                paused = match paused {
                    Some(_) => None,
                    None => Some(scene::current_frame()),
                }
            }
            KeyCode::Char('.') => paused = Some(paused.unwrap_or_else(scene::current_frame) + 1),
            _ => {}
        }
    };

    ratatui::restore();
    result
}

/// Writes `frames` of every scene at `size` canvas pixels into `dir`, each
/// pixel a `zoom`-sized block (by default, as large as keeps the image about
/// 1200 wide).
fn write_pngs(
    dir: &str,
    (w, h): (usize, usize),
    zoom: Option<usize>,
    frames: &[u64],
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let zoom = zoom.unwrap_or((1200 / w.max(1)).max(1)).max(1);
    for scene in Scene::ALL {
        for &frame in frames {
            let canvas = scene.render(frame, w, h);
            let image = image::RgbImage::from_fn((w * zoom) as u32, (h * zoom) as u32, |x, y| {
                image::Rgb(canvas.get(x as usize / zoom, y as usize / zoom))
            });
            let name = scene.name().to_lowercase().replace(' ', "-");
            let path = format!("{dir}/{name}-{w}x{h}-{frame:03}.png");
            image.save(&path).map_err(std::io::Error::other)?;
            println!("{path}");
        }
    }
    Ok(())
}
