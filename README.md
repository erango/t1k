# T1K

T1K is a terminal for working with many Claude Code agents at once. It's built in Rust on [GPUI](https://www.gpui.rs) (Zed's UI framework) and [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal). It runs on macOS only.

- **Calm sidebar.** Tabs are grouped by git repo. Rows stay quiet until a tab needs you, rest the pointer on one for a detail card, or hold ⌥ to see every tab's status line. Rows never move.
- **Claude-aware tabs.**
  - Titles come from Claude's own session title.
  - The second line shows what the agent is doing now ("Editing src/auth.rs") or its latest recap.
  - Status changes the moment Claude reports it: working, needs input, done or error.
- **Needs-you queue.** Tabs waiting on input, tabs with errors and reminders that have gone off are counted in the sidebar, the window title and the Dock badge. ⌘J jumps to the tab that has waited longest. Banners say when a tab needs you; clicking one opens that tab (when T1K runs as T1K.app).
- **Finding tabs.**
  - ⌘P is a fuzzy switcher over titles, projects, recaps, branches, PR numbers and paths.
  - ⌘O is an overview of every agent as a card.
- **Outputs panel.** ⌘⇧O lists the PRs (with live state and CI from `gh`), artifacts, docs, tickets, deploys and edited files each session produced.
- **Tab tools.** Right-click a tab to:
  - mark it unread
  - rename it (also by double-click or ⌘⇧R)
  - set a reminder (15m / 30m / 1h / 3h / tomorrow 9:00)
  - resume an ended Claude session
  - copy its branch or resume command
  - reveal its folder in Finder
  - move it to another group
- **Groups.** Right-click a group header to open a new tab there, rename it, give it a color, change how its tabs sort (needs attention, recently used, order opened) or collapse it. Closed Claude sessions fold into a "Recent" row at the end of their group; one click resumes one in a new tab.
- **Themes.**
  - 20 built-in themes. You pick one theme for dark mode and one for light; T1K follows macOS by default, and ⌘⇧L toggles.
  - Import Ghostty, iTerm2, Alacritty, Windows Terminal or base16 schemes, from the command line or by dropping the file on the window.
- **Terminal.**
  - Full color: truecolor and 256 colors.
  - Links: URLs and OSC 8 hyperlinks are always underlined. Holding ⌘ also lights up file paths (with `:line:col`), `#123` PR references and ticket keys like `ENG-42`. ⌘-click opens a link: files in your editor at the line, PRs in the tab's GitHub repo, tickets in your tracker. ⌘⇧U labels every link with a letter, so you can open one from the keyboard.
  - Find (⌘F) searches the scrollback as you type. In zsh, T1K marks each prompt, so ⌘↑ / ⌘↓ jump between commands. It loads your own zsh config as usual; turn it off with `terminal.shell_integration = false`.
  - Block and box-drawing characters are drawn by T1K, so bars are solid and borders join at any line height.
  - macOS line editing: ⌘⌫ deletes the line, ⌘← / ⌘→ jump to its start or end, ⌥← / ⌥→ move by word.
  - Fonts: Powerline and Nerd Font icons come through fallback fonts.
  - Scrollback, mouse reporting and focus reporting.
- **Restore.** On relaunch, tabs, folders, reminders and each tab's last Claude session come back.

## Install

You need Rust (stable) and `rsvg-convert` (`brew install librsvg`).

```sh
scripts/bundle.sh             # builds target/release/T1K.app
scripts/bundle.sh --install   # also copies it to /Applications and links the `t1k` CLI
```

For development, run `cargo run -p t1k-app`.

## Connect Claude Code

```sh
t1k setup-claude
```

This adds hooks to `~/.claude/settings.json` and backs the file up to `settings.json.t1k-backup` first. Each hook is `[ -n "$T1K_BIN" ] && "$T1K_BIN" hook <Event> || true`, so it exits immediately in any other terminal. Inside T1K, hooks report to the window that owns the tab over a per-process unix socket.

## Keys

| Key | Action |
|---|---|
| ⌘T / ⌘W | New tab (in the current folder) / close tab |
| ⌘1–9, ⌘⇧[ ⌘⇧] | Select tab (in sidebar order) |
| ⌘[ / ⌘] | Back / forward through the tabs you visited (also the ‹ › buttons atop the sidebar) |
| ⌘J | Jump to the next tab that needs you |
| ⌘P | Find a tab |
| ⌘⇧P | Command palette: every action, Settings page, toggle and theme |
| ⌘O | Overview |
| ⌘⇧O | Outputs panel |
| ⌘⇧R | Rename tab |
| ⌘\ | Toggle the sidebar |
| ⌘⇧L | Toggle dark / light |
| ⌘K | Clear the screen and scrollback |
| ⌘-click | Open a link |
| ⌘⇧U | Label every link; type its letter to open it |
| ⌘⌫ | Delete the line |
| ⌘F, ⌘G / ⌘⇧G | Find in scrollback, next / previous match |
| ⌘D / ⌘⇧D | Split the tab right / down (a second shell); ⌘⌥← / ⌘⌥→ switch pane, ⌘W closes the focused split |
| ⌘↑ / ⌘↓ | Jump to the previous / next prompt (zsh) |
| ⌘+ / ⌘− / ⌘0 | Bigger / smaller / default text |

## Configure

Settings live in `~/.config/t1k/config.toml`. T1K writes a commented file on first run, and saved changes apply immediately. The file covers:

- appearance
- the dark and light themes
- terminal font, size, line height, fallback fonts and padding
- Option as Meta
- sidebar width and hover delay
- notification banners and the Dock badge

### Themes

```sh
t1k theme import ~/Downloads/Dracula.itermcolors   # Ghostty, iTerm2, Alacritty, Windows Terminal, base16
t1k theme list
```

Imported themes are stored in `~/.config/t1k/themes/` and appear in View › Dark Theme / Light Theme. The window chrome (sidebar, title bar, text colors) is derived from each theme's colors.

## Layout

| Crate | Purpose |
|---|---|
| `crates/t1k-terminal` | pty, alacritty grid, GPUI renderer, input encoding, link detection, process inspection |
| `crates/t1k-themes` | Built-in themes, importers, UI color derivation |
| `crates/t1k-agent` | Claude Code hooks, socket, transcript tailing, status machine, outputs |
| `crates/t1k-app` | The `t1k` app and CLI: workspace, sidebar, overlays, outputs panel, config |

The design spec is in [`docs/design.md`](docs/design.md).

## Debugging

- `T1K_DEBUG_DUMP=/tmp/t1k.txt` writes the sidebar model as text whenever it changes.
- `cargo run -p t1k-agent --example inspect -- <transcript.jsonl>` prints the title, recap and outputs T1K extracts from a Claude transcript.
