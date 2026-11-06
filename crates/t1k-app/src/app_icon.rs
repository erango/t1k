//! Little program badges for sidebar rows: what's running in each tab at a glance.

use gpui::{div, prelude::*, px, rgb, Div, Hsla};
use t1k_themes::UiColors;

use crate::workspace::Tab;

/// (glyph, color) for a program name; None means "use a monogram".
fn known(program: &str) -> Option<(&'static str, u32)> {
    let p = program.trim_start_matches('-').to_ascii_lowercase();
    Some(match p.as_str() {
        "claude" => ("✳", 0xD97757),
        "codex" => ("◎", 0x10A37F),
        "gemini" => ("✦", 0x4C8BF5),
        "aider" => ("◆", 0x14B014),
        "vim" | "nvim" | "vi" => ("v", 0x57A143),
        "emacs" => ("e", 0x7F5AB6),
        "nano" | "micro" | "hx" | "helix" => ("✎", 0x8C939C),
        "ssh" | "mosh" | "et" => ("⇄", 0x6CA6F5),
        "git" | "lazygit" | "tig" | "gitui" => ("⑂", 0xF05133),
        "node" | "npm" | "npx" | "pnpm" | "yarn" | "bun" | "deno" => ("⬢", 0x5FA04E),
        "python" | "python3" | "ipython" | "uv" | "pip" => ("py", 0x4B8BBE),
        "cargo" | "rustc" | "rust-analyzer" => ("⚙", 0xDEA584),
        "go" => ("go", 0x00ADD8),
        "docker" | "lazydocker" | "podman" => ("◫", 0x2496ED),
        "kubectl" | "k9s" | "helm" | "kubectx" => ("⎈", 0x326CE5),
        "htop" | "btop" | "top" | "glances" => ("▤", 0x5FCB8E),
        "less" | "more" | "man" | "bat" => ("≡", 0x8C939C),
        "make" | "just" => ("⚒", 0xC99AF0),
        "psql" | "mysql" | "sqlite3" | "redis-cli" | "mongosh" => ("⛁", 0x336791),
        "tail" | "watch" => ("↧", 0x8C939C),
        _ => return None,
    })
}

/// A 16 px rounded badge for the tab's program: Claude's spark when a session is attached,
/// a prompt mark at the shell, a known tool's mark, else the program's first letter.
pub fn badge(tab: &Tab, ui: UiColors) -> Div {
    let base = div().size(px(16.)).flex_none().rounded(px(4.)).flex().items_center().justify_center();
    let program = tab.program.as_deref().or(tab.agent.as_ref().map(|_| "claude"));
    let Some(program) = program else {
        // Shell prompt.
        return base
            .bg(ui.bg_control)
            .text_size(px(9.))
            .font_family("JetBrains Mono")
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(ui.text_secondary)
            .child(">_");
    };
    let (glyph, color): (String, Hsla) = match known(program) {
        Some((g, c)) => (g.to_string(), rgb(c).into()),
        None => {
            let first = program.trim_start_matches('-').chars().next().unwrap_or('?').to_ascii_uppercase();
            (first.to_string(), ui.text_secondary)
        }
    };
    let small = glyph.chars().count() > 1;
    base.bg(color.opacity(0.18))
        .text_size(px(if small { 8. } else { 11. }))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(color)
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::known;

    #[test]
    fn maps_common_programs() {
        assert_eq!(known("claude").unwrap().0, "✳");
        assert_eq!(known("-zsh"), None);
        assert_eq!(known("nvim").unwrap().0, "v");
        assert!(known("someprog").is_none());
    }
}
