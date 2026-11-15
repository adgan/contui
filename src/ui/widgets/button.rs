use crate::theme::{Theme, radius, type_scale};
use crate::ui::root::RootView;
use gpui::prelude::*;
use gpui::{Context, Div, ElementId, FontWeight, Rgba, Stateful, div, px, svg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Ghost,
    Danger,
}

pub struct ButtonSpec<'a> {
    pub label: &'a str,
    pub icon: Option<&'static str>,
    pub variant: ButtonVariant,
    pub enabled: bool,
    pub emphasized: bool,
}

impl<'a> ButtonSpec<'a> {
    pub fn primary(label: &'a str, icon: Option<&'static str>, enabled: bool) -> Self {
        Self {
            label,
            icon,
            variant: ButtonVariant::Primary,
            enabled,
            emphasized: true,
        }
    }

    pub fn ghost(label: &'a str, icon: Option<&'static str>, enabled: bool) -> Self {
        Self {
            label,
            icon,
            variant: ButtonVariant::Ghost,
            enabled,
            emphasized: true,
        }
    }

    pub fn danger(label: &'a str, icon: Option<&'static str>, enabled: bool) -> Self {
        Self {
            label,
            icon,
            variant: ButtonVariant::Danger,
            enabled,
            emphasized: true,
        }
    }
}

fn colors(variant: ButtonVariant, theme: &Theme) -> (Rgba, Option<Rgba>) {
    match variant {
        ButtonVariant::Primary => (theme.text, Some(theme.accent)),
        ButtonVariant::Ghost => (theme.text, Some(theme.panel_raised)),
        ButtonVariant::Danger => (theme.error, None),
    }
}

/// A labeled 32px button. Disabled buttons stay visible and keep their layout.
pub fn button(
    id: impl Into<ElementId>,
    spec: ButtonSpec<'_>,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_click: impl Fn(&mut RootView, &mut gpui::Window, &mut Context<RootView>) + 'static,
) -> Stateful<Div> {
    let (fg, fill) = colors(spec.variant, theme);
    let mut element = div()
        .id(id)
        .h(px(32.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(radius::MD))
        .text_size(px(type_scale::META + 1.0))
        .line_height(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .when_some(fill, |this, fill| this.bg(fill))
        .when(spec.variant == ButtonVariant::Ghost, |this| {
            this.border_1().border_color(theme.border)
        })
        .when(spec.variant == ButtonVariant::Danger, |this| {
            this.border_1().border_color(theme.error_soft)
        });

    if let Some(icon) = spec.icon {
        element = element.child(svg().path(icon).size(px(14.)).text_color(fg));
    }
    element = element.child(gpui::SharedString::from(spec.label.to_string()));

    if spec.enabled {
        let hover = match spec.variant {
            ButtonVariant::Primary => theme.accent_soft,
            ButtonVariant::Ghost => theme.hover,
            ButtonVariant::Danger => theme.error_soft,
        };
        let hover_fg = match spec.variant {
            ButtonVariant::Primary => theme.text,
            _ => fg,
        };
        element = element
            .cursor_pointer()
            .hover(move |style| style.bg(hover).text_color(hover_fg))
            .on_click(cx.listener(move |this, _event, window, cx| {
                on_click(this, window, cx);
                cx.notify();
            }));
    } else {
        element = element.opacity(0.4);
    }

    element
}

/// A 28px ghost icon button used by rows, the top bar and the detail chrome.
/// At rest row buttons sit at 35% opacity; the row's hover state raises
/// `emphasized` to true, exactly like the old hand-rolled clusters.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    size: f32,
    tooltip_color: Rgba,
    danger: bool,
    enabled: bool,
    emphasized: bool,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_click: impl Fn(&mut RootView, &mut gpui::Window, &mut Context<RootView>) + 'static,
) -> Stateful<Div> {
    let fg = if danger { theme.error } else { tooltip_color };
    let mut element = div()
        .id(id)
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius::SM))
        .text_color(fg)
        .child(svg().path(icon).size(px(14.)).text_color(fg));

    if enabled {
        let hover = if danger {
            theme.error_soft
        } else {
            theme.hover
        };
        element = element
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(move |this, _event, window, cx| {
                on_click(this, window, cx);
                cx.notify();
            }));
    } else {
        element = element.opacity(0.3);
    }

    if enabled && !emphasized {
        element = element.opacity(0.35);
    }

    element
}
