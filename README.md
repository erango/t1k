<p align="center"><img src="resources/brand/readme-header.png" alt="T1K" width="600"></p>

<p align="center"><b>The terminal for running many coding agents at once.</b><br>
See which one needs you, without switching tabs.</p>

<p align="center">
<a href="https://github.com/erango/t1k/releases/latest">Download</a> ·
<a href="#install">Install</a> ·
<a href="#connect-your-agents">Connect agents</a> ·
<a href="#keys">Keys</a> ·
<a href="https://github.com/erango/t1k/issues">Report a bug</a>
</p>

---

When you run ten or thirty agent sessions side by side (Claude Code, Codex, Antigravity, Grok), the hard part isn't the terminal. It's knowing which session finished, which one is stuck on a question, and which one you haven't looked at yet. T1K is a native macOS terminal built around that one question: **what needs me now?**

- **A calm sidebar.** Tabs group by git repo. A row stays quiet until its agent needs you: amber for a question, red for an error, a mercury dot for something new. Working agents get a small gray spinner. Rows never jump around.
- **Live agent status.** Agents report through their own hooks, so the status is exact rather than guessed from screen text: working, needs you, done, error. Claude Code tabs also take their title from the session and show what the agent is doing right now ("Editing src/auth.rs") or its recap.
- **A needs-you queue.** ⌘J jumps to the tab that has waited longest. The Dock shows a count, and the icon's eyes turn red while anything waits. Banners say which tab needs you; clicking one opens it.
- **Find anything.** ⌘P finds a tab by title, project, branch, PR number, recap or path. ⌘O shows every agent as a card. ⌘⇧P runs any command.
- **Outputs.** ⌘⇧O lists what each session produced: PRs with live state and CI, edited files, docs, tickets and deploys.
- **A real terminal.** GPU-rendered with Zed's GPUI and alacritty's terminal core: truecolor, font fallbacks for Powerline and Nerd Font icons, splits, find, prompt jumping, clickable file paths, PRs and ticket keys, a Markdown viewer for files, 20 themes plus importers for Ghostty, iTerm2, Alacritty, Windows Terminal and base16.

T1K is macOS only, written in Rust.

## Install

**Download** the latest `T1K-<version>.zip` from [Releases](https://github.com/erango/t1k/releases/latest), unzip it and move T1K.app to Applications. It's signed and notarized, and keeps itself up to date (Settings › Advanced › Updates).

**Or build it** with Rust (stable) and `rsvg-convert` (`brew install librsvg`):

```sh
git clone https://github.com/erango/t1k && cd t1k
scripts/bundle.sh --install   # builds T1K.app, copies it to /Applications, links the `t1k` CLI
```

A build without a Developer ID certificate is signed ad-hoc and runs on your own Mac.

## Connect your agents

T1K learns each agent's state from its hooks. Install them for every agent it finds:

```sh
t1k setup-agents
```

or press **Connect agents** on the welcome screen, or use Settings › Agents.

| Agent | Hook config | Notes |
|---|---|---|
| Claude Code | `~/.claude/settings.json` | Also titles, recaps, PRs and context use from the session log |
| Codex | `~/.codex/hooks.json` | "Needs you" when Codex asks a question |
| Antigravity CLI (`agy`) | `~/.gemini/config/hooks.json` | Tool permissions are left untouched |
| Gemini CLI | `~/.gemini/settings.json` | Retired by Google in June 2026; supported while it lasts |
| Grok CLI | `~/.grok/hooks/t1k.json` | |

Each config is backed up next to itself first (`*.json.t1k-backup`). The hooks only act inside T1K; in any other terminal they exit at once and print nothing.

## Keys

| Key | Action |
|---|---|
| ⌘T / ⌘W | New tab in the current folder / close tab |
| ⌘1–9, ⌘⇧[ ⌘⇧] | Select a tab (sidebar order) |
| ⌘[ / ⌘] | Back / forward through the tabs you visited |
| ⌘J / ⌘⇧J | Next / previous tab that needs you |
| ⌘P | Find a tab |
| ⌘⇧P | Command palette |
| ⌘O | Overview of every agent |
| ⌘⇧O | Outputs panel |
| ⌘⇧R | Rename tab (or double-click it) |
| ⌘\ | Show / hide the sidebar |
| ⌘⇧L | Toggle dark / light |
| ⌘D / ⌘⇧D | Split right / down; ⌘⌥ arrows switch panes |
| ⌘F, ⌘G / ⌘⇧G | Find in scrollback, next / previous match |
| ⌘↑ / ⌘↓ | Previous / next prompt (zsh) |
| ⌘-click | Open a link, file path, PR or ticket; ⌘-right-click for Open With |
| ⌘⇧U | Label every link with a letter, then type it to open |
| ⌘K | Clear screen and scrollback |
| ⌘⌫, ⌘← ⌘→, ⌥← ⌥→ | Delete line, line start / end, word left / right |
| ⌘+ / ⌘− / ⌘0 | Bigger / smaller / default text |

Right-click a tab to mark it unread, set a reminder, resume an ended Claude session, copy its branch or move it to another group. Right-click a group header to rename it, color it or change how it sorts.

## Configure

Every setting is a line in `~/.config/t1k/config.toml`. T1K writes a commented copy on first run, and saved changes apply immediately. Settings (⌘,) edits the same file, keeps your comments, and can search across every page.

Themes: pick one for dark mode and one for light, and T1K follows macOS. Import more from the command line or by dropping a file on the window:

```sh
t1k theme import ~/Downloads/Dracula.itermcolors   # Ghostty, iTerm2, Alacritty, Windows Terminal, base16
t1k theme list
```

Not a fan of the android? Settings › Appearance › App icon offers a chrome `>_` prompt or the T1K wordmark.

## Build and contribute

```sh
cargo run -p t1k-app    # debug build, runs from the terminal
cargo test              # all crates
cargo clippy
```

| Crate | What lives there |
|---|---|
| `crates/t1k-terminal` | pty and alacritty grid, GPUI rendering, input, selection, links, find |
| `crates/t1k-themes` | Theme model, built-in themes, importers, derived UI colors |
| `crates/t1k-agent` | Hook parsing, the status machine, per-agent installers, transcript tailing, outputs |
| `crates/t1k-app` | The app and CLI: workspace, sidebar, overlays, settings, updates |

[`CLAUDE.md`](CLAUDE.md) has the working notes: how config and session state flow, debug environment variables for opening any screen at launch, and conventions. The design spec is [`docs/design.md`](docs/design.md).

Bugs and ideas go in [Issues](https://github.com/erango/t1k/issues). Pull requests are welcome; for anything bigger than a fix, open an issue first so we can agree on the shape. Keep the product rule in mind: T1K stays quiet until something needs a person.

## Thanks

T1K stands on [GPUI](https://www.gpui.rs) from the Zed team, [alacritty_terminal](https://github.com/alacritty/alacritty), [Sparkle](https://sparkle-project.org) for updates, and the color-scheme formats of Ghostty, iTerm2 and friends.

## License

[MIT](LICENSE)
