use crate::docker::model::ContainerState;
use crate::theme::{Theme, radius, type_scale};
use gpui::prelude::*;
use gpui::{Div, FontWeight, Rgba, SharedString, div, px};

/// A 22px status pill: soft background, colored dot and label.
pub fn pill(label: impl Into<SharedString>, fg: Rgba, soft: Rgba) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(22.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(radius::PILL))
        .bg(soft)
        .text_color(fg)
        .child(div().size(px(6.)).rounded_full().bg(fg))
        .child(
            div()
                .text_size(px(type_scale::META))
                .line_height(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
}

pub fn state_label(state: ContainerState) -> &'static str {
    match state {
        ContainerState::Running => "running",
        ContainerState::Paused => "paused",
        ContainerState::Restarting => "restarting",
        ContainerState::Exited => "exited",
        ContainerState::Created => "created",
        ContainerState::Dead => "dead",
        ContainerState::Unknown => "unknown",
    }
}

/// Foreground and soft background for one container state.
pub fn state_colors(state: ContainerState, theme: &Theme) -> (Rgba, Rgba) {
    match state {
        ContainerState::Running => (theme.running, theme.running_soft),
        ContainerState::Paused | ContainerState::Restarting => (theme.paused, theme.paused_soft),
        ContainerState::Dead => (theme.error, theme.error_soft),
        ContainerState::Exited | ContainerState::Created | ContainerState::Unknown => {
            (theme.exited, theme.exited_soft)
        }
    }
}

pub fn state_pill(state: ContainerState, theme: &Theme) -> Div {
    let (fg, soft) = state_colors(state, theme);
    pill(state_label(state), fg, soft)
}

/// The smaller detail-page variant: 19px high, 10px label.
pub fn pill_inline(label: impl Into<SharedString>, fg: Rgba, soft: Rgba) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(19.))
        .px(px(7.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(radius::PILL))
        .bg(soft)
        .text_color(fg)
        .child(div().size(px(5.)).rounded_full().bg(fg))
        .child(
            div()
                .text_size(px(10.))
                .line_height(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
}

pub fn state_pill_inline(state: ContainerState, theme: &Theme) -> Div {
    let (fg, soft) = state_colors(state, theme);
    pill_inline(state_label(state), fg, soft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_label() {
        for state in [
            ContainerState::Running,
            ContainerState::Paused,
            ContainerState::Restarting,
            ContainerState::Exited,
            ContainerState::Created,
            ContainerState::Dead,
            ContainerState::Unknown,
        ] {
            assert!(!state_label(state).is_empty());
        }
    }

    #[test]
    fn running_uses_green_and_dead_uses_error() {
        let theme = Theme::dark();
        let (running, running_soft) = state_colors(ContainerState::Running, &theme);
        assert_eq!(running, theme.running);
        assert_eq!(running_soft, theme.running_soft);

        let (dead, dead_soft) = state_colors(ContainerState::Dead, &theme);
        assert_eq!(dead, theme.error);
        assert_eq!(dead_soft, theme.error_soft);
    }

    #[test]
    fn paused_and_restarting_share_amber() {
        let theme = Theme::dark();
        assert_eq!(
            state_colors(ContainerState::Paused, &theme),
            state_colors(ContainerState::Restarting, &theme)
        );
        assert_eq!(state_colors(ContainerState::Paused, &theme).0, theme.paused);
    }
}
