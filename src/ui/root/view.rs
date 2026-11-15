use super::*;
use crate::docker::actions::{container_actions, ActionKind};
use crate::docker::model::{format_bytes, ContainerState, ResourceKind};
use crate::theme::Theme;
use crate::ui::detail::{render_logs, DetailTab};
use crate::ui::inspect::InspectNode;
use crate::ui::mutations::RowMenuState;
use crate::ui::table::{ActionsColumn, ColumnWidth};
use gpui::{
    AnyElement, Context, Div, IntoElement, MouseButton, Render, SharedString, Window, div, px, svg,
    uniform_list,
};

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
        let pending = self
            .mutations
            .pending
            .get(&(kind, id.to_string()))
            .copied();
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

        let mut cluster = div().flex().items_center().gap_1();
        if kind == ResourceKind::Image {
            let prefill = if title == "<none>" || title.ends_with('…') {
                id.to_string()
            } else {
                title.to_string()
            };
            let mut run_button = div()
                .id(SharedString::from(format!("run-image-{id}")))
                .size(px(24.))
                .occlude()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .opacity(if emphasized { 1.0 } else { 0.25 })
                .child(svg().path("icons/run.svg").size(px(14.)).text_color(theme.text));
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
                .size(px(24.))
                .occlude()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .opacity(if emphasized { 1.0 } else { 0.25 })
                .child(svg().path(icon_for(action)).size(px(14.)).text_color(color));
            if enabled {
                button = button
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.hover))
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
        let page = Page::for_kind(menu.kind);
        let title = self.tables[page.index()]
            .rows
            .iter()
            .find(|row| row.id == menu.id)
            .map(|row| row.cells[0].to_string())
            .unwrap_or_else(|| menu.id.clone());
        let state = if menu.kind == ResourceKind::Container {
            self.container_state(&menu.id)
        } else {
            None
        };

        let viewport = window.viewport_size();
        let (menu_x, menu_y) = crate::ui::mutations::clamp_menu_position(
            f32::from(menu.at.x),
            f32::from(menu.at.y),
            200.0,
            240.0,
            f32::from(viewport.width),
            f32::from(viewport.height),
        );

        let backdrop = div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
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
            .w(px(200.))
            .p_1()
            .occlude()
            .flex()
            .flex_col()
            .rounded_md()
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border)
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
                    this.request_action(
                        remove_kind,
                        &remove_id,
                        &remove_title,
                        ActionKind::Remove,
                    );
                    this.mutations.row_menu = None;
                },
            ));
        }

        Some(
            div()
                .absolute()
                .inset_0()
                .child(backdrop)
                .child(panel),
        )
    }

    fn render_toast(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let toast = self.mutations.toast.clone()?;
        let theme = self.theme.clone();
        let border = if toast.error { theme.error } else { theme.border };
        let text_color = if toast.error { theme.error } else { theme.text };
        Some(
            div()
                .absolute()
                .bottom(px(16.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w(px(420.))
                        .px_3()
                        .py_2()
                        .occlude()
                        .rounded_md()
                        .bg(theme.panel)
                        .border_1()
                        .border_color(border)
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(text_color)
                                .child(toast.text),
                        )
                        .child(
                            div()
                                .id("toast-close")
                                .cursor_pointer()
                                .text_color(theme.text_dim)
                                .hover(|style| style.text_color(theme.text))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.mutations.toast = None;
                                    cx.notify();
                                }))
                                .child("×"),
                        ),
                ),
        )
    }

    fn render_detail(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let Some(detail) = self.detail.clone() else {
            return div();
        };

        let tabs: Vec<DetailTab> = if detail.kind == ResourceKind::Container {
            vec![DetailTab::Inspect, DetailTab::Logs]
        } else {
            vec![DetailTab::Inspect]
        };

        let mut tab_strip = div().flex().items_center().gap_1().px_3().h(px(36.)).border_b_1().border_color(theme.border);
        for tab in tabs {
            let selected = detail.tab == tab;
            let label = match tab {
                DetailTab::Inspect => "Inspect",
                DetailTab::Logs => "Logs",
            };
            let color = if selected { theme.text } else { theme.text_dim };
            tab_strip = tab_strip.child(
                div()
                    .id(SharedString::from(format!("tab-{label}")))
                    .px_3()
                    .h_full()
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(color)
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.select_tab(tab);
                        cx.notify();
                    }))
                    .child(label),
            );
        }

        let status = if detail.missing {
            Some("no longer exists".to_string())
        } else {
            None
        };
        let pending_label = self
            .mutations
            .pending
            .get(&(detail.kind, detail.id.clone()))
            .map(|action| action.action.pending_label());

        let body: gpui::AnyElement = if let Some(error) = detail.error.clone() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .px_8()
                .text_center()
                .child(div().text_sm().text_color(theme.error).child(error))
                .child(
                    div()
                        .id("detail-retry")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme.text)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.hover))
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.retry_detail();
                            cx.notify();
                        }))
                        .child("Retry"),
                )
                .into_any_element()
        } else if detail.missing {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.text_dim)
                .child(format!("{} no longer exists", detail.title))
                .into_any_element()
        } else {
            match detail.tab {
                DetailTab::Inspect => match &detail.inspect {
                    Some(nodes) => render_inspect_tree(nodes, &theme).into_any_element(),
                    None => div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.text_dim)
                        .child("Loading inspect…")
                        .into_any_element(),
                },
                DetailTab::Logs => match &detail.logs {
                    Some(lines) => render_logs(lines, &theme).into_any_element(),
                    None => div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.text_dim)
                        .child("Loading logs…")
                        .into_any_element(),
                },
            }
        };

        let back_label = format!("‹ {}", self.page.title());
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(56.))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .id("back")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_sm()
                            .text_color(theme.text_dim)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.hover))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.close_detail();
                                cx.notify();
                            }))
                            .child(back_label),
                    )
                    .child(div().text_base().child(detail.title.clone()))
                    .child({
                        let state = if detail.kind == ResourceKind::Container {
                            self.container_state(&detail.id)
                        } else {
                            None
                        };
                        div()
                            .w(px(actions_width(detail.kind)))
                            .child(self.render_action_buttons(
                                detail.kind,
                                &detail.id,
                                detail.title.as_ref(),
                                state,
                                true,
                                cx,
                            ))
                    })
                    .when_some(pending_label, |this, label| {
                        this.child(div().text_xs().text_color(theme.paused).child(label))
                    })
                    .when(detail.tab == DetailTab::Logs, |this| {
                        this.child(
                            div()
                                .id("logs-refresh")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_xs()
                                .text_color(theme.text_dim)
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.hover))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.request_detail_refresh();
                                    cx.notify();
                                }))
                                .child("Reload"),
                        )
                    })
                    .when_some(status, |this, status| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.paused)
                                .child(status),
                        )
                    }),
            )
            .child(tab_strip)
            .child(body)
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

    fn render_pull_strip(&mut self) -> Option<Div> {
        let pull = self.mutations.pull.clone()?;
        let theme = self.theme.clone();
        let bytes = pull.bytes();
        let fraction = pull.fraction();

        let mut strip = div()
            .h(px(32.))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .bg(theme.panel)
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .child(div().text_color(theme.text_dim).child("Pulling"))
            .child(
                div()
                    .font_family("JetBrains Mono")
                    .text_color(theme.text)
                    .child(pull.reference.clone()),
            );
        if let Some((current, total)) = bytes {
            strip = strip.child(
                div()
                    .text_color(theme.text_dim)
                    .child(format!(
                        "{} / {}",
                        format_bytes(current as i64),
                        format_bytes(total as i64)
                    )),
            );
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
                    .w(px(160.))
                    .h(px(3.))
                    .rounded_full()
                    .bg(theme.border)
                    .child(
                        div()
                            .h_full()
                            .w(px(160. * fraction))
                            .rounded_full()
                            .bg(theme.accent),
                    ),
            );
        }
        Some(strip)
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

    fn render_rail(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let mut rail = div()
            .w(px(56.))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .py_3()
            .gap_2()
            .bg(theme.panel)
            .border_r_1()
            .border_color(theme.border);

        for page in [Page::Containers, Page::Images, Page::Volumes] {
            let selected = self.page == page;
            let bg = if selected { theme.selected } else { theme.panel };
            let fg = if selected { theme.text } else { theme.text_dim };
            rail = rail.child(
                div()
                    .id(("rail", page.index()))
                    .size(px(38.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(bg)
                    .text_color(fg)
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hover))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.switch_page(page);
                        cx.notify();
                    }))
                    .child(page.initial()),
            );
        }

        rail.child(div().flex_1()).child(
            div()
                .size(px(10.))
                .rounded_full()
                .bg(self.status_color()),
        )
    }

    fn render_header(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let query = self.current_table().query.clone();

        let search = div()
            .id("search")
            .w(px(240.))
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .bg(theme.bg)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .track_focus(&self.search_focus)
            .cursor_text()
            .on_click(cx.listener(|this, _event, window, _cx| {
                window.focus(&this.search_focus);
            }))
            .on_key_down(cx.listener(|this, event, _window, cx| this.on_search_key(event, cx)))
            .child(if query.is_empty() {
                div()
                    .text_color(theme.text_dim)
                    .child("Search  ⌘F")
                    .into_any_element()
            } else {
                div().text_color(theme.text).child(query).into_any_element()
            });

        let pulling = self.mutations.pull.is_some();
        let pull_color = if pulling { theme.text_dim } else { theme.text };
        let mut pull_button = div()
            .id("pull-button")
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .text_sm()
            .text_color(pull_color)
            .child(svg().path("icons/pull.svg").size(px(14.)).text_color(pull_color))
            .child(if pulling { "Pulling…" } else { "Pull" });
        if !pulling {
            pull_button = pull_button
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover))
                .on_click(cx.listener(|this, _event, window, cx| {
                    this.mutations.pull_open = !this.mutations.pull_open;
                    this.host_menu_open = false;
                    if this.mutations.pull_open {
                        window.focus(&this.pull_focus);
                    }
                    cx.notify();
                }));
        }

        let run_dialog_open = self.mutations.run.is_some();
        let run_color = if run_dialog_open { theme.text_dim } else { theme.text };
        let mut run_button = div()
            .id("run-button")
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .text_sm()
            .text_color(run_color)
            .child(svg().path("icons/run.svg").size(px(14.)).text_color(run_color))
            .child("Run");
        if !run_dialog_open {
            run_button = run_button
                .cursor_pointer()
                .hover(|style| style.bg(theme.hover))
                .on_click(cx.listener(|this, _event, window, cx| {
                    this.open_run(None, window, cx);
                    cx.notify();
                }));
        }

        div()
            .h(px(56.))
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_3()
                    .child(div().text_base().child(self.page.title()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_dim)
                            .child(self.counts()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(self.page == Page::Containers, |this| this.child(run_button))
                    .when(self.page == Page::Images, |this| this.child(pull_button))
                    .child(search)
                    .child(
                        div()
                            .id("refresh")
                            .size(px(28.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.text_dim)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.hover))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.refresh();
                                cx.notify();
                            }))
                            .child("⟳"),
                    )
                    .child(
                        div()
                            .id("host-chip")
                            .h(px(28.))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .bg(theme.bg)
                            .border_1()
                            .border_color(theme.border)
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.hover))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.host_menu_open = !this.host_menu_open;
                                this.mutations.pull_open = false;
                                if this.host_menu_open
                                    && let Some(service) = &this.service
                                {
                                    service.request(Request::ProbeHosts);
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(self.status_color()),
                            )
                            .child(self.active_host_name()),
                    ),
            )

    }

    fn render_host_menu(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.host_menu_open {
            return None;
        }
        let theme = self.theme.clone();
        let mut menu = div()
            .absolute()
            .top(px(60.))
            .right(px(16.))
            .w(px(360.))
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .rounded_md()
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border)
            .shadow_lg();

        if self.hosts.is_empty() {
            menu = menu.child(
                div()
                    .p_2()
                    .text_sm()
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
                (host.id.clone(), host.name.clone(), host.detail.clone(), color)
            })
            .collect();

        let active = self.active_host.clone();

        for (id, name, detail, color) in items {
            let is_active = active.as_deref() == Some(id.as_str());
            let bg = if is_active { theme.selected } else { theme.panel };
            let click_id = id.clone();
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("host-{id}")))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
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
                            .child(div().text_sm().child(name))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_dim)
                                    .truncate()
                                    .child(detail),
                            ),
                    ),
            );
        }

        Some(menu)
    }

    fn render_table(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme.clone();
        let index = self.page.index();
        let table = &self.tables[index];
        let columns: Vec<&'static str> = table.spec.columns.clone();
        let widths = table.spec.widths.clone();
        let active_sort = table.sort;
        let visible_count = table.visible().len();
        let selected = table.selected;
        let actions = table.spec.actions;

        let mut header = div()
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.text_dim);

        for (column, title) in columns.iter().enumerate() {
            let width = widths[column];
            let sorted = active_sort.map(|(c, asc)| (c == column, asc));
            header = header.child(
                div()
                    .id(("header", column))
                    .h_full()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme.text))
                    .map_width(width)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        let table = this.current_table_mut();
                        table.click_header(column);
                        cx.notify();
                    }))
                    .child(match sorted {
                        Some((true, true)) => format!("{title} ↑"),
                        Some((true, false)) => format!("{title} ↓"),
                        _ => title.to_string(),
                    }),
            );
        }

        if let Some(actions) = actions {
            header = header.child(div().w(px(actions.width())).flex_shrink_0());
        }

        let scroll_handle = self.tables[index].scroll.clone();
        let list = uniform_list(
            self.tables[index].spec.id,
            visible_count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let mut rows = Vec::with_capacity(range.len());
                let theme = this.theme.clone();
                for position in range {
                    let Some(row_index) = this.tables[index]
                        .visible()
                        .get(position)
                        .copied()
                    else {
                        continue;
                    };
                    let Some((row_id, cells, accent)) = this.tables[index]
                        .rows
                        .get(row_index)
                        .map(|row| (row.id.clone(), row.cells.clone(), row.accent))
                    else {
                        continue;
                    };
                    let is_selected = selected == Some(row_index);
                    let kind = actions.map(|actions| actions.kind());
                    let pending = kind.and_then(|kind| {
                        this.mutations
                            .pending
                            .get(&(kind, row_id.clone()))
                            .copied()
                    });
                    let hovered_id = row_id.clone();
                    let click_id = row_id.clone();
                    let menu_kind = kind;
                    let menu_id = row_id.clone();

                    let mut element = div()
                        .id(SharedString::from(format!("row-{row_id}")))
                        .h(px(ROW_HEIGHT))
                        .px_3()
                        .flex()
                        .items_center()
                        .text_sm()
                        .cursor_pointer()
                        .bg(if is_selected { theme.selected } else { theme.bg })
                        .hover(|style| style.bg(theme.hover))
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
                                cx.listener(move |this, event: &gpui::MouseDownEvent, _window, cx| {
                                    this.current_table_mut().selected = Some(row_index);
                                    this.mutations.row_menu = Some(RowMenuState {
                                        kind,
                                        id: menu_id.clone(),
                                        at: event.position,
                                    });
                                    cx.notify();
                                }),
                            )
                        });

                    for (column, cell) in cells.iter().enumerate() {
                        let width = widths[column];
                        let pending_container = kind == Some(ResourceKind::Container)
                            && column == 1
                            && pending.is_some();
                        let color = if pending_container {
                            theme.paused
                        } else if column == 1 {
                            accent.unwrap_or(theme.text)
                        } else {
                            theme.text
                        };
                        let text = if pending_container {
                            pending
                                .map(|action| action.action.pending_label())
                                .unwrap_or_default()
                        } else {
                            cell.as_ref()
                        };
                        element = element.child(
                            div()
                                .map_width(width)
                                .truncate()
                                .text_color(color)
                                .child(SharedString::from(text.to_string())),
                        );
                    }

                    if let (Some(actions), Some(kind)) = (actions, kind) {
                        let state = if kind == ResourceKind::Container {
                            this.container_state(&click_id)
                        } else {
                            None
                        };
                        let emphasized = is_selected
                            || this.mutations.hovered_row.as_deref()
                                == Some(click_id.as_str());
                        let title = cells
                            .first()
                            .map(|cell| cell.to_string())
                            .unwrap_or_default();
                        let cluster = this.render_action_buttons(
                            kind,
                            &click_id,
                            &title,
                            state,
                            emphasized,
                            cx,
                        );
                        let cluster_hover_id = click_id.clone();
                        element = element.child(
                            div()
                                .id(SharedString::from(format!(
                                    "row-actions-{click_id}"
                                )))
                                .w(px(actions.width()))
                                .flex_shrink_0()
                                .on_hover(cx.listener(
                                    move |this, hovered: &bool, _window, cx| {
                                        if *hovered {
                                            this.mutations.hovered_row =
                                                Some(cluster_hover_id.clone());
                                        } else if this.mutations.hovered_row.as_deref()
                                            == Some(cluster_hover_id.as_str())
                                        {
                                            this.mutations.hovered_row = None;
                                        }
                                        cx.notify();
                                    },
                                ))
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

        let body: gpui::AnyElement = if visible_count == 0 {
            let (title, detail) = crate::ui::empty::message_for(
                self.page,
                &self.status,
                &self.current_table().query,
            )
            .unwrap_or_else(|| (self.page.empty_title().to_string(), String::new()));
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .px_8()
                .text_center()
                .child(div().text_sm().text_color(theme.text).child(title))
                .child(div().text_xs().text_color(theme.text_dim).child(detail))
                .into_any_element()
        } else {
            div().flex_1().min_h(px(0.)).child(list).into_any_element()
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h(px(0.))
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

fn actions_width(kind: ResourceKind) -> f32 {
    ActionsColumn::for_kind(kind).width()
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
        .px_2()
        .py_1()
        .rounded_sm()
        .text_sm()
        .text_color(color)
        .cursor_pointer()
        .hover(|style| style.bg(theme.hover))
        .on_click(cx.listener(move |this, _event, _window, cx| {
            on_click(this, cx);
            cx.notify();
        }))
        .child(label)
}

fn render_inspect_tree(nodes: &[InspectNode], theme: &Theme) -> impl IntoElement {
    let mut container = div()
        .id("inspect-tree")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p_3()
        .flex()
        .flex_col()
        .gap_2();

    for node in nodes {
        let mut group = div().flex().flex_col().gap_1().child(
            div()
                .text_xs()
                .text_color(theme.text_dim)
                .child(node.key.clone()),
        );

        if let Some(value) = node.value.clone() {
            group = group.child(
                div()
                    .text_xs()
                    .font_family("JetBrains Mono")
                    .text_color(theme.text)
                    .child(value),
            );
        }

        for child in node.children.iter() {
            let mut row = div().pl_3().flex().gap_3();
            row = row.child(
                div()
                    .w(px(180.))
                    .flex_shrink_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.text_dim)
                    .child(child.key.clone()),
            );
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_xs()
                    .font_family("JetBrains Mono")
                    .text_color(theme.text)
                    .child(child.value.clone().unwrap_or_default()),
            );
            group = group.child(row);
        }

        container = container.child(group);
    }

    container
}


impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.activation =
                Some(cx.observe_window_activation(window, |this, _window, cx| {
                    this.refresh_if_stale();
                    cx.notify();
                }));
        }
        if !self.focus_initialized {
            self.focus_initialized = true;
            window.focus(&self.root_focus);
        }

        let theme = self.theme.clone();
        let rail = self.render_rail(cx);
        let menu = self.render_host_menu(cx);
        let content: gpui::AnyElement = if self.detail.is_some() {
            self.render_detail(cx).into_any_element()
        } else {
            let header = self.render_header(cx);
            let table = self.render_table(cx);
            let mut column = div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .flex_col()
                .child(header);

            if let Some(strip) = self.render_pull_strip() {
                column = column.child(strip);
            }

            if let Some(error) = self.last_error.clone() {
                column = column.child(
                    div()
                        .h(px(32.))
                        .px_4()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .bg(theme.panel)
                        .border_b_1()
                        .border_color(theme.error)
                        .text_xs()
                        .text_color(theme.error)
                        .child(div().truncate().child(error))
                        .child(
                            div()
                                .id("retry")
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.hover))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.last_error = None;
                                    this.refresh();
                                    cx.notify();
                                }))
                                .child("Retry"),
                        ),
                );
            }

            column.child(table).into_any_element()
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
            .on_action(cx.listener(|this, _: &crate::actions::ShowContainers, _window, cx| {
                this.switch_page(Page::Containers);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::ShowImages, _window, cx| {
                this.switch_page(Page::Images);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::ShowVolumes, _window, cx| {
                this.switch_page(Page::Volumes);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::Refresh, _window, cx| {
                this.refresh();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::FocusSearch, window, cx| {
                window.focus(&this.search_focus);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::Back, _window, cx| {
                this.go_back();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::RunContainer, window, cx| {
                if this.mutations.confirm.is_some() || this.mutations.run.is_some() {
                    return;
                }
                this.open_run(None, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &crate::actions::PullImage, window, cx| {
                if this.mutations.confirm.is_some() || this.mutations.run.is_some() {
                    return;
                }
                this.close_detail();
                this.switch_page(Page::Images);
                this.mutations.pull_open = true;
                this.host_menu_open = false;
                window.focus(&this.pull_focus);
                cx.notify();
            }))
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

