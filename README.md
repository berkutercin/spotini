<p>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/logo/lockup-dark.svg" />
    <img src="brand/logo/lockup-light.svg" alt="spotini" width="360" />
  </picture>
</p>

A tiny, keyboard-first player for Spotify, for Windows, styled like a terminal app.

**[Website and live demo](https://berkutercin.github.io/spotini/)** · **[Download](https://github.com/berkutercin/spotini/releases/latest)** · [Brand](brand/)

> **Unofficial.** spotini is a personal project, not affiliated with or endorsed by Spotify.
> It needs a **Spotify Premium** account.

Playback runs through [librespot](https://github.com/librespot-org/librespot) (320 kbps Ogg Vorbis), your library comes from the Spotify Web API, and synced lyrics come from [LRCLIB](https://lrclib.net). The interface is native, drawn with [Slint](https://slint.dev), with no browser engine inside: the whole app uses under 20 MB of memory while it plays.

## Download

Get the latest version from [Releases](https://github.com/berkutercin/spotini/releases/latest): [`spotini-setup.exe`](https://github.com/berkutercin/spotini/releases/latest/download/spotini-setup.exe) to install, or [`spotini-portable.exe`](https://github.com/berkutercin/spotini/releases/latest/download/spotini-portable.exe) if you'd rather not install anything. Both are built from the tagged source by GitHub Actions.

The files aren't code-signed, so Windows SmartScreen may warn "Windows protected your PC". Choose **More info → Run anyway**.

Then follow [Setup](#setup) to connect your Spotify account.

## Features

- Liked Songs and all your playlists, including followed ones and Spotify's mixes
- Make playlists; add songs to them or remove them, several at once (ctrl / shift + click), from a right-click menu. Songs already in a playlist or in Liked Songs aren't added twice
- Song radio: an endless station of songs like the one you pick
- Search tracks, albums and artists; album and artist pages
- Synced lyrics that follow the song (click a line to jump there)
- The play queue, with a jump to any upcoming track, and where it's playing from; put songs up next or take them out
- Shuffle, repeat (all / one), like and unlike
- Media keys and the Windows media overlay
- Closes to the tray and keeps playing
- A mini player in the taskbar (previous, play/pause, next, volume), turned on from the tray menu
- Shows up in your Spotify account as a device, so what you play shows wherever Spotify shares it (such as Discord)
- Themes: eleven palettes (Amber, Catppuccin, Tokyo Night, Nord, Gruvbox, Dracula, Rosé Pine, Solarized, One Dark, Everforest, Kanagawa) or your own colours, three pane styles (minimal, boxed, glass), custom accent, accent taken from the cover, text size

## Keyboard

| Key | Action |
| --- | --- |
| `space` | play / pause |
| `j` `k` / `↓` `↑` | move |
| `enter` | open / play |
| `tab` | switch pane |
| `/` | search |
| `esc` / `backspace` | back |
| `n` `p` | next / previous |
| `←` `→` | seek 5 s |
| `+` `-` | volume |
| `s` `r` | shuffle / repeat |
| `f` | like the playing track |
| `o` | go to what's playing |
| `R` | radio from the selected (or playing) song |
| `e` | play the selected tracks next |
| `shift` + `↓` `↑` | select several tracks (or ctrl / shift + click) |
| right-click | track menu (play next, radio, playlists, like) or queue menu |
| `del` | remove from the queue (queue panel) |
| `a` `A` | open the album / artist of the selected track |
| `l` `q` | lyrics / queue panel (again to hide) |
| `g` `G` | top / bottom |
| `,` | settings |
| `?` | all shortcuts |

## Setup

spotini reads your library through your **own** Spotify developer app, so no secrets live in this repository.

1. Go to [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard), log in and accept the developer terms.
2. **Create app** with:
   - **Redirect URI:** `http://127.0.0.1:8899/callback`
   - **APIs used:** Web API
3. Open the app's settings and copy its **Client ID**.
4. Start spotini, log in with Spotify, paste the Client ID, and connect your library.

Login details are stored in Windows Credential Manager.

## Building from source

Requirements: Windows 10 or 11, [Rust](https://rustup.rs) and the Visual Studio C++ Build Tools. The app is in `app/`:

```bash
cd app
cargo run
```

The first build compiles every dependency with optimizations and takes a while. A release build goes to `app/target/release`:

```bash
cargo build --release
```

The website lives in `site/` and deploys to GitHub Pages from `main`. Its live demo is the app's own screens built to WebAssembly from `demo/`, with a made-up library and no sound. Building it needs the `wasm32-unknown-unknown` Rust target, [wasm-bindgen-cli](https://crates.io/crates/wasm-bindgen-cli) 0.2.129 and [Node.js](https://nodejs.org) 20+:

```bash
npm install
npm run demo
npm run site:dev
```

## Limitations

Spotify restricted apps in development mode in February 2026. Because of that:

- The Web API only lists the tracks of playlists you own, so followed playlists and song radio use Spotify's internal endpoints through librespot instead. Like playback itself, they're unofficial and may break if Spotify changes them.
- Search returns 10 results per type at a time.
- Artist pages list albums and singles; Spotify no longer offers artists' top tracks to these apps.
- A developer app can have at most 5 users, which is why everyone uses their own Client ID.

Also:

- There is no lossless audio. librespot tops out at 320 kbps.
- Spotify occasionally changes its backend in ways that break librespot until librespot ships a fix.
- LRCLIB is community-made, so some songs have no lyrics or slightly off timings.

## License

[MIT](LICENSE)

## Credits

[librespot](https://github.com/librespot-org/librespot), [LRCLIB](https://lrclib.net), [souvlaki](https://github.com/Sinono3/souvlaki), [Slint](https://slint.dev) and [JetBrains Mono](https://www.jetbrains.com/lp/mono/) (SIL Open Font License). Layout inspired by [spotify-player](https://github.com/aome510/spotify-player).
