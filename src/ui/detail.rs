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
    pub error: Option<String>,
    pub scroll: UniformListScrollHandle,
}

impl DetailState {
    /// Records an inspect/logs failure against this detail when the failure
    /// names this resource. Returns whether it was ours to own.
    pub fn apply_failed(&mut self, what: &str, message: &str) -> bool {
        if what.contains(&self.id) {
            self.error = Some(message.to_string());
            true
        } else {
            false
        }
    }

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
            error: None,
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
        .overflow_x_scroll()
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
        let color = if line.stderr { theme.paused } else { theme.text };
        let mut row = div().flex().gap_2().whitespace_nowrap();
        if let Some(timestamp) = &line.timestamp {
            row = row.child(div().text_color(theme.text_dim).child(timestamp.clone()));
        }
        container = container.child(row.child(div().text_color(color).child(line.text.clone())));
    }

    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_failure_is_recorded_for_the_open_resource_only() {
        let mut detail = DetailState::new(ResourceKind::Container, "abc".into(), "web".into());
        assert!(detail.apply_failed("inspect abc", "no such container"));
        assert_eq!(detail.error.as_deref(), Some("no such container"));

        let mut other = DetailState::new(ResourceKind::Container, "zzz".into(), "db".into());
        assert!(!other.apply_failed("inspect abc", "no such container"));
        assert!(other.error.is_none());
    }
}
