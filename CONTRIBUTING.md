# Contributing to T1K

Thanks for helping. T1K is a native macOS terminal built around one question: which agent session needs me now? Changes that make that question easier to answer, or that make T1K a better terminal, are welcome.

## Before you start

- **Bugs:** open an issue with the bug template, or go straight to a pull request if the fix is small.
- **Anything bigger than a fix:** open an issue first so we can agree on the shape. It saves you rework.
- **The product rule:** T1K stays quiet until something needs a person. Color is for states that need action (amber needs you, red error, the accent for unread). New UI should keep that.

## Build and run

You need macOS, Rust (stable) and, for the app bundle and icons, `rsvg-convert` (`brew install librsvg`).

```sh
cargo run -p t1k-app      # debug build, runs from the terminal
cargo test                # all crates
cargo clippy
scripts/bundle.sh         # T1K.app in target/release (signed ad-hoc without a Developer ID)
```

Run a debug build against a throwaway HOME so your real tabs and settings stay untouched:

```sh
HOME=$PWD/target/home SHELL=/bin/zsh ./target/debug/t1k
```

[`CLAUDE.md`](CLAUDE.md) has the working notes: how the crates fit together, how config and session state flow, and debug environment variables that open any screen at launch (useful for screenshots in a PR).

## Making a change

- **Settings** live in `~/.config/t1k/config.toml`, which is the source of truth. A new setting needs a commented line in `config.rs` `TEMPLATE`, a field with a hand-written default, and an entry in `settings.rs` so Settings search finds it. A test checks the template matches the defaults.
- **Shortcuts** are all in `keymap.rs`. Add new bindable actions there, not in `main.rs`.
- **Agents** normalize into Claude Code's hook format (`t1k-agent/src/agents.rs`). Adding an agent means an adapter there and an installer; hooks must never disturb the agent and must do nothing outside T1K.
- **Visual changes** follow the design canvas linked in `CLAUDE.md`. For something the canvas doesn't cover, describe it in the issue first.
- **Never log terminal contents.** Error reports go to Sentry in release builds and are scrubbed, but the rule is simpler than the scrubber.

## Pull requests

- Keep one change per pull request, with tests where the logic can be tested.
- `cargo test` and `cargo clippy` must pass (CI runs both on macOS).
- Commit messages: an imperative summary with a type prefix (`feat(sidebar):`, `fix(terminal):`, `docs:`), then a body that says what changed for the user and why.
- Include a screenshot for anything visual.

By contributing you agree your work is licensed under the [MIT License](LICENSE).
