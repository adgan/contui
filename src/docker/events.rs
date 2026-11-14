use crate::docker::model::ResourceKind;
use bollard::models::{EventMessage, EventMessageTypeEnum};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// Maps a daemon event to the resource list it invalidates, plus the specific id
/// when the event names one.
pub fn classify(event: &EventMessage) -> Option<(ResourceKind, Option<String>)> {
    let kind = match event.typ {
        Some(EventMessageTypeEnum::CONTAINER) => ResourceKind::Container,
        Some(EventMessageTypeEnum::IMAGE) => ResourceKind::Image,
        Some(EventMessageTypeEnum::VOLUME) => ResourceKind::Volume,
        _ => return None,
    };
    let id = event.actor.as_ref().and_then(|a| a.id.clone());
    Some((kind, id))
}

/// Collects dirty resources and releases them once the daemon has been quiet for
/// `DEBOUNCE`, so a burst of events causes one refetch.
#[derive(Debug, Default)]
pub struct Debouncer {
    pending: BTreeSet<ResourceKind>,
    ids: BTreeSet<(ResourceKind, String)>,
    last_event: Option<Instant>,
}

impl Debouncer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, event: &EventMessage, now: Instant) {
        if let Some((kind, id)) = classify(event) {
            self.pending.insert(kind);
            if let Some(id) = id {
                self.ids.insert((kind, id));
            }
            self.last_event = Some(now);
        }
    }

    pub fn take_if_due(
        &mut self,
        now: Instant,
    ) -> Option<(Vec<ResourceKind>, Vec<(ResourceKind, String)>)> {
        if self.pending.is_empty() {
            return None;
        }
        let last = self.last_event?;
        if now.duration_since(last) < DEBOUNCE {
            return None;
        }
        let kinds: Vec<ResourceKind> = self.pending.iter().copied().collect();
        let ids: Vec<(ResourceKind, String)> = self.ids.iter().cloned().collect();
        self.pending.clear();
        self.ids.clear();
        self.last_event = None;
        Some((kinds, ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{EventMessage, EventMessageTypeEnum};
    use std::time::{Duration, Instant};

    fn container_event(action: &str, id: &str) -> EventMessage {
        EventMessage {
            typ: Some(EventMessageTypeEnum::CONTAINER),
            action: Some(action.to_string()),
            actor: Some(bollard::models::EventActor {
                id: Some(id.to_string()),
                attributes: None,
            }),
            scope: None,
            time: None,
            time_nano: None,
        }
    }

    fn typed_event(typ: EventMessageTypeEnum, action: &str) -> EventMessage {
        EventMessage {
            typ: Some(typ),
            action: Some(action.to_string()),
            actor: Some(bollard::models::EventActor {
                id: Some("x".to_string()),
                attributes: None,
            }),
            scope: None,
            time: None,
            time_nano: None,
        }
    }

    #[test]
    fn classifies_container_start_and_die_events() {
        for action in ["start", "die", "destroy", "stop"] {
            let event = container_event(action, "abc");
            assert_eq!(
                classify(&event),
                Some((ResourceKind::Container, Some("abc".to_string()))),
                "action {action}"
            );
        }
    }

    #[test]
    fn classifies_image_and_volume_events_but_ignores_network() {
        assert_eq!(
            classify(&typed_event(EventMessageTypeEnum::IMAGE, "tag")),
            Some((ResourceKind::Image, Some("x".to_string())))
        );
        assert_eq!(
            classify(&typed_event(EventMessageTypeEnum::VOLUME, "create")),
            Some((ResourceKind::Volume, Some("x".to_string())))
        );
        assert_eq!(
            classify(&typed_event(EventMessageTypeEnum::NETWORK, "connect")),
            None
        );
        assert_eq!(
            classify(&typed_event(EventMessageTypeEnum::DAEMON, "reload")),
            None
        );
    }

    #[test]
    fn debouncer_flushes_once_after_250ms() {
        let mut d = Debouncer::new();
        let t0 = Instant::now();
        d.record(&container_event("start", "a"), t0);
        d.record(&container_event("start", "b"), t0 + Duration::from_millis(100));
        assert!(d.take_if_due(t0 + Duration::from_millis(200)).is_none());
        assert!(d.take_if_due(t0 + Duration::from_millis(300)).is_none());
        let (kinds, ids) = d
            .take_if_due(t0 + Duration::from_millis(351))
            .expect("flush");
        assert_eq!(kinds, vec![ResourceKind::Container]);
        assert_eq!(ids.len(), 2);
        assert!(d.take_if_due(t0 + Duration::from_millis(400)).is_none());
    }
}
