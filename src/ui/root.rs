use crate::docker::endpoint::{discover, HostInfo, HostStatus};
use crate::docker::model::{ContainerState, Snapshot};
use crate::docker::service::{DockerService, Request, Response};
use crate::theme::Theme;
use crate::docker::model::ResourceKind;
use crate::ui::detail::{render_logs, DetailState, DetailTab};
use crate::ui::inspect::{build_tree, InspectNode};
use crate::ui::table::{ColumnWidth, SortValue, TableRow, TableSpec, TableState};
use chrono::Utc;
use gpui::prelude::*;
use gpui::{
    Context, Div, FocusHandle, KeyDownEvent, Render, SharedString, Subscription, Task, Window, div,
    px, uniform_list,
};
use std::collections::HashMap;
use std::path::PathBuf;

const STALE_AFTER_SECS: i64 = 5;
const ROW_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Containers,
    Images,
    Volumes,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Containers => "Containers",
            Page::Images => "Images",
            Page::Volumes => "Volumes",
        }
    }

    pub fn initial(self) -> &'static str {
        match self {
            Page::Containers => "C",
            Page::Images => "I",
            Page::Volumes => "V",
        }
    }

    fn index(self) -> usize {
        match self {
            Page::Containers => 0,
            Page::Images => 1,
            Page::Volumes => 2,
        }
    }

    pub fn empty_title(self) -> &'static str {
        match self {
            Page::Containers => "No containers yet",
            Page::Images => "No images yet",
            Page::Volumes => "No volumes yet",
        }
    }
}

pub struct RootView {
    pub page: Page,
    pub theme: Theme,
    pub service: Option<DockerService>,
    pub snapshot: Snapshot,
    pub status: HostStatus,
    pub hosts: Vec<HostInfo>,
    pub active_host: Option<String>,
    pub tables: [TableState; 3],
    pub last_error: Option<String>,
    pub host_menu_open: bool,
    pub detail: Option<DetailState>,
    search_focus: FocusHandle,
    root_focus: FocusHandle,
    focus_initialized: bool,
    activation: Option<Subscription>,
    _pump: Option<Task<()>>,
}

