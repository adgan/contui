use crate::docker::actions::{container_actions, ActionKind, ContainerAction, Target};
use crate::docker::endpoint::{discover, HostInfo, HostStatus};
use crate::docker::model::{format_bytes, ContainerState, Snapshot};
use crate::docker::service::{DockerService, PullOrigin, Request, Response};
use crate::theme::Theme;
use crate::docker::model::ResourceKind;
use crate::ui::detail::{render_logs, DetailState, DetailTab};
use crate::ui::inspect::{build_tree, InspectNode};
use crate::ui::mutations::{Mutations, PullState, RowMenuState, Toast};
use crate::ui::table::{ActionsColumn, ColumnWidth, SortValue, TableRow, TableSpec, TableState};
use chrono::Utc;
use gpui::prelude::*;use gpui::{
    Context, Div, FocusHandle, KeyDownEvent, MouseButton, Render, SharedString, Subscription, Task,
    Window, div, px, svg, uniform_list,
};
use std::collections::{BTreeMap, HashMap};
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

/// Order of operations when the user presses Esc: a filter is the most local
/// thing to clear, then the detail page, then the whole list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeAction {
    ClearQuery,
    Back,
}

pub fn escape_action(query: &str) -> EscapeAction {
    if query.is_empty() {
        EscapeAction::Back
    } else {
        EscapeAction::ClearQuery
    }
}

pub fn is_stale(fetched_at: Option<chrono::DateTime<Utc>>, now: chrono::DateTime<Utc>) -> bool {
    fetched_at
        .map(|fetched| (now - fetched).num_seconds() > STALE_AFTER_SECS)
        .unwrap_or(true)
}

/// A reconnect is only useful if we refetch: events that happened while the
/// engine was down were never delivered.
pub fn should_refetch_on_status(previous: &HostStatus, next: &HostStatus) -> bool {
    !matches!(previous, HostStatus::Connected) && matches!(next, HostStatus::Connected)
}

pub fn tables_from(snapshot: &Snapshot, previous: &[TableState; 3]) -> [TableState; 3] {
    let mut next = [
        RootView::containers_table(snapshot),
        RootView::images_table(snapshot),
        RootView::volumes_table(snapshot),
    ];
    for (fresh, old) in next.iter_mut().zip(previous.iter()) {
        fresh.adopt_state(old);
    }
    next
}

