use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{Div, Rgba, div, px, relative};

/// A compact label over a 3px meter, shared by the table cells and the detail
/// hero. `None` renders no bar (missing reading or a memory limit of 0).
pub fn usage_meter(label: &str, fraction: Option<f32>, color: Rgba, theme: &Theme) -> Div {
    div()
        .w_full()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .text_size(px(11.))
        .line_height(px(14.))
        .font_family("JetBrains Mono")
        .text_color(if label == "—" {
            theme.text_faint
        } else {
            theme.text_dim
        })
        .child(div().truncate().child(label.to_string()))
        .when_some(fraction, |element, fraction| {
            let fraction = fraction.clamp(0.0, 1.0);
            let width = if fraction <= 0.0 {
                relative(0.)
            } else {
                relative(fraction.max(0.05))
            };
            element.child(
                div()
                    .w_full()
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(theme.border_subtle)
                    .child(div().h_full().w(width).rounded(px(2.)).bg(color)),
            )
        })
}

pub fn memory_color(percent: Option<f32>, theme: &Theme) -> Rgba {
    match percent {
        Some(percent) if percent >= 90.0 => theme.paused,
        _ => theme.running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_color_turns_amber_at_ninety_percent() {
        let theme = crate::theme::Theme::dark();
        assert_eq!(memory_color(Some(89.9), &theme), theme.running);
        assert_eq!(memory_color(Some(90.0), &theme), theme.paused);
        assert_eq!(memory_color(None, &theme), theme.running);
    }
}
