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
}

impl RunState {
    pub fn retry_spec(&self) -> Option<RunSpec> {
        if self.pull_then_retry {
            self.submitted.clone()
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct PullState {
    pub origin: PullOrigin,
    pub reference: String,
    pub status: String,
    pub layers: BTreeMap<String, (u64, u64)>,
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
    pub pending: BTreeMap<(ResourceKind, String), ActionKind>,
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
        self.pending.insert(key, action);
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
        error: Option<String>,
    ) -> ActionOutcome {
        let key = key(&target);
        if self.pending.get(&key) != Some(&action) {
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
        layer: Option<String>,
        status: String,
        detail: Option<(u64, u64)>,
    ) -> bool {
        let Some(pull) = &mut self.pull else {
            return false;
        };
        if pull.origin != origin || pull.reference != reference {
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
        error: Option<String>,
    ) -> PullOutcome {
        let ours = self
            .pull
            .as_ref()
            .is_some_and(|pull| pull.origin == origin && pull.reference == reference);
        if !ours {
            return PullOutcome {
                refetch: error.is_none(),
                ..Default::default()
            };
        }
        self.pull = None;

        let retry_dialog = self
            .run
            .as_ref()
            .is_some_and(|run| run.pull_then_retry);
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

    pub fn apply_run_created(&mut self, id: String, warnings: Vec<String>) -> RunOutcome {
        let applies = self
            .run
            .as_ref()
            .is_some_and(|run| run.submitting);
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
    ) -> RunOutcome {
        let applies = self
            .run
            .as_ref()
            .is_some_and(|run| run.submitting);
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
        assert!(m.apply_pull_progress(
            PullOrigin::Header,
            "nginx",
            Some("l1".into()),
            "Downloading".into(),
            Some((1, 10))
        ));
        assert!(!m.apply_pull_progress(
            PullOrigin::Header,
            "redis",
            Some("l1".into()),
            "Downloading".into(),
            Some((1, 10))
        ));
        assert!(!m.apply_pull_progress(
            PullOrigin::RunRetry,
            "nginx",
            Some("l1".into()),
            "Downloading".into(),
            Some((1, 10))
        ));
        let stale_error = m.apply_pull_done(PullOrigin::Header, "redis", Some("boom".into()));
        assert!(!stale_error.refetch && stale_error.toast.is_none() && m.pull.is_some());
        let stale_success = m.apply_pull_done(PullOrigin::Header, "redis", None);
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
            Some("manifest unknown".into()),
        );
        assert!(out.refetch);
        assert!(!out.retry_run);
        assert_eq!(out.toast.unwrap().text, "manifest unknown");
        assert_eq!(
            m.pull_input, "bad:1",
            "the reference is kept for a one-click retry"
        );
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
        let out = m.apply_pull_done(PullOrigin::RunRetry, "nginx", None);
        assert!(out.retry_run && !out.refetch);
        assert!(m.run.as_ref().unwrap().submitting);
        assert!(m.pull.is_none());

        let mut failed = Mutations::new();
        failed.run = Some(RunState {
            missing_image: true,
            ..run_state(true, true)
        });
        failed.pull = Some(pull_state(PullOrigin::RunRetry, "nginx"));
        let out = failed.apply_pull_done(
            PullOrigin::RunRetry,
            "nginx",
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
        let out = m.apply_run_created("abc".into(), vec![]);
        assert!(out.close_dialog && out.refetch);
        assert_eq!(out.select_container.as_deref(), Some("abc"));
        assert!(m.run.is_none());

        let mut m = Mutations::new();
        m.run = Some(run_state(true, false));
        let out = m.apply_run_failed("port is allocated".into(), false, Some("abc".into()));
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
        let out = m.apply_run_failed("No such image".into(), true, None);
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
}
