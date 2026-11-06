//! A plain text tooltip in the theme's raised style.

use gpui::{div, prelude::*, px, AnyView, App, Context, SharedString, Window};
use t1k_themes::UiColors;

pub struct TextTooltip {
    text: SharedString,
    ui: UiColors,
}

impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let ui = self.ui;
        div()
            .max_w(px(280.))
            .px(px(10.))
            .py(px(7.))
            .rounded(px(7.))
            .bg(ui.bg_raised)
            .border_1()
            .border_color(ui.border)
            .shadow_md()
            .font_family(".SystemUIFont")
            .text_size(px(12.))
            .line_height(px(17.))
            .text_color(ui.text_secondary)
            .child(self.text.clone())
    }
}

/// For `.tooltip(...)` on a stateful element.
pub fn text(ui: UiColors, text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |_, cx| cx.new(|_| TextTooltip { text: text.into(), ui }).into()
}
