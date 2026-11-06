# T1K design spec

This is the source of truth for implementation. The full canvas with all ten boards is at https://claude.ai/artifact/Fm4Hyv7MYFsH7ACopnt1Ka (private).

## Tokens (dark / light)

### Backgrounds

| Token | Dark | Light |
|---|---|---|
| bg.chrome | #0E0F11 | #F2F3F5 |
| bg.terminal | #121316 | #FBFBFC |
| bg.raised | #16181B | #FFFFFF |
| bg.overlay | #1A1C20 | #FFFFFF |
| bg.hover | #1A1C20 | #ECEEF1 |
| bg.selected | #23262B | #E3E7EC |
| bg.control | #2B3038 | #DCE1E7 |

### Borders

| Token | Dark | Light |
|---|---|---|
| border.subtle | #24272C | #DDE1E6 |
| border.strong | #33373D | #CDD2D8 |

### Text

| Token | Dark | Light |
|---|---|---|
| text.primary | #E6E8EB | #17191C |
| text.secondary | #A0A6AE | #4E555E |
| text.tertiary | #7A818B | #6A717B |
| text.disabled | #5E656E | #9AA0A8 |

### Status

| Token | Dark | Light |
|---|---|---|
| status.working | #5B9DF5 | #2F6FD6 |
| status.needs | #F0A63A | #B86A00 |
| status.done | #4CC38A | #1E8A55 |
| status.idle | #6B727C | #7A818B |
| status.error | #F2555A | #D33A3F |
| status.ended | #4A4F57 | #A3A9B1 |

The status halo is the state color at 22% opacity. The row rim uses 35% and the card rim uses 45%.

### Accent

- Accent: #A9C1DE (light: #3F6A99). The match highlight is the accent at 20% opacity.
- Chrome gradient: #F2F5F9 → #9AA4B2 (at 55%) → #D9DEE5.

### Identity hues

A tab's hue is chosen by `hash(session_id) % 8`. The chip background is that hue at 14% opacity on dark and 24% on light.

#E88AA6 #BE98F0 #74B4EC #66CCB8 #A8CC6E #E6C066 #EC966E #96A4F0

### Mercury Dark ANSI palette

- Normal (0–7): #1A1C20 #EF6B73 #5FCB8E #E8C468 #6CA6F5 #C99AF0 #5FC6D0 #D5D9DE
- Bright (8–15): #4A4F57 #F58A90 #7FDCA6 #F0D486 #8DBAF8 #D8B3F5 #80D6DE #F4F6F8
- Foreground #D5D9DE, background #121316, cursor #C9D1DB, selection #2A3442.

## Type

- The UI font is SF Pro Text, with Inter as the fallback. The terminal font is JetBrains Mono at 13px on a 19px line.
- Tab row goal: 13/17, weight 400. Weight 500 when the row is selected, 600 when it is unseen.
- Step or recap line: 12/15.
- Queue row: 12.5.
- Group header: 12, weight 600.
- Section label: 11, weight 600, with 8% extra letter spacing.
- Elapsed time: mono 11 with tabular numbers.
- Chips: mono 10/15, weight 600.
- Overlay title: 15, weight 600.
- Switcher query: 16.

## Sizes

- Sidebar: 300 wide, resizable from 240 to 420.
- Outputs panel: 380 wide, resizable from 320 to 520.
- Title bar: 38 high.
- Tab row: 44 high (36 in compact mode). Padding 5/10/5/12, inset 6 from the sidebar edge.
- Group header: 28. Queue row: 26. Recent row: 28.
- Status glyph: 10×10.
- Terminal padding: 14 vertical, 20 horizontal.
- Switcher: 720 wide, with 54px rows.
- Overview cards: about 264×164, with a 12px gap.
- Spacing scale: 2/4/6/8/10/12/16/24/32.
- Radii: chip 3, row 6, card 9, overlay 12.

## Status glyphs

Each state has a different shape as well as a different color, so states stay readable in grayscale and for colorblind users.

| State | Glyph |
|---|---|
| Working | spinning arc |
| NeedsInput | dot with halo |
| Done | dot |
| Idle | ring |
| Error | diamond |
| Ended | dash |

Amber and red are used only for states where a person needs to act.

## Ordering and the attention queue

- Tabs within a group are ordered: NeedsInput, Error, Done-unseen, Working, Done-seen, Idle.
- Groups are sorted by their most urgent tab.
- Ended tabs move into a collapsed "Recent" section after 10 minutes. Pressing Enter on one resumes it.
- The badge counts NeedsInput and Error tabs only.
- ⌘J walks the queue from the longest-waiting tab. ⌘⇧J walks it backwards.

## Motion

