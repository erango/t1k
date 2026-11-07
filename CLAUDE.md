# T1K

A native macOS terminal for running many coding-agent sessions at once (Claude Code, Codex,
Antigravity, Gemini CLI, Grok). Rust, GPUI 0.2 (Zed's UI framework), alacritty_terminal 0.26.
The product goal is attention: which session needs me now, without switching tabs.

## Layout

| Crate | What lives there |
|---|---|
| `crates/t1k-terminal` | The terminal widget: pty + alacritty grid (`terminal.rs`), GPUI rendering, input, selection, links, find, hint mode, prompt marks (`view.rs`), drawn block/box glyphs (`glyphs.rs`), link detection (`links.rs`), key encoding (`input.rs`) |
| `crates/t1k-themes` | Theme model, 22 built-in themes, importers (Ghostty, iTerm2, Alacritty, Windows Terminal, base16), derived UI colors |
| `crates/t1k-agent` | Agent integration: hook parsing and status machine (`hooks.rs`, `state.rs`), per-agent adapters and installers (`agents.rs`), transcript tailing (`transcript.rs`), outputs classification (`outputs.rs`), the per-process unix socket (`server.rs`) |
| `crates/t1k-app` | The app: `workspace.rs` (tabs, session, polling, actions), `sidebar.rs`, `strip.rs` (title bar), `overlay.rs` (menus, switcher, overview, palette), `settings.rs`, `outputs_panel.rs`, `split.rs`, `config.rs`, `session.rs`, `cli.rs` (`t1k hook`, `setup-agents`, `theme import`), `platform.rs` (Dock, notifications) |

`docs/design.md` is the written spec. The visual source of truth is the design canvas:
https://claude.ai/artifact/Fm4Hyv7MYFsH7ACopnt1Ka (v3 row at the top; v2 and Settings below; v1 is superseded).

## Commands

```sh
cargo build                       # debug
cargo test                        # all crates (terminal, themes, agent, app)
cargo clippy
./scripts/bundle.sh               # release T1K.app in target/release
./scripts/bundle.sh --install     # also copy to /Applications and link ~/.local/bin/t1k
python3 scripts/make_icon.py      # regenerate icons and brand images from the pixel face
```

## Working on it

- **Config is the source of truth.** Every setting is a key in `~/.config/t1k/config.toml`. The UI writes the file through `config::set` / `set_value` (comments are preserved), and the watcher applies changes live. A new setting needs a line in `config.rs` `TEMPLATE`, a field with a hand-written `Default` (the test `template_is_complete` checks the template), and an entry in `settings.rs` `entries()` so Settings search finds it.
- **Session state** (tabs, groups, colors, Recent, window frame) is `session.json` in `~/Library/Application Support/t1k/`. Running a second T1K with your real HOME overwrites it; back it up first or use an isolated HOME (see below).
- **Agents.** All agents normalize into Claude Code's hook schema (`agents::normalize`), so `state.rs` has one status machine. Only Claude's transcripts are read (titles, recaps, PRs, context %). Hooks must never disturb the agent: `t1k hook` exits silently outside T1K and prints whatever the agent expects on stdout. Claude's plan usage limits (5-hour, weekly) reach T1K only through its status line: `setup-claude` wraps the user's `statusLine.command` so it pipes the input to `t1k statusline` and then to the original command (`t1k-agent/src/statusline.rs`; `unwrap` restores it).
- **Rendering.** The terminal view is cached (`AnyView::cached`) by the workspace, so anything that changes how a terminal looks must `cx.notify()` on that view. Paint shapes each row once with a run per style; keep it that way.
- **Overlays** occlude what's under them. A new overlay is a variant of `overlay::Overlay`, its key handling in `overlay_key`, and its render arm in `overlay::render`.
- **Design.** Follow the canvas: Mercury tokens, quiet by default, color only for states that need a person (amber needs-you, red error, mercury unread). Rows never change height on hover. Add a board to the canvas before building anything visual that the canvas doesn't cover.

## Checking UI without a keyboard

Debug environment variables (all read at launch):

| Variable | Effect |
|---|---|
| `T1K_DEBUG_OVERLAY=palette\|switcher\|overview\|settings\|settings-interface\|settings-search\|settings-keys\|themes\|remind\|icons\|icons-search\|menu\|groupmenu\|termmenu\|sidebarview\|more\|card\|about\|welcome\|settings-advanced\|update\|update-background\|quit` | Opens that surface (`update` also starts a check) |
| `T1K_DEBUG_UPDATE=install` | Presses Restart now as soon as an update is ready (with `T1K_DEBUG_OVERLAY=update-background`) |
| `T1K_DEBUG_CRASH=error\|panic` | Sends a test error report / crashes on purpose (needs a DSN) |
| `T1K_DEBUG_AGENT=working\|needs\|error\|done\|helper` | Fakes an agent session on the active tab (`helper`: an untitled Codex turn that just finished, for the helper) |
| `T1K_DEBUG_PROGRAM=<name>` | Shows that program's icon on every tab |
| `T1K_DEBUG_VIEW=<path>[:line]` | Opens that file in the side panel |
| `T1K_DEBUG_HINTS=1`, `T1K_DEBUG_FIND=<query>` | Starts terminals in hint mode / with the find bar open |
| `T1K_DEBUG_DUMP=<file>` | Writes the sidebar model as text on changes |
| `T1K_PROFILE=1` | Logs frame build/paint times |
| `T1K_DEBUG_SIDEBAR=rail\|hidden\|peek\|drag:<x>` | Starts with the sidebar collapsed, peeking from the rail, or mid-drag with the pointer at x |
| `T1K_FRESH=1` | Ignores the saved tabs |
| `T1K_DEBUG_OVERLAY=rename\|quit-running\|quit-list` | The rename field / the quit warning / its running list. For something to be running, add `[ -n "$T1K_TEST_RUN" ] && sleep 300` to `$HOME/.zshrc` of the test HOME and launch with `T1K_TEST_RUN=1 ZDOTDIR=$HOME` (login resets HOME, so ZDOTDIR is what reaches the shell) |
| `T1K_DEBUG_BACKGROUND=1` | Opens the window without taking focus, so screenshots don't steal your typing; capture it by window id (`screencapture -l <id>`) |

Run against an isolated HOME so your real session and config stay untouched, e.g.
`HOME=$PWD/target/home SHELL=/bin/zsh T1K_DEBUG_OVERLAY=settings ./target/debug/t1k`, then
`screencapture -x shot.png`. Keep that HOME path short: the agent socket path has a ~104-byte
limit.

## Conventions

- Commit messages: imperative summary with a type prefix (`feat(sidebar):`, `fix(terminal):`, `design:`, `perf:`, `docs:`), then a body that says what changed for the user and why.
- Comments explain why, not what. User-facing copy is plain and short.
- macOS only for now. `bundle.sh` signs with the first "Developer ID Application" identity in the keychain (hardened runtime, `scripts/T1K.entitlements`), or `T1K_SIGN_IDENTITY` (`-` for ad-hoc). Notarize with `T1K_NOTARY_PROFILE=<profile> ./scripts/notarize.sh target/release/T1K.app` (profile from `xcrun notarytool store-credentials`).
- Crash reporting: Sentry (org `tv-tracker`, project `t1k`), in `telemetry.rs`. On only when the build has a DSN (`T1K_SENTRY_DSN` in the gitignored `.env.local`, read by `bundle.sh` and baked in with `option_env!`) and `privacy.crash_reports` is true. `log::error!` becomes an event and warn/info become breadcrumbs, all scrubbed (home folder to `~`, emails, URL queries, backtick-quoted panic text). `release.sh` uploads the dSYM with `SENTRY_AUTH_TOKEN` from `.env.local`. Never log terminal contents.
- Helper (`[helper]`, off by default): `t1k-agent/src/helper.rs` runs one-shot prompts through an agent CLI (claude, codex, agy, gemini, grok) for titles, recaps and switcher search; `t1k-app/src/helper.rs` queues them. Claude Code must run with `--setting-sources "" --strict-mcp-config --tools "" --system-prompt …`, or it loads the user's whole setup (measured 376k tokens, $0.38 for one word; stripped: ~700 tokens, $0.0001). Every call is logged to `helper.jsonl`; `t1k helper bench` / `t1k helper stats`. Codex and Grok parsing is written from their docs, not yet run.
- Keys: every shortcut is in `keymap.rs` (`ALL`); `[keys]` in config.toml overrides by id. Add new bindable actions there, not in `main.rs`.
- Updates: Sparkle 2.10.1 is embedded by `bundle.sh` and loaded at runtime (`updater.rs`); T1K draws the UI (`update_ui.rs`). `scripts/release.sh <version> [--beta]` builds, notarizes, signs the zip with the EdDSA key (login keychain, account `t1k`), publishes a GitHub release and pushes `appcast.xml` (the feed, read from raw.githubusercontent on main). `--local <url>` writes everything to `target/updates/` for testing. Release notes: `docs/releases/<version>.md`, one `- ` bullet per line.
