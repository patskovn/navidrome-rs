# AGENTS.md

Guidance for AI agents working in this repository.

## What this is

A terminal client for [Navidrome](https://www.navidrome.org/) and other
Subsonic/OpenSubsonic servers. Rust, edition 2024, ~3.5k LoC.

It is the second consumer of [`tca-rs`](https://crates.io/crates/tca-rs), a
Redux-style architecture crate maintained in a sibling repo
(`../tca-rs`). The first is `../tgpt`, from which this project's widget kit was
copied wholesale. When something here looks like tgpt, that is why — and when
tgpt does something differently, this project is usually the corrected version.

## Commands

```bash
cargo build              # or --release
cargo test               # 40 tests, all fast and offline
cargo clippy --all-targets
cargo run --release      # needs a real server to be useful
cargo fmt
```

Both `cargo clippy --all-targets` and `cargo test` are expected to be **clean**.
Do not leave new warnings behind.

## Layout

```
src/
├── main.rs          terminal setup, event loop, dependency wiring
├── config.rs        server config, including the password, on disk
├── directory.rs     radio-browser.info search: mirror discovery, failover
├── probe.rs         checks a radio URL is audio before the add form plays it
├── subsonic/        the server boundary
│   ├── mod.rs       `Library` trait + app-owned domain types + test fake
│   └── client.rs    `opensubsonic`-backed implementation
├── audio/
│   ├── engine.rs    playback thread, command in / event out
│   ├── cache.rs     2 GiB LRU disk cache; also a stream-download backend
│   └── tap.rs       pass-through Source copying samples for the visualiser
├── app/             tca features
│   ├── entry.rs     root: session gate, tabs, global keys, visualiser tick
│   ├── auth.rs      sign-in form
│   ├── albums.rs    Albums tab; also the album detail the Artists tab reuses
│   ├── artists.rs   Artists tab: local filter, artist → albums → tracks
│   ├── songs.rs     All Music tab
│   ├── player.rs    queue and transport
│   ├── radio_search.rs  station search, scoped inside the Radios tab
│   ├── radios.rs    Radios tab: the server's stations, add (with preview), delete
│   └── ui.rs        drawing for the root feature
└── ui/              widgets ported from tgpt (see note below)
```

## Invariants — break these and things get subtly wrong

These are the mistakes most likely to be made here. Several were made and fixed
during the initial build.

### 1. Nothing audio-related goes in reducer `State`

`State` must stay `Send + Sync` and cheaply comparable. Two consequences:

- The rodio `Player` and the `cpal` device live on a **dedicated OS thread**
  (`audio/engine.rs`). `cpal`'s stream handle is not `Send` on every platform.
  Reducers reach it only by sending `audio::Command`; it reports back as
  `audio::Event`, which `main.rs` turns into actions.
- The visualiser's sample ring (`audio/tap.rs`) is **not** in state either. It is
  written thousands of times a second, and interior mutability in `State` makes
  `tca::TestStore`'s expected-vs-actual comparison meaningless — both sides would
  alias the same cell. The UI reads the tap directly at draw time.

If you find yourself wanting to put a handle, a lock, or a cache in `State`,
that is the signal to pass it through the `Feature` instead. Reducers are values
and can hold dependencies.

### 2. `Scope` state projections must be `fn` pointers, not closures

```rust
Scope::new(|s: &mut State| &mut s.player, case!(Action::Player), player::Feature { .. })
```

That closure works because it captures nothing and coerces to a `fn` pointer. A
capturing closure will not: it infers a signature bound to one concrete
lifetime rather than the higher-ranked `for<'a>` the bound requires, and the
error surfaces as an opaque *"the method `try_reduce` exists but its trait
bounds were not satisfied"* at the call site, nowhere near the cause.

### 3. Composition routes, it does not broadcast

`.then()` is first-match-wins: each action goes to the first reducer that claims
it. Rust ownership makes this the only option — an owned action cannot be handed
to two reducers. The tail of every chain is `.or_else(...)`, and child action
variants there should be `unreachable!("claimed by a scope")`.

### 4. The store notifies once per *action*, not per state change

`tca` dropped clone-based change detection, so a notification means "an action
was processed", not "state definitely changed". Two rules follow:

- **Filter inert input at the event loop.** `main.rs::is_actionable` drops mouse
  and focus events; without it, mouse motion under `EnableMouseCapture` would
  cost a full redraw per pixel. That match is exhaustive on purpose — adding
  mouse support should fail to compile there rather than silently swallow events.
- **The visualiser tick is the frame rate.** `entry.rs` starts a 50ms tick as a
  `cancellable` effect when playback begins and cancels it when playback stops.
  Never leave it running while idle.

### 5. Writing tests with `TestStore`

`store.send(action, expect)` applies **one** action and *queues* whatever the
reducer emitted. It does not run the chain. So:

- Use `store.settle()` when the test is about the state a key press settles
  into. It applies queued actions until the queue drains, like the real engine.
- Use `receive` / `receive_next` when the intermediate steps are the point.
- `finish()` fails if any emitted action went unasserted. That is the feature,
  not an annoyance.
- Effects touching `tokio::time` need `#[tokio::test]`: `TestStore` polls async
  effects once to capture what they emit, and that poll needs a reactor.

Reducers are testable because the server sits behind the `Library` trait; use
`subsonic::fake::FakeLibrary`. Never construct `OpenSubsonicLibrary` in a test.

## Dependency pins that are not arbitrary

- **`tui-textarea` is `ratatui-textarea` held at 0.8.** The original
  `tui-textarea` is unmaintained (last release 2024-10) and pins ratatui 0.29.
  Do **not** bump the fork to 0.9+: it added an internal screen-map cache built
  on `RefCell`/`Cell`, which makes `TextArea` `!Sync`, and state must be `Sync`.
  The same cache is in the `tui-textarea-2` fork.
- **`crossterm` must match what ratatui re-exports** (0.29 for ratatui 0.30).
  A mismatch gives two distinct `Event` types and nothing type-checks across the
  boundary.

## Things that surprised us

- **The first frame must be drawn explicitly.** The event loop only paints on a
  store notification, and a signed-out app sends no actions until a key is
  pressed — leaving a blank terminal on launch. `main.rs` draws once before the
  loop.
- **`ui/textfield.rs` is a vim-modal editor** that starts in Normal mode, where
  letters are commands. A search box needs to accept typing immediately, so
  `search.rs` replays an `i` keypress when focusing the field, reusing the
  editor's own transition rather than duplicating its block/cursor handling.
- **Cache index persistence.** `TrackCache::insert` must persist the index
  itself; relying on `enforce_budget` to do it silently loses entries whenever
  the cache is under budget, which is the common case.
- **The info panel sizes itself facts-first.** `albums::draw_info` gives the
  metadata its rows and lets the cover take what is left, not the reverse. A
  square-ish cover fills a short terminal outright, which used to push the album
  name off screen — and since the tab border stopped repeating that name, the
  panel is the only place it appears.
- **Cover art is colour, on purpose.** `ui/cover.rs` draws artwork as half
  blocks (`▀`, fg = upper pixel, bg = lower), so a cell carries two real
  pixels. This is a deliberate exception to the monochrome rule that governs
  the visualiser. ASCII and braille ramps were rejected: both encode intensity
  in the character and so discard colour. Note the aspect correction — a cell
  is ~1:2 and half-blocks halve it again, so a square image wants
  `cols == rows * 2`, and the widget letterboxes rather than stretching.
- **Audio devices are identified by id, not name.** `DeviceTrait::name` is
  deprecated; `id()` is stable across reboots and reconnections and `
  description()` is for display only. Matching on the human-readable name
  breaks as soon as two devices share one.
- **A rebuilt sink resets the player.** `MixerDeviceSink` and `Player` are
  rebuilt together — a player is bound to one mixer — and volume lives on the
  `Player`, so it has to be reapplied or the track resumes at full volume.
  Resuming also means re-opening the track and seeking, since the new player
  starts empty.
- **The Queue tab is not a scoped child feature, deliberately.** Its only state
  is a cursor (`Library::queue_cursor`), but every key needs the queue's *length*
  to clamp against, and the queue lives in `player::State`. A child reducer would
  see only the cursor, so it would have to be handed a copy of the length to keep
  in sync — two sources of truth for one number. Its keys are handled in
  `entry::reduce_queue_key`, where both are visible.
- **`player::State::current` is an index, so every queue edit must maintain
  it.** Dropping an entry before the playhead shifts the playhead down with it;
  dropping the playhead's own entry leaves the index pointing at whatever
  followed, which is the track to start next. `State::remove` returns a
  `Removal` saying which of those happened, so the reducer — not the drawing
  code — decides what playback does. Get this wrong and removing a row silently
  changes the song.
- **Window focus is a global, on purpose.** `Theme::accent()` reads a process
  `AtomicBool` rather than a parameter. `entry::State::focused` is still the
  source of truth — `ui::draw` copies it into the theme once per frame — but the
  accent is used from a dozen draw sites and threading a flag to all of them
  would buy nothing. It defaults to *focused*, so a terminal that never reports
  (Terminal.app, tmux without `focus-events on`) looks normal rather than
  permanently grey. Only one test may touch that flag; a second would race it
  under the parallel runner. Note `ACCENT_UNFOCUSED` is `Gray` rather than the
  darker `DIM`: the accent also backs the selected row through `selection()`,
  which reverses it into a background.
- **Download progress is bytes, drawn against time.** `DownloadProgress`
  (`audio/cache.rs`) is a byte high-water mark over the content length, and the
  scrubber plots it on an axis that is really elapsed/duration. Those agree only
  at a constant bitrate; it is an indicator, not a seek target. It is a
  high-water mark rather than a running sum because the writer can be seeked,
  and it is clamped to never render behind the playhead — audio cannot come out
  of bytes we do not have, so a backwards reading is a stale sample.
- **Buffer events are rate-limited, not just throttled by the tick.** The engine
  only emits `Buffered` when the figure moves by `BUFFER_REPORT_STEP`. Every
  event wakes the store and costs a repaint, so a stalled download would
  otherwise redraw five times a second to say nothing.
- **Partial downloads.** A track streams into `<id>.<ext>.part` and is promoted
  to its final name only once the full content length has landed, so an
  interrupted download is never mistaken for a complete file. Stale `.part`
  files are swept on startup.

## Conventions

- Domain types (`subsonic::Album`, `subsonic::Track`) are **ours**, not
  `opensubsonic`'s. The app should not change shape when the API crate does.
  Map at the boundary in `subsonic/client.rs`.
- `ui/` is ported code and carries `#![allow(dead_code)]` — parts of the widget
  API are unused here. Do not trim it; the cost of re-porting later exceeds the
  benefit. It also arrived with colours of its own: `ui/list.rs` had a
  hard-coded blue highlight that ignored `Theme` entirely, so the album and
  track lists stayed bright when everything else greyed out. Anything that reads
  as *the accent* must come from `Theme::accent()`/`selection()`, or it will
  quietly opt out of focus dimming.
- Comments should explain *why*, especially where a constraint is non-obvious.
  Most of the invariants above exist as comments at their site too.
- Errors surface to the user through state (`error: Option<String>`) and get
  rendered; they should not panic. `Scope::optional` logs and drops actions
  arriving for absent child state, which is normal when an in-flight effect
  races a state transition.

- **List rows must reserve every column they spend.** A `List` with a
  `highlight_symbol` takes those columns from the item text, so a row's width
  budget is the inner width *minus* the symbol, the leading number, the gap and
  the trailing field. `draw_queue`'s `FURNITURE` spells that sum out; getting it
  wrong truncates the right-hand column rather than the title, which reads as a
  formatting bug ("11:33" becoming "11:3").

- **Radio is a queue of one live track.** A station plays as a `Track` whose
  `live_url` is set, so the player, the bar and Now Playing need no second
  code path. The engine branches on `live_url`: a fixed 2 MiB memory ring
  instead of the disk cache (a station never ends, so a cache file would grow
  for as long as it played), a decoder built non-seekable (or it hunts for an
  end that does not exist), and a 48 KiB prefetch instead of the library's
  256 KiB — sixteen seconds of silence on a 128k station.
- **Live streams need a User-Agent.** `stream-download`'s default client sends
  none, and some stations (Hunter.FM) answer 403 to it — while the probe,
  which sends one, said the same URL was fine. Both now send
  `probe::USER_AGENT`. `cargo test tune_in -- --ignored` with
  `NAVIDROME_RS_LIVE_URL` set decodes a few seconds of a real station; it is
  ignored by default because it reaches someone else's server.
- **Searching radio-browser.info asks twice.** Its filters AND together, so a
  query is sent once as a name and once as a tag, in parallel, and merged by
  station id, most listened first. A query with commas is instead a list of
  tags that must all match (`tagList`; `lofi, chill`). Commas, not spaces,
  because tags and names contain spaces. Mirrors come from
  `all.api.radio-browser.info/json/servers` on first use (not at startup),
  with a short fallback list; requests fail over between them. HLS stations
  are dropped from results, since they cannot be played. Picking a result
  reports a click, as the directory asks, and opens the add form already
  previewing — the search stays open underneath, so Esc returns to the
  results. `cargo test real_directory -- --ignored` queries the real
  directory.
- **The station form owns the keyboard, `?` included.** Stream URLs carry
  query strings, so the help key is text there. A preview is stopped only if
  it is still what is playing (`radios::PREVIEW_ID`): the user may have
  started something else meanwhile.

## Not done yet

No playlists, no shuffle/repeat. Radio stations cannot be edited, only
added and deleted, and live ICY song titles are not shown. HE-AAC stations
play at their base rate only (symphonia has no SBR), so they sound dull. The queue can be viewed
and pruned but not reordered. Seek is implemented in the
engine (`audio::Command::Seek`) but not bound to a key. "All Music" loads a page
of albums and flattens them rather than paging by track, because Subsonic has no
"all songs" endpoint.
