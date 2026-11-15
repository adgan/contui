use crate::theme::{Theme, radius, space, type_scale};
use crate::ui::root::RootView;
use crate::ui::style::Layout;
use gpui::prelude::*;
use gpui::{Context, FontWeight, Stateful, div, px, relative, svg};

pub struct StatCard<'a> {
    pub id: &'static str,
    pub kicker: &'a str,
    pub icon: &'static str,
    pub value: String,
    pub unit: &'a str,
    pub right: String,
    pub foot: String,
    pub progress: Option<f32>,
    pub active: bool,
}

/// One overview card. Height is authoritative: content can truncate but never
/// changes the strip height.
pub fn stat_card(
    card: StatCard<'_>,
    layout: &Layout,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_click: impl Fn(&mut RootView, &mut gpui::Window, &mut Context<RootView>) + 'static,
) -> Stateful<gpui::Div> {
    let padding = if layout.compact { 10.0 } else { space::LG };
    let value_size = if layout.compact {
        24.0
    } else {
        type_scale::DISPLAY
    };
    let kicker_size = if layout.compact {
        9.0
    } else {
        type_scale::KICKER
    };
    let kicker_line = if layout.compact { 12.0 } else { 13.0 };
    let foot_size = if layout.compact {
        9.0
    } else {
        type_scale::META
    };
    let foot_line = if layout.compact { 12.0 } else { 14.0 };
    let icon_color = if card.active {
        theme.accent
    } else {
        theme.text_faint
    };

    let mut element = div()
        .id(card.id)
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h(px(layout.overview_height))
        .px(px(padding))
        .pt(px(if layout.compact { padding } else { 12.0 }))
        .pb(px(if layout.compact { padding } else { 12.0 }))
        .flex()
        .flex_col()
        .rounded(px(radius::LG))
        .bg(theme.panel)
        .border_1()
        .border_color(if card.active {
            theme.accent
        } else {
            theme.border
        })
        .cursor_pointer()
        .hover(|style| style.bg(theme.hover))
        .on_click(cx.listener(move |this, _event, window, cx| {
            on_click(this, window, cx);
            cx.notify();
        }));

    if card.active {
        element = element.child(
            div()
                .absolute()
                .top_0()
                .left(px(padding))
                .w(px(34.))
                .h(px(2.))
                .bg(theme.accent),
        );
    }

    element = element.child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .truncate()
                    .text_size(px(kicker_size))
                    .line_height(px(kicker_line))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_dim)
                    .child(card.kicker.to_uppercase()),
            )
            .child(svg().path(card.icon).size(px(14.)).text_color(icon_color)),
    );

    element = element.child(
        div()
            .mt(px(space::XS))
            .flex()
            .items_baseline()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(value_size))
                    .line_height(px(value_size + 2.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(card.value),
            )
            .child(
                div()
                    .text_size(px(type_scale::META))
                    .line_height(px(14.))
                    .text_color(theme.text_dim)
                    .child(gpui::SharedString::from(card.unit.to_string())),
            )
            .child(div().flex_1())
            .child(
                div()
                    .truncate()
                    .text_size(px(type_scale::KICKER))
                    .line_height(px(14.))
                    .text_color(theme.text_faint)
                    .child(card.right),
            ),
    );

    element = element.child(
        div()
            .mt_auto()
            .truncate()
            .text_size(px(foot_size))
            .line_height(px(foot_line))
            .text_color(theme.text_faint)
            .child(card.foot),
    );

    if let Some(progress) = card.progress {
        let fraction = progress.clamp(0.0, 1.0);
        element = element.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(3.))
                .bg(theme.border_subtle)
                .child(
                    div()
                        .h_full()
                        .w(if fraction <= 0.0 {
                            relative(0.)
                        } else {
                            relative(fraction)
                        })
                        .bg(theme.running),
                ),
        );
    }

    element
}
