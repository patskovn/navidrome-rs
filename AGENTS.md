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
│   ├── albums.rs    Albums tab
│   ├── songs.rs     All Music tab
│   ├── search.rs    Search tab
│   ├── player.rs    queue and transport
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
- **Cover art is colour, on purpose.** `ui/cover.rs` draws artwork as half
  blocks (`▀`, fg = upper pixel, bg = lower), so a cell carries two real
  pixels. This is a deliberate exception to the monochrome rule that governs
  the visualiser. ASCII and braille ramps were rejected: both encode intensity
  in the character and so discard colour. Note the aspect correction — a cell
  is ~1:2 and half-blocks halve it again, so a square image wants
  `cols == rows * 2`, and the widget letterboxes rather than stretching.
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
  benefit.
- Comments should explain *why*, especially where a constraint is non-obvious.
  Most of the invariants above exist as comments at their site too.
- Errors surface to the user through state (`error: Option<String>`) and get
  rendered; they should not panic. `Scope::optional` logs and drops actions
  arriving for absent child state, which is normal when an in-flight effect
  races a state transition.

## Not done yet

No playlists, no artist browsing, no shuffle/repeat. Seek is implemented in the
engine (`audio::Command::Seek`) but not bound to a key. "All Music" loads a page
of albums and flattens them rather than paging by track, because Subsonic has no
"all songs" endpoint.