- The Working arc completes one rotation every 900 ms.
- Entering NeedsInput plays two 600 ms pulses.
- Rows slide to their new position over 120 ms when reordered.
- Overlays fade in over 90 ms while rising 4px.
- The ⌘J heads-up display stays for 1.6 s.
- The jump-to-output highlight lasts 1.2 s.
- With Reduce Motion turned on, the Working arc is drawn as a static ¾ ring.

## v2: calm sidebar (supersedes the v1 sidebar)

### Modes

- **Calm** (the default): rows are 32px high and show only the status glyph and the title. Only NeedsInput rows show elapsed time.
  - The attention queue collapses into a single bar ("Needs you N · ⌘J").
  - Idle and seen-done tabs fold into a "+N quiet" row.
- **Detail**: rows are 52px high and add a second line, the identity chip and elapsed time. Detail mode turns on when any of these is true:
  - the pointer has rested over the sidebar for 150 ms while moving slower than 0.3 px/ms
  - ⌥ is held down
  - the sidebar has keyboard focus
- **Transition**: rows grow over 120 ms. The second line fades in during the last 60 ms. The width never changes, and the row under the pointer stays in place.
  - The sidebar returns to calm 400 ms after the pointer leaves, or on the first key sent to the terminal.

### Selected tab

The selected row fills with `bg.terminal` and runs flush to the sidebar's right edge. The sidebar itself uses `bg.sidebar` and has no border, so the selected tab looks connected to the terminal. The row also gets a 3px accent bar (inset 7px top and bottom) and a weight-600 title in `text.primary`.

### States

- **Loud** states are NeedsInput (amber), Error (red), and Unread or Reminder due (accent).
- **Quiet** states are Working (a gray spinner), plus seen-done and Idle (no glyph). Ended tabs live in the Recent section.
- The queue and the Dock badge count NeedsInput, Error and Reminder due. Unread tabs are not counted.

### New and changed tokens

| Token | Dark | Light |
|---|---|---|
| bg.sidebar | #0B0C0E | #EDEEF1 |
| status.unread (= accent) | #A9C1DE | #3F6A99 |
| status.working (now gray) | #7A818B | #8A9099 |
| text.quiet | #8C939C | #5E656E |

`status.done` is now used only for CI checks and diffs.

### Sizes

| Element | Size |
|---|---|
| Row height (calm / detail) | 32 / 52 |
| Row height, compact density | 28 / 44 |
| Row height, spacious density | 36 / 60 |
| Gap between rows | 2 |
| Gap between groups | 20 (8 + hairline + 12) |
| Row padding | 0 12 |
| Second-line indent | 21 |
| Group header | 28 high, 13/600 |
| Needs-you bar | 30 |
| Queue row | 28 |
| Quiet-fold row | 26 |
| Glyph slot | 12×12 |

### Tab context menu

The menu has these items:

- Open in split
- Mark as unread
- Rename
- Reset to auto title
- Remind me in, with a submenu: 15m, 30m, 1h, 3h, Tomorrow 9:00, Custom…
- Move to group
- Copy branch
- Copy resume command
- Reveal in Finder
- Close

Sidebar keys:

| Key | Action |
|---|---|
| u | Mark as unread |
| ↵ | Rename |
| r1–r5 / rc | Set a reminder (preset or custom) |
| m | Move to group |

### Group header

- Expanded: shows counts for loud states only.
- Collapsed: shows a summary line.
- The header is sticky while scrolling.
- It can be renamed inline. The real folder name stays visible as a small label.
- The group menu offers rename, color, sort, mark all read, collapse others, and close quiet tabs.

## Appearance

- The mode is Dark, Light or System, with one theme slot for each of dark and light. ⌘⇧L or the moon button in the title bar toggles between them.
- When an imported theme is active, the chrome colors are derived from it:
  - chrome = the theme's background mixed 4% toward black or gray
  - borders = the foreground at 10%
  - text tiers = the foreground at 100%, 65% and 48%

## config.toml

```toml
appearance = "system"          # "system" | "dark" | "light"
[theme]     dark = "Mercury Dark"; light = "Mercury Light"
[window]    chrome_follows_theme, opacity
[ui]        density, text_size
[sidebar]   mode, hover_intent_ms = 150, show_elapsed, fold_quiet, position, width
[outputs]   default_open, width
[groups]    by, sort, follow_cwd
[tabs]      recent_after_min, new_tab_cwd, title_sources = ["manual","ai_title","osc_title","process"], auto_unread
[reminders] presets, snooze
[terminal]  padding, scrollback, option_as_meta, copy_on_select, bell
[terminal.font]   family, size, line_height, letter_spacing, ligatures, fallbacks
[terminal.cursor] style, blink
[shell]     program, args, login, cwd, term
[shell.env]
[agents]    badge, focus_reporting, helper_model
[notify]    needs/error/reminder/done = { banner, sound }
[keys]      # overrides only
```
