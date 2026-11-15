use crate::docker::actions::{
    build_run_spec, ActionKind, ContainerAction, Proto, RunField, RunForm, Target,
};
use crate::docker::endpoint::{discover, HostInfo, HostStatus};
use crate::docker::model::{ContainerState, ResourceKind, Snapshot};
use crate::docker::service::{DockerService, PullOrigin, Request, Response};
use crate::theme::Theme;
use crate::ui::detail::{DetailState, DetailTab};
use crate::ui::inspect::build_tree;
use crate::ui::mutations::{Mutations, PullState, RunOutcome, RunState, Toast};
use crate::ui::table::TableState;
use chrono::Utc;
use gpui::prelude::*;
use gpui::{Context, FocusHandle, KeyDownEvent, SharedString, Subscription, Task, Window};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

mod view;

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
            status: HostStatus::Unknown,
            hosts: discovery.hosts,
            active_host: discovery.active,
            tables: [
                crate::ui::tables::containers_table(&empty, &theme),
                crate::ui::tables::images_table(&empty),
                crate::ui::tables::volumes_table(&empty),
            ],
            last_error: None,
            mutations: Mutations::new(),
            host_menu_open: false,
            detail: None,
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
            Response::Dirty { ids } => {
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
                let outcome = self
                    .mutations
                    .apply_action_done(target.clone(), action, epoch, error);
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
                epoch,
                layer,
                status,
                detail,
            } => {
                let _ = self.mutations.apply_pull_progress(
                    origin, &reference, epoch, layer, status, detail,
                );
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
        self.tables = crate::ui::tables::tables_from(&self.snapshot, &self.tables, &self.theme);
    }

    pub fn switch_page(&mut self, page: Page) {
        self.page = page;
        self.host_menu_open = false;
        self.mutations.pull_open = false;
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

    fn container_state(&self, id: &str) -> Option<ContainerState> {
        self.snapshot
            .containers
            .iter()
            .find(|container| container.id == id)
            .map(|container| container.state)
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
                ResourceKind::Container => service.request(Request::RemoveContainer {
                    id,
                    force,
                    epoch,
                }),
                ResourceKind::Image => service.request(Request::RemoveImage {
                    id,
                    force,
                    epoch,
                }),
                ResourceKind::Volume => service.request(Request::RemoveVolume { name: id, epoch }),
            }
        }
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
            RunField::PortContainer(index) => {
                run.form.ports.get_mut(index).map(|port| &mut port.container)
            }
            RunField::EnvKey(index) => run.form.env.get_mut(index).map(|variable| &mut variable.key),
            RunField::EnvValue(index) => {
                run.form.env.get_mut(index).map(|variable| &mut variable.value)
            }
            RunField::MountSource(index) => {
                run.form.mounts.get_mut(index).map(|mount| &mut mount.source)
            }
            RunField::MountTarget(index) => {
                run.form.mounts.get_mut(index).map(|mount| &mut mount.target)
            }
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
            run.form.ports.push(crate::docker::actions::PortForm::default());
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
            run.form.env.push(crate::docker::actions::EnvForm::default());
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
            run.form.mounts.push(crate::docker::actions::MountForm::default());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::ContainerRow;
    use crate::theme::Theme;
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
        }
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
        let theme = Theme::dark();
        let mut tables = [
            crate::ui::tables::containers_table(&snapshot, &theme),
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