impl RootView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let env: HashMap<String, String> = std::env::vars().collect();
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let discovery = discover(&env, &home);

        let mut view = Self {
            page: Page::Containers,
            theme: Theme::dark(),
            service: None,
            snapshot: Snapshot::empty(),
            status: HostStatus::Unknown,
            hosts: discovery.hosts,
            active_host: discovery.active,
            tables: [
                Self::containers_table(&Snapshot::empty()),
                Self::images_table(&Snapshot::empty()),
                Self::volumes_table(&Snapshot::empty()),
            ],
            last_error: None,
            host_menu_open: false,
            detail: None,
            search_focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            focus_initialized: false,
            activation: None,
            _pump: None,
        };

        if let Some(id) = view.active_host.clone() {
            if let Some(host) = view.hosts.iter().find(|h| h.id == id).cloned() {
                view.connect(host, cx);
            }
        }

        view
    }

    pub fn containers_table(snapshot: &Snapshot) -> TableState {
        let spec = TableSpec {
            id: "containers-table",
            columns: vec!["Name", "State", "Image", "Ports", "Created"],
            widths: vec![
                ColumnWidth::Flex(2.0),
                ColumnWidth::Fixed(96.0),
                ColumnWidth::Flex(2.0),
                ColumnWidth::Fixed(120.0),
                ColumnWidth::Fixed(96.0),
            ],
        };

        let mut sorted: Vec<&crate::docker::model::ContainerRow> =
            snapshot.containers.iter().collect();
        sorted.sort_by(|a, b| {
            let a_running = a.state == ContainerState::Running;
            let b_running = b.state == ContainerState::Running;
            b_running
                .cmp(&a_running)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        let rows = sorted
            .into_iter()
            .map(|c| TableRow {
                id: c.id.clone(),
                cells: vec![
                    c.name.clone(),
                    state_label(c.state).into(),
                    c.image.clone(),
                    c.ports.clone(),
                    c.created_label.clone(),
                ],
                sort_values: vec![
                    SortValue::Text(c.name.to_string()),
                    SortValue::Text(state_label(c.state).to_string()),
                    SortValue::Text(c.image.to_string()),
                    SortValue::Text(c.ports.to_string()),
                    SortValue::Time(c.created),
                ],
                accent: Some(state_color(c.state)),
            })
            .collect();

        let mut table = TableState::new(spec);
        table.set_rows(rows);
        table
    }

    pub fn images_table(snapshot: &Snapshot) -> TableState {
        let spec = TableSpec {
            id: "images-table",
            columns: vec!["Repository", "ID", "Size", "Created", "In Use"],
            widths: vec![
                ColumnWidth::Flex(3.0),
                ColumnWidth::Fixed(110.0),
                ColumnWidth::Fixed(86.0),
                ColumnWidth::Fixed(96.0),
                ColumnWidth::Fixed(72.0),
            ],
        };

        let mut sorted: Vec<&crate::docker::model::ImageRow> = snapshot.images.iter().collect();
        sorted.sort_by(|a, b| b.created.cmp(&a.created));

        let rows = sorted
            .into_iter()
            .map(|image| TableRow {
                id: image.id.clone(),
                cells: vec![
                    image.repository_tag.clone(),
                    image.short_id.clone(),
                    image.size_label.clone(),
                    image.created_label.clone(),
                    if image.in_use { "yes" } else { "—" }.into(),
                ],
                sort_values: vec![
                    SortValue::Text(image.repository_tag.to_string()),
                    SortValue::Text(image.short_id.to_string()),
                    SortValue::Number(image.size),
                    SortValue::Time(image.created),
                    SortValue::Text(if image.in_use { "yes" } else { "no" }.to_string()),
                ],
                accent: None,
            })
            .collect();

        let mut table = TableState::new(spec);
        table.set_rows(rows);
        table
    }

    pub fn volumes_table(snapshot: &Snapshot) -> TableState {
        let spec = TableSpec {
            id: "volumes-table",
            columns: vec!["Name", "Driver", "Mountpoint", "Created"],
            widths: vec![
                ColumnWidth::Flex(2.0),
                ColumnWidth::Fixed(86.0),
                ColumnWidth::Flex(3.0),
                ColumnWidth::Fixed(96.0),
            ],
        };

        let mut sorted: Vec<&crate::docker::model::VolumeRow> = snapshot.volumes.iter().collect();
        sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let rows = sorted
            .into_iter()
            .map(|volume| TableRow {
                id: volume.name.to_string(),
                cells: vec![
                    volume.name.clone(),
                    volume.driver.clone(),
                    volume.mountpoint.clone(),
                    volume.created_label.clone(),
                ],
                sort_values: vec![
                    SortValue::Text(volume.name.to_string()),
                    SortValue::Text(volume.driver.to_string()),
                    SortValue::Text(volume.mountpoint.to_string()),
                    SortValue::Text(volume.created_label.to_string()),
                ],
                accent: None,
            })
            .collect();

        let mut table = TableState::new(spec);
        table.set_rows(rows);
        table
    }

    fn connect(&mut self, host: HostInfo, cx: &mut Context<Self>) {
        self.active_host = Some(host.id.clone());
        self.status = HostStatus::Unknown;
        match DockerService::connect(host) {
            Ok((service, responses)) => {
                self.service = Some(service);
                self.last_error = None;
                let entity = cx.entity();
                let pump = cx.spawn(async move |_weak, cx| {
                    loop {
                        let Ok(response) = responses.recv().await else {
                            break;
                        };
                        let updated = entity.update(cx, |view, cx| {
                            view.apply(response);
                            cx.notify();
                        });
                        if updated.is_err() {
                            break;
                        }
                    }
                });
                self._pump = Some(pump);
                if let Some(service) = &self.service {
                    service.request(Request::FetchAll);
                }
            }
            Err(err) => {
                self.last_error = Some(err.clone());
                self.status = HostStatus::Unreachable(err);
            }
        }
    }

    fn apply(&mut self, response: Response) {
        match response {
            Response::Snapshot(snapshot) => {
                self.snapshot = snapshot;
                self.rebuild_tables();
                self.last_error = None;
                if let Some(detail) = &mut self.detail {
                    let present = match detail.kind {
                        ResourceKind::Container => {
                            self.snapshot.containers.iter().any(|c| c.id == detail.id)
                        }
                        ResourceKind::Image => {
                            self.snapshot.images.iter().any(|i| i.id == detail.id)
                        }
                        ResourceKind::Volume => self
                            .snapshot
                            .volumes
                            .iter()
                            .any(|v| v.name.as_ref() == detail.id),
                    };
                    if !present {
                        detail.missing = true;
                    }
                }
            }
            Response::Status(status) => self.status = status,
            Response::HostStatuses(statuses) => {
                for (id, status) in statuses {
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == id) {
                        host.status = status;
                    }
                }
            }
            Response::Dirty { kinds, ids } => {
                let _ = kinds;
                if let Some(detail) = &mut self.detail {
                    let touched = ids
                        .iter()
                        .any(|(kind, id)| *kind == detail.kind && *id == detail.id);
                    if touched && !detail.missing {
                        detail.inspect = None;
                    }
                }
                if let Some(service) = &self.service {
                    service.request(Request::FetchAll);
                }
                if let Some(detail) = self.detail.clone() {
                    if !detail.missing {
                        if let Some(service) = &self.service {
                            service.request(Request::Inspect {
                                kind: detail.kind,
                                id: detail.id.clone(),
                            });
                        }
                    }
                }
            }
            Response::Failed { what, message } => {
                self.last_error = Some(format!("{what}: {message}"));
            }
            Response::Inspect { kind, id, json } => {
                if let Some(detail) = &mut self.detail {
                    if detail.kind == kind && detail.id == id {
                        detail.inspect = Some(build_tree(&json, 2));
                        detail.missing = false;
                    }
                }
            }
            Response::Logs { id, lines } => {
                if let Some(detail) = &mut self.detail {
                    if detail.id == id {
                        detail.logs = Some(lines);
                    }
                }
            }
        }
    }

    fn rebuild_tables(&mut self) {
        let selections: Vec<Option<String>> = self
            .tables
            .iter()
            .map(|t| t.selected_row().map(|row| row.id.clone()))
            .collect();

        self.tables = [
            Self::containers_table(&self.snapshot),
            Self::images_table(&self.snapshot),
            Self::volumes_table(&self.snapshot),
        ];

        for (table, selected_id) in self.tables.iter_mut().zip(selections) {
            if let Some(id) = selected_id {
                if let Some(ix) = table.rows.iter().position(|row| row.id == id) {
                    table.selected = Some(ix);
                }
            }
        }
    }

    pub fn switch_page(&mut self, page: Page) {
        self.page = page;
        self.host_menu_open = false;
    }

    pub fn select_host(&mut self, host_id: &str, cx: &mut Context<Self>) {
        if let Some(host) = self.hosts.iter().find(|h| h.id == host_id).cloned() {
            self.host_menu_open = false;
            self.tables_query_reset();
            self.connect(host, cx);
        }
    }

    fn tables_query_reset(&mut self) {
        for table in self.tables.iter_mut() {
            table.set_query("");
            table.selected = None;
        }
    }

    fn refresh(&mut self) {
        if let Some(service) = &self.service {
            service.request(Request::FetchAll);
        }
    }

    fn refresh_if_stale(&mut self) {
        let stale = self
            .snapshot
            .fetched_at
            .map(|fetched| (Utc::now() - fetched).num_seconds() > STALE_AFTER_SECS)
            .unwrap_or(true);
        if stale {
            self.refresh();
        }
    }

    fn current_table(&self) -> &TableState {
        &self.tables[self.page.index()]
    }

    fn current_table_mut(&mut self) -> &mut TableState {
        let index = self.page.index();
        &mut self.tables[index]
    }

    fn on_search_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "backspace" => {
                let mut query = self.current_table().query.clone();
                query.pop();
                let table = self.current_table_mut();
                let q = query.clone();
                table.set_query(&q);
                cx.notify();
            }
            "escape" => {
                let table = self.current_table_mut();
                table.set_query("");
                cx.notify();
            }
            _ => {
                let modifiers = event.keystroke.modifiers;
                if !modifiers.platform && !modifiers.control && key.chars().count() == 1 {
                    let mut query = self.current_table().query.clone();
                    query.push_str(key);
                    let table = self.current_table_mut();
                    let q = query.clone();
                    table.set_query(&q);
                    cx.notify();
                }
            }
        }
    }

    fn on_root_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform {
            match key {
                "1" => self.switch_page(Page::Containers),
                "2" => self.switch_page(Page::Images),
                "3" => self.switch_page(Page::Volumes),
                "r" => self.refresh(),
                "f" => window.focus(&self.search_focus),
                _ => return,
            }
            cx.notify();
            return;
        }
        match key {
            "up" => {
                let table = self.current_table_mut();
                table.move_selection(-1);
                cx.notify();
            }
            "down" => {
                let table = self.current_table_mut();
                table.move_selection(1);
                cx.notify();
            }
            "escape" => {
                if self.detail.is_some() {
                    self.close_detail();
                }
                self.host_menu_open = false;
                cx.notify();
            }
            _ => {}
        }
    }

    fn counts(&self) -> String {
        match self.page {
            Page::Containers => {
                let running = self
                    .snapshot
                    .containers
                    .iter()
                    .filter(|c| c.state == ContainerState::Running)
                    .count();
                let total = self.snapshot.containers.len();
                format!("{total} total · {running} running · {} stopped", total - running)
            }
            Page::Images => format!("{} images", self.snapshot.images.len()),
            Page::Volumes => format!("{} volumes", self.snapshot.volumes.len()),
        }
    }

    fn status_color(&self) -> gpui::Rgba {
        match &self.status {
            HostStatus::Connected => self.theme.running,
            HostStatus::Unknown => self.theme.text_dim,
            _ => self.theme.error,
        }
    }

    fn active_host_name(&self) -> SharedString {
        self.active_host
            .as_ref()
            .and_then(|id| self.hosts.iter().find(|h| h.id == *id))
            .map(|h| h.name.clone())
            .unwrap_or_else(|| "no host".to_string())
            .into()
    }

    fn open_detail(&mut self, row_index: usize) {
        let page = self.page;
        let Some(row) = self.tables[page.index()].rows.get(row_index) else {
            return;
        };
        let kind = match page {
            Page::Containers => ResourceKind::Container,
            Page::Images => ResourceKind::Image,
            Page::Volumes => ResourceKind::Volume,
        };
        let detail = DetailState::new(kind, row.id.clone(), row.cells[0].clone());
        let id = detail.id.clone();
        self.detail = Some(detail);
        if let Some(service) = &self.service {
            service.request(Request::Inspect { kind, id });
        }
    }

    fn close_detail(&mut self) {
        self.detail = None;
    }

    fn select_tab(&mut self, tab: DetailTab) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        if detail.kind != ResourceKind::Container && tab == DetailTab::Logs {
            return;
        }
        detail.tab = tab;
        let needs_logs =
            tab == DetailTab::Logs && !detail.logs_requested && detail.kind == ResourceKind::Container;
        if needs_logs {
            detail.logs_requested = true;
            let id = detail.id.clone();
            if let Some(service) = &self.service {
                service.request(Request::Logs { id });
            }
        }
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

        let body: gpui::AnyElement = if detail.missing {
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
                                if this.host_menu_open {
                                    if let Some(service) = &this.service {
                                        service.request(Request::ProbeHosts);
                                    }
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
            .when_some(self.last_error.clone(), |this, error| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.error)
                        .child(error),
                )
            })
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

        let list = uniform_list(
            self.tables[index].spec.id,
            visible_count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let mut rows = Vec::with_capacity(range.len());
                let theme = this.theme.clone();
                let table = &this.tables[index];
                for position in range {
                    let Some(&row_index) = table.visible().get(position) else {
                        continue;
                    };
                    let row = &table.rows[row_index];
                    let is_selected = selected == Some(row_index);
                    let mut element = div()
                        .id(SharedString::from(format!("row-{}", row.id)))
                        .h(px(ROW_HEIGHT))
                        .px_3()
                        .flex()
                        .items_center()
                        .text_sm()
                        .cursor_pointer()
                        .bg(if is_selected { theme.selected } else { theme.bg })
                        .hover(|s| s.bg(theme.hover))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.current_table_mut().selected = Some(row_index);
                            this.open_detail(row_index);
                            cx.notify();
                        }));

                    for (column, cell) in row.cells.iter().enumerate() {
                        let width = widths[column];
                        let color = if column == 1 {
                            row.accent.unwrap_or(theme.text)
                        } else {
                            theme.text
                        };
                        element = element.child(
                            div()
                                .map_width(width)
                                .truncate()
                                .text_color(color)
                                .child(cell.clone()),
                        );
                    }
                    rows.push(element);
                }
                rows
            }),
        )
        .h_full();

        let body: gpui::AnyElement = if visible_count == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.text_dim)
                .child(if self.current_table().query.is_empty() {
                    self.page.empty_title().to_string()
                } else {
                    format!("No matches for \"{}\"", self.current_table().query)
                })
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
                    .font_family("Menlo")
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
                    .font_family("Menlo")
                    .text_color(theme.text)
                    .child(child.value.clone().unwrap_or_default()),
            );
            group = group.child(row);
        }

        container = container.child(group);
    }

    container
}

