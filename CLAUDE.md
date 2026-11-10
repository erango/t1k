# T1K

A native macOS terminal for running many coding-agent sessions at once (Claude Code, Codex,
Antigravity, Gemini CLI, Grok). Rust, GPUI 0.2 (Zed's UI framework), alacritty_terminal 0.26.
The product goal is attention: which session needs me now, without switching tabs.

## Layout

| Crate | What lives there |
|---|---|
| `crates/t1k-terminal` | The terminal widget: pty + alacritty grid (`terminal.rs`), GPUI rendering, input, selection, links, find, hint mode, prompt marks (`view.rs`), the overlay scrollbar and scroll-to-bottom button (`scrollbar.rs`, canvas v3-al A), drawn block/box glyphs (`glyphs.rs`), link detection (`links.rs`), key encoding (`input.rs`) |
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
- **Set up T1K** (`setup.rs`, canvas v3-ah, design pass v3-ah′): a step's config keys are listed in `SETTINGS` (its Undo puts them back); a new setting worth walking people through goes there too. Every preview is one 230 × 284 frame with its caption inside, following the control used last (`touched`). The Keys step works on `SAMPLES` (plain structs, never `Tab`s or session.json): ⌘J and ⌘D act in the preview, and while ⌘P / ⌘O are tried (`Workspace::setup_samples`) the switcher and overview render the samples. Something opened from setup (a key it shows, the theme picker, Match, Import) calls `setup_leave` first, and `close_overlay` brings setup back. Progress is `setup_step` in session.json. Agent switches change the agents' settings files only on a click.
- **Windows** (`windows.rs`, canvas v3-aq): each window is a `Workspace`; what's the app's lives in the hub there (the hook socket, session.json as a list of windows, the Needs-you queue across windows, plan usage, clicked banners, the order windows are asked at quit). A workspace never reads session.json or the socket itself: `save()` hands its session to the hub, and `tick` takes the hook messages for its own tabs.
- **Quick terminal** (`quick.rs`, canvas v3-ay): a GPUI pop-up window showing a tab's real `TerminalView`. One view can't be drawn in two windows, so `split.rs` draws a placeholder while `quick::shows(view)`; anything that changes what the panel shows must repaint the workspace windows too. The global shortcut is a Carbon hot key in `platform.rs` that only sets a flag; a loop in `main.rs` turns it into `quick::toggle`.
- **Overlays** occlude what's under them. A new overlay is a variant of `overlay::Overlay`, its key handling in `overlay_key`, and its render arm in `overlay::render`.
- **Design.** Follow the canvas: Mercury tokens, quiet by default, color only for states that need a person (amber needs-you, red error, mercury unread). Rows never change height on hover. Add a board to the canvas before building anything visual that the canvas doesn't cover.

## Checking UI without a keyboard

Debug environment variables (all read at launch):

| Variable | Effect |
|---|---|
| `T1K_DEBUG_OVERLAY=palette\|switcher\|overview\|settings\|settings-interface\|settings-search\|settings-keys\|themes\|remind\|icons\|icons-search\|icons-apps\|icons-image\|menu\|groupmenu\|termmenu\|sidebarview\|more\|card\|about\|welcome\|settings-advanced\|update\|update-background\|quit` | Opens that surface (`update` also starts a check) |
| `T1K_DEBUG_UPDATE=install` | Presses Restart now as soon as an update is ready (with `T1K_DEBUG_OVERLAY=update-background`) |
| `T1K_DEBUG_CRASH=error\|panic` | Sends a test error report / crashes on purpose (needs a DSN) |
| `T1K_DEBUG_AGENT=working\|needs\|error\|done\|watching\|watching-agents\|helper` | Fakes an agent session on the active tab (`watching`: canvas v3-ax, with background agents, a monitor and a wake-up in 4 minutes; `watching-agents`: only background agents, so no countdown; `helper`: an untitled Codex turn that just finished, for the helper) |
| `T1K_DEBUG_PRS=1\|open\|done` | Puts PRs in every state on the active tab (open: checks passing / running / failing, each review state, a draft, one gh hasn't answered for; done: merged, closed, merged at an unknown time), with made-up gh answers, and opens the outputs panel. gh is never asked while it's set. Works with `T1K_DEBUG_OVERLAY=switcher` for the PR in ⌘P |
| `T1K_DEBUG_PROGRAM=<name>` | Shows that program's icon on every tab |
| `T1K_DEBUG_VIEW=<path>[:line]` | Opens that file in the side panel |
| `T1K_DEBUG_HINTS=1`, `T1K_DEBUG_FIND=<query>` | Starts terminals in hint mode / with the find bar open |
| `T1K_DEBUG_SCROLL=up:<n>[,new:<n>][,hover]` | About 1.5 s after launch (once the history has n lines), scrolls terminals up n lines and pins the scrollbar and the scroll-to-bottom button on, with "N new lines" on the button and, with `hover`, the wide hover track with prompt ticks. Add `T1K_DEBUG_FIND` for find-match ticks. To fill the history, give the test HOME a `.zshrc` that prints a few hundred lines (launch with `ZDOTDIR=$HOME`); prompt ticks need prompt marks (`\e]8;;t1k://prompt\e\\` … `\e]8;;\e\\` around a fake prompt) |
| `T1K_DEBUG_DROP=pane\|row\|group\|theme:<path>` | Shows files dragged over the pane / the active row / its group header ("Set as icon"), or the theme-file choice (a Finder drag can't be scripted) |
| `T1K_DEBUG_OVERLAY=icons-apps\|icons-image` | The icon picker on its Apps tab (apps are scanned and their icons made before the first frame) / its Image tab |
| `T1K_DEBUG_ICON_IMAGE=<path>`, `T1K_DEBUG_ICON_ZOOM=<1-4>`, `T1K_DEBUG_ICON_QUERY=<text>`, `T1K_DEBUG_ICON_TAB=1` | With the `icons*` overlays: open that image in the cropper (`icons-image`) at that zoom / type a search / pick the active tab's icon instead of its group's |
| `T1K_DEBUG_DUMP=<file>` | Writes the sidebar model as text on changes |
| `T1K_PROFILE=1` | Logs frame build/paint times |
| `T1K_DEBUG_SIDEBAR=rail\|hidden\|peek\|drag:<x>` | Starts with the sidebar collapsed, peeking from the rail, or mid-drag with the pointer at x |
| `T1K_FRESH=1` | Ignores the saved tabs |
| `T1K_DEBUG_OVERLAY=rename\|quit-running\|quit-list` | The rename field / the quit warning / its running list. For something to be running, add `[ -n "$T1K_TEST_RUN" ] && sleep 300` to `$HOME/.zshrc` of the test HOME and launch with `T1K_TEST_RUN=1 ZDOTDIR=$HOME` (login resets HOME, so ZDOTDIR is what reaches the shell) |
| `T1K_DEBUG_USAGE=low\|high5\|high7\|limit\|stale\|multi` | Fakes plan usage limits for the meter (`multi`: Claude, Codex and Antigravity); `T1K_DEBUG_OVERLAY=usage` pins its popover |
| `T1K_DEBUG_OVERLAY=settings-fonts` | Settings › Terminal with the font picker open |
| `T1K_DEBUG_OVERLAY=setup\|setup-<n>\|setup-done\|setup-switcher\|setup-overview` | "Set up T1K" at step 1, step n (1-7) or "All set"; or ⌘P / ⌘O tried on the Keys step (the switcher / overview on the sample tabs). `T1K_DEBUG_SETUP` is a comma-separated list: `tried` ticks every key on the Keys step; `changed` makes each step start from the defaults, so "All set" lists what the test config.toml changed; `touch:<key>` makes that control the last one used, so the preview shows it (last config key segment: `touch:new_tab`, `touch:close_stays_in_group`, `touch:error`, `touch:dock_badge`...); `jump`, `split`, `alt`, `alt-hint` show the Keys preview after ⌘J, after ⌘D, with ⌥ held, after Try it on ⌥; `query:<text>` types into the sample switcher. A `~/.warp/settings.toml` in the test HOME shows the Look step's match offer |
| `T1K_DEBUG_FONTS=states[:query]\|install:<id>[+icons]\|match` | Fakes a "Get more fonts" row in each state (downloading, installed, failed, the pointer on Hack; `:query` types a search to reach lower rows) / starts a real install of a catalog font (`fonts.rs` ids) / runs the welcome card's one-click Match with its download. Installs go to `$HOME/Library/Fonts`, so use an isolated HOME |
| `T1K_DEBUG_OVERLAY=bug\|bug-open\|bug-long` | "Report a bug" with a sample report (and a sample warning in its log list): as it opens / with Attached details expanded / too long for a GitHub link (the note and "Copy and open GitHub"). With `T1K_DEBUG_AGENT=error` and `T1K_DEBUG_OVERLAY=card`, the hover card shows "Report this…" |
| `T1K_DEMO=1\|needs\|outputs` | Demo mode for screenshots: a made-up workspace (15 tabs in 5 projects, agents in every state, PRs, artifacts, terminal screens fed in by `demo.rs`) in a 1440×900 window, Mercury Dark. Reads nothing of yours (no session.json, config.toml, other terminals or transcripts), saves nothing, opens no agent socket, and its terminals run a silent placeholder instead of a shell. `needs` opens on the tab waiting for `terraform apply` with its hover card; `outputs` opens the outputs panel. Combine with `T1K_DEBUG_OVERLAY`, `T1K_THEME` and `T1K_DEBUG_BACKGROUND`. `docs/screenshots/README.md` has the commands |
| `T1K_DEBUG_COPY=chips\|toast` | About 2.5 s after launch, shows the copy chips on the first command's prompt line, or the toast ⌘⇧C would show (clipboard untouched). To have commands, give the test HOME a `.zshrc` whose zle `line-init` hook sets `BUFFER` and calls `zle accept-line` once or twice, and launch with `ZDOTDIR=$HOME` |
| `T1K_DEBUG_PANES=right,down,zoom,menu,confirm,ask,err,other` | Steps run in order on the active tab at launch: split right / down, zoom, the pane menu, the "Stop …?" confirm in pane 1's header, an agent in the first helper pane asking for you / in error (canvas v3-av), then `other` to select the next tab so the row shows unselected |
| `T1K_DEBUG_QUICK=needs\|shell[,top\|bottom\|left\|right][,s\|l]` | Shows the quick terminal (canvas v3-ay) 2.5 s after launch as its shortcut would, pinned and without taking the keyboard, on the main screen. It is a second window: capture it by its own window id (layer 101). `T1K_DEBUG_OVERLAY=settings-quick` opens its Settings section |
| `T1K_DEBUG_WINDOWS=new,move-new,move-back` | Steps a second apart in the first window (canvas v3-aq): ⌘N; the active tab to a new window (needs two tabs); the other window's active tab into this one. Check the result in the test HOME's session.json (`windows`) |
| `T1K_DEBUG_BROADCAST=panes\|group\|pick[,text:<line>][,send:<text>][,paste]` | Turns broadcast input on (canvas v3-aw): for the active tab's panes (split first with `T1K_DEBUG_PANES`), its group (in the demo that includes agents, so the compose bar shows), or the tab-ticking state. `text:` fills the compose bar, `paste` opens the several-lines confirm, and `send:` repeats that text from the main pane into the other receiving terminals 2.5 s after launch, as typing would (needs real shells: no `T1K_DEMO`, an isolated HOME) |
| `T1K_DEBUG_CONTEXT=<transcript>[:pane]` | Shows that Claude transcript as the active tab's context details (canvas v3-au); `:pane` opens the panel on its Context tab, `T1K_DEBUG_OVERLAY=context` opens the popover. `T1K_DEBUG_CONTEXT_OPEN=tools,memory,...` flips those sections from their default (now, compactions, session, request, tools, memory, instructions, skills, subagents, deferred; `tools:<server>` for a tool group). Use a made-up transcript, not a real one |
| `T1K_DEBUG_QUERY=<text>` | Types that into ⌘P / ⌘⇧P (with `T1K_DEBUG_OVERLAY=switcher\|palette`) |
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
- Report a bug (`bug_report.rs`, canvas v3-ai A): a sheet that opens a prefilled issue (`issues/new?title=…&body=…&labels=bug`; over 7,500 characters it copies the body instead). Everything is gathered locally: the commit is baked in by `crates/t1k-app/build.rs` (`T1K_COMMIT` overrides), and the logger in `telemetry.rs` keeps T1K's last 50 warnings and errors in memory, scrubbed, even with Sentry off. Settings are diffed against `config::template()`. Never add terminal output, tab titles or folder names to it.
- Helper (`[helper]`, off by default): `t1k-agent/src/helper.rs` runs one-shot prompts through an agent CLI (claude, codex, agy, gemini, grok) for titles, recaps and switcher search; `t1k-app/src/helper.rs` queues them. Claude Code must run with `--setting-sources "" --strict-mcp-config --tools "" --system-prompt …`, or it loads the user's whole setup (measured 376k tokens, $0.38 for one word; stripped: ~700 tokens, $0.0001). Every call is logged to `helper.jsonl`; `t1k helper bench` / `t1k helper stats`. Codex and Grok parsing is written from their docs, not yet run.
- Get more fonts (`fonts.rs`, `fonts_ui.rs`): a pinned catalog (release URL, GitHub asset id, SHA-256, size) per font and its Nerd Font build. curl downloads it (falling back to the API asset URL when github.com's release endpoint fails), the hash is checked, bsdtar extracts only .ttf/.otf, and files are copied into ~/Library/Fonts without replacing any. New files are handed to GPUI with `add_fonts` so they work without a restart. Row samples are PNGs from `swift scripts/font_samples.swift <font file> resources/font-samples/<id>`. Updating a font means downloading the new asset and pinning its hash and size; never type a hash in.
- Keys: every shortcut is in `keymap.rs` (`ALL`); `[keys]` in config.toml overrides by id. Add new bindable actions there, not in `main.rs`.
- Updates: Sparkle 2.10.1 is embedded by `bundle.sh` and loaded at runtime (`updater.rs`); T1K draws the UI (`update_ui.rs`). `scripts/release.sh <version> [--beta]` builds, notarizes, signs the zip with the EdDSA key (login keychain, account `t1k`), publishes a GitHub release and pushes `appcast.xml` (the feed, read from raw.githubusercontent on main). `--local <url>` writes everything to `target/updates/` for testing. Release notes: `docs/releases/<version>.md`, one `- ` bullet per line.
