use super::*;
use crate::docker::actions::{ActionKind, container_actions};
use crate::docker::model::{ContainerState, ResourceKind, format_bytes};
use crate::theme::Theme;
use crate::ui::detail::{DetailTab, render_logs};
use crate::ui::inspect::InspectNode;
use crate::ui::mutations::RowMenuState;
use crate::ui::style::Layout;
use crate::ui::table::{CellStyle, ColumnWidth};
use crate::ui::usage::DetailUsage;
use crate::ui::widgets::button::{ButtonSpec, button};
use crate::ui::widgets::pill::{state_pill, state_pill_inline};
use crate::ui::widgets::stat_card::{StatCard, stat_card};
use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, MouseButton, Render, SharedString, Window,
    div, px, svg, uniform_list,
};
use std::collections::BTreeMap;

impl RootView {
    fn render_action_buttons(
        &mut self,
        kind: ResourceKind,
        id: &str,
        title: &str,
        state: Option<ContainerState>,
        emphasized: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = self.theme.clone();
        let pending = self.mutations.pending.get(&(kind, id.to_string())).copied();
        let actions: Vec<ActionKind> = if kind == ResourceKind::Container {
            vec![
                ActionKind::Start,
                ActionKind::Stop,
                ActionKind::Restart,
                ActionKind::Remove,
            ]
        } else {
            vec![ActionKind::Remove]
        };

        let mut cluster = div().flex().items_center().gap(px(2.));
        if kind == ResourceKind::Image {
            let prefill = if title == "<none>" || title.ends_with('…') {
                id.to_string()
            } else {
                title.to_string()
            };
            let mut run_button = div()
                .id(SharedString::from(format!("run-image-{id}")))
                .size(px(28.))
                .occlude()
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .opacity(if emphasized { 1.0 } else { 0.35 })
                .child(
                    svg()
                        .path("icons/run.svg")
                        .size(px(14.))
                        .text_color(theme.text),
                );
            if pending.is_none() {
                run_button = run_button
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.hover))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.open_run(Some(prefill.clone()), window, cx);
                        cx.notify();
                    }));
            }
            cluster = cluster.child(run_button);
        }
        for action in actions {
            let enabled = action_enabled(state, kind, action) && pending.is_none();
            let color = if enabled { theme.text } else { theme.text_dim };
            let id_owned = id.to_string();
            let title_owned = title.to_string();
            let mut button = div()
                .id(SharedString::from(format!(
                    "act-{kind:?}-{id}-{}",
                    action.verb()
                )))
                .size(px(28.))
                .occlude()
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .opacity(if emphasized { 1.0 } else { 0.35 })
                .child(svg().path(icon_for(action)).size(px(14.)).text_color(color));
            if enabled {
                let hover = if action == ActionKind::Remove {
                    theme.error_soft
                } else {
                    theme.hover
                };
                button = button
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.request_action(kind, &id_owned, &title_owned, action);
                        cx.notify();
                    }));
            }
            cluster = cluster.child(button);
        }
        cluster
    }

    fn render_row_menu(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        let menu = self.mutations.row_menu.clone()?;
        let theme = self.theme.clone();
        let page = match menu.kind {
            ResourceKind::Container => Page::Containers,
            ResourceKind::Image => Page::Images,
            ResourceKind::Volume => Page::Volumes,
        };
        let title = self.tables[page.index()]
            .rows
            .iter()
            .find(|row| row.id == menu.id)
            .map(|row| row.cells[0].to_string())
            .unwrap_or_else(|| menu.id.clone());
        let state = if menu.kind == ResourceKind::Container {
            self.snapshot
                .containers
                .iter()
                .find(|container| container.id == menu.id)
                .map(|container| container.state)
        } else {
            None
        };

        let viewport = window.viewport_size();
        let (menu_x, menu_y) = crate::ui::mutations::clamp_menu_position(
            f32::from(menu.at.x),
            f32::from(menu.at.y),
            216.0,
            320.0,
            f32::from(viewport.width),
            f32::from(viewport.height),
        );

        let backdrop = div().absolute().inset_0().occlude().on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _event, _window, cx| {
                this.mutations.row_menu = None;
                cx.notify();
            }),
        );

        let menu_kind = menu.kind;
        let menu_id = menu.id.clone();

        let mut panel = div()
            .absolute()
            .left(px(menu_x))
            .top(px(menu_y))
            .w(px(216.))
            .p(px(6.))
            .occlude()
            .flex()
            .flex_col()
            .gap(px(1.))
            .rounded(px(14.))
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border_subtle)
            .shadow_lg();

        panel = panel.child(row_menu_item(
            "Open",
            theme.text,
            &theme,
            cx,
            move |this, _cx| {
                this.open_detail_by_id(menu_kind, &menu_id);
                this.mutations.row_menu = None;
            },
        ));

        if menu.kind == ResourceKind::Container {
            for action in [ActionKind::Start, ActionKind::Stop, ActionKind::Restart] {
                if action_enabled(state, menu.kind, action) {
                    let id = menu.id.clone();
                    let title = title.clone();
                    panel = panel.child(row_menu_item(
                        action.verb(),
                        theme.text,
                        &theme,
                        cx,
                        move |this, _cx| {
                            this.request_action(ResourceKind::Container, &id, &title, action);
                            this.mutations.row_menu = None;
                        },
                    ));
                }
            }
            panel = panel.child(crate::ui::dialog::menu_separator(&theme));
        }

        let remove_id = menu.id.clone();
        let remove_title = title.clone();
        let remove_kind = menu.kind;
        let remove_pending = self
            .mutations
            .pending
            .contains_key(&(menu.kind, menu.id.clone()));
        if !remove_pending {
            panel = panel.child(row_menu_item(
                "Remove",
                theme.error,
                &theme,
                cx,
                move |this, _cx| {
                    this.request_action(remove_kind, &remove_id, &remove_title, ActionKind::Remove);
                    this.mutations.row_menu = None;
                },
            ));
        }

        Some(div().absolute().inset_0().child(backdrop).child(panel))
    }

    fn render_toast(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let toast = self.mutations.toast.clone()?;
        let theme = self.theme.clone();
        let text_color = if toast.error { theme.error } else { theme.text };
        let (icon, icon_color, icon_bg) = if toast.error {
            ("icons/alert.svg", theme.error, theme.error_soft)
        } else {
            ("icons/check.svg", theme.running, theme.running_soft)
        };
        Some(
            div()
                .absolute()
                .bottom(px(24.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w(px(440.))
                        .p(px(12.))
                        .occlude()
                        .rounded(px(14.))
                        .bg(theme.panel_raised)
                        .border_1()
                        .border_color(theme.border_subtle)
                        .shadow_lg()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .size(px(28.))
                                .rounded(px(8.))
                                .flex_shrink_0()
                                .bg(icon_bg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(svg().path(icon).size(px(15.)).text_color(icon_color)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(12.))
                                .line_height(px(17.))
                                .text_color(text_color)
                                .child(toast.text),
                        )
                        .child(
                            div()
                                .id("toast-close")
                                .size(px(24.))
                                .flex_shrink_0()
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|style| style.bg(theme.hover))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.mutations.toast = None;
                                    cx.notify();
                                }))
                                .child(
                                    svg()
                                        .path("icons/close.svg")
                                        .size(px(13.))
                                        .text_color(theme.text_dim),
                                ),
                        ),
                ),
        )
    }

    fn retry_detail(&mut self) {
        if let Some(detail) = &mut self.detail {
            detail.error = None;
            detail.inspect = None;
            detail.logs = None;
        }
        self.request_detail_refresh();
    }

    fn select_tab(&mut self, tab: DetailTab) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        if detail.kind != ResourceKind::Container && tab == DetailTab::Logs {
            return;
        }
        detail.tab = tab;
        let needs_logs = tab == DetailTab::Logs
            && !detail.logs_requested
            && detail.kind == ResourceKind::Container;
        if needs_logs {
            detail.logs_requested = true;
            let id = detail.id.clone();
            if let Some(service) = &self.service {
                service.request(Request::Logs { id });
            }
        }
    }

    fn render_detail(&mut self, layout: &Layout, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let Some(detail) = self.detail.clone() else {
            return div();
        };
        let page = page_for_kind(detail.kind);

        let container = if detail.kind == ResourceKind::Container {
            self.snapshot
                .containers
                .iter()
                .find(|container| container.id == detail.id)
                .cloned()
        } else {
            None
        };
        let image = if detail.kind == ResourceKind::Image {
            self.snapshot
                .images
                .iter()
                .find(|image| image.id == detail.id)
                .cloned()
        } else {
            None
        };
        let volume = if detail.kind == ResourceKind::Volume {
            self.snapshot
                .volumes
                .iter()
                .find(|volume| volume.name.as_ref() == detail.id)
                .cloned()
        } else {
            None
        };
        let state = container.as_ref().map(|container| container.state);
        let detail_usage = if detail.kind == ResourceKind::Container {
            let connected = matches!(&self.status, HostStatus::Connected);
            Some(DetailUsage::from(
                self.usage.get(&detail.id),
                state.unwrap_or(ContainerState::Unknown),
                connected,
            ))
        } else {
            None
        };

        let mut meta = div().flex().items_center().gap(px(8.));
        if let Some(state) = state {
            meta = meta.child(state_pill_inline(state, &theme));
        }
        if let Some(container) = &container {
            meta = meta
                .child(meta_dot(&theme))
                .child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(11.))
                        .text_color(theme.text_dim)
                        .child(SharedString::from(
                            container.id.chars().take(12).collect::<String>(),
                        )),
                )
                .child(meta_dot(&theme))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_dim)
                        .child(container.image.clone()),
                )
                .child(meta_dot(&theme))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.text_faint)
                        .child(container.created_label.clone()),
                );
        } else if let Some(image) = &image {
            meta = meta
                .child(meta_dot(&theme))
                .child(
                    div()
                        .font_family("JetBrains Mono")
                        .text_size(px(11.))
                        .text_color(theme.text_dim)
                        .child(image.short_id.clone()),
                )
                .child(meta_dot(&theme))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_dim)
                        .child(image.size_label.clone()),
                )
                .child(meta_dot(&theme))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.text_faint)
                        .child(image.created_label.clone()),
                );
        } else if let Some(volume) = &volume {
            meta = meta
                .child(meta_dot(&theme))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text_dim)
                        .child(volume.driver.clone()),
                )
                .child(meta_dot(&theme))
                .child(
                    div()
                        .max_w(px(420.))
                        .truncate()
                        .font_family("JetBrains Mono")
                        .text_size(px(11.))
                        .text_color(theme.text_dim)
                        .child(volume.mountpoint.clone()),
                )
                .child(meta_dot(&theme))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme.text_faint)
                        .child(volume.created_label.clone()),
                );
        }

        let pending = self
            .mutations
            .pending
            .get(&(detail.kind, detail.id.clone()))
            .copied();
        let pending_label = pending.map(|action| action.action.pending_label());

        let mut hero_actions = div().flex().items_center().gap(px(4.)).flex_shrink_0();
        if detail.kind == ResourceKind::Container {
            for (action, label) in [
                (ActionKind::Start, "Start"),
                (ActionKind::Stop, "Stop"),
                (ActionKind::Restart, "Restart"),
            ] {
                let enabled = action_enabled(state, detail.kind, action) && pending.is_none();
                let id = detail.id.clone();
                let title = detail.title.to_string();
                hero_actions = hero_actions.child(button(
                    SharedString::from(format!("detail-action-{}", action.verb())),
                    ButtonSpec::ghost(label, Some(icon_for(action)), enabled),
                    &theme,
                    cx,
                    move |this, _window, _cx| {
                        this.request_action(detail.kind, &id, &title, action);
                    },
                ));
            }
        }
        let remove_enabled = pending.is_none();
        let id = detail.id.clone();
        let title = detail.title.to_string();
        let kind = detail.kind;
        hero_actions = hero_actions.child(button(
            "detail-remove",
            ButtonSpec::danger("Remove", Some("icons/trash.svg"), remove_enabled),
            &theme,
            cx,
            move |this, _window, _cx| {
                this.request_action(kind, &id, &title, ActionKind::Remove);
            },
        ));

        let mut action_column = div()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(4.))
            .child(hero_actions);
        if let Some(label) = pending_label {
            action_column = action_column.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.paused)
                    .child(label),
            );
        }

        let hero = crate::ui::style::card(&theme)
            .p(px(20.))
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .size(px(44.))
                    .rounded(px(12.))
                    .bg(theme.panel_raised)
                    .border_1()
                    .border_color(theme.border_subtle)
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .child(
                        svg()
                            .path(page_icon(page))
                            .size(px(22.))
                            .text_color(theme.accent),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(22.))
                            .line_height(px(26.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(detail.title.clone()),
                    )
                    .child(meta),
            )
            .child(action_column);

        let tabs: Vec<DetailTab> = if detail.kind == ResourceKind::Container {
            vec![DetailTab::Inspect, DetailTab::Logs]
        } else {
            vec![DetailTab::Inspect]
        };
        let mut tab_strip = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(3.))
            .rounded(px(10.))
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border_subtle);
        for tab in tabs {
            let selected = detail.tab == tab;
            let label = match tab {
                DetailTab::Inspect => "Inspect",
                DetailTab::Logs => "Logs",
            };
            tab_strip = tab_strip.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .px(px(12.))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .bg(if selected {
                        theme.panel_raised
                    } else {
                        theme.bg
                    })
                    .text_color(if selected { theme.text } else { theme.text_dim })
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.select_tab(tab);
                        cx.notify();
                    }))
                    .child(label),
            );
        }

        let body: gpui::AnyElement =
            if let Some(error) = detail.error.clone() {
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .px_8()
                    .text_center()
                    .child(
                        div()
                            .size(px(40.))
                            .rounded(px(10.))
                            .bg(theme.error_soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                svg()
                                    .path("icons/alert.svg")
                                    .size(px(20.))
                                    .text_color(theme.error),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Could not load this resource"),
                    )
                    .child(
                        div()
                            .max_w(px(420.))
                            .text_size(px(12.))
                            .text_color(theme.text_dim)
                            .child(error),
                    )
                    .child(button(
                        "detail-retry",
                        ButtonSpec::ghost("Retry", None, true),
                        &theme,
                        cx,
                        |this, _window, _cx| this.retry_detail(),
                    ))
                    .into_any_element()
            } else if detail.missing {
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .px_8()
                    .text_center()
                    .child(
                        div()
                            .size(px(40.))
                            .rounded(px(10.))
                            .bg(theme.panel_raised)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                svg()
                                    .path("icons/alert.svg")
                                    .size(px(20.))
                                    .text_color(theme.text_faint),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(format!("{} no longer exists", detail.title)),
                    )
                    .child(div().text_size(px(12.)).text_color(theme.text_dim).child(
                        "It may have been removed on the host. Go back to refresh the list.",
                    ))
                    .into_any_element()
            } else {
                match detail.tab {
                    DetailTab::Inspect => match &detail.inspect {
                        Some(nodes) => render_inspect_cards(nodes, &theme, &detail.inspect_scroll)
                            .into_any_element(),
                        None => loading_body("Loading inspect…", &theme),
                    },
                    DetailTab::Logs => match &detail.logs {
                        Some(lines) => {
                            let line_count = lines.len();
                            let stderr_count = lines.iter().filter(|line| line.stderr).count();
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .h(px(44.))
                                        .px(px(16.))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .border_b_1()
                                        .border_color(theme.border_subtle)
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(12.))
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(theme.text_dim)
                                                        .child(format!("{line_count} lines")),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(if stderr_count > 0 {
                                                            theme.paused
                                                        } else {
                                                            theme.text_faint
                                                        })
                                                        .child(format!("{stderr_count} stderr")),
                                                ),
                                        )
                                        .child(button(
                                            "logs-refresh",
                                            ButtonSpec::ghost(
                                                "Reload",
                                                Some("icons/refresh.svg"),
                                                true,
                                            ),
                                            &theme,
                                            cx,
                                            |this, _window, _cx| this.request_detail_refresh(),
                                        )),
                                )
                                .child(render_logs(lines, &theme))
                                .into_any_element()
                        }
                        None => loading_body("Loading logs…", &theme),
                    },
                }
            };

        let host_chip = self.render_host_chip(cx);
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(60.))
                    .px(px(layout.page_padding))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.border_subtle)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(crate::ui::widgets::button::icon_button(
                                "back",
                                "icons/back.svg",
                                28.0,
                                theme.text_dim,
                                false,
                                true,
                                true,
                                &theme,
                                cx,
                                |this, _window, _cx| this.close_detail(),
                            ))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text)
                                    .child(page.title()),
                            ),
                    )
                    .child(host_chip),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(if layout.compact { 12. } else { 14. }))
                    .px(px(layout.page_padding))
                    .pt(px(if layout.compact { 16. } else { 20. }))
                    .pb(px(layout.page_padding))
                    .child(hero)
                    .when_some(detail_usage, |this, usage| {
                        this.child(crate::ui::usage::render_usage_band(&usage, &theme, layout))
                    })
                    .child(tab_strip)
                    .child(
                        crate::ui::style::card(&theme)
                            .flex_1()
                            .min_h(px(0.))
                            .flex()
                            .flex_col()
                            .child(body),
                    ),
            )
    }

    pub(crate) fn start_pull(&mut self, reference: &str, origin: PullOrigin) -> bool {
        let reference = reference.trim();
        if reference.is_empty() || self.mutations.pull.is_some() {
            return false;
        }
        if self.service.is_none() {
            self.mutations.toast = Some(Toast {
                text: "No host connection".to_string(),
                error: true,
            });
            return false;
        }
        let epoch = self.mutations.epoch;
        self.mutations.pull = Some(PullState {
            origin,
            reference: reference.to_string(),
            status: "Starting…".to_string(),
            layers: BTreeMap::new(),
            epoch,
        });
        self.mutations.pull_error = None;
        self.mutations.pull_open = false;
        if let Some(service) = &self.service {
            service.request(Request::PullImage {
                reference: reference.to_string(),
                origin,
                epoch,
            });
        }
        true
    }

    fn render_pull_popover(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.page != Page::Images || !self.mutations.pull_open {
            return None;
        }
        let theme = self.theme.clone();
        let reference = self.mutations.pull_input.clone();
        let error = self.mutations.pull_error.clone();
        let pulling = self.mutations.pull.is_some();
        let focused = self.pull_focus.is_focused(window);
        Some(crate::ui::dialog::pull_popover(
            &reference,
            error.as_deref(),
            pulling,
            &self.pull_focus,
            focused,
            &theme,
            cx,
        ))
    }

    fn render_pull_strip(&mut self, layout: &Layout) -> Option<Div> {
        let pull = self.mutations.pull.clone()?;
        let theme = self.theme.clone();
        let bytes = pull.bytes();
        let fraction = pull.fraction();

        let mut strip = div()
            .h(px(36.))
            .px(px(layout.page_padding))
            .flex()
            .items_center()
            .gap(px(12.))
            .bg(theme.accent_soft)
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(12.))
            .child(div().text_color(theme.text_dim).child("Pulling"))
            .child(
                div()
                    .font_family("JetBrains Mono")
                    .text_color(theme.text)
                    .child(pull.reference.clone()),
            );
        if let Some((current, total)) = bytes {
            strip = strip.child(div().text_color(theme.text_dim).child(format!(
                "{} / {}",
                format_bytes(current as i64),
                format_bytes(total as i64)
            )));
        }
        strip = strip.child(
            div()
                .flex_1()
                .truncate()
                .text_color(theme.text_dim)
                .child(pull.status.clone()),
        );
        if let Some(fraction) = fraction {
            strip = strip.child(
                div()
                    .w(px(120.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(theme.border)
                    .child(
                        div()
                            .h_full()
                            .w(px(120. * fraction))
                            .rounded_full()
                            .bg(theme.accent),
                    ),
            );
        }
        Some(strip)
    }

    pub(crate) fn open_run(
        &mut self,
        prefill: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mutations.run.is_some() {
            return;
        }
        self.mutations.pull_open = false;
        self.host_menu_open = false;
        self.mutations.run = Some(RunState {
            form: RunForm {
                image: prefill.unwrap_or_default(),
                ..Default::default()
            },
            ..Default::default()
        });
        let focus = crate::ui::dialog::RunFocus::new(cx);
        window.focus(&focus.image);
        self.run_focus = Some(focus);
    }

    pub(crate) fn cancel_run(&mut self) {
        let Some(run) = &self.mutations.run else {
            return;
        };
        if run.submitting {
            return;
        }
        self.mutations.run = None;
        self.run_focus = None;
    }

    pub(crate) fn pull_run_image(&mut self) {
        if self.mutations.pull.is_some() {
            return;
        }
        let reference = match &self.mutations.run {
            Some(run) => run
                .submitted
                .as_ref()
                .map(|spec| spec.image.clone())
                .unwrap_or_else(|| run.form.image.trim().to_string()),
            None => return,
        };
        if reference.is_empty() {
            return;
        }
        if self.start_pull(&reference, PullOrigin::RunRetry)
            && let Some(run) = &mut self.mutations.run
        {
            run.pull_then_retry = true;
        }
    }

    pub(crate) fn submit_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let epoch = self.mutations.epoch;
        let Some(run) = self.mutations.run.as_mut() else {
            return;
        };
        if run.submitting || run.pull_then_retry {
            return;
        }
        match build_run_spec(&run.form) {
            Err(errors) => {
                let first_field = errors.first().map(|error| error.field);
                run.field_errors = errors;
                run.daemon_error = None;
                if let (Some(field), Some(focus)) = (first_field, self.run_focus.as_ref())
                    && let Some(handle) = focus.handle(field)
                {
                    window.focus(&handle);
                }
            }
            Ok(spec) => {
                run.field_errors.clear();
                run.submitted = Some(spec.clone());
                run.submitting = true;
                run.pull_then_retry = false;
                run.daemon_error = None;
                run.epoch = epoch;
                if let Some(service) = &self.service {
                    service.request(Request::RunContainer { spec, epoch });
                } else {
                    run.submitting = false;
                    run.daemon_error = Some("No host connection".to_string());
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn run_field_text_mut(&mut self, field: RunField) -> Option<&mut String> {
        let run = self.mutations.run.as_mut()?;
        match field {
            RunField::Image => Some(&mut run.form.image),
            RunField::Name => Some(&mut run.form.name),
            RunField::Command => Some(&mut run.form.command),
            RunField::PortHost(index) => run.form.ports.get_mut(index).map(|port| &mut port.host),
            RunField::PortContainer(index) => run
                .form
                .ports
                .get_mut(index)
                .map(|port| &mut port.container),
            RunField::EnvKey(index) => run
                .form
                .env
                .get_mut(index)
                .map(|variable| &mut variable.key),
            RunField::EnvValue(index) => run
                .form
                .env
                .get_mut(index)
                .map(|variable| &mut variable.value),
            RunField::MountSource(index) => run
                .form
                .mounts
                .get_mut(index)
                .map(|mount| &mut mount.source),
            RunField::MountTarget(index) => run
                .form
                .mounts
                .get_mut(index)
                .map(|mount| &mut mount.target),
        }
    }

    pub(crate) fn clear_run_field_error(&mut self, field: RunField) {
        if let Some(run) = &mut self.mutations.run {
            run.field_errors.retain(|error| error.field != field);
            run.daemon_error = None;
        }
    }

    pub(crate) fn add_run_port(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &mut self.mutations.run {
            if run.submitting {
                return;
            }
            run.form
                .ports
                .push(crate::docker::actions::PortForm::default());
        }
        if let Some(focus) = &mut self.run_focus {
            focus.add_port(cx);
        }
    }

    pub(crate) fn remove_run_port(&mut self, index: usize) {
        if let Some(run) = &mut self.mutations.run {
            if run.submitting || index >= run.form.ports.len() {
                return;
            }
            run.form.ports.remove(index);
        }
        if let Some(focus) = &mut self.run_focus {
            focus.remove_port(index);
        }
    }

    pub(crate) fn set_run_port_proto(&mut self, index: usize, proto: Proto) {
        if let Some(port) = self
            .mutations
            .run
            .as_mut()
            .and_then(|run| run.form.ports.get_mut(index))
        {
            port.proto = proto;
        }
    }

    pub(crate) fn add_run_env(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &mut self.mutations.run {
            if run.submitting {
                return;
            }
            run.form
                .env
                .push(crate::docker::actions::EnvForm::default());
        }
        if let Some(focus) = &mut self.run_focus {
            focus.add_env(cx);
        }
    }

    pub(crate) fn remove_run_env(&mut self, index: usize) {
        if let Some(run) = &mut self.mutations.run {
            if run.submitting || index >= run.form.env.len() {
                return;
            }
            run.form.env.remove(index);
        }
        if let Some(focus) = &mut self.run_focus {
            focus.remove_env(index);
        }
    }

    pub(crate) fn add_run_mount(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &mut self.mutations.run {
            if run.submitting {
                return;
            }
            run.form
                .mounts
                .push(crate::docker::actions::MountForm::default());
        }
        if let Some(focus) = &mut self.run_focus {
            focus.add_mount(cx);
        }
    }

    pub(crate) fn remove_run_mount(&mut self, index: usize) {
        if let Some(run) = &mut self.mutations.run {
            if run.submitting || index >= run.form.mounts.len() {
                return;
            }
            run.form.mounts.remove(index);
        }
        if let Some(focus) = &mut self.run_focus {
            focus.remove_mount(index);
        }
    }

    pub(crate) fn toggle_run_mount_read_only(&mut self, index: usize) {
        if let Some(mount) = self
            .mutations
            .run
            .as_mut()
            .and_then(|run| run.form.mounts.get_mut(index))
        {
            mount.read_only = !mount.read_only;
        }
    }

    fn render_run_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let run = self.mutations.run.clone()?;
        let focus = self.run_focus.clone()?;
        let theme = self.theme.clone();
        let pulling = self.mutations.pull.is_some();
        Some(crate::ui::dialog::run_dialog(
            &run, &focus, pulling, &theme, window, cx,
        ))
    }

    fn render_rail(&mut self, layout: &Layout, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let mut rail = div()
            .w(px(layout.rail_width))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(20.))
            .pb(px(16.))
            .bg(theme.rail)
            .border_r_1()
            .border_color(theme.border_subtle);

        rail = rail.child(
            div()
                .size(px(32.))
                .rounded(px(10.))
                .bg(theme.accent_soft)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path("icons/containers.svg")
                        .size(px(18.))
                        .text_color(theme.accent),
                ),
        );
        rail = rail.child(div().h(px(26.)));

        let mut nav = div().w_full().flex().flex_col().items_center().gap(px(8.));

        for page in [Page::Containers, Page::Images, Page::Volumes] {
            let selected = self.page == page;
            let fg = if selected {
                theme.accent
            } else {
                theme.text_dim
            };
            let label_color = if selected {
                theme.accent
            } else {
                theme.text_faint
            };
            let mut item = div()
                .id(("rail", page.index()))
                .relative()
                .w(px(60.))
                .h(px(54.))
                .rounded(px(10.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .bg(if selected { theme.selected } else { theme.rail })
                .cursor_pointer()
                .hover(|s| s.bg(theme.hover))
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.switch_page(page);
                    cx.notify();
                }));

            if selected {
                item = item.child(
                    div()
                        .absolute()
                        .left(px(-8.))
                        .top(px(10.))
                        .bottom(px(10.))
                        .w(px(3.))
                        .rounded_r(px(3.))
                        .bg(theme.accent),
                );
            }

            nav = nav.child(
                item.child(svg().path(page_icon(page)).size(px(20.)).text_color(fg))
                    .child(
                        div()
                            .text_size(px(9.))
                            .line_height(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(label_color)
                            .child(page.title().to_uppercase()),
                    ),
            );
        }
        rail = rail.child(nav);

        rail.child(div().flex_1()).child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .child(div().size(px(8.)).rounded_full().bg(self.status_color()))
                .child(
                    div()
                        .text_size(px(9.))
                        .line_height(px(11.))
                        .text_color(theme.text_faint)
                        .child("HOST"),
                ),
        )
    }

    fn render_host_chip(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let theme = self.theme.clone();
        let status_color = self.status_color();
        let host_name = self.active_host_name();
        div()
            .id("host-chip")
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(10.))
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border)
            .text_size(px(12.))
            .text_color(theme.text)
            .cursor_pointer()
            .hover(|s| s.bg(theme.hover))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.host_menu_open = !this.host_menu_open;
                this.mutations.pull_open = false;
                if this.host_menu_open {
                    if let Some(service) = &this.service {
                        service.request(Request::ProbeHosts);
                    }
                }
                cx.notify();
            }))
            .child(div().size(px(8.)).rounded_full().bg(status_color))
            .child(host_name)
            .child(
                svg()
                    .path("icons/chevron-down.svg")
                    .size(px(14.))
                    .text_color(theme.text_dim),
            )
    }

    fn render_header(&mut self, layout: &Layout, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let query = self.current_table().query.clone();

        let mut search = div()
            .id("search")
            .w(px(if layout.compact { 200. } else { 260. }))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(10.))
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border)
            .track_focus(&self.search_focus)
            .cursor_text()
            .on_click(cx.listener(|this, _event, window, _cx| {
                window.focus(&this.search_focus);
            }))
            .on_key_down(cx.listener(|this, event, _window, cx| this.on_search_key(event, cx)))
            .child(
                svg()
                    .path("icons/search.svg")
                    .size(px(14.))
                    .text_color(theme.text_faint),
            );
        search = search.child(if query.is_empty() {
            div()
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .text_color(theme.text_dim)
                .child("Search")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .text_color(theme.text)
                .child(query)
                .into_any_element()
        });
        search = search.child(
            div()
                .px(px(5.))
                .h(px(18.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(theme.border)
                .text_size(px(9.))
                .text_color(theme.text_faint)
                .child("⌘F"),
        );

        let pulling = self.mutations.pull.is_some();
        let run_open = self.mutations.run.is_some();

        let mut actions = div().flex().items_center().gap(px(8.));
        if self.page == Page::Containers {
            actions = actions.child(button(
                "run-button",
                ButtonSpec::primary("Run", Some("icons/run.svg"), !run_open),
                &theme,
                cx,
                |this, window, cx| {
                    this.open_run(None, window, cx);
                    cx.notify();
                },
            ));

            let has_compose = self
                .snapshot
                .containers
                .iter()
                .any(|container| container.compose_project.is_some());
            if has_compose {
                let on = self.group_containers;
                actions = actions.child(
                    div()
                        .id("group-toggle")
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(if on { theme.accent } else { theme.border })
                        .bg(if on { theme.accent_soft } else { theme.panel })
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if on { theme.text } else { theme.text_dim })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.hover))
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.group_containers = !this.group_containers;
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .path("icons/compose.svg")
                                .size(px(14.))
                                .text_color(if on { theme.accent } else { theme.text_faint }),
                        )
                        .child("Group"),
                );
            }
        }
        if self.page == Page::Images {
            actions = actions.child(button(
                "pull-button",
                ButtonSpec::primary(
                    if pulling { "Pulling…" } else { "Pull" },
                    Some("icons/pull.svg"),
                    !pulling,
                ),
                &theme,
                cx,
                |this, window, _cx| {
                    this.mutations.pull_open = !this.mutations.pull_open;
                    this.host_menu_open = false;
                    if this.mutations.pull_open {
                        window.focus(&this.pull_focus);
                    }
                },
            ));
        }
        actions = actions
            .child(search)
            .child(crate::ui::widgets::button::icon_button(
                "refresh",
                "icons/refresh.svg",
                32.0,
                theme.text_dim,
                false,
                true,
                true,
                &theme,
                cx,
                |this, _window, _cx| this.refresh(),
            ))
            .child(self.render_host_chip(cx));

        div()
            .h(px(layout.top_bar_height))
            .px(px(layout.page_padding))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme.border_subtle)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .line_height(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(self.page.title()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(px(14.))
                            .text_color(theme.text_dim)
                            .child(self.subtitle()),
                    ),
            )
            .child(actions)
    }

    fn render_overview(&mut self, layout: &Layout, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let stats = self.stats.clone();
        let loading = stats.is_loading();
        let page = self.page;
        let usage_totals = UsageTotals::from_snapshot(&self.snapshot, &self.usage);
        let usage_labels = UsageCardLabels::from(
            &usage_totals,
            self.snapshot.fetched_at.is_some(),
            matches!(&self.status, HostStatus::Connected),
        );

        let containers = stat_card(
            StatCard {
                id: "overview-containers",
                kicker: "Containers",
                icon: "icons/containers.svg",
                value: if loading {
                    "—".to_string()
                } else {
                    stats.containers_running.to_string()
                },
                unit: "running",
                right: if loading {
                    "of —".to_string()
                } else {
                    format!("of {}", stats.containers_total)
                },
                foot: if loading {
                    "connecting…".to_string()
                } else {
                    format!("{} stopped", stats.containers_stopped)
                },
                progress: if loading {
                    None
                } else {
                    Some(stats.containers_share())
                },
                active: page == Page::Containers,
            },
            layout,
            &theme,
            cx,
            |this, _window, _cx| this.switch_page(Page::Containers),
        );

        let usage = stat_card(
            StatCard {
                id: "overview-usage",
                kicker: "Usage",
                icon: "icons/activity.svg",
                value: usage_labels.value,
                unit: "CPU",
                right: usage_labels.right,
                foot: usage_labels.foot,
                progress: None,
                active: false,
            },
            layout,
            &theme,
            cx,
            |this, _window, _cx| this.switch_page(Page::Containers),
        );

        let images = stat_card(
            StatCard {
                id: "overview-images",
                kicker: "Images",
                icon: "icons/images.svg",
                value: if loading {
                    "—".to_string()
                } else {
                    stats.images_total.to_string()
                },
                unit: "images",
                right: if loading {
                    "— in use".to_string()
                } else {
                    format!("{} in use", stats.images_in_use)
                },
                foot: if loading {
                    "connecting…".to_string()
                } else {
                    format!("{} total", stats.images_size_label)
                },
                progress: None,
                active: page == Page::Images,
            },
            layout,
            &theme,
            cx,
            |this, _window, _cx| this.switch_page(Page::Images),
        );

        let volumes = stat_card(
            StatCard {
                id: "overview-volumes",
                kicker: "Volumes",
                icon: "icons/volumes.svg",
                value: if loading {
                    "—".to_string()
                } else {
                    stats.volumes_total.to_string()
                },
                unit: if stats.volumes_total == 1 {
                    "volume"
                } else {
                    "volumes"
                },
                right: if loading {
                    "—".to_string()
                } else {
                    stats.volume_driver_label()
                },
                foot: if loading {
                    "connecting…".to_string()
                } else {
                    stats.volume_foot()
                },
                progress: None,
                active: page == Page::Volumes,
            },
            layout,
            &theme,
            cx,
            |this, _window, _cx| this.switch_page(Page::Volumes),
        );

        div()
            .flex()
            .gap(px(12.))
            .child(containers)
            .child(usage)
            .child(images)
            .child(volumes)
    }

    fn render_host_menu(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.host_menu_open {
            return None;
        }
        let theme = self.theme.clone();
        let mut menu = div()
            .absolute()
            .top(px(64.))
            .right(px(16.))
            .w(px(380.))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .rounded(px(14.))
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border)
            .shadow_lg();

        if self.hosts.is_empty() {
            menu = menu.child(
                div()
                    .p(px(12.))
                    .text_size(px(13.))
                    .text_color(theme.text_dim)
                    .child("No Docker host found"),
            );
        }

        let items: Vec<(String, String, String, gpui::Rgba)> = self
            .hosts
            .iter()
            .map(|host| {
                let color = match &host.status {
                    HostStatus::Connected => theme.running,
                    HostStatus::Unknown => theme.text_dim,
                    _ => theme.error,
                };
                (
                    host.id.clone(),
                    host.name.clone(),
                    host.detail.clone(),
                    color,
                )
            })
            .collect();

        let active = self.active_host.clone();

        for (id, name, detail, color) in items {
            let is_active = active.as_deref() == Some(id.as_str());
            let bg = if is_active {
                theme.selected
            } else {
                theme.panel
            };
            let click_id = id.clone();
            let mut row = div()
                .id(SharedString::from(format!("host-{id}")))
                .h(px(52.))
                .px(px(12.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .bg(bg)
                .cursor_pointer()
                .hover(|s| s.bg(theme.hover))
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.select_host(&click_id, cx);
                    cx.notify();
                }))
                .child(div().size(px(8.)).rounded_full().bg(color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(name),
                        )
                        .child(
                            div()
                                .font_family("JetBrains Mono")
                                .text_size(px(11.))
                                .text_color(theme.text_dim)
                                .truncate()
                                .child(detail),
                        ),
                );
            if is_active {
                row = row.child(
                    svg()
                        .path("icons/check.svg")
                        .size(px(16.))
                        .text_color(theme.accent),
                );
            }
            menu = menu.child(row);
        }

        Some(menu)
    }

    fn render_table(&mut self, layout: &Layout, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let index = self.page.index();
        let table = &self.tables[index];
        let columns: Vec<&'static str> = table.spec.columns.clone();
        let widths = table.spec.widths.clone();
        let styles = table.spec.styles.clone();
        let active_sort = table.sort;
        let visible_count = table.visible().len();
        let selected = table.selected;
        let actions = table.spec.actions;
        let row_height = layout.row_height;
        let cell_padding = layout.cell_padding;
        let slots = self.container_slots();
        let list_len = slots.as_ref().map_or(visible_count, |slots| slots.len());

        let mut header = div()
            .h(px(layout.table_header_height))
            .px(px(layout.cell_padding))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme.border_subtle)
            .text_size(px(10.))
            .line_height(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.text_dim);

        for (column, title) in columns.iter().enumerate() {
            let width = widths[column];
            let sorted = active_sort.map(|(c, asc)| (c == column, asc));
            let mut cell = div()
                .id(("header", column))
                .h_full()
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .hover(|s| s.text_color(theme.text))
                .map_width(width)
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    let table = this.current_table_mut();
                    table.click_header(column);
                    cx.notify();
                }))
                .child(SharedString::from(title.to_uppercase()));
            if let Some((_, ascending)) = sorted {
                cell = cell.child(div().text_color(theme.accent).child(if ascending {
                    "↑"
                } else {
                    "↓"
                }));
            }
            header = header.child(cell);
        }

        if let Some(actions) = actions {
            header = header.child(div().w(px(actions.width())).flex_shrink_0());
        }

        let scroll_handle = self.tables[index].scroll.clone();
        let list = uniform_list(
            self.tables[index].spec.id,
            list_len,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let mut rows = Vec::with_capacity(range.len());
                let theme = this.theme.clone();
                for position in range {
                    if let Some(slots) = &slots {
                        match slots.get(position) {
                            Some(ContainerSlot::Group {
                                project,
                                count,
                                running,
                            }) => {
                                let collapsed = this.collapsed_projects.contains(project.as_ref());
                                rows.push(group_header(
                                    project.clone(),
                                    *count,
                                    *running,
                                    collapsed,
                                    row_height,
                                    cell_padding,
                                    &theme,
                                    cx,
                                ));
                                continue;
                            }
                            Some(ContainerSlot::Ungrouped { count, running }) => {
                                rows.push(ungrouped_header(
                                    *count,
                                    *running,
                                    this.ungrouped_collapsed,
                                    row_height,
                                    cell_padding,
                                    &theme,
                                    cx,
                                ));
                                continue;
                            }
                            Some(ContainerSlot::Row(_)) => {}
                            None => continue,
                        }
                    }
                    let row_index = match &slots {
                        Some(slots) => match slots.get(position) {
                            Some(ContainerSlot::Row(index)) => *index,
                            _ => continue,
                        },
                        None => match this.tables[index].visible().get(position).copied() {
                            Some(index) => index,
                            None => continue,
                        },
                    };
                    let Some((row_id, cells)) = this.tables[index]
                        .rows
                        .get(row_index)
                        .map(|row| (row.id.clone(), row.cells.clone()))
                    else {
                        continue;
                    };
                    let is_selected = selected == Some(row_index);
                    let kind = actions.map(|actions| actions.kind());
                    let pending = kind.and_then(|kind| {
                        this.mutations.pending.get(&(kind, row_id.clone())).copied()
                    });
                    let hovered_id = row_id.clone();
                    let click_id = row_id.clone();
                    let menu_kind = kind;
                    let menu_id = row_id.clone();

                    let mut element = div()
                        .id(SharedString::from(format!("row-{row_id}")))
                        .w_full()
                        .h(px(row_height))
                        .px(px(cell_padding))
                        .relative()
                        .flex()
                        .items_center()
                        .text_size(px(13.))
                        .cursor_pointer()
                        .border_b_1()
                        .border_color(theme.border_subtle)
                        .bg(if is_selected {
                            theme.selected
                        } else {
                            theme.panel
                        })
                        .when(!is_selected, |this| {
                            this.hover(|style| style.bg(theme.hover))
                        })
                        .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                            if *hovered {
                                this.mutations.hovered_row = Some(hovered_id.clone());
                            } else if this.mutations.hovered_row.as_deref()
                                == Some(hovered_id.as_str())
                            {
                                this.mutations.hovered_row = None;
                            }
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.current_table_mut().selected = Some(row_index);
                            this.open_detail(row_index);
                            cx.notify();
                        }))
                        .when_some(menu_kind, |this, kind| {
                            this.on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, _window, cx| {
                                        this.current_table_mut().selected = Some(row_index);
                                        this.mutations.row_menu = Some(RowMenuState {
                                            kind,
                                            id: menu_id.clone(),
                                            at: event.position,
                                        });
                                        cx.notify();
                                    },
                                ),
                            )
                        });

                    if is_selected {
                        element = element.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(3.))
                                .bg(theme.accent),
                        );
                    }

                    for (column, cell) in cells.iter().enumerate() {
                        let width = widths[column];
                        let style = styles.get(column).copied().unwrap_or(CellStyle::Text);
                        let pending_container = kind == Some(ResourceKind::Container)
                            && column == 1
                            && pending.is_some();

                        if style == CellStyle::Pill {
                            let state = if kind == Some(ResourceKind::Container) {
                                this.snapshot
                                    .containers
                                    .iter()
                                    .find(|container| container.id == row_id)
                                    .map(|container| container.state)
                            } else {
                                None
                            };
                            let pill = if pending_container {
                                crate::ui::widgets::pill::pill(
                                    pending
                                        .map(|action| action.action.pending_label())
                                        .unwrap_or_default(),
                                    theme.paused,
                                    theme.paused_soft,
                                )
                            } else if let Some(state) = state {
                                state_pill(state, &theme)
                            } else {
                                crate::ui::widgets::pill::pill(
                                    cell.clone(),
                                    theme.exited,
                                    theme.exited_soft,
                                )
                            };
                            element = element.child(
                                div()
                                    .map_width(width)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .child(pill),
                            );
                            continue;
                        }

                        let mut text = div().map_width(width).truncate().child(cell.clone());
                        match style {
                            CellStyle::Text => {
                                text = text
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .font_weight(if column == 0 {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(theme.text);
                            }
                            CellStyle::Mono => {
                                text = text
                                    .font_family("JetBrains Mono")
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .text_color(theme.text_dim);
                            }
                            CellStyle::Timestamp => {
                                text = text
                                    .text_size(px(11.))
                                    .line_height(px(14.))
                                    .text_color(theme.text_dim);
                            }
                            CellStyle::Pill => {}
                        }
                        element = element.child(text);
                    }

                    if let (Some(actions), Some(kind)) = (actions, kind) {
                        let state = if kind == ResourceKind::Container {
                            this.snapshot
                                .containers
                                .iter()
                                .find(|container| container.id == click_id)
                                .map(|container| container.state)
                        } else {
                            None
                        };
                        let emphasized = is_selected
                            || this.mutations.hovered_row.as_deref() == Some(click_id.as_str());
                        let title = cells
                            .first()
                            .map(|cell| cell.to_string())
                            .unwrap_or_default();
                        let cluster = this
                            .render_action_buttons(kind, &click_id, &title, state, emphasized, cx);
                        let cluster_hover_id = click_id.clone();
                        element = element.child(
                            div()
                                .id(SharedString::from(format!("row-actions-{click_id}")))
                                .w(px(actions.width()))
                                .h_full()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_end()
                                .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                                    if *hovered {
                                        this.mutations.hovered_row = Some(cluster_hover_id.clone());
                                    } else if this.mutations.hovered_row.as_deref()
                                        == Some(cluster_hover_id.as_str())
                                    {
                                        this.mutations.hovered_row = None;
                                    }
                                    cx.notify();
                                }))
                                .child(cluster),
                        );
                    }

                    rows.push(element);
                }
                rows
            }),
        )
        .track_scroll(scroll_handle)
        .h_full();

        let body: gpui::AnyElement = if list_len == 0 {
            let query = self.current_table().query.clone();
            let (title, detail) = crate::ui::empty::message_for(self.page, &self.status, &query)
                .unwrap_or_else(|| (self.page.empty_title().to_string(), String::new()));
            let icon = if query.is_empty() {
                match self.page {
                    Page::Containers => "icons/containers.svg",
                    Page::Images => "icons/images.svg",
                    Page::Volumes => "icons/volumes.svg",
                }
            } else {
                "icons/search.svg"
            };
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .px_8()
                .text_center()
                .child(
                    div()
                        .size(px(40.))
                        .rounded(px(10.))
                        .bg(theme.panel_raised)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(svg().path(icon).size(px(20.)).text_color(theme.text_faint)),
                )
                .child(
                    div()
                        .text_size(px(15.))
                        .line_height(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(title),
                )
                .child(
                    div()
                        .max_w(px(360.))
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(theme.text_dim)
                        .child(detail),
                )
                .into_any_element()
        } else {
            div().flex_1().min_h(px(0.)).child(list).into_any_element()
        };

        crate::ui::style::card(&theme)
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .child(body)
    }
}

