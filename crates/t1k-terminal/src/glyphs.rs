//! Built-in drawing for block elements and box-drawing lines.
//!
//! Fonts draw these inconsistently: shades (`░▒▓`) come out as dot patterns, and vertical lines
//! leave gaps once the line height is above 1.0. Like Ghostty and Kitty, T1K recognizes them by
//! code point and paints rectangles that fill the cell exactly, so bars are solid and borders join
//! across cells whatever the font.

/// One piece of a drawn glyph, in cell-local pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Piece {
    /// Filled rectangle, with an opacity multiplier (shades use 0.25 / 0.5 / 0.75).
    Rect { x: f32, y: f32, w: f32, h: f32, alpha: f32 },
    /// Rounded corner: an arc of thickness `t` joining two arms. `right`/`down` say which way
    /// the arms leave the cell center.
    Arc { right: bool, down: bool, t: f32 },
}

/// True if `c` is drawn by [`pieces`] instead of the font.
pub fn is_builtin(c: char) -> bool {
    let u = c as u32;
    (0x2580..=0x259F).contains(&u) || arms(c).is_some() || (0x256D..=0x2570).contains(&u)
}

/// Line thickness for light lines at this font size; heavy lines are twice that.
pub fn light(font_size: f32) -> f32 {
    (font_size / 12.).round().max(1.)
}

/// Pieces for `c` in a `w`×`h` cell, or None if the font should draw it.
pub fn pieces(c: char, w: f32, h: f32, font_size: f32) -> Option<Vec<Piece>> {
    let u = c as u32;
    let rect = |x: f32, y: f32, rw: f32, rh: f32| Piece::Rect { x, y, w: rw, h: rh, alpha: 1. };
    let t = light(font_size);
    match u {
        0x2580 => Some(vec![rect(0., 0., w, (h / 2.).round())]),
        // Lower one-eighth .. seven-eighths, then full.
        0x2581..=0x2588 => {
            let eighths = (u - 0x2580) as f32;
            let bh = (h * eighths / 8.).round();
            Some(vec![rect(0., h - bh, w, bh)])
        }
        // Left seven-eighths .. one-eighth.
        0x2589..=0x258F => {
            let bw = (w * (0x2590 - u) as f32 / 8.).round();
            Some(vec![rect(0., 0., bw, h)])
        }
        0x2590 => {
            let half = (w / 2.).round();
            Some(vec![rect(half, 0., w - half, h)])
        }
        0x2591..=0x2593 => {
            let alpha = (u - 0x2590) as f32 * 0.25;
            Some(vec![Piece::Rect { x: 0., y: 0., w, h, alpha }])
        }
        0x2594 => Some(vec![rect(0., 0., w, (h / 8.).round().max(1.))]),
        0x2595 => {
            let bw = (w / 8.).round().max(1.);
            Some(vec![rect(w - bw, 0., bw, h)])
        }
        // Quadrants: bits are upper-left, upper-right, lower-left, lower-right.
        0x2596..=0x259F => {
            let bits: u8 = match u {
                0x2596 => 0b0010,
                0x2597 => 0b0001,
                0x2598 => 0b1000,
                0x2599 => 0b1011,
                0x259A => 0b1001,
                0x259B => 0b1110,
                0x259C => 0b1101,
                0x259D => 0b0100,
                0x259E => 0b0110,
                _ => 0b0111,
            };
            let (hw, hh) = ((w / 2.).round(), (h / 2.).round());
            let mut v = Vec::new();
            if bits & 0b1000 != 0 {
                v.push(rect(0., 0., hw, hh));
            }
            if bits & 0b0100 != 0 {
                v.push(rect(hw, 0., w - hw, hh));
            }
            if bits & 0b0010 != 0 {
                v.push(rect(0., hh, hw, h - hh));
            }
            if bits & 0b0001 != 0 {
                v.push(rect(hw, hh, w - hw, h - hh));
            }
            Some(v)
        }
        0x256D => Some(vec![Piece::Arc { right: true, down: true, t }]),
        0x256E => Some(vec![Piece::Arc { right: false, down: true, t }]),
        0x256F => Some(vec![Piece::Arc { right: false, down: false, t }]),
        0x2570 => Some(vec![Piece::Arc { right: true, down: false, t }]),
        _ => {
            let [up, right, down, left] = arms(c)?;
            let thick = |a: u8| a as f32 * t;
            // Line centers snapped to whole pixels so 1px lines stay crisp.
            let cx = (w / 2.).floor();
            let cy = (h / 2.).floor();
            let vt = thick(up.max(down));
            let ht = thick(left.max(right));
            let mut v = Vec::new();
            // Each arm runs from the edge to the center, overlapping the crossing arms so the
            // junction is filled.
            if up > 0 {
                let a = thick(up);
                v.push(rect(cx - (a / 2.).floor(), 0., a, cy + (ht / 2.).ceil().max(a / 2.)));
            }
            if down > 0 {
                let a = thick(down);
                let y = cy - (ht / 2.).floor().max((a / 2.).floor());
                v.push(rect(cx - (a / 2.).floor(), y, a, h - y));
            }
            if left > 0 {
                let a = thick(left);
                v.push(rect(0., cy - (a / 2.).floor(), cx + (vt / 2.).ceil().max(a / 2.), a));
            }
            if right > 0 {
                let a = thick(right);
                let x = cx - (vt / 2.).floor().max((a / 2.).floor());
                v.push(rect(x, cy - (a / 2.).floor(), w - x, a));
            }
            Some(v)
        }
    }
}

