use crate::docker::actions::{ActionKind, FieldError, RunForm, RunSpec, Target};
use crate::docker::model::ResourceKind;
use crate::docker::service::PullOrigin;
use gpui::{Pixels, Point};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub error: bool,
}

#[derive(Clone, Debug)]
pub struct ConfirmState {
    pub kind: ResourceKind,
    pub id: String,
    pub title: String,
    pub tag_count: usize,
    pub force: bool,
    pub submitting: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RunState {
    pub form: RunForm,
    pub field_errors: Vec<FieldError>,
    pub daemon_error: Option<String>,
    pub missing_image: bool,
    pub submitting: bool,
    pub submitted: Option<RunSpec>,
    pub pull_then_retry: bool,
    pub epoch: u64,
}

impl RunState {
    pub fn retry_spec(&self) -> Option<RunSpec> {
        if self.pull_then_retry {
            self.submitted.clone()
        } else {
            None
        }
    }

    /// Starts the retry attempt: freezes the form as submitting, clears the
    /// stale error, and returns the exact spec to send.
    pub fn begin_retry(&mut self, epoch: u64) -> Option<RunSpec> {
        if !self.pull_then_retry {
            return None;
        }
        let spec = self.submitted.clone()?;
        self.pull_then_retry = false;
        self.daemon_error = None;
        self.missing_image = false;
        self.submitting = true;
        self.epoch = epoch;
        Some(spec)
    }

    /// The form is frozen while the daemon is creating the container and while
    /// a retry pull is in flight, so what is visible is what will run.
    pub fn is_frozen(&self, pulling: bool) -> bool {
        self.submitting || (self.pull_then_retry && pulling)
    }
}

#[derive(Clone, Debug)]
pub struct PullState {
    pub origin: PullOrigin,
    pub reference: String,
    pub status: String,
    pub layers: BTreeMap<String, (u64, u64)>,
    pub epoch: u64,
}

impl PullState {
    pub fn record(&mut self, layer: Option<String>, detail: Option<(u64, u64)>) {
        if let (Some(layer), Some((current, total))) = (layer, detail) {
            if total > 0 {
                self.layers.insert(layer, (current, total));
            }
        }
    }

    pub fn bytes(&self) -> Option<(u64, u64)> {
        if self.layers.is_empty() {
            return None;
        }
        let mut current = 0;
        let mut total = 0;
        for (layer_current, layer_total) in self.layers.values() {
            current += layer_current;
            total += layer_total;
        }
        Some((current, total))
    }