trait MapWidth: Styled + Sized {
    fn map_width(self, width: ColumnWidth) -> Self {
        match width {
            ColumnWidth::Fixed(value) => self.w(px(value)).flex_shrink_0(),
            ColumnWidth::Flex(_weight) => self.flex_grow().min_w(px(0.)),
        }
    }
}

impl<E: Styled + Sized> MapWidth for E {}
fn icon_for(action: ActionKind) -> &'static str {
    match action {
        ActionKind::Start => "icons/play.svg",
        ActionKind::Stop => "icons/stop.svg",
        ActionKind::Restart => "icons/restart.svg",
        ActionKind::Remove => "icons/trash.svg",
    }
}

fn action_enabled(state: Option<ContainerState>, kind: ResourceKind, action: ActionKind) -> bool {
    if action == ActionKind::Remove {
        return true;
    }
    if kind != ResourceKind::Container {
        return false;
    }
    let Some(state) = state else {
        return false;
    };
    let available = container_actions(state);
    match action {
        ActionKind::Start => available.start,
        ActionKind::Stop => available.stop,
        ActionKind::Restart => available.restart,
        ActionKind::Remove => available.remove,
    }
}

/// Height of the blank band that separates one grouped section from the next.
/// It is part of the 44px header so `uniform_list` keeps a single row height.
const SECTION_GAP: f32 = 12.0;

