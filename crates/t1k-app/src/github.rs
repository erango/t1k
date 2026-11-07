//! GitHub's own colors for pull-request states and checks. They're the same in every theme:
//! people know green open, purple merged and red closed from GitHub itself.

use gpui::Hsla;

fn hex(v: u32) -> Hsla {
    gpui::rgb(v).into()
}

pub fn open() -> Hsla {
    hex(0x1f883d)
}
pub fn merged() -> Hsla {
    hex(0x8250df)
}
pub fn closed() -> Hsla {
    hex(0xcf222e)
}
pub fn draft() -> Hsla {
    hex(0x6e7781)
}
/// Checks still running (GitHub's pending yellow).
pub fn pending() -> Hsla {
    hex(0xbf8700)
}

/// Label and color for a PR's state (`state` as `gh` reports it).
pub fn pr_state(state: &str, draft_pr: bool) -> (&'static str, Hsla) {
    match state {
        "MERGED" => ("Merged", merged()),
        "CLOSED" => ("Closed", closed()),
        _ if draft_pr => ("Draft", draft()),
        _ => ("Open", open()),
    }
}
