use crate::docker::model::{ContainerState, ContainerStats, format_cpu, format_memory};
use crate::theme::{Theme, space, type_scale};
use crate::ui::style::Layout;
use gpui::prelude::*;
use gpui::{Div, FontWeight, div, px, relative};

/// The detail band's text and bar. Kept separate from GPUI so every placeholder
/// state is unit-tested.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailUsage {
    pub cpu: String,
    pub memory: String,
    pub cpu_sub: String,
    pub memory_sub: String,
    pub memory_fraction: Option<f32>,
}

impl DetailUsage {
    pub fn from(stats: Option<&ContainerStats>, state: ContainerState, connected: bool) -> Self {
        if !connected {
            return Self::placeholder("host disconnected");
        }
        if state != ContainerState::Running {
            return Self::placeholder("container is not running");
        }
        let Some(stats) = stats else {
            return Self::placeholder("waiting for stats…");
        };

        let cpu = format_cpu(stats.cpu_percent).unwrap_or_else(|| "—".into());
        let cpu_sub = if stats.cpu_percent.is_some() {
            "of one core".into()
        } else {
            "waiting for stats…".into()
        };
        let memory = format_memory(stats.memory).unwrap_or_else(|| "—".into());
        let memory_sub = if stats.memory.is_some() {
            String::new()
        } else {
            "waiting for stats…".into()
        };
        let memory_fraction = stats
            .memory
            .and_then(|memory| memory.percent)
            .map(|percent| (percent / 100.0).clamp(0.0, 1.0));

        Self {
            cpu,
            memory,
            cpu_sub,
            memory_sub,
            memory_fraction,
        }
    }

    fn placeholder(sub: &str) -> Self {
        Self {
            cpu: "—".into(),
            memory: "—".into(),
            cpu_sub: sub.into(),
            memory_sub: sub.into(),
            memory_fraction: None,
        }
    }
}

pub fn render_usage_band(usage: &DetailUsage, theme: &Theme, layout: &Layout) -> Div {
    let height = if layout.compact { 76.0 } else { 88.0 };
    let padding = if layout.compact {
        space::LG
    } else {
        space::XXL
    };
    let value_size = if layout.compact {
        20.0
    } else {
        type_scale::HERO
    };

    crate::ui::style::card(theme)
        .h(px(height))
        .px(px(padding))
        .flex()
        .items_center()
        .child(metric_cell(
            "CPU",
            &usage.cpu,
            &usage.cpu_sub,
            None,
            value_size,
            theme,
        ))
        .child(
            div()
                .w(px(1.))
                .h(px(40.))
                .mx(px(padding))
                .bg(theme.border_subtle),
        )
        .child(metric_cell(
            "MEMORY",
            &usage.memory,
            &usage.memory_sub,
            usage.memory_fraction,
            value_size,
            theme,
        ))
}

fn metric_cell(
    kicker: &str,
    value: &str,
    sub: &str,
    fraction: Option<f32>,
    value_size: f32,
    theme: &Theme,
) -> Div {
    let bar = match fraction {
        Some(fraction) => div()
            .h(px(3.))
            .w_full()
            .rounded(px(2.))
            .bg(theme.border_subtle)
            .child(
                div()
                    .h_full()
                    .w(relative(fraction.clamp(0.0, 1.0)))
                    .rounded(px(2.))
                    .bg(theme.accent),
            ),
        None => div().h(px(3.)).w_full(),
    };

    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .truncate()
                .text_size(px(type_scale::KICKER))
                .line_height(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_dim)
                .child(kicker.to_uppercase()),
        )
        .child(
            div()
                .truncate()
                .text_size(px(value_size))
                .line_height(px(value_size + 4.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(value.to_string()),
        )
        .child(
            div()
                .truncate()
                .text_size(px(type_scale::META))
                .line_height(px(14.))
                .text_color(theme.text_dim)
                .child(sub.to_string()),
        )
        .child(bar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::{ContainerState, ContainerStats, MemoryUsage};

    fn usage(cpu: Option<f32>, memory: Option<u64>, limit: Option<u64>) -> ContainerStats {
        let limit = limit.unwrap_or(0);
        let percent = memory.and_then(|used| {
            (limit > 0).then(|| used as f32 / limit as f32 * 100.0)
        });
        ContainerStats {
            cpu_percent: cpu,
            memory: memory.map(|used| MemoryUsage {
                used,
                limit,
                percent,
            }),
            online_cpus: 1,
        }
    }

    #[test]
    fn running_container_shows_cpu_memory_and_fraction() {
        let value = usage(Some(0.42), Some(1_048_576), Some(2_097_152));
        let detail = DetailUsage::from(Some(&value), ContainerState::Running, true);
        assert_eq!(detail.cpu, "0.4%");
        assert_eq!(detail.cpu_sub, "of one core");
        assert_eq!(detail.memory, "1.0 MB / 2.0 MB");
        assert_eq!(detail.memory_fraction, Some(0.5));
    }

    #[test]
    fn stopped_container_has_no_usage_text() {
        let detail = DetailUsage::from(None, ContainerState::Exited, true);
        assert_eq!(detail.cpu, "—");
        assert_eq!(detail.memory, "—");
        assert_eq!(detail.cpu_sub, "container is not running");
        assert_eq!(detail.memory_sub, "container is not running");
        assert_eq!(detail.memory_fraction, None);
    }

    #[test]
    fn disconnected_host_wins_over_running_state() {
        let value = usage(Some(50.0), Some(1_048_576), None);
        let detail = DetailUsage::from(Some(&value), ContainerState::Running, false);
        assert_eq!(detail.cpu, "—");
        assert_eq!(detail.cpu_sub, "host disconnected");
    }

    #[test]
    fn partial_sample_shows_memory_without_cpu() {
        let value = usage(None, Some(1_048_576), None);
        let detail = DetailUsage::from(Some(&value), ContainerState::Running, true);
        assert_eq!(detail.cpu, "—");
        assert_eq!(detail.cpu_sub, "waiting for stats…");
        assert_eq!(detail.memory, "1.0 MB");
        assert_eq!(detail.memory_fraction, None);
    }

    #[test]
    fn memory_fraction_clamps_to_one() {
        let value = usage(Some(1.0), Some(3_000), Some(1_000));
        let detail = DetailUsage::from(Some(&value), ContainerState::Running, true);
        assert_eq!(detail.memory_fraction, Some(1.0));
    }
}
