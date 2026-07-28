# navidrome-rs

A terminal client for [Navidrome](https://www.navidrome.org/) (and any
Subsonic/OpenSubsonic server), built on [`tca-rs`](https://crates.io/crates/tca-rs)
and [ratatui](https://ratatui.rs).

Sibling project to `tgpt`, and the second consumer of `tca-rs` — the widget kit
is ported from it.

## What works

- **Sign in** with server URL, username and password. Credentials are verified
  before being saved; the password goes to the OS keychain, never to disk.
- **Tabs**: Albums, All Music, Search. `1`/`2`/`3` or `Tab` to switch.
- **Search** over tracks and albums via `search3`.
- **Playback** with play/pause, next/previous, and volume.
- **Progressive streaming** — a track starts playing before it has finished
  downloading.
- **2 GiB LRU disk cache** of original files, populated as a side effect of
  streaming rather than by a second download.
- **Monochrome spectrum visualiser** driven by a real FFT over the decoded
  audio, with peak-hold caps. It sets no colour at all, so it inherits the
  terminal's foreground and reads correctly on light and dark themes.

## Keys

| Key | Action |
|---|---|
| `1` `2` `3` / `Tab` | switch tab |
| `j` / `k` | move in a list |
| `Enter` | play album / track |
| `space` | play / pause |
| `n` / `p` | next / previous track |
| `+` / `-` | volume |
| `/` | focus the search box (Search tab) |
| `r` | reload the current tab |
| `q` / `Ctrl-C` | quit |

`q` is only "quit" when you are not typing a search.

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

Config lives at `~/.config/navidrome-rs/config.json` (URL and username only),
cached audio under the platform cache directory, and a debug log at
`<state-dir>/navidrome-rs/latest.log`.

## Status

Not yet exercised end-to-end against a live server — sign-in, streaming and the
visualiser have not been run with real credentials. Everything below that line
(cache eviction, FFT bucketing, reducer logic, API mapping) is covered by tests.

Known gaps: no playlists, no artist browsing, no shuffle or repeat, no seek
binding (the command exists), and "All Music" loads a page of albums rather than
paging by track.