fn group_header(
    project: SharedString,
    count: usize,
    running: usize,
    collapsed: bool,
    row_height: f32,
    cell_padding: f32,
    theme: &Theme,
    cx: &mut Context<RootView>,
) -> gpui::Stateful<Div> {
    let click_project = project.clone();
    div()
        .id(SharedString::from(format!("group-{project}")))
        .w_full()
        .h(px(row_height))
        .flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .h(px(SECTION_GAP))
                .flex_shrink_0()
                .bg(theme.panel),
        )
        .child(
            div()
                .id(SharedString::from(format!("group-row-{project}")))
                .flex_1()
                .min_h(px(0.))
                .px(px(cell_padding))
                .flex()
                .items_center()
                .gap(px(10.))
                .bg(theme.bg)
                .border_b_1()
                .border_color(theme.border_subtle)
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover))
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.toggle_project(click_project.as_ref());
                    cx.notify();
                }))
                .child(
                    svg()
                        .path(if collapsed {
                            "icons/chevron-right.svg"
                        } else {
                            "icons/chevron-down.svg"
                        })
                        .size(px(14.))
                        .text_color(theme.text_faint),
                )
                .child(
                    svg()
                        .path("icons/compose.svg")
                        .size(px(16.))
                        .text_color(theme.accent),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(project.clone()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(theme.text_faint)
                        .child(format!(
                            "{} container{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        )),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .size(px(6.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(if running > 0 {
                            theme.running
                        } else {
                            theme.exited
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(if running > 0 {
                            theme.text_dim
                        } else {
                            theme.text_faint
                        })
                        .child(format!("{running} running")),
                ),
        )
}

/// Section header for containers without a Compose project. Same 44px shape as
/// a project header, so it doubles as the spacer between the two sections.
fn ungrouped_header(
    count: usize,
    running: usize,
    collapsed: bool,
    row_height: f32,
    cell_padding: f32,
    theme: &Theme,
    cx: &mut Context<RootView>,
) -> gpui::Stateful<Div> {
    div()
        .id("ungrouped-header")
        .w_full()
        .h(px(row_height))
        .flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .h(px(SECTION_GAP))
                .flex_shrink_0()
                .bg(theme.panel),
        )
        .child(
            div()
                .id("ungrouped-header-row")
                .flex_1()
                .min_h(px(0.))
                .px(px(cell_padding))
                .flex()
                .items_center()
                .gap(px(10.))
                .bg(theme.bg)
                .border_b_1()
                .border_color(theme.border_subtle)
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover))
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.toggle_ungrouped();
                    cx.notify();
                }))
                .child(
                    svg()
                        .path(if collapsed {
                            "icons/chevron-right.svg"
                        } else {
                            "icons/chevron-down.svg"
                        })
                        .size(px(14.))
                        .text_color(theme.text_faint),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_dim)
                        .child("UNGROUPED"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(theme.text_faint)
                        .child(format!(
                            "{} container{}",
                            count,
                            if count == 1 { "" } else { "s" }
                        )),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .size(px(6.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(if running > 0 {
                            theme.running
                        } else {
                            theme.exited
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(if running > 0 {
                            theme.text_dim
                        } else {
                            theme.text_faint
                        })
                        .child(format!("{running} running")),
                ),
        )
}

fn row_menu_item(
    label: &'static str,
    color: gpui::Rgba,
    theme: &Theme,
    cx: &mut Context<RootView>,
    on_click: impl Fn(&mut RootView, &mut Context<RootView>) + 'static,
) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(format!("row-menu-{label}")))
        .h(px(32.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(8.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .cursor_pointer()
        .hover(|style| style.bg(theme.hover))
        .on_click(cx.listener(move |this, _event, _window, cx| {
            on_click(this, cx);
            cx.notify();
        }))
        .child(label)
}

fn meta_dot(theme: &Theme) -> Div {
    div()
        .size(px(3.))
        .flex_shrink_0()
        .rounded_full()
        .bg(theme.text_faint)
}

fn loading_body(label: &'static str, theme: &Theme) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(theme.text_dim)
        .child(label)
        .into_any_element()
}

fn page_for_kind(kind: ResourceKind) -> Page {
    match kind {
        ResourceKind::Container => Page::Containers,
        ResourceKind::Image => Page::Images,
        ResourceKind::Volume => Page::Volumes,
    }
}

fn page_icon(page: Page) -> &'static str {
    match page {
        Page::Containers => "icons/containers.svg",
        Page::Images => "icons/images.svg",
        Page::Volumes => "icons/volumes.svg",
    }
}

/// Inspect rendering stays mechanical: one Overview card for top-level
/// scalars, then one card per top-level group, all in `build_tree` order.
fn render_inspect_cards(
    nodes: &[InspectNode],
    theme: &Theme,
    scroll: &gpui::ScrollHandle,
) -> impl IntoElement {
    let mut container = div()
        .id("inspect-tree")
        .flex_1()
        .min_h(px(0.))
        .track_scroll(scroll)
        .overflow_hidden()
        .overflow_y_scroll()
        .p(px(20.))
        .flex()
        .flex_col()
        .gap(px(12.));

    let scalars: Vec<&InspectNode> = nodes.iter().filter(|node| node.value.is_some()).collect();
    if !scalars.is_empty() {
        container = container.child(inspect_overview_card(&scalars, theme));
    }
    for node in nodes.iter().filter(|node| node.value.is_none()) {
        container = container.child(inspect_card(
            node.key.as_ref(),
            &node.children.iter().collect::<Vec<_>>(),
            theme,
        ));
    }

    container
}

fn inspect_card_shell(title: &str, theme: &Theme) -> Div {
    div()
        .flex_shrink_0()
        .rounded(px(10.))
        .bg(theme.bg)
        .border_1()
        .border_color(theme.border_subtle)
        .overflow_hidden()
        .child(
            div()
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .truncate()
                .text_size(px(11.))
                .line_height(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_dim)
                .child(title.to_uppercase()),
        )
}

/// Top-level scalars as a two-column key/value grid, in `build_tree` order.
fn inspect_overview_card(entries: &[&InspectNode], theme: &Theme) -> Div {
    let mut card = inspect_card_shell("Overview", theme);

    for (index, pair) in entries.chunks(2).enumerate() {
        let mut row = div()
            .h(px(32.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(24.));
        if index > 0 {
            row = row.border_t_1().border_color(theme.border_subtle);
        }
        for entry in pair {
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(120.))
                            .flex_shrink_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.text_dim)
                            .child(entry.key.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_family("JetBrains Mono")
                            .text_size(px(12.))
                            .text_color(theme.text)
                            .child(entry.value.clone().unwrap_or_default()),
                    ),
            );
        }
        if pair.len() == 1 {
            row = row.child(div().flex_1());
        }
        card = card.child(row);
    }

    card
}

