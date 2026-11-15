use crate::docker::actions::{
    ActionKind, ContainerAction, Proto, RunField, RunForm, Target, build_run_spec,
};
use crate::docker::endpoint::{HostInfo, HostStatus, discover};
use crate::docker::model::{ContainerState, ResourceKind, Snapshot};
use crate::docker::service::{DockerService, PullOrigin, Request, Response};
use crate::theme::Theme;
use crate::ui::detail::DetailState;
use crate::ui::inspect::build_tree;
use crate::ui::mutations::{Mutations, PullState, RunOutcome, RunState, Toast};
use crate::ui::stats::OverviewStats;
use crate::ui::table::TableState;
use chrono::Utc;
use gpui::prelude::*;
use gpui::{Context, FocusHandle, KeyDownEvent, SharedString, Subscription, Task, Window};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

mod view;

const STALE_AFTER_SECS: i64 = 5;

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

    pub fn resource(self) -> ResourceKind {
        match self {
            Page::Containers => ResourceKind::Container,
            Page::Images => ResourceKind::Image,
            Page::Volumes => ResourceKind::Volume,
        }
    }

    pub fn for_kind(kind: ResourceKind) -> Self {
        match kind {
            ResourceKind::Container => Page::Containers,
            ResourceKind::Image => Page::Images,
            ResourceKind::Volume => Page::Volumes,
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
    pub stats: OverviewStats,
    pub status: HostStatus,
    pub hosts: Vec<HostInfo>,
    pub active_host: Option<String>,
    pub tables: [TableState; 3],
    pub last_error: Option<String>,
    pub mutations: Mutations,
    pub host_menu_open: bool,
    pub detail: Option<DetailState>,
    /// Group Compose containers under their project in the containers list.
    pub group_containers: bool,
    pub collapsed_projects: HashSet<String>,
    pub ungrouped_collapsed: bool,
    search_focus: FocusHandle,
    root_focus: FocusHandle,
    pull_focus: FocusHandle,
    run_focus: Option<crate::ui::dialog::RunFocus>,
    focus_initialized: bool,
    activation: Option<Subscription>,
    _pump: Option<Task<()>>,
}

impl RootView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let env: HashMap<String, String> = std::env::vars().collect();
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
        let discovery = discover(&env, &home);
        let theme = Theme::dark();
        let empty = Snapshot::empty();

        let mut view = Self {
            page: Page::Containers,
            theme: theme.clone(),
            service: None,
            snapshot: Snapshot::empty(),
            stats: OverviewStats::default(),
            status: HostStatus::Unknown,
            hosts: discovery.hosts,
            active_host: discovery.active,
            tables: [
                crate::ui::tables::containers_table(&empty),
                crate::ui::tables::images_table(&empty),
                crate::ui::tables::volumes_table(&empty),
            ],
            last_error: None,
            mutations: Mutations::new(),
            host_menu_open: false,
            detail: None,
            group_containers: true,
            collapsed_projects: HashSet::new(),
            ungrouped_collapsed: false,
            search_focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            pull_focus: cx.focus_handle(),
            run_focus: None,
            focus_initialized: false,
            activation: None,
            _pump: None,
        };

        if let Some(id) = view.active_host.clone()
            && let Some(host) = view.hosts.iter().find(|h| h.id == id).cloned()
        {
            view.connect(host, cx);
        }

        view
    }

    fn connect(&mut self, host: HostInfo, cx: &mut Context<Self>) {
        let host_id = host.id.clone();
        self.service = None;
        self._pump = None;
        self.host_menu_open = false;
        reset_host_data(&mut self.snapshot, &mut self.tables, &mut self.detail);
        self.mutations = Mutations::new();
        self.run_focus = None;
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
                self.stats = OverviewStats::from_snapshot(&snapshot);
                self.snapshot = snapshot;
                self.rebuild_tables();
                self.last_error = None;
                if let Some(id) = self.mutations.pending_select.take() {
                    let page_index = Page::Containers.index();
                    if let Some(row_index) = self.tables[page_index]
                        .rows
                        .iter()
                        .position(|row| row.id == id)
                    {
                        self.page = Page::Containers;
                        self.tables[page_index].selected = Some(row_index);
                        self.open_detail(row_index);
                    }
                }
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
                let had_work = self.mutations.has_in_flight();
                if should_refetch_on_status(&self.status, &status) {
                    self.status = status;
                    self.last_error = None;
                    self.refresh();
                } else {
                    self.status = status;
                }
                if !matches!(self.status, HostStatus::Connected) && had_work {
                    self.mutations.clear_in_flight();
                    self.mutations.confirm = None;
                    self.mutations.run = None;
                    self.run_focus = None;
                    self.mutations.row_menu = None;
                    self.mutations.toast = Some(Toast {
                        text: "Host disconnected — action interrupted".to_string(),
                        error: true,
                    });
                }
            }
            Response::HostStatuses(statuses) => {
                for (id, status) in statuses {
                    if let Some(host) = self.hosts.iter_mut().find(|h| h.id == id) {
                        host.status = status;
                    }
                }
            }
            Response::Dirty { .. } => {
                // Keep the open inspect tree on screen while the refetch runs.
                // Clearing it unmounts the scroll area and jumps the user back
                // to the top on every container event.
                if let Some(service) = &self.service {
                    service.request(Request::FetchAll);
                }
                if let Some(detail) = self.detail.clone()
                    && !detail.missing
                    && let Some(service) = &self.service
                {
                    service.request(Request::Inspect {
                        kind: detail.kind,
                        id: detail.id.clone(),
                    });
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
                if let Some(detail) = &mut self.detail
                    && detail.kind == kind
                    && detail.id == id
                {
                    detail.inspect = Some(build_tree(&json, 2));
                    detail.missing = false;
                }
            }
            Response::Logs { id, lines } => {
                if let Some(detail) = &mut self.detail
                    && detail.id == id
                {
                    detail.logs = Some(lines);
                }
            }
            Response::ActionDone {
                target,
                action,
                epoch,
                error,
            } => {
                let outcome =
                    self.mutations
                        .apply_action_done(target.clone(), action, epoch, error);
                if outcome.close_detail {
                    let open = self
                        .detail
                        .as_ref()
                        .map(|detail| (detail.kind, detail.id.clone()));
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
                epoch,
                layer,
                status,
                detail,
            } => {
                let _ = self
                    .mutations
                    .apply_pull_progress(origin, &reference, epoch, layer, status, detail);
            }
            Response::PullDone {
                origin,
                reference,
                epoch,
                error,
            } => {
                let outcome = self
                    .mutations
                    .apply_pull_done(origin, &reference, epoch, error);
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
            Response::ContainerCreated {
                id,
                warnings,
                epoch,
            } => {
                let outcome = self.mutations.apply_run_created(id, warnings, epoch);
                self.apply_run_outcome(outcome);
            }
            Response::RunFailed {
                message,
                missing_image,
                created,
                epoch,
            } => {
                let outcome =
                    self.mutations
                        .apply_run_failed(message, missing_image, created, epoch);
                self.apply_run_outcome(outcome);
            }
        }
    }

    fn rebuild_tables(&mut self) {
        self.tables = crate::ui::tables::tables_from(&self.snapshot, &self.tables);
    }

    pub fn switch_page(&mut self, page: Page) {
        self.page = page;
        self.detail = None;
        self.host_menu_open = false;
        self.mutations.pull_open = false;
        self.refresh_if_stale();
    }

    /// The containers list as displayed: Compose project headers with their
    /// rows, then standalone containers. `None` means render the plain table.
    fn container_slots(&self) -> Option<Vec<ContainerSlot>> {
        if self.page != Page::Containers || !self.group_containers {
            return None;
        }
        compose_slots(
            &self.tables[Page::Containers.index()],
            &self.snapshot.containers,
            &self.collapsed_projects,
            self.ungrouped_collapsed,
        )
    }

    fn toggle_ungrouped(&mut self) {
        self.ungrouped_collapsed = !self.ungrouped_collapsed;
        if !self.ungrouped_collapsed {
            return;
        }
        let selected_id = {
            let table = &self.tables[Page::Containers.index()];
            table
                .selected
                .and_then(|ix| table.rows.get(ix))
                .map(|row| row.id.clone())
        };
        let selected_is_hidden = selected_id.is_some_and(|id| {
            self.snapshot
                .containers
                .iter()
                .find(|container| container.id == id)
                .is_some_and(|container| container.compose_project.is_none())
        });
        if selected_is_hidden {
            self.tables[Page::Containers.index()].selected = None;
        }
    }

    fn toggle_project(&mut self, project: &str) {
        if self.collapsed_projects.remove(project) {
            return;
        }
        self.collapsed_projects.insert(project.to_string());
        let selected_id = {
            let table = &self.tables[Page::Containers.index()];
            table
                .selected
                .and_then(|ix| table.rows.get(ix))
                .map(|row| row.id.clone())
        };
        let selected_is_hidden = selected_id.is_some_and(|id| {
            self.snapshot
                .containers
                .iter()
                .find(|container| container.id == id)
                .and_then(|container| container.compose_project.as_ref().map(|p| p.as_ref()))
                == Some(project)
        });
        if selected_is_hidden {
            self.tables[Page::Containers.index()].selected = None;
        }
    }

    /// Arrow-key selection walks the grouped slots so a collapsed project's
    /// rows are skipped.
    fn move_selection(&mut self, delta: isize) {
        let index = self.page.index();
        if index == Page::Containers.index() {
            if let Some(slots) = self.container_slots() {
                let rows: Vec<usize> = slots
                    .iter()
                    .filter_map(|slot| match slot {
                        ContainerSlot::Row(ix) => Some(*ix),
                        _ => None,
                    })
                    .collect();
                let table = &mut self.tables[index];
                if rows.is_empty() {
                    table.selected = None;
                    return;
                }
                table.selected = match table.selected {
                    None => Some(if delta >= 0 {
                        rows[0]
                    } else {
                        *rows.last().unwrap()
                    }),
                    Some(selected) => {
                        let position = rows.iter().position(|&ix| ix == selected).unwrap_or(0);
                        let next =
                            (position as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
                        Some(rows[next])
                    }
                };
                return;
            }
        }
        self.tables[index].move_selection(delta);
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
        let selected = self.tables[index].selected;
        let grouped_position = if index == Page::Containers.index() {
            selected.and_then(|selected| {
                self.container_slots().and_then(|slots| {
                    slots
                        .iter()
                        .position(|slot| matches!(slot, ContainerSlot::Row(ix) if *ix == selected))
                })
            })
        } else {
            None
        };
        let position = grouped_position.or_else(|| self.tables[index].selection_position());
        if let Some(position) = position {
            self.tables[index]
                .scroll
                .scroll_to_item(position, gpui::ScrollStrategy::Center);
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
        if let Some(confirm) = &self.mutations.confirm {
            if !confirm.submitting {
                self.cancel_confirm();
            }
            return;
        }
        if let Some(run) = &self.mutations.run {
            if !run.submitting {
                self.cancel_run();
            }
            return;
        }
        if self.mutations.row_menu.take().is_some() {
            return;
        }
        if self.mutations.pull_open {
            self.mutations.pull_open = false;
            return;
        }
        if self.host_menu_open {
            self.host_menu_open = false;
            return;
        }
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
                self.move_selection(-1);
                self.scroll_selection_into_view(index);
                cx.notify();
            }
            "down" => {
                let index = self.page.index();
                self.move_selection(1);
                self.scroll_selection_into_view(index);
                cx.notify();
            }
            "escape" => {
                self.go_back();
                cx.notify();
            }
            "enter" => {
                if self.detail.is_none()
                    && let Some(row) = self.current_table().selected
                {
                    self.open_detail(row);
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn subtitle(&self) -> String {
        match self.page {
            Page::Containers => self.stats.containers_subtitle(),
            Page::Images => self.stats.images_subtitle(),
            Page::Volumes => self.stats.volumes_subtitle(),
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
        let kind = page.resource();
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
        let page = Page::for_kind(kind);
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
                epoch: self.mutations.epoch,
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
        let epoch = self.mutations.epoch;
        confirm.submitting = true;
        confirm.error = None;

        let target = Target {
            kind,
            id: id.clone(),
        };
        if !self.mutations.begin(&target, ActionKind::Remove) {
            if let Some(confirm) = &mut self.mutations.confirm {
                confirm.submitting = false;
                confirm.error = Some("An action is already running".to_string());
            }
            return;
        }
        if let Some(service) = &self.service {
            match kind {
                ResourceKind::Container => {
                    service.request(Request::RemoveContainer { id, force, epoch })
                }
                ResourceKind::Image => service.request(Request::RemoveImage { id, force, epoch }),
                ResourceKind::Volume => service.request(Request::RemoveVolume { name: id, epoch }),
            }
        }
    }

    fn retry_run_after_pull(&mut self) {
        let epoch = self.mutations.epoch;
        let Some(run) = &mut self.mutations.run else {
            return;
        };
        if let Some(spec) = run.begin_retry(epoch)
            && let Some(service) = &self.service
        {
            service.request(Request::RunContainer { spec, epoch });
        }
    }

    fn apply_run_outcome(&mut self, outcome: RunOutcome) {
        if outcome.close_dialog {
            self.mutations.run = None;
            self.run_focus = None;
        }
        if let Some(id) = outcome.select_container {
            self.page = Page::Containers;
            self.mutations.pending_select = Some(id);
        }
        if let Some(toast) = outcome.toast {
            self.mutations.toast = Some(toast);
        }
        if outcome.refetch {
            self.refresh();
        }
    }
}

/// One visible line in the grouped containers list. Group headers are 44px
/// like data rows, so `uniform_list` keeps working.
#[derive(Clone, Debug, PartialEq)]
enum ContainerSlot {
    Group {
        project: SharedString,
        count: usize,
        running: usize,
    },
    /// Separates containers without a Compose project from the grouped ones.
    Ungrouped {
        count: usize,
        running: usize,
    },
    Row(usize),
}

/// Buckets the visible container rows by Compose project. Returns `None` when
/// no visible container carries a project label, so plain hosts keep the flat
/// list. Groups sort by project name; standalone rows follow.
fn compose_slots(
    table: &TableState,
    containers: &[crate::docker::model::ContainerRow],
    collapsed: &HashSet<String>,
    ungrouped_collapsed: bool,
) -> Option<Vec<ContainerSlot>> {
    let state_by_id: HashMap<&str, ContainerState> = containers
        .iter()
        .map(|container| (container.id.as_str(), container.state))
        .collect();
    let project_by_id: HashMap<&str, Option<&str>> = containers
        .iter()
        .map(|container| {
            (
                container.id.as_str(),
                container.compose_project.as_ref().map(|p| p.as_ref()),
            )
        })
        .collect();

    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut standalone = Vec::new();
    for &index in table.visible() {
        let Some(row) = table.rows.get(index) else {
            continue;
        };
        match project_by_id.get(row.id.as_str()).copied().flatten() {
            Some(project) => groups.entry(project).or_default().push(index),
            None => standalone.push(index),
        }
    }
    if groups.is_empty() {
        return None;
    }

    let mut slots = Vec::new();
    for (project, rows) in groups {
        let running = rows
            .iter()
            .filter(|&&index| {
                table
                    .rows
                    .get(index)
                    .and_then(|row| state_by_id.get(row.id.as_str()))
                    == Some(&ContainerState::Running)
            })
            .count();
        slots.push(ContainerSlot::Group {
            project: SharedString::from(project.to_string()),
            count: rows.len(),
            running,
        });
        if !collapsed.contains(project) {
            slots.extend(rows.into_iter().map(ContainerSlot::Row));
        }
    }
    if !standalone.is_empty() {
        let running = standalone
            .iter()
            .filter(|&&index| {
                table
                    .rows
                    .get(index)
                    .and_then(|row| state_by_id.get(row.id.as_str()))
                    == Some(&ContainerState::Running)
            })
            .count();
        slots.push(ContainerSlot::Ungrouped {
            count: standalone.len(),
            running,
        });
        if !ungrouped_collapsed {
            slots.extend(standalone.into_iter().map(ContainerSlot::Row));
        }
    }
    Some(slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::ContainerRow;
    use chrono::{DateTime, Utc};

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn container(name: &str, state: ContainerState) -> ContainerRow {
        ContainerRow {
            id: format!("{name}-id"),
            name: SharedString::from(name.to_string()),
            state,
            status: SharedString::from("Up 2 minutes"),
            image: SharedString::from("postgres:18"),
            ports: SharedString::from("5432→5432"),
            created: now(),
            created_label: SharedString::from("2m ago"),
            compose_project: None,
        }
    }

    fn compose_container(name: &str, state: ContainerState, project: Option<&str>) -> ContainerRow {
        let mut row = container(name, state);
        row.compose_project = project.map(|project| SharedString::from(project.to_string()));
        row
    }
    #[test]
    fn compose_slots_group_projects_alphabetically_and_hide_collapsed_rows() {
        let snapshot = Snapshot {
            containers: vec![
                compose_container("db-1", ContainerState::Running, Some("shop")),
                compose_container("api-1", ContainerState::Exited, Some("api")),
                compose_container("solo", ContainerState::Running, None),
                compose_container("web-1", ContainerState::Running, Some("shop")),
                compose_container("api-2", ContainerState::Running, Some("api")),
            ],
            ..Snapshot::empty()
        };
        let table = crate::ui::tables::containers_table(&snapshot);
        let slots = compose_slots(&table, &snapshot.containers, &HashSet::new(), false).unwrap();

        let groups: Vec<(&str, usize, usize)> = slots
            .iter()
            .filter_map(|slot| match slot {
                ContainerSlot::Group {
                    project,
                    count,
                    running,
                } => Some((project.as_ref(), *count, *running)),
                _ => None,
            })
            .collect();
        assert_eq!(groups, vec![("api", 2, 1), ("shop", 2, 2)]);
        assert_eq!(
            slots.iter().find_map(|slot| match slot {
                ContainerSlot::Ungrouped { count, running } => Some((*count, *running)),
                _ => None,
            }),
            Some((1, 1)),
            "standalone containers get their own section header"
        );
        match slots.last().unwrap() {
            ContainerSlot::Row(index) => assert_eq!(table.rows[*index].id, "solo-id"),
            _ => panic!("standalone rows follow the groups"),
        }

        let collapsed = HashSet::from(["shop".to_string()]);
        let slots = compose_slots(&table, &snapshot.containers, &collapsed, false).unwrap();
        let visible: Vec<&str> = slots
            .iter()
            .filter_map(|slot| match slot {
                ContainerSlot::Row(index) => Some(table.rows[*index].cells[0].as_ref()),
                _ => None,
            })
            .collect();
        assert!(!visible.contains(&"web-1"));
        assert!(!visible.contains(&"db-1"));
        assert!(visible.contains(&"solo"));
        assert!(visible.contains(&"api-2"));

        let slots = compose_slots(&table, &snapshot.containers, &HashSet::new(), true).unwrap();
        let visible: Vec<&str> = slots
            .iter()
            .filter_map(|slot| match slot {
                ContainerSlot::Row(index) => Some(table.rows[*index].cells[0].as_ref()),
                _ => None,
            })
            .collect();
        assert!(
            slots
                .iter()
                .any(|slot| matches!(slot, ContainerSlot::Ungrouped { .. }))
        );
        assert!(!visible.contains(&"solo"), "collapsed ungrouped rows hide");
        assert!(visible.contains(&"api-2"));
    }

    #[test]
    fn compose_slots_stay_flat_without_project_labels() {
        let snapshot = Snapshot {
            containers: vec![container("plain", ContainerState::Running)],
            ..Snapshot::empty()
        };
        let table = crate::ui::tables::containers_table(&snapshot);
        assert!(compose_slots(&table, &snapshot.containers, &HashSet::new(), false).is_none());
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
            crate::ui::tables::containers_table(&snapshot),
            crate::ui::tables::images_table(&snapshot),
            crate::ui::tables::volumes_table(&snapshot),
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
}
