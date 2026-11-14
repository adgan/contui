use crate::docker::model::{LogLine, ResourceKind};
use crate::theme::Theme;
use gpui::prelude::*;
use gpui::{IntoElement, SharedString, div, px};
use gpui::UniformListScrollHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailTab {
    Inspect,
    Logs,
}

#[derive(Clone, Debug)]
pub struct DetailState {
    pub kind: ResourceKind,
    pub id: String,
    pub title: SharedString,
    pub tab: DetailTab,
    pub inspect: Option<Vec<crate::ui::inspect::InspectNode>>,
    pub logs: Option<Vec<LogLine>>,
    pub logs_requested: bool,
    pub missing: bool,
    pub scroll: UniformListScrollHandle,
}

impl DetailState {
    pub fn new(kind: ResourceKind, id: String, title: SharedString) -> Self {
        Self {
            kind,
            id,
            title,
            tab: DetailTab::Inspect,
            inspect: None,
            logs: None,
            logs_requested: false,
            missing: false,
            scroll: UniformListScrollHandle::new(),
        }
    }
}

/// Renders a log tail in the monospace stack. 500 lines is small enough that a
/// plain scroll container is cheaper than a virtualized list.
pub fn render_logs(lines: &[LogLine], theme: &Theme) -> impl IntoElement {
    let mut container = div()
        .id("logs-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p_3()
        .flex()
        .flex_col()
        .text_xs()
        .font_family("JetBrains Mono");

    if lines.is_empty() {
        return container.child(
            div()
                .text_color(theme.text_dim)
                .child("No log output in the last 500 lines"),
        );
    }

    for line in lines {
        container = container.child(
            div()
                .text_color(if line.stderr { theme.paused } else { theme.text })
                .child(format!("{}\n", line.text)),
        );
    }

    container
}
