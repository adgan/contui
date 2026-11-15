use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{ClipboardItem, Context, FontWeight, Render, SharedString, Window, div, px};

/// The full first-column value. GPUI shows it after the tooltip delay and keeps
/// it open while the pointer is inside, so Copy is reachable.
pub struct CellTooltip {
    text: SharedString,
    theme: Theme,
    copied: bool,
}

impl CellTooltip {
    pub fn new(text: SharedString, theme: Theme) -> Self {
        Self {
            text,
            theme,
            copied: false,
        }
    }
}

impl Render for CellTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let hover_bg = theme.panel;
        div()
            .max_w(px(360.))
            .p(px(10.))
            .rounded(px(10.))
            .bg(theme.panel_raised)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(theme.text)
                    .child(self.text.clone()),
            )
            .child(
                div().flex().items_center().child(
                    div()
                        .id("tooltip-copy")
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .occlude()
                        .cursor_pointer()
                        .bg(if self.copied {
                            theme.running_soft
                        } else {
                            theme.hover
                        })
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if self.copied { theme.running } else { theme.text })
                        .hover(move |style| style.bg(hover_bg))
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(this.text.to_string()));
                            this.copied = true;
                            cx.notify();
                        }))
                        .child(if self.copied { "Copied" } else { "Copy" }),
                ),
            )
    }
}
