<div align="center">
  <img src="assets/icon.png" width="160" alt="Lyrixearch icon">
  <h1 align="center">LYRIXEARCH</h1>
  <p align="center"><strong>Find lyrics for your music, review them, and embed them in the files.</strong></p>
  <p align="center">One Rust engine behind a CLI, a terminal UI and a desktop app. Drop a folder; it scans every subfolder, searches many sources at once, matches the results to your tracks and writes the lyrics into the tags.</p>
  <p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1e1e1e?style=for-the-badge&logo=rust&logoColor=5fe27f" alt="Built with Rust"></a>
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/Tauri%202-1e1e1e?style=for-the-badge&logo=tauri&logoColor=5fe27f" alt="Tauri 2 desktop app"></a>
  <a href="https://ratatui.rs/"><img src="https://img.shields.io/badge/ratatui%20TUI-1e1e1e?style=for-the-badge&logo=gnometerminal&logoColor=5fe27f" alt="Terminal UI built with ratatui"></a>
  </p>
  <p align="center">
  <a href="#platform-status"><img src="https://img.shields.io/badge/macOS-1e1e1e?style=for-the-badge&logo=apple&logoColor=c9ccd1" alt="macOS"></a>
  <a href="#platform-status"><img src="https://img.shields.io/badge/Windows-1e1e1e?style=for-the-badge&logo=windows&logoColor=c9ccd1" alt="Windows"></a>
  <a href="#platform-status"><img src="https://img.shields.io/badge/Linux-1e1e1e?style=for-the-badge&logo=linux&logoColor=c9ccd1" alt="Linux"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/MIT-1e1e1e?style=for-the-badge&logo=opensourceinitiative&logoColor=c9ccd1" alt="MIT license"></a>
  </p>
  <p align="center"><a href="#quick-start">Quick Start</a> | <a href="#the-app">App</a> | <a href="#the-terminal-ui">TUI</a> | <a href="#sources">Sources</a> | <a href="#plugins">Plugins</a> | <a href="#build-from-source">Build</a></p>
</div>

<details>
<summary><strong>Contents</strong></summary>