fn state_label(state: ContainerState) -> &'static str {
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

fn state_color(state: ContainerState) -> gpui::Rgba {
    let theme = Theme::dark();
    match state {
        ContainerState::Running => theme.running,
        ContainerState::Paused | ContainerState::Restarting => theme.paused,
        ContainerState::Dead => theme.error,
        ContainerState::Exited | ContainerState::Created | ContainerState::Unknown => {
            theme.exited
        }
    }
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
            div()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .flex()
                .flex_col()
                .child(header)
                .child(table)
                .into_any_element()
        };

        let mut root = div()
            .size_full()
            .relative()
            .flex()
            .bg(theme.bg)
            .text_color(theme.text)
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, event, window, cx| this.on_root_key(event, window, cx)));

        root = root.child(rail).child(content);

        if let Some(menu) = menu {
            root = root.child(menu);
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::{ContainerRow, ImageRow, VolumeRow};
    use chrono::{DateTime, Utc};

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn container(name: &str, state: ContainerState) -> ContainerRow {
        ContainerRow {
            id: format!("{name}-id"),
            short_id: format!("{name}-id").chars().take(12).collect(),
            name: SharedString::from(name.to_string()),
            state,
            status: SharedString::from("Up 2 minutes"),
            image: SharedString::from("postgres:18"),
            ports: SharedString::from("5432→5432"),
            created: now(),
            created_label: SharedString::from("2m ago"),
        }
    }

    fn image(tag: &str, in_use: bool) -> ImageRow {
        ImageRow {
            id: format!("{tag}-id"),
            short_id: SharedString::from("abcdef123456"),
            repo: SharedString::from(tag.to_string()),
            repository_tag: SharedString::from(tag.to_string()),
            size: 456_654_848,
            size_label: SharedString::from("436 MB"),
            created: now(),
            created_label: SharedString::from("2m ago"),
            in_use,
        }
    }

    fn volume(name: &str) -> VolumeRow {
        VolumeRow {
            name: SharedString::from(name.to_string()),
            driver: SharedString::from("local"),
            mountpoint: SharedString::from("/var/lib/docker/volumes/x/_data"),
            created_label: SharedString::from("2m ago"),
        }
    }

    #[test]
    fn containers_table_puts_running_first_then_name() {
        let snapshot = Snapshot {
            containers: vec![
                container("aaa", ContainerState::Exited),
                container("zzz", ContainerState::Running),
                container("bbb", ContainerState::Running),
            ],
            ..Snapshot::empty()
        };
        let table = RootView::containers_table(&snapshot);
        let names: Vec<&str> = table
            .visible()
            .iter()
            .map(|&ix| table.rows[ix].cells[0].as_ref())
            .collect();
        assert_eq!(names, vec!["bbb", "zzz", "aaa"]);
    }

    #[test]
    fn images_table_marks_in_use_from_container_list() {
        let snapshot = Snapshot {
            images: vec![image("postgres:18", true), image("alpine:3", false)],
            ..Snapshot::empty()
        };
        let table = RootView::images_table(&snapshot);
        let used = table
            .rows
            .iter()
            .find(|r| r.cells[0].as_ref() == "postgres:18")
            .unwrap();
        let unused = table
            .rows
            .iter()
            .find(|r| r.cells[0].as_ref() == "alpine:3")
            .unwrap();
        assert_eq!(used.cells[4].as_ref(), "yes");
        assert_eq!(unused.cells[4].as_ref(), "—");
    }

    #[test]
    fn volumes_table_has_expected_columns() {
        let table = RootView::volumes_table(&Snapshot {
            volumes: vec![volume("sploney_db-data")],
            ..Snapshot::empty()
        });
        assert_eq!(
            table.spec.columns,
            vec!["Name", "Driver", "Mountpoint", "Created"]
        );
        assert_eq!(table.rows.len(), 1);
    }

    #[test]
    fn empty_snapshot_produces_empty_tables_with_headers() {
        let table = RootView::containers_table(&Snapshot::empty());
        assert!(table.visible().is_empty());
        assert_eq!(
            table.spec.columns,
            vec!["Name", "State", "Image", "Ports", "Created"]
        );
    }
}
