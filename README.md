# navidrome-rs

A terminal client for [Navidrome](https://www.navidrome.org/) (and any
Subsonic/OpenSubsonic server), built on [`tca-rs`](https://crates.io/crates/tca-rs)
and [ratatui](https://ratatui.rs).

Sibling project to `tgpt`, and the second consumer of `tca-rs` — the widget kit
is ported from it.

## What works

- **Sign in** with server URL, username and password. Credentials are verified
  before being saved. Credentials live in `config.json`, written `0600`.
  Set `NAVIDROME_PASSWORD` to override the stored one and keep it off disk.
- **Tabs**: Albums and All Music, `Tab` to switch. They ride the content
  frame's own border rather than taking a row of their own.
- **Live search in both tabs.** The field is focused on arrival and takes every
  key except the three reserved ones; results update as you type, debounced at
  250ms. An empty query falls back to the full listing.
- **Album view**: Enter opens an album — tracks on the left, cover art and
  metadata on the right. The panel is where the album's name lives; the border
  does not repeat it. Artwork is drawn as colour half-blocks, so it needs no
  terminal graphics protocol.
- **Playback** with play/pause, next/previous, and volume. Unplugging an output
  moves playback to another device and resumes where it left off; `Ctrl-O`
  picks one. The sounding track
  is marked in whichever list it appears in.
- **Progressive streaming** — a track starts playing before it has finished
  downloading.
- **2 GiB LRU disk cache** of original files, populated as a side effect of
  streaming rather than by a second download.
- **Focus-aware accent.** When the terminal window loses focus the accent drops
  from magenta to grey, so a background session recedes. Needs a terminal that
  reports focus (most do; Terminal.app does not, and tmux wants
  `focus-events on`) — where it is unsupported nothing changes.
- **Monochrome spectrum visualiser** driven by a real FFT over the decoded
  audio, with peak-hold caps. It sets no colour at all, so it inherits the
  terminal's foreground and reads correctly on light and dark themes.

## Keys

| Key | Action |
|---|---|
| `Tab` | switch tab (works from anywhere) |
| `↓` | leave the search field for the list |
| `↑` | back to the search field, from the top of the list |
| `j` / `k` / `↑` / `↓` | move in a list |
| `Enter` | open an album, or play a track |
| `Esc` / `←` | back out of an open album |
| `space` | play / pause |
| `n` / `p` | next / previous track |
| `+` / `-` | volume |
| `?` | show all keys |
| `Ctrl-O` | output device picker |
| `q` / `Ctrl-C` | quit |

Single-key shortcuts (`space`, `n`, `p`, `q`, `+`, `-`) are commands only when
focus is in the list. `Tab`, `Ctrl-O` and `Ctrl-C` always work. In the search field they are text — `q` is a query, not
quit. `Tab` and `Ctrl-C` always work.

## Design notes

**Audio lives outside reducer state.** `cpal`'s stream handle is not `Send` on
every platform, so the device owns a dedicated OS thread; and `tca` state must
stay `Send + Sync` and cheaply comparable, so the player is reached only through
commands. Playback reports back as events, which re-enter the store as actions.

**The sample buffer is not in state either.** It is written thousands of times a
second from the audio thread. Interior mutability in `State` would break
`TestStore`'s expected-vs-actual comparison — the same trap `tgpt`'s
`SharedFocus` fell into. The UI reads the tap directly at draw time.

**Streaming and caching are one pass.** `stream-download` is pointed at a custom
`StorageProvider` that writes into the cache directory. A partial download lives
at `<id>.<ext>.part` and is promoted to its final name only once the full
content length has landed, so an interrupted download can never be mistaken for
a complete file.

**The server is behind a trait.** Reducers depend on `Library`, not on
`opensubsonic`, so features can be driven by `tca::TestStore` against a fake.
That is the fix for `tgpt`, where the API client was built inside the reducer
and nothing could be tested.

**The visualiser tick is cancellable.** `tca` notifies observers once per
processed action, so the tick *is* the frame rate. It is cancelled the moment
playback stops, and mouse/focus events are filtered at the event loop, so an
idle app does no work.

## Running

```bash
cargo run --release
```

Config lives at `~/.config/navidrome-rs/config.json` (URL, username, password),
cached audio under the platform cache directory, and a debug log at
`<state-dir>/navidrome-rs/latest.log`.

## Status

Not yet exercised end-to-end against a live server — sign-in, streaming and the
visualiser have not been run with real credentials. Everything below that line
(cache eviction, FFT bucketing, reducer logic, API mapping) is covered by tests.

Known gaps: no playlists, no artist browsing, no shuffle or repeat, no seek
binding (the command exists), and "All Music" loads a page of albums rather than
paging by track.
