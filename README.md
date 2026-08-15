<p align="center">
  <img src="res/icon.png" width="128" height="128" alt="Compono icon">
</p>

<h1 align="center">Compono</h1>

<p align="center">Grid based window placement overlay for Windows.</p>

<p align="center">
  <a href="https://github.com/infinition/compono/actions/workflows/ci.yml"><img src="https://github.com/infinition/compono/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <a href="https://github.com/infinition/compono/releases/latest"><img src="https://img.shields.io/github/v/release/infinition/compono" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/platform-windows-blue" alt="Platform: Windows">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/infinition/compono" alt="License"></a>
</p>

## Overview

Compono drags the fine tuning out of window placement. Move a window to a
screen edge and hold it there; a grid overlay opens on that edge, the window
hides, and drawing a rectangle on the grid moves and resizes the window to
match it exactly. The same placements are reachable from the keyboard with
`Win` plus an arrow key.

It covers the same job as Windows' native Snap, but drives placement itself
through `SetWindowPos` rather than relying on the OS to recognize the drag.
That reaches windows Snap sometimes does not, including a Command Prompt or
Windows Terminal window running as administrator, and windows hosted by
WinUI or XAML Islands.

## Features

- Edge drag and hold: reach a screen edge, hold for 0.5 second, drop the
  window, draw the destination rectangle on the grid.
- Keyboard placement: `Win+Left` / `Win+Right` / `Win+Up` / `Win+Down` snaps
  the active window to a half; pressing the same arrow again halves it
  further into a quarter. Combining a horizontal and a vertical arrow places
  it in a corner.
- Places windows other tools miss, including elevated processes (see
  Administrator privileges below) and Windows Terminal.
- Per monitor DPI aware, multi monitor.
- Tray icon: open the grid, toggle Windows' native Snap on or off, quit.
- French and English interface, following the system locale by default.

## Requirements

- Windows 10 or Windows 11.
- Administrator privileges. See below.

### Administrator privileges

Compono's manifest requests `requireAdministrator`. Windows blocks a
standard process from resizing or moving a window owned by a higher
integrity process (User Interface Privilege Isolation), which covers any
terminal or tool running as administrator. Running Compono elevated removes
that restriction, so it can place any window on the desktop, not only the
ones running at its own privilege level.

## Installation

Download the latest release from the
[Releases page](https://github.com/infinition/compono/releases), extract
the archive, and run `compono.exe`. Windows prompts for elevation on
launch; accept it, that is the administrator manifest described above.

## Usage

### Drag gesture

1. Drag any window to a screen edge (top, left, right or bottom) and hold
   it there for half a second.
2. The grid opens with a halo on that edge. Release the window; it hides
   and the grid stays open.
3. Click and drag on the grid to draw the destination rectangle.
4. Release the click; the window reappears, resized and placed on that
   rectangle, and gets focus.
5. Right click, or click outside the grid, cancels and restores the window
   where it was.

### Keyboard

| Shortcut | Action |
|---|---|
| `Win+Alt+G` | Open the grid for the current foreground window |
| `Win+Left` / `Win+Right` | Snap to the left / right half; press again for a quarter |
| `Win+Up` / `Win+Down` | Snap to the top / bottom half; press again for a quarter |
| One horizontal arrow, then one vertical arrow | Snap to that corner |

Each window keeps its own half/quarter state, so switching to another
window does not disturb it.

### Tray menu

Right click the tray icon for:

- **Show grid** (`Win+Alt+G`) for the foreground window.
- **Windows Snap** on/off, toggling the OS's own window arrangement through
  the registry and `SystemParametersInfo`, the same effect as disabling it
  from Windows Settings.
- **Quit**.

## Configuration

Compono reads `%APPDATA%\Compono\config.toml` on startup. The only setting
at the moment is the interface language:

```toml
lang = "en"
```

Omit it, or leave the file absent, to follow the system locale (falls back
to French).

## Building from source

Requires Rust 1.85 or later, on Windows.

```
cargo build --release
```

The binary is written to `target/release/compono.exe`. `cargo test`
requires an already elevated terminal, since the compiled test binary
carries the same administrator manifest as the application.

## License

MIT, see [LICENSE](LICENSE).
