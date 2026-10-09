<p align="center">
  <img src="ui/icon.png" width="72" height="72" alt="Zima icon">
</p>

<h1 align="center">Zima</h1>

<p align="center">A quiet, fast Markdown notes app for Windows.</p>

<p align="center">
  <a href="https://github.com/thefoultarnished/zima/actions/workflows/ci.yml"><img src="https://github.com/thefoultarnished/zima/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI status"></a>
  <a href="https://github.com/thefoultarnished/zima/actions/workflows/ci.yml"><img src="https://img.shields.io/endpoint?url=https%3A%2F%2Fraw.githubusercontent.com%2Fthefoultarnished%2Fzima%2Fbadges%2Ftests.json" alt="Tests passed"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-2024%20edition-B7410E?logo=rust" alt="Rust 2024 edition"></a>
  <a href="https://slint.dev"><img src="https://img.shields.io/badge/UI-Slint%201.18-2379F4" alt="UI: Slint 1.18"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-6C757D" alt="License: MIT"></a>
  <a href="https://github.com/thefoultarnished/zima/commits/main"><img src="https://img.shields.io/github/last-commit/thefoultarnished/zima" alt="Last commit"></a>
</p>

---

Zima is a place to write things down without the app getting in the way. Notes are plain Markdown files on your own disk, the window opens in a blink, and everything you need is one keystroke away. It's written in Rust with [Slint](https://slint.dev), so it stays light on memory and doesn't ship a web browser inside it.

This is a rewrite of an older Zima built with Tauri and React. It has also been called Note along the way, which is why older data may still live in `%APPDATA%\Note`. Zima copies it over on first start and leaves the old folder alone as a backup.

## What it does

**Writing.** Edit, split or preview mode (`Ctrl+E` cycles between them), a Markdown preview with code highlighting, tables, checklists and a formatting bar if you like buttons (with text colours: `==red:text==`). Zen mode hides everything but the text, with typewriter scrolling and a spotlight that fades the lines you're not working on.

**Finding things.** `Ctrl+K` opens a quick switcher that searches notes and runs any command. The sidebar keeps your open notes, pinned notes, favourites, recent notes, notebooks, tags and saved searches in one column, and the Bin sits at the bottom. There's also a daily note (`Ctrl+D`), a random note, "on this day", and a Tasks view (`Ctrl+T`) that collects every checkbox from every note. To add a task with a deadline, type a line like `@due rent tomorrow` and press Enter: it goes straight into Tasks.

**Commands you type.** Start a line with one of these and press Enter:

| Type | What happens |
|---|---|
| `@remind me to call Sam tomorrow at 5` | Sets a reminder. Repeats work too: `every weekday at 9:30`. |
| `@table 3,4` | Inserts a table with 3 rows and 4 columns. |
| `@calc 12*3.5 + 8` | Replaces the line with the answer. |
| `@time 3pm IST to PST` | Converts a time between zones. Countries and cities work too: `@time India to Estonia`, `@time tokyo`. |
| `@curr 100 usd to inr` | Converts money with today's rates (`@currency` works too, and names like "euros" or "$"). |
| `@goal 500` | Sets a word goal for the note. |
| `@timer 25` | Starts a focus timer. |

If you write something like "dentist friday 10am" without the `@remind`, Zima notices and offers to remind you.

**Getting out of the way.** Close the window and Zima keeps running in the tray, so reminders still go off. If you'd rather the close button quit the app, turn off **Keep running in the tray** in Settings. Opening Zima again while it's already running just brings back the window you have. `Ctrl+Alt+N` opens a small capture box from any app. Any note can pop out as a sticky note that stays on your desktop, and any note with `---` between sections can be presented as slides.

**Making it yours.** Several built-in themes (Light, Dark, Ethereal, Zima Blue, Sakura, Cyberpunk and more), an Auto theme that follows the time of day, your own accent colour, and a fully custom theme in `theme.json`. Text size and the size of the whole interface can be changed separately. Keyboard shortcuts can be rebound in Settings (`Ctrl+,`).

**Also in there.** Spell check, word lookup, writing stats, a calendar, notebooks, note colours, archiving, merging notes, and import and export of Markdown folders and zip files.

## Sync

Notes live in `%APPDATA%\Zima` by default. In Settings you can move them into Google Drive, OneDrive, Dropbox or any other folder, and Zima will pick up changes made on your other computers. If the same note was edited in two places, both versions are kept.

## Version history

While you edit, Zima keeps an older copy of each note about every 10 minutes: every copy from the last day, then one a day for a month, then one a week. Right-click a note and choose **Version history…** (or find it in `Ctrl+K`) to see what each copy would change and restore it. Your current text is kept as a copy first, and `Ctrl+Z` undoes a restore. Like backups, copies stay on this PC in `%APPDATA%\Zima\history`. You can turn this off or clear it in Settings → Sync and backups.

## Backups

Once a day Zima zips your whole notes folder into `%APPDATA%\Zima\backups` and keeps the last 14. Backups always stay on this PC, even when your notes are in a synced folder. **Back up now** in Settings makes one straight away, and **Open backups folder** shows them. To get notes back, unzip a backup into your notes folder while Zima is closed.

## Command line

A few things work without opening the window at all:

```sh
zima add "buy milk"
zima add "Groceries\nmilk\neggs"        # the first line becomes the title
zima remind "stretch in 20 min"
zima --help
```

## Building

You'll need a recent [Rust](https://rustup.rs) toolchain on Windows.

```sh
cargo build --release      # target/release/zima.exe (fully optimised, takes several minutes)
cargo test                 # unit tests and command line tests
```

To make an installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`winget install JRSoftware.InnoSetup`), then after the release build run:

```sh
ISCC installer/zima.iss    # target/installer/Zima-Setup-<version>.exe
```

The installer needs no admin rights. It puts Zima in `%LOCALAPPDATA%\Programs\Zima` and adds it to the Start menu, which also lets reminders show up as coming from Zima rather than Windows PowerShell. Uninstalling leaves your notes alone.

To try it without touching your real notes, point it at an empty folder:

```sh
ZIMA_DATA_DIR=/path/to/scratch ./target/release/zima.exe
```

If your graphics driver gives you trouble, turn on **Low-memory rendering** in Settings, or set `SLINT_BACKEND=winit-software` to draw everything on the CPU.

## Privacy

Zima has no accounts, no telemetry and no analytics. Your notes never leave your computer unless you put them in a synced folder yourself. The only time it goes online is when you look up a word, which asks the free [Dictionary API](https://dictionaryapi.dev) for a definition.

## Fonts

Zima bundles Inter, JetBrains Mono, Literata and Newsreader. All four are under the SIL Open Font License, and their licence files are in `fonts/`.

## License

Zima is under the [MIT License](LICENSE). You can use, change and share it, including in commercial projects, as long as you keep the copyright notice and licence text with it.

The bundled fonts keep their own licence (see Fonts above).