fn reset_host_data(
    snapshot: &mut Snapshot,
    tables: &mut [TableState; 3],
    detail: &mut Option<DetailState>,
) {
    *snapshot = Snapshot::empty();
    for table in tables.iter_mut() {
        table.set_rows(Vec::new());
        table.set_query("");
        table.selected = None;
    }
    *detail = None;
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
    pub mutations: Mutations,
    pub host_menu_open: bool,
    pub detail: Option<DetailState>,
    search_focus: FocusHandle,
    root_focus: FocusHandle,
    pull_focus: FocusHandle,
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
            mutations: Mutations::new(),
            host_menu_open: false,
            detail: None,
            search_focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            pull_focus: cx.focus_handle(),
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
            actions: Some(ActionsColumn::Container),
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
            actions: Some(ActionsColumn::Image),
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
            actions: Some(ActionsColumn::Volume),
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
        let host_id = host.id.clone();
        self.service = None;
        self._pump = None;
        self.host_menu_open = false;
        reset_host_data(&mut self.snapshot, &mut self.tables, &mut self.detail);
        self.last_error = None;
        self.status = HostStatus::Unknown;
        match DockerService::connect(host) {
            Ok((service, responses)) => {
                self.active_host = Some(host_id.clone());
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
                self.active_host = Some(host_id);
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
            Response::Status(status) => {
                if should_refetch_on_status(&self.status, &status) {
                    self.status = status;
                    self.last_error = None;
                    self.refresh();
                } else {
                    self.status = status;
                }
            }
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
                let owned = self
                    .detail
                    .as_mut()
                    .map(|detail| detail.apply_failed(&what, &message))
                    .unwrap_or(false);
                if !owned {
                    self.last_error = Some(format!("{what}: {message}"));
                }
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
            Response::ActionDone {
                target,
                action,
                error,
            } => {
                let outcome = self
                    .mutations
                    .apply_action_done(target.clone(), action, error);
                if outcome.close_detail {
                    let open = self.detail.as_ref().map(|detail| (detail.kind, detail.id.clone()));
                    if open == Some((target.kind, target.id)) {
                        self.close_detail();
                    }
                }
                if let Some(toast) = outcome.toast {
                    self.mutations.toast = Some(toast);
                }
                if outcome.refetch {
                    self.refresh();
                }
            }
            Response::PullProgress {
                origin,
                reference,
                layer,
                status,
                detail,
            } => {
                let _ = self
                    .mutations
                    .apply_pull_progress(origin, &reference, layer, status, detail);
            }
            Response::PullDone {
                origin,
                reference,
                error,
            } => {
                let outcome = self.mutations.apply_pull_done(origin, &reference, error);
                if outcome.retry_run {
                    self.retry_run_after_pull();
                }
                if let Some(toast) = outcome.toast {
                    self.mutations.toast = Some(toast);
                }
                if outcome.refetch {
                    self.refresh();
                }
            }
            Response::ContainerCreated { .. } | Response::RunFailed { .. } => {}
        }
    }

    fn rebuild_tables(&mut self) {
        self.tables = tables_from(&self.snapshot, &self.tables);
    }

    pub fn switch_page(&mut self, page: Page) {
        self.page = page;
        self.host_menu_open = false;
        self.refresh_if_stale();
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

    fn scroll_selection_into_view(&mut self, index: usize) {
        let table = &self.tables[index];
        if let Some(position) = table.selection_position() {
            table.scroll.scroll_to_item(position, gpui::ScrollStrategy::Center);
        }
    }

    fn request_detail_refresh(&mut self) {
        let Some(detail) = self.detail.clone() else {
            return;
        };
        let Some(service) = &self.service else {
            return;
        };
        service.request(Request::Inspect {
            kind: detail.kind,
            id: detail.id.clone(),
        });
        if detail.logs_requested {
            service.request(Request::Logs {
                id: detail.id.clone(),
            });
        }
    }

    fn refresh(&mut self) {
        if let Some(service) = &self.service {
            service.request(Request::FetchAll);
        }
    }

    fn refresh_if_stale(&mut self) {
        if is_stale(self.snapshot.fetched_at, Utc::now()) {
            self.refresh();
        }
    }

    /// Esc is bound to the `Back` action, which gpui dispatches before element
    /// key handlers, so the query-clearing behaviour lives here.
    fn go_back(&mut self) {
        let query = self.current_table().query.clone();
        match escape_action(&query) {
            EscapeAction::ClearQuery => {
                let table = self.current_table_mut();
                table.set_query("");
            }
            EscapeAction::Back => {
                if self.detail.is_some() {
                    self.close_detail();
                } else {
                    self.host_menu_open = false;
                }
            }
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
                let index = self.page.index();
                self.tables[index].move_selection(-1);
                self.scroll_selection_into_view(index);
                cx.notify();
            }
            "down" => {
                let index = self.page.index();
                self.tables[index].move_selection(1);
                self.scroll_selection_into_view(index);
                cx.notify();
            }
            "escape" => {
                self.go_back();
                cx.notify();
            }
            "enter" => {
                if self.detail.is_none() {
                    if let Some(row) = self.current_table().selected {
                        self.open_detail(row);
                    }
                }
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

    fn open_detail_by_id(&mut self, kind: ResourceKind, id: &str) {
        let page = match kind {
            ResourceKind::Container => Page::Containers,
            ResourceKind::Image => Page::Images,
            ResourceKind::Volume => Page::Volumes,
        };
        let row_index = self.tables[page.index()]
            .rows
            .iter()
            .position(|row| row.id == id);
        if let Some(row_index) = row_index {
            self.page = page;
            self.tables[page.index()].selected = Some(row_index);
            self.open_detail(row_index);
        }
    }

    fn request_action(&mut self, kind: ResourceKind, id: &str, title: &str, action: ActionKind) {
        if action == ActionKind::Remove {
            let tag_count = if kind == ResourceKind::Image {
                self.snapshot
                    .images
                    .iter()
                    .find(|image| image.id == id)
                    .map(|image| image.tag_count)
                    .unwrap_or(1)
            } else {
                0
            };
            self.mutations.open_confirm(
                Target {
                    kind,
                    id: id.to_string(),
                },
                title.to_string(),
                tag_count,
            );
            return;
        }

        if kind != ResourceKind::Container {
            return;
        }
        if self.service.is_none() {
            self.mutations.toast = Some(Toast {
                text: "No host connection".to_string(),
                error: true,
            });
            return;
        }
        let container_action = match action {
            ActionKind::Start => ContainerAction::Start,
            ActionKind::Stop => ContainerAction::Stop,
            ActionKind::Restart => ContainerAction::Restart,
            ActionKind::Remove => return,
        };
        let target = Target {
            kind,
            id: id.to_string(),
        };
        if !self.mutations.begin(&target, action) {
            return;
        }
        if let Some(service) = &self.service {
            service.request(Request::ContainerAction {
                id: id.to_string(),
                action: container_action,
            });
        }
    }

    pub(crate) fn cancel_confirm(&mut self) {
        let Some(confirm) = &self.mutations.confirm else {
            return;
        };
        if confirm.submitting {
            return;
        }
        self.mutations.confirm = None;
    }

    pub(crate) fn submit_confirm(&mut self) {
        let Some(confirm) = self.mutations.confirm.as_mut() else {
            return;
        };
        if confirm.submitting {
            return;
        }
        if self.service.is_none() {
            confirm.error = Some("No host connection".to_string());
            return;
        }
        let kind = confirm.kind;
        let id = confirm.id.clone();
        let force = confirm.force;
        confirm.submitting = true;
        confirm.error = None;

        let target = Target {
            kind,
            id: id.clone(),
        };
        if !self.mutations.begin(&target, ActionKind::Remove) {
            if let Some(confirm) = &mut self.mutations.confirm {
                confirm.submitting = false;
            }
            return;
        }
        if let Some(service) = &self.service {
            match kind {
                ResourceKind::Container => service.request(Request::RemoveContainer { id, force }),
                ResourceKind::Image => service.request(Request::RemoveImage { id, force }),
                ResourceKind::Volume => service.request(Request::RemoveVolume { name: id }),
            }
        }
    }

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

    fn render_row_menu(&mut self, cx: &mut Context<Self>) -> Option<Div> {
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

        let backdrop = div().absolute().inset_0().on_mouse_down(
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
            .left(menu.at.x)
            .top(menu.at.y)
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
        panel = panel.child(row_menu_item(
            "Remove",
            theme.error,
            &theme,
            cx,
            move |this, _cx| {
                this.request_action(menu.kind, &remove_id, &remove_title, ActionKind::Remove);
                this.mutations.row_menu = None;
            },
        ));

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
        let pending_label = self
            .mutations
            .pending
            .get(&(detail.kind, detail.id.clone()))
            .map(|action| action.pending_label());

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
                            self.snapshot
                                .containers
                                .iter()
                                .find(|container| container.id == detail.id)
                                .map(|container| container.state)
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

    pub(crate) fn start_pull(&mut self, reference: &str, origin: PullOrigin) {
        let reference = reference.trim();
        if reference.is_empty() || self.mutations.pull.is_some() {
            return;
        }
        if self.service.is_none() {
            self.mutations.toast = Some(Toast {
                text: "No host connection".to_string(),
                error: true,
            });
            return;
        }
        self.mutations.pull = Some(PullState {
            origin,
            reference: reference.to_string(),
            status: "Starting…".to_string(),
            layers: BTreeMap::new(),
        });
        self.mutations.pull_error = None;
        self.mutations.pull_open = false;
        if let Some(service) = &self.service {
            service.request(Request::PullImage {
                reference: reference.to_string(),
                origin,
            });
        }
    }

    fn retry_run_after_pull(&mut self) {
        let Some(run) = &mut self.mutations.run else {
            return;
        };
        let Some(spec) = run.retry_spec() else {
            return;
        };
        run.pull_then_retry = false;
        if let Some(service) = &self.service {
            service.request(Request::RunContainer { spec });
        }
    }

    fn render_pull_popover(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.mutations.pull_open {
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
                                .map(|action| action.pending_label())
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
                            this.snapshot
                                .containers
                                .iter()
                                .find(|container| container.id == click_id)
                                .map(|container| container.state)
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
                        element = element.child(
                            div()
                                .w(px(actions.width()))
                                .flex_shrink_0()
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
    match kind {
        ResourceKind::Container => ActionsColumn::Container.width(),
        ResourceKind::Image => ActionsColumn::Image.width(),
        ResourceKind::Volume => ActionsColumn::Volume.width(),
    }
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
        if let Some(row_menu) = self.render_row_menu(cx) {
            root = root.child(row_menu);
        }
        if let Some(confirm) = self.mutations.confirm.clone() {
            root = root.child(crate::ui::dialog::confirm(&confirm, &theme, cx));
        }
        if let Some(toast) = self.render_toast(cx) {
            root = root.child(toast);
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
            tag_count: 1,
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
    fn each_table_declares_its_action_column() {
        let snapshot = Snapshot::empty();
        assert_eq!(
            RootView::containers_table(&snapshot).spec.actions,
            Some(ActionsColumn::Container)
        );
        assert_eq!(
            RootView::images_table(&snapshot).spec.actions,
            Some(ActionsColumn::Image)
        );
        assert_eq!(
            RootView::volumes_table(&snapshot).spec.actions,
            Some(ActionsColumn::Volume)
        );
        assert_eq!(ActionsColumn::Container.width(), 124.0);
        assert_eq!(ActionsColumn::Image.width(), 84.0);
        assert_eq!(ActionsColumn::Volume.width(), 44.0);
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
    fn rebuilding_tables_keeps_query_sort_and_selection() {
        let snapshot = Snapshot {
            containers: vec![
                container("alpha", ContainerState::Running),
                container("beta", ContainerState::Exited),
            ],
            ..Snapshot::empty()
        };
        let mut previous = [
            RootView::containers_table(&snapshot),
            RootView::images_table(&snapshot),
            RootView::volumes_table(&snapshot),
        ];
        previous[0].set_query("alpha");
        previous[0].click_header(0);
        previous[0].selected = Some(0);

        let next = tables_from(&snapshot, &previous);
        assert_eq!(next[0].query, "alpha");
        assert_eq!(next[0].sort, Some((0, true)));
        assert!(next[0].visible().len() <= 1);
    }

    #[test]
    fn reconnect_from_disconnected_refetches() {
        assert!(should_refetch_on_status(
            &HostStatus::Unreachable("gone".into()),
            &HostStatus::Connected
        ));
        assert!(should_refetch_on_status(
            &HostStatus::Unknown,
            &HostStatus::Connected
        ));
        assert!(!should_refetch_on_status(
            &HostStatus::Connected,
            &HostStatus::Connected
        ));
        assert!(!should_refetch_on_status(
            &HostStatus::Connected,
            &HostStatus::Unreachable("gone".into())
        ));
    }

    #[test]
    fn stale_after_five_seconds_or_with_no_snapshot() {
        let now = now();
        assert!(is_stale(None, now));
        assert!(!is_stale(Some(now - chrono::Duration::seconds(2)), now));
        assert!(is_stale(Some(now - chrono::Duration::seconds(9)), now));
    }

    #[test]
    fn escape_clears_a_query_before_going_back() {
        assert_eq!(escape_action("postg"), EscapeAction::ClearQuery);
        assert_eq!(escape_action(""), EscapeAction::Back);
    }

    #[test]
    fn host_switch_resets_rows_and_detail() {
        let snapshot = Snapshot {
            containers: vec![container("alpha", ContainerState::Running)],
            ..Snapshot::empty()
        };
        let mut tables = [
            RootView::containers_table(&snapshot),
            RootView::images_table(&snapshot),
            RootView::volumes_table(&snapshot),
        ];
        tables[0].set_query("alpha");
        let mut snapshot = snapshot;
        let mut detail = Some(DetailState::new(
            crate::docker::model::ResourceKind::Container,
            "alpha-id".into(),
            "alpha".into(),
        ));

        reset_host_data(&mut snapshot, &mut tables, &mut detail);

        assert!(snapshot.containers.is_empty());
        assert!(tables[0].rows.is_empty());
        assert_eq!(tables[0].query, "");
        assert!(detail.is_none());
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
