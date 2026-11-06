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