- [Quick Start](#quick-start) · [Install](#install) · [The App](#the-app) · [The Terminal UI](#the-terminal-ui)
- [Sources](#sources) · [Plugins](#plugins) · [How lyrics are written](#how-lyrics-are-written)
- [Project Structure](#project-structure) · [Build from source](#build-from-source) · [Contributing and legal](#contributing-and-legal) · [License](#license)

</details>

---

## <img src="https://api.iconify.design/mdi:rocket-launch-outline.svg?color=%235fe27f" height="22" alt=""> Quick Start

```bash
# 1. get it: build from source (or grab a build from Releases)
cargo build --release                    # target/release/lyrixearch  +  lyrixearch-gui

# 2. see what is missing, then fill it in
lyrixearch scan ~/Music -r               # what is there, what has no lyrics
lyrixearch fetch ~/Music -r --dry-run    # search, change nothing
lyrixearch fetch ~/Music -r              # search and embed where missing

# 3. or just open the interface
lyrixearch                               # terminal UI (no arguments)
lyrixearch-gui                           # desktop app
```

> [!TIP]
> Run `lyrixearch` with no arguments for the **terminal UI**. Searching never writes by itself there: review the results, then press `w`.

## <img src="https://api.iconify.design/mdi:download.svg?color=%23c9ccd1" height="22" alt=""> Install

Download a build from [Releases](../../releases) (macOS, Windows, Linux), or build it yourself below.

> [!NOTE]
> Release builds are **unsigned**. On macOS right-click the app and choose Open the first time, or run `xattr -dr com.apple.quarantine Lyrixearch.app`. On Windows choose "More info, Run anyway" in SmartScreen.

To run the CLI from anywhere, link it onto your `PATH`:

```bash
ln -s "$PWD/target/release/lyrixearch" ~/.local/bin/lyrixearch
```

## <img src="https://api.iconify.design/mdi:application-outline.svg?color=%235fe27f" height="22" alt=""> The App

| | Feature |
| --- | --- |
| <img src="https://api.iconify.design/mdi:folder-multiple-plus-outline.svg?color=%235fe27f" height="16" alt=""> | **Add music** by dropping folders or files on the window (or the app icon on macOS), **Files** / **Folder**, or the command line. Every subfolder is scanned. **Library** remembers folders and re-scans them in one go. |
| <img src="https://api.iconify.design/mdi:playlist-music-outline.svg?color=%23c9ccd1" height="16" alt=""> | **Track list** with cover art, duration and a colored lamp per state. Tabs filter by **All, Missing, Embedded, Found, NA, Error**; select-all, **Search**, **Embed**, **Files**, **More** and **Drop embedded** act on the selection. |
| <img src="https://api.iconify.design/mdi:lightning-bolt-outline.svg?color=%235fe27f" height="16" alt=""> | **Find &amp; embed all**: the green top-bar button searches every track currently listed and embeds what it finds. A **Replace existing** switch decides whether tracks that already have lyrics are searched too. |
| <img src="https://api.iconify.design/mdi:file-music-outline.svg?color=%23c9ccd1" height="16" alt=""> | **Files menu**: format **Auto / .lrc / .txt**, then **Embed from file** (the file named like the track) or **Write to file**. |
| <img src="https://api.iconify.design/mdi:play-circle-outline.svg?color=%235fe27f" height="16" alt=""> | **Details**: compact player, then **Lyrics**, **Edit**, **Web** and **Info**. Copy, paste from clipboard, edit, reveal, remove. Buttons can show icons, text or both. |
| <img src="https://api.iconify.design/mdi:web.svg?color=%23c9ccd1" height="16" alt=""> | **Web search** inside the window (Google, Bing, DuckDuckGo, Yahoo, Yandex, Brave AI) in phone layout so pages fit; Google AI Mode opens in your own browser. |

<details>
<summary><strong>State colors</strong></summary>

| Lamp | Meaning |
| --- | --- |
| <img src="https://api.iconify.design/mdi:circle.svg?color=%235fe27f" height="12" alt=""> green | lyrics embedded |
| <img src="https://api.iconify.design/mdi:circle.svg?color=%23e0b040" height="12" alt=""> amber | missing lyrics |
| <img src="https://api.iconify.design/mdi:circle.svg?color=%2356b4f0" height="12" alt=""> blue | found, not embedded yet |
| <img src="https://api.iconify.design/mdi:circle.svg?color=%23b88af0" height="12" alt=""> purple | **NA**: no source had them |
| <img src="https://api.iconify.design/mdi:circle.svg?color=%23f0675a" height="12" alt=""> red | error |

</details>

## <img src="https://api.iconify.design/mdi:console.svg?color=%23c9ccd1" height="22" alt=""> The Terminal UI

Run `lyrixearch` with no arguments.

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| `a` | add folder | `w` | write found lyrics |
| `space` | mark | `e` | edit in `$EDITOR` |
| `s` / `S` | search marked or current / all shown | `d` | delete embedded lyrics |
| `l` | embed the `.lrc`/`.txt` beside the track | `o` | show in the file manager |
| `b` | web search in browser | `f` | filter (all / without / with lyrics) |
| `/` | find | `t` / `x` | tags on-off / sidecar mode |
| `?` | help | `q` | quit |

The header counts files, with lyrics, left to search and NA; while searching, the footer shows a progress bar with the current track, source, found and NA counts.

<details>
<summary><strong>All CLI commands</strong></summary>

| Command | What it does |
| --- | --- |
| `scan <paths> [-r]` | list tracks and which lack lyrics |
| `fetch <paths> [-r] [--dry-run] [--overwrite] [--sidecar auto\|lrc\|txt]` | search and embed |
| `embed-files <paths…>` | embed the `.lrc`/`.txt` beside each track |
| `show <file>` / `remove <paths…>` | print or delete embedded lyrics |
| `providers [--test]` | list sources, optionally health-check them |
| `plugins list\|install\|test\|path` | manage plugins |
| `config show\|set\|path\|reset` | settings (TOML) |

</details>

## <img src="https://api.iconify.design/mdi:database-search-outline.svg?color=%235fe27f" height="22" alt=""> Sources

| Source | Notes |
| --- | --- |
| `local_files` | `.lrc` / `.txt` next to the track or in a `Lyrics/` folder |
| `genius` | plain lyrics from the public song page |
| `lrclib` | open database, **synced** lyrics |
| `lyricsfreak` | plain lyrics |
| `darklyrics` | metal lyrics, album pages (plain http only) |
| `ddg_search` | web-search fallback: finds a page one of the scrapers understands |
| `netease`, `qqmusic`, `kugou` | unofficial public endpoints, **synced** lyrics |
| `utaten` | Japanese lyrics, only asked for Japanese titles |
| `bandcamp` | lyrics the artist published on the track page |
| `lyrics_ovh` | exact-name API |

Per track, sources are tried in order with a per-source timeout; sources that keep failing are skipped for a while. Ten tracks are searched at once by default. Sources that show a captcha or browser check (DuckDuckGo, and any page behind Cloudflare) open a verification window in the desktop app; the cookies from it are reused for the rest of the search. Not yet included: AZLyrics, Letras and Uta-Net (all three sit behind bot checks), and Musixmatch (needs a private signed token).

## <img src="https://api.iconify.design/mdi:puzzle-outline.svg?color=%23c9ccd1" height="22" alt=""> Plugins

A plugin is any script that speaks a small JSON protocol over stdin/stdout. See [docs/PLUGINS.md](docs/PLUGINS.md). `plugins/metallum` is an example; it needs the separate `metallum` command-line tool.

## <img src="https://api.iconify.design/mdi:tag-text-outline.svg?color=%235fe27f" height="22" alt=""> How lyrics are written

ID3v2 `USLT`, MP4 `©lyr`, Vorbis/FLAC `LYRICS`, ASF `WM/Lyrics` and APE, through [`lofty`](https://crates.io/crates/lofty). File modification times are preserved by default. Options: sidecar `.lrc`/`.txt`, strip `[Chorus]`-style headers, leading blank line. Synced text goes to a sidecar; the tag holds the plain text. Settings live in a TOML file (`lyrixearch config path`) and are all editable in the app.

## <img src="https://api.iconify.design/mdi:file-tree.svg?color=%23c9ccd1" height="22" alt=""> Project Structure

```text
lyrixearch/
├── crates/
│   ├── core/            # lyrixearch-core: sources, matching, tag writing, engine
│   │   └── src/providers/   # one file per source, plus plugin and web helpers
│   ├── cli/             # lyrixearch: CLI and ratatui terminal UI
│   └── gui/             # lyrixearch-gui: Tauri 2 app
│       ├── src/         #   Rust backend (commands, player, embedded browser)
│       └── ui/          #   HTML/CSS/JS front end, fonts, no build step
├── plugins/metallum/    # example plugin
├── docs/PLUGINS.md      # plugin protocol
├── DESIGN.md            # design system and rationale
└── .github/workflows/   # CI and release builds
```

## <img src="https://api.iconify.design/mdi:hammer-wrench.svg?color=%235fe27f" height="22" alt=""> Build from source

You need a recent stable Rust toolchain.

```bash
cargo build --release
cargo test --workspace
cargo run -p lyrixearch-core --example probe -- "Queen" "Bohemian Rhapsody"   # live check of every source
```

Package the desktop app (`.app`/`.dmg`, `.msi`/`.exe`, `.deb`/`.AppImage`):

```bash
cargo install tauri-cli --version "^2" --locked
cd crates/gui && cargo tauri build
```

<details>
<summary><strong>Linux and Windows prerequisites</strong></summary>

**Linux** needs the Tauri prerequisites and ALSA headers for the built-in player:

```bash
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libgtk-3-dev \
  librsvg2-dev libxdo-dev libssl-dev libasound2-dev patchelf
```

**Windows** needs the Microsoft C++ build tools; the app uses the system WebView2 runtime.

</details>

### Platform status

CI builds, lints and tests on macOS, Windows and Linux. **Hands-on testing has so far been done on macOS only.** The in-window web panel uses Tauri's unstable multi-webview support, the part most likely to behave differently on Linux and Windows; the "open in browser" button always works. Bug reports with the OS and version are welcome.

## <img src="https://api.iconify.design/mdi:scale-balance.svg?color=%23c9ccd1" height="22" alt=""> Contributing and legal

Issues and pull requests are welcome. Run `cargo fmt`, `cargo clippy --workspace --all-targets` and `cargo test --workspace` first; CI runs the same checks.

> [!WARNING]
> Lyrics are copyrighted by their owners. Lyrixearch fetches and embeds them for personal use; respect each source's terms and do not redistribute what you download. Sources can change or block access at any time.

Not affiliated with any other lyrics tool or service.

## <img src="https://api.iconify.design/mdi:license.svg?color=%235fe27f" height="22" alt=""> License

<div align="center">

[**MIT LICENSE**](LICENSE)

**COPYRIGHT © 2026 LYRIXEARCH CONTRIBUTORS**

</div>
