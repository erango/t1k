# T1K

An agent-first terminal for macOS, built with Rust, [GPUI](https://crates.io/crates/gpui) and [alacritty_terminal](https://crates.io/crates/alacritty_terminal).

It is designed for running many Claude Code sessions side by side:
- tabs titled automatically from Claude's session data
- tabs grouped by project
- live agent status, plus an attention queue for tabs waiting on you
- easy import of color schemes

## Build

```sh
cargo run -p t1k-app
```

GPUI's `runtime_shaders` feature is enabled, so Xcode's separate Metal Toolchain download is not required.

## Status

The first milestones (M0 and M1) are in progress: a single terminal with full color, scrollback, selection, copy/paste and mouse reporting, plus basic tabs.