fn inspect_card(title: &str, entries: &[&InspectNode], theme: &Theme) -> Div {
    let mut card = inspect_card_shell(title, theme);

    for (index, entry) in entries.iter().enumerate() {
        let mut row = div()
            .h(px(32.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(12.));
        if index > 0 {
            row = row.border_t_1().border_color(theme.border_subtle);
        }
        card = card.child(
            row.child(
                div()
                    .w(px(200.))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(theme.text_dim)
                    .child(entry.key.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .font_family("JetBrains Mono")
                    .text_size(px(12.))
                    .text_color(if entry.value.is_some() {
                        theme.text
                    } else {
                        theme.text_dim
                    })
                    .child(entry.value.clone().unwrap_or_default()),
            ),
        );
    }

    card
}
impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.activation = Some(cx.observe_window_activation(window, |this, _window, cx| {
                this.refresh_if_stale();
                cx.notify();
            }));
        }
        if !self.focus_initialized {
            self.focus_initialized = true;
            window.focus(&self.root_focus);
        }

        let theme = self.theme.clone();
        let layout = Layout::for_viewport(window.viewport_size());
        let rail = self.render_rail(&layout, cx);
        let menu = self.render_host_menu(cx);
        let content: gpui::AnyElement = if self.detail.is_some() {
            self.render_detail(&layout, cx).into_any_element()
        } else {
            let header = self.render_header(&layout, cx);
            let overview = self.render_overview(&layout, cx);
            let table = self.render_table(&layout, cx);
            let mut column = div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .flex_col()
                .child(header);

            if let Some(strip) = self.render_pull_strip(&layout) {
                column = column.child(strip);
            }

            if let Some(error) = self.last_error.clone() {
                column = column.child(
                    div()
                        .h(px(36.))
                        .px(px(layout.page_padding))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .bg(theme.error_soft)
                        .border_b_1()
                        .border_color(theme.error)
                        .text_size(px(12.))
                        .text_color(theme.error)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    svg()
                                        .path("icons/alert.svg")
                                        .size(px(14.))
                                        .text_color(theme.error),
                                )
                                .child(div().truncate().child(error)),
                        )
                        .child(
                            div()
                                .id("retry")
                                .px(px(8.))
                                .h(px(24.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.error_soft))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.last_error = None;
                                    this.refresh();
                                    cx.notify();
                                }))
                                .child("Retry"),
                        ),
                );
            }

            column
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(layout.section_gap))
                        .px(px(layout.page_padding))
                        .pt(px(layout.page_padding))
                        .pb(px(layout.page_padding))
                        .child(overview)
                        .child(table),
                )
                .into_any_element()
        };

        let mut root = div()
            .size_full()
            .relative()
            .flex()
            .font_family("Inter")
            .bg(theme.bg)
            .text_color(theme.text)
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, event, window, cx| this.on_root_key(event, window, cx)))
            .on_action(
                cx.listener(|this, _: &crate::actions::ShowContainers, _window, cx| {
                    this.switch_page(Page::Containers);
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::ShowImages, _window, cx| {
                    this.switch_page(Page::Images);
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::ShowVolumes, _window, cx| {
                    this.switch_page(Page::Volumes);
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::Refresh, _window, cx| {
                    this.refresh();
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::FocusSearch, window, cx| {
                    window.focus(&this.search_focus);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this, _: &crate::actions::Back, _window, cx| {
                this.go_back();
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &crate::actions::RunContainer, window, cx| {
                    if this.mutations.confirm.is_some() || this.mutations.run.is_some() {
                        return;
                    }
                    this.open_run(None, window, cx);
                    cx.notify();
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::PullImage, window, cx| {
                    if this.mutations.confirm.is_some() || this.mutations.run.is_some() {
                        return;
                    }
                    this.close_detail();
                    this.switch_page(Page::Images);
                    this.mutations.pull_open = true;
                    this.host_menu_open = false;
                    window.focus(&this.pull_focus);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|_, _: &crate::actions::Quit, _window, cx| {
                cx.quit();
            }));

        root = root.child(rail).child(content);

        if let Some(popover) = self.render_pull_popover(window, cx) {
            root = root.child(popover);
        }
        if let Some(menu) = menu {
            root = root.child(menu);
        }
        if let Some(row_menu) = self.render_row_menu(window, cx) {
            root = root.child(row_menu);
        }
        if let Some(confirm) = self.mutations.confirm.clone() {
            root = root.child(crate::ui::dialog::confirm(&confirm, &theme, cx));
        }
        if let Some(dialog) = self.render_run_dialog(window, cx) {
            root = root.child(dialog);
        }
        if let Some(toast) = self.render_toast(cx) {
            root = root.child(toast);
        }

        root
    }
}
