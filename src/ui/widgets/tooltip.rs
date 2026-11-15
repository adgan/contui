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
            .debug_selector(|| "tooltip-panel".to_string())
            .occlude()
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
                        .debug_selector(|| "tooltip-copy".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
    use std::time::Duration;

    struct Harness {
        clicks: usize,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = crate::theme::Theme::dark();
            let tip_theme = theme.clone();
            div().relative().size_full().bg(theme.bg).child(
                div()
                    .id("row")
                    .absolute()
                    .top(px(20.))
                    .left(px(20.))
                    .w(px(320.))
                    .h(px(52.))
                    .bg(theme.panel)
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.clicks += 1;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id("anchor")
                            .absolute()
                            .top(px(0.))
                            .left(px(0.))
                            .w(px(220.))
                            .h(px(40.))
                            .hoverable_tooltip(move |_window, cx| -> gpui::AnyView {
                                cx.new(|_cx| {
                                    CellTooltip::new("a-very-long-name".into(), tip_theme.clone())
                                })
                                .into()
                            }),
                    ),
            )
        }
    }

    fn hover_tooltip(cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(point(px(30.), px(30.)), None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_millis(700));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn clicking_the_tooltip_panel_does_not_reach_the_row(cx: &mut TestAppContext) {
        let (harness, cx) = cx.add_window_view(|_window, _cx| Harness { clicks: 0 });
        hover_tooltip(cx);

        let panel = cx.debug_bounds("tooltip-panel").expect("tooltip is visible");
        cx.simulate_click(panel.origin + point(px(3.), px(3.)), Modifiers::none());
        cx.run_until_parked();

        let clicks = cx.update(|_, cx| harness.read(cx).clicks);
        assert_eq!(clicks, 0, "clicking the tooltip must not open the row");
    }

    #[gpui::test]
    fn copy_inside_the_tooltip_writes_the_full_name(cx: &mut TestAppContext) {
        let (harness, cx) = cx.add_window_view(|_window, _cx| Harness { clicks: 0 });
        hover_tooltip(cx);

        let copy = cx.debug_bounds("tooltip-copy").expect("copy button visible");
        cx.simulate_click(copy.origin + point(px(4.), px(4.)), Modifiers::none());
        cx.run_until_parked();

        let text = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(text.as_deref(), Some("a-very-long-name"));
        assert_eq!(cx.update(|_, cx| harness.read(cx).clicks), 0);
    }
}
