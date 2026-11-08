# Screenshots

Every image here comes from demo mode (`T1K_DEMO`): a made-up workspace of 15 tabs in five projects. No real tabs, paths or terminal output appear. Window 1440 points wide, Retina (2880 px), Mercury Dark, JetBrains Mono.

| File | Shows |
|---|---|
| [`hero.png`](hero.png) | The main window: the sidebar with Needs you (two questions, one error), working spinners, unread recaps and the project groups; a finished Claude Code turn in the terminal; the title bar with status, context and the PR chip. |
| [`needs-you.png`](needs-you.png) | A tab waiting to run `terraform apply`: the permission prompt in the terminal, its amber row and the hover card with the last lines, branch, PR and context. |
| [`switcher.png`](switcher.png) | ⌘P with "pay" typed: tabs that need you first, then matches by title, project, recap and PR. |
| [`overview.png`](overview.png) | ⌘O: every agent as a card with its state, branch, PR and last lines. |
| [`outputs.png`](outputs.png) | ⌘⇧O: PR cards with title, checks and review, plus an artifact, a ticket, a deploy and edited files. |
| [`setup.png`](setup.png) | Set up T1K, the Sidebar step, with its live preview. |
| [`themes.png`](themes.png) | The theme picker (View › Choose Theme…) with its preview. |
| [`hero-light.png`](hero-light.png) | The main window in Mercury Light. |

`social/` has the same images sized for sharing:

| File | For |
|---|---|
| `producthunt-1-hero-1270x760.png` … `producthunt-5-outputs-1270x760.png` | Product Hunt gallery (1270 × 760), in order: hero, needs you, switcher, overview, outputs |
| `kofi-hero-1200x630.png`, `kofi-hero-light-1200x630.png` | Ko-fi and link previews (1200 × 630), the whole window on a dark backdrop |
| `kofi-hero-crop-1200x630.png` | The same size, full bleed: the top of the window, bigger text |
| `slack-hero.png` | Slack: the hero at full size |

## Remaking them

```sh
cargo build
scripts/demo_screenshots.sh
```

The script runs the debug build with `HOME=target/demo`, `T1K_DEMO` and `T1K_DEBUG_BACKGROUND=1` (the window opens without taking focus), captures each window with `screencapture -l`, and makes the social sizes with ImageMagick. To look at one scene by hand:

```sh
HOME=$PWD/target/demo T1K_DEMO=1 ./target/debug/t1k                                  # hero
HOME=$PWD/target/demo T1K_DEMO=needs ./target/debug/t1k                              # needs you + card
HOME=$PWD/target/demo T1K_DEMO=outputs ./target/debug/t1k                            # outputs panel
HOME=$PWD/target/demo T1K_DEMO=1 T1K_DEBUG_OVERLAY=switcher T1K_DEBUG_QUERY=pay ./target/debug/t1k
```

The demo workspace is in [`crates/t1k-app/src/demo.rs`](../../crates/t1k-app/src/demo.rs).