/// Arms of a straight box-drawing character: (up, right, down, left), each 0 none, 1 light,
/// 2 heavy. Covers U+2500–U+254B (minus dashed lines) and the half lines U+2574–U+257F.
fn arms(c: char) -> Option<[u8; 4]> {
    let u = c as u32;
    let a = match u {
        0x2500 => [0, 1, 0, 1],
        0x2501 => [0, 2, 0, 2],
        0x2502 => [1, 0, 1, 0],
        0x2503 => [2, 0, 2, 0],
        0x250C => [0, 1, 1, 0],
        0x250D => [0, 2, 1, 0],
        0x250E => [0, 1, 2, 0],
        0x250F => [0, 2, 2, 0],
        0x2510 => [0, 0, 1, 1],
        0x2511 => [0, 0, 1, 2],
        0x2512 => [0, 0, 2, 1],
        0x2513 => [0, 0, 2, 2],
        0x2514 => [1, 1, 0, 0],
        0x2515 => [1, 2, 0, 0],
        0x2516 => [2, 1, 0, 0],
        0x2517 => [2, 2, 0, 0],
        0x2518 => [1, 0, 0, 1],
        0x2519 => [1, 0, 0, 2],
        0x251A => [2, 0, 0, 1],
        0x251B => [2, 0, 0, 2],
        0x251C => [1, 1, 1, 0],
        0x251D => [1, 2, 1, 0],
        0x251E => [2, 1, 1, 0],
        0x251F => [1, 1, 2, 0],
        0x2520 => [2, 1, 2, 0],
        0x2521 => [2, 2, 1, 0],
        0x2522 => [1, 2, 2, 0],
        0x2523 => [2, 2, 2, 0],
        0x2524 => [1, 0, 1, 1],
        0x2525 => [1, 0, 1, 2],
        0x2526 => [2, 0, 1, 1],
        0x2527 => [1, 0, 2, 1],
        0x2528 => [2, 0, 2, 1],
        0x2529 => [2, 0, 1, 2],
        0x252A => [1, 0, 2, 2],
        0x252B => [2, 0, 2, 2],
        0x252C => [0, 1, 1, 1],
        0x252D => [0, 1, 1, 2],
        0x252E => [0, 2, 1, 1],
        0x252F => [0, 2, 1, 2],
        0x2530 => [0, 1, 2, 1],
        0x2531 => [0, 1, 2, 2],
        0x2532 => [0, 2, 2, 1],
        0x2533 => [0, 2, 2, 2],
        0x2534 => [1, 1, 0, 1],
        0x2535 => [1, 1, 0, 2],
        0x2536 => [1, 2, 0, 1],
        0x2537 => [1, 2, 0, 2],
        0x2538 => [2, 1, 0, 1],
        0x2539 => [2, 1, 0, 2],
        0x253A => [2, 2, 0, 1],
        0x253B => [2, 2, 0, 2],
        0x253C => [1, 1, 1, 1],
        0x253D => [1, 1, 1, 2],
        0x253E => [1, 2, 1, 1],
        0x253F => [1, 2, 1, 2],
        0x2540 => [2, 1, 1, 1],
        0x2541 => [1, 1, 2, 1],
        0x2542 => [2, 1, 2, 1],
        0x2543 => [2, 1, 1, 2],
        0x2544 => [2, 2, 1, 1],
        0x2545 => [1, 1, 2, 2],
        0x2546 => [1, 2, 2, 1],
        0x2547 => [2, 2, 1, 2],
        0x2548 => [1, 2, 2, 2],
        0x2549 => [2, 1, 2, 2],
        0x254A => [2, 2, 2, 1],
        0x254B => [2, 2, 2, 2],
        0x2574 => [0, 0, 0, 1],
        0x2575 => [1, 0, 0, 0],
        0x2576 => [0, 1, 0, 0],
        0x2577 => [0, 0, 1, 0],
        0x2578 => [0, 0, 0, 2],
        0x2579 => [2, 0, 0, 0],
        0x257A => [0, 2, 0, 0],
        0x257B => [0, 0, 2, 0],
        0x257C => [0, 2, 0, 1],
        0x257D => [1, 0, 2, 0],
        0x257E => [0, 1, 0, 2],
        0x257F => [2, 0, 1, 0],
        _ => return None,
    };
    Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(p: &[Piece]) -> f32 {
        p.iter()
            .map(|p| match p {
                Piece::Rect { w, h, alpha, .. } => w * h * alpha,
                Piece::Arc { .. } => 0.,
            })
            .sum()
    }

    #[test]
    fn blocks_fill_exact_fractions() {
        let (w, h) = (8., 16.);
        assert_eq!(area(&pieces('█', w, h, 13.).unwrap()), w * h);
        assert_eq!(area(&pieces('▄', w, h, 13.).unwrap()), w * h / 2.);
        assert_eq!(area(&pieces('▌', w, h, 13.).unwrap()), w * h / 2.);
        assert_eq!(area(&pieces('░', w, h, 13.).unwrap()), w * h / 4.);
        assert_eq!(area(&pieces('▓', w, h, 13.).unwrap()), w * h * 0.75);
        assert_eq!(area(&pieces('▟', w, h, 13.).unwrap()), w * h * 0.75);
    }

    #[test]
    fn vertical_line_spans_the_whole_cell() {
        // No gap at line height > 1: the arms reach both edges.
        let p = pieces('│', 8.4, 17., 13.).unwrap();
        let (top, bottom) = p.iter().fold((f32::MAX, 0f32), |(t, b), p| match p {
            Piece::Rect { y, h, .. } => (t.min(*y), b.max(y + h)),
            _ => (t, b),
        });
        assert_eq!((top, bottom), (0., 17.));
    }

    #[test]
    fn recognizes_ranges() {
        assert!(is_builtin('░') && is_builtin('╭') && is_builtin('┼') && is_builtin('━'));
        assert!(!is_builtin('a') && !is_builtin('┄') && !is_builtin('═'));
    }
}