    pub fn fraction(&self) -> Option<f32> {
        self.bytes()
            .filter(|(_, total)| *total > 0)
            .map(|(current, total)| (current as f32 / total as f32).clamp(0.0, 1.0))
    }
}

#[derive(Clone, Debug)]
pub struct RowMenuState {
    pub kind: ResourceKind,
    pub id: String,
    pub at: Point<Pixels>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionOutcome {
    pub refetch: bool,
    pub close_detail: bool,
    pub toast: Option<Toast>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullOutcome {
    pub retry_run: bool,
    pub refetch: bool,
    pub toast: Option<Toast>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunOutcome {
    pub close_dialog: bool,
    pub select_container: Option<String>,
    pub refetch: bool,
    pub toast: Option<Toast>,
}

#[derive(Default)]
pub struct Mutations {
    pub pending: BTreeMap<(ResourceKind, String), PendingAction>,
    pub confirm: Option<ConfirmState>,
    pub run: Option<RunState>,
    pub pull: Option<PullState>,
    pub pull_open: bool,
    pub pull_input: String,
    pub pull_error: Option<String>,
    pub toast: Option<Toast>,
    pub hovered_row: Option<String>,
    pub row_menu: Option<RowMenuState>,
    pub pending_select: Option<String>,
    /// Bumped whenever in-flight state is abandoned, so a late response from
    /// the previous attempt can never clear or complete the next one.
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingAction {
    pub action: ActionKind,
    pub epoch: u64,
}

fn key(target: &Target) -> (ResourceKind, String) {
    (target.kind, target.id.clone())
}

impl Mutations {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_in_flight(&self) -> bool {
        !self.pending.is_empty()
            || self.pull.is_some()
            || self.confirm.as_ref().is_some_and(|confirm| confirm.submitting)
            || self.run.as_ref().is_some_and(|run| run.submitting)
    }

    pub fn clear_in_flight(&mut self) {
        self.pending.clear();
        self.pull = None;
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(confirm) = &mut self.confirm {
            confirm.submitting = false;
        }
        if let Some(run) = &mut self.run {
            run.submitting = false;
            run.pull_then_retry = false;
        }
    }

    pub fn is_pending(&self, target: &Target) -> bool {
        self.pending.contains_key(&key(target))
    }


    /// Returns false when this target already has an action in flight, so a
    /// double click never queues the same mutation twice.
    pub fn begin(&mut self, target: &Target, action: ActionKind) -> bool {
        let key = key(target);
        if self.pending.contains_key(&key) {
            return false;
        }
        self.pending.insert(
            key,
            PendingAction {
                action,
                epoch: self.epoch,
            },
        );
        true
    }

    pub fn open_confirm(&mut self, target: Target, title: String, tag_count: usize) {
        self.confirm = Some(ConfirmState {
            kind: target.kind,
            id: target.id,
            title,
            tag_count,
            force: false,
            submitting: false,
            error: None,
        });
    }

    pub fn apply_action_done(
        &mut self,
        target: Target,
        action: ActionKind,
        epoch: u64,
        error: Option<String>,
    ) -> ActionOutcome {
        let key = key(&target);
        let expected = PendingAction { action, epoch };
        if self.pending.get(&key) != Some(&expected) {
            return ActionOutcome {
                refetch: error.is_none(),
                ..Default::default()
            };
        }
        self.pending.remove(&key);

        if action == ActionKind::Remove {
            let confirm_matches = self
                .confirm
                .as_ref()
                .is_some_and(|confirm| confirm.kind == target.kind && confirm.id == target.id);
            if confirm_matches {
                if let Some(message) = error {
                    if let Some(confirm) = &mut self.confirm {
                        confirm.submitting = false;
                        confirm.error = Some(message);
                    }
                    return ActionOutcome::default();
                }
                self.confirm = None;
                return ActionOutcome {
                    refetch: true,
                    close_detail: true,
                    toast: None,
                };
            }
        }

        match error {
            Some(message) => ActionOutcome {
                refetch: false,
                close_detail: false,
                toast: Some(Toast {
                    text: message,
                    error: true,
                }),
            },
            None => ActionOutcome {
                refetch: true,
                close_detail: action == ActionKind::Remove,
                toast: None,
            },
        }
    }

    pub fn apply_pull_progress(
        &mut self,
        origin: PullOrigin,
        reference: &str,
        epoch: u64,
        layer: Option<String>,
        status: String,
        detail: Option<(u64, u64)>,
    ) -> bool {
        let Some(pull) = &mut self.pull else {
            return false;
        };
        if pull.origin != origin || pull.reference != reference || pull.epoch != epoch {
            return false;
        }
        pull.status = status;
        pull.record(layer, detail);
        true
    }

    pub fn apply_pull_done(
        &mut self,
        origin: PullOrigin,
        reference: &str,
        epoch: u64,
        error: Option<String>,
    ) -> PullOutcome {
        let ours = self.pull.as_ref().is_some_and(|pull| {
            pull.origin == origin && pull.reference == reference && pull.epoch == epoch
        });
        if !ours {
            return PullOutcome {
                refetch: error.is_none(),
                ..Default::default()
            };
        }
        self.pull = None;

        let retry_dialog = origin == PullOrigin::RunRetry
            && self.run.as_ref().is_some_and(|run| run.pull_then_retry);
        match (error, retry_dialog) {
            (None, true) => PullOutcome {
                retry_run: true,
                refetch: false,
                toast: None,
            },
            (None, false) => PullOutcome {
                retry_run: false,
                refetch: true,
                toast: None,
            },
            (Some(message), true) => {
                if let Some(run) = &mut self.run {
                    run.daemon_error = Some(message);
                    run.submitting = false;
                    run.pull_then_retry = false;
                }
                PullOutcome::default()
            }
            (Some(message), false) => {
                self.pull_input = reference.to_string();
                self.pull_error = Some(message.clone());
                PullOutcome {
                    retry_run: false,
                    refetch: true,
                    toast: Some(Toast {
                        text: message,
                        error: true,
                    }),
                }
            }
        }
    }

    pub fn apply_run_created(
        &mut self,
        id: String,
        warnings: Vec<String>,
        epoch: u64,
    ) -> RunOutcome {
        let applies = self
            .run
            .as_ref()
            .is_some_and(|run| run.submitting && run.epoch == epoch);
        if !applies {
            return RunOutcome {
                refetch: true,
                ..Default::default()
            };
        }
        self.run = None;
        RunOutcome {
            close_dialog: true,
            select_container: Some(id),
            refetch: true,
            toast: (!warnings.is_empty()).then(|| Toast {
                text: warnings.join(" "),
                error: false,
            }),
        }
    }

    pub fn apply_run_failed(
        &mut self,
        message: String,
        missing_image: bool,
        created: Option<String>,
        epoch: u64,
    ) -> RunOutcome {
        let applies = self
            .run
            .as_ref()
            .is_some_and(|run| run.submitting && run.epoch == epoch);
        if !applies {
            return RunOutcome {
                refetch: created.is_some(),
                ..Default::default()
            };
        }

        if let Some(id) = created {
            self.run = None;
            return RunOutcome {
                close_dialog: true,
                select_container: Some(id),
                refetch: true,
                toast: Some(Toast {
                    text: message,
                    error: true,
                }),
            };
        }

        if let Some(run) = &mut self.run {
            run.daemon_error = Some(message);
            run.missing_image = missing_image;
            run.submitting = false;
        }
        RunOutcome::default()
    }
}

pub fn confirm_copy(kind: ResourceKind, title: &str, tag_count: usize) -> (String, String) {
    match kind {
        ResourceKind::Container => (
            format!("Remove container \"{title}\"?"),
            "This permanently deletes the container and its writable layer. Volumes are not removed."
                .to_string(),
        ),
        ResourceKind::Image => {
            let mut body = "This deletes the image from this host. Containers using it keep running, but cannot be started again.".to_string();
            if tag_count > 1 {
                body.push_str(" This removes every tag for this image.");
            }
            (format!("Remove image \"{title}\"?"), body)
        }
        ResourceKind::Volume => (
            format!("Remove volume \"{title}\"?"),
            "This permanently deletes the volume and its data.".to_string(),
        ),
    }
}

pub fn clamp_menu_position(
    x: f32,
    y: f32,
    menu_width: f32,
    menu_height: f32,
    viewport_width: f32,
    viewport_height: f32,
) -> (f32, f32) {
    (
        x.min((viewport_width - menu_width).max(0.0)).max(0.0),
        y.min((viewport_height - menu_height).max(0.0)).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::ResourceKind;

    fn target(kind: ResourceKind, id: &str) -> Target {
        Target {
            kind,
            id: id.into(),
        }
    }

    fn run_state(submitting: bool, pull_then_retry: bool) -> RunState {
        RunState {
            submitting,
            pull_then_retry,
            ..Default::default()
        }
    }

    fn pull_state(origin: PullOrigin, reference: &str) -> PullState {
        PullState {
            origin,
            reference: reference.into(),
            status: String::new(),
            layers: Default::default(),
            epoch: 0,
        }
    }

    #[test]
    fn begin_blocks_a_second_action_on_the_same_target() {
        let mut m = Mutations::new();
        assert!(m.begin(&target(ResourceKind::Container, "a"), ActionKind::Stop));
        assert!(!m.begin(&target(ResourceKind::Container, "a"), ActionKind::Stop));
        assert!(m.begin(&target(ResourceKind::Container, "b"), ActionKind::Stop));
        assert!(m.has_in_flight());
    }

    #[test]
    fn a_late_action_never_clears_a_newer_pending_action() {
        let mut m = Mutations::new();
        assert!(m.begin(&target(ResourceKind::Container, "a"), ActionKind::Start));
        let late = m.apply_action_done(
            target(ResourceKind::Container, "a"),
            ActionKind::Stop,
            m.epoch,
            Some("gone".into()),
        );
        assert!(!late.refetch && late.toast.is_none() && !m.pending.is_empty());
        assert!(m.confirm.is_none());
    }

    #[test]
    fn remove_errors_stay_in_the_confirm_dialog() {
        let mut m = Mutations::new();
        m.open_confirm(target(ResourceKind::Container, "a"), "web".into(), 0);
        m.confirm.as_mut().unwrap().submitting = true;
        assert!(m.begin(&target(ResourceKind::Container, "a"), ActionKind::Remove));
        let out = m.apply_action_done(
            target(ResourceKind::Container, "a"),
            ActionKind::Remove,
            m.epoch,
            Some("is running".into()),
        );
        assert!(out.toast.is_none());
        assert_eq!(
            m.confirm.as_ref().unwrap().error.as_deref(),
            Some("is running")
        );
        assert!(!m.confirm.as_ref().unwrap().submitting);
        assert!(m.pending.is_empty());
    }

    #[test]
    fn stale_pull_updates_are_ignored() {
        let mut m = Mutations::new();
        m.pull = Some(pull_state(PullOrigin::Header, "nginx"));
        let epoch = m.pull.as_ref().unwrap().epoch;
        assert!(m.apply_pull_progress(
            PullOrigin::Header,
            "nginx",
            epoch,
            Some("l1".into()),
            "Downloading".into(),
            Some((1, 10))
        ));
        assert!(!m.apply_pull_progress(
            PullOrigin::Header,
            "redis",
            epoch,
            Some("l1".into()),
            "Downloading".into(),
            Some((1, 10))
        ));
        assert!(!m.apply_pull_progress(
            PullOrigin::RunRetry,
            "nginx",
            epoch,
            Some("l1".into()),
            "Downloading".into(),
            Some((1, 10))
        ));
        let stale_error = m.apply_pull_done(
            PullOrigin::Header,
            "redis",
            epoch,
            Some("boom".into()),
        );
        assert!(!stale_error.refetch && stale_error.toast.is_none() && m.pull.is_some());
        let stale_success = m.apply_pull_done(PullOrigin::Header, "redis", epoch, None);
        assert!(
            stale_success.refetch && m.pull.is_some(),
            "a late success still refetches"
        );
    }

    #[test]
    fn pull_totals_dedupe_layers_and_ignore_missing_totals() {
        let mut p = pull_state(PullOrigin::Header, "nginx");
        p.record(Some("a".into()), Some((10, 100)));
        p.record(Some("a".into()), Some((40, 100)));
        p.record(Some("b".into()), Some((20, 50)));
        p.record(None, Some((999, 999)));
        p.record(Some("c".into()), None);
        assert_eq!(p.bytes(), Some((60, 150)));
        assert!((p.fraction().unwrap() - 0.4).abs() < 0.001);
    }

    #[test]
    fn pull_failure_reopens_nothing_and_keeps_the_reference() {
        let mut m = Mutations::new();
        m.pull = Some(pull_state(PullOrigin::Header, "bad:1"));
        let out = m.apply_pull_done(
            PullOrigin::Header,
            "bad:1",
            m.pull.as_ref().unwrap().epoch,
            Some("manifest unknown".into()),
        );
        assert!(out.refetch);
        assert!(!out.retry_run);
        assert_eq!(out.toast.unwrap().text, "manifest unknown");
        assert_eq!(
            m.pull_input, "bad:1",
            "the reference is kept for a one-click retry"
        );
        assert_eq!(m.pull_error.as_deref(), Some("manifest unknown"));
        assert!(m.pull.is_none());
    }

    #[test]
    fn a_pull_retry_resumes_the_open_run_dialog() {
        let mut m = Mutations::new();
        m.run = Some(RunState {
            daemon_error: Some("No such image".into()),
            missing_image: true,
            ..run_state(true, true)
        });
        m.pull = Some(pull_state(PullOrigin::RunRetry, "nginx"));
        let epoch = m.pull.as_ref().unwrap().epoch;
        let out = m.apply_pull_done(PullOrigin::RunRetry, "nginx", epoch, None);
        assert!(out.retry_run && !out.refetch);
        assert!(m.run.as_ref().unwrap().submitting);
        assert!(m.pull.is_none());

        let mut failed = Mutations::new();
        failed.run = Some(RunState {
            missing_image: true,
            ..run_state(true, true)
        });
        failed.pull = Some(pull_state(PullOrigin::RunRetry, "nginx"));
        let epoch = failed.pull.as_ref().unwrap().epoch;
        let out = failed.apply_pull_done(
            PullOrigin::RunRetry,
            "nginx",
            epoch,
            Some("manifest unknown".into()),
        );
        assert!(!out.retry_run && !out.refetch && out.toast.is_none());
        let run = failed.run.as_ref().unwrap();
        assert!(!run.submitting && !run.pull_then_retry);
        assert_eq!(run.daemon_error.as_deref(), Some("manifest unknown"));
    }

    #[test]
    fn run_outcomes_select_the_created_container() {
        let mut m = Mutations::new();
        m.run = Some(run_state(true, false));
        let out = m.apply_run_created("abc".into(), vec![], m.epoch);
        assert!(out.close_dialog && out.refetch);
        assert_eq!(out.select_container.as_deref(), Some("abc"));
        assert!(m.run.is_none());

        let mut m = Mutations::new();
        m.run = Some(run_state(true, false));
        let out = m.apply_run_failed(
            "port is allocated".into(),
            false,
            Some("abc".into()),
            m.epoch,
        );
        assert!(out.close_dialog && out.refetch);
        assert_eq!(out.select_container.as_deref(), Some("abc"));
        assert_eq!(out.toast.unwrap().text, "port is allocated");
    }

    #[test]
    fn a_pull_retry_reuses_the_submitted_spec_not_later_edits() {
        let submitted = RunSpec {
            image: "postgres:18".into(),
            name: None,
            command: None,
            ports: vec![],
            env: vec![],
            mounts: vec![],
        };
        let mut run = RunState {
            form: RunForm {
                image: "edited:latest".into(),
                ..Default::default()
            },
            submitted: Some(submitted.clone()),
            pull_then_retry: true,
            ..Default::default()
        };
        assert_eq!(run.retry_spec(), Some(submitted));
        run.pull_then_retry = false;
        assert_eq!(run.retry_spec(), None);
    }

    #[test]
    fn a_missing_image_stays_in_the_dialog() {
        let mut m = Mutations::new();
        m.run = Some(run_state(true, false));
        let out = m.apply_run_failed("No such image".into(), true, None, m.epoch);
        assert!(!out.close_dialog && !out.refetch && out.toast.is_none());
        assert!(m.run.as_ref().unwrap().missing_image);
        assert!(!m.run.as_ref().unwrap().submitting);
    }

    #[test]
    fn clear_in_flight_empties_everything_pending() {
        let mut m = Mutations::new();
        m.begin(&target(ResourceKind::Container, "a"), ActionKind::Stop);
        m.pull = Some(pull_state(PullOrigin::Header, "x"));
        m.open_confirm(target(ResourceKind::Image, "i"), "img".into(), 1);
        m.confirm.as_mut().unwrap().submitting = true;
        m.clear_in_flight();
        assert!(!m.has_in_flight());
        assert!(m.pending.is_empty() && m.pull.is_none());
    }

    #[test]
    fn confirm_copy_names_every_tag_for_a_multi_tag_image() {
        let (title, body) = confirm_copy(ResourceKind::Image, "postgres:18", 2);
        assert!(title.contains("postgres:18"));
        assert!(body.contains("every tag"));
        let (_, single) = confirm_copy(ResourceKind::Image, "postgres:18", 1);
        assert!(!single.contains("every tag"));
        let (_, volume) = confirm_copy(ResourceKind::Volume, "db-data", 0);
        assert!(volume.contains("data"));
    }

    #[test]
    fn run_retry_sets_submitting_and_returns_the_submitted_spec() {
        let submitted = RunSpec {
            image: "postgres:19".into(),
            ..Default::default()
        };
        let mut run = RunState {
            form: RunForm {
                image: "edited:latest".into(),
                ..Default::default()
            },
            submitted: Some(submitted.clone()),
            pull_then_retry: true,
            daemon_error: Some("No such image".into()),
            missing_image: true,
            ..Default::default()
        };
        assert_eq!(run.begin_retry(7), Some(submitted));
        assert!(run.submitting);
        assert!(!run.pull_then_retry);
        assert!(run.daemon_error.is_none());
        assert!(!run.missing_image);
        assert_eq!(run.epoch, 7);
        assert_eq!(run.begin_retry(8), None, "a second retry must not start");
    }

    #[test]
    fn run_is_frozen_while_submitting_or_retry_pulling() {
        let mut run = RunState::default();
        assert!(!run.is_frozen(false));
        run.submitting = true;
        assert!(run.is_frozen(false));
        let run = RunState {
            pull_then_retry: true,
            ..Default::default()
        };
        assert!(run.is_frozen(true));
        assert!(!run.is_frozen(false));
    }

    #[test]
    fn disconnect_epoch_ignores_a_late_response_for_the_same_action() {
        let mut m = Mutations::new();
        let target = target(ResourceKind::Container, "a");
        assert!(m.begin(&target, ActionKind::Stop));
        let old_epoch = m.epoch;
        m.clear_in_flight();
        assert!(m.begin(&target, ActionKind::Stop));

        let late = m.apply_action_done(
            target.clone(),
            ActionKind::Stop,
            old_epoch,
            Some("old".into()),
        );
        assert!(!late.refetch && late.toast.is_none());
        assert!(
            m.pending.contains_key(&(ResourceKind::Container, "a".to_string())),
            "the late response must not clear the new attempt"
        );

        let current = m.apply_action_done(target, ActionKind::Stop, m.epoch, None);
        assert!(current.refetch && m.pending.is_empty());
    }

    #[test]
    fn a_pull_from_a_previous_epoch_is_ignored() {
        let mut m = Mutations::new();
        m.pull = Some(PullState {
            epoch: 4,
            ..pull_state(PullOrigin::Header, "nginx")
        });
        assert!(!m.apply_pull_progress(
            PullOrigin::Header,
            "nginx",
            3,
            Some("l".into()),
            "Downloading".into(),
            Some((1, 2))
        ));
        assert!(m.apply_pull_progress(
            PullOrigin::Header,
            "nginx",
            4,
            Some("l".into()),
            "Downloading".into(),
            Some((1, 2))
        ));
        let stale = m.apply_pull_done(PullOrigin::Header, "nginx", 3, None);
        assert!(stale.refetch && m.pull.is_some());
    }

    #[test]
    fn a_run_result_from_a_previous_epoch_is_ignored() {
        let mut m = Mutations::new();
        m.run = Some(RunState {
            submitting: true,
            epoch: 5,
            ..Default::default()
        });
        let out = m.apply_run_created("old".into(), vec![], 4);
        assert!(out.refetch && out.select_container.is_none());
        assert!(m.run.is_some(), "the current dialog must stay open");
    }

    #[test]
    fn a_run_result_with_no_dialog_only_refetches() {
        let mut m = Mutations::new();
        let out = m.apply_run_created("old".into(), vec![], 0);
        assert!(out.refetch && out.select_container.is_none() && !out.close_dialog);
    }

    #[test]
    fn menu_position_is_clamped_to_the_viewport() {
        assert_eq!(
            clamp_menu_position(10.0, 10.0, 200.0, 240.0, 800.0, 600.0),
            (10.0, 10.0)
        );
        assert_eq!(
            clamp_menu_position(700.0, 500.0, 200.0, 240.0, 800.0, 600.0),
            (600.0, 360.0)
        );
        assert_eq!(
            clamp_menu_position(900.0, 900.0, 200.0, 240.0, 100.0, 100.0),
            (0.0, 0.0)
        );
    }
}
