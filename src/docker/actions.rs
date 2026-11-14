use crate::docker::model::{ContainerState, ResourceKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Start,
    Stop,
    Restart,
    Remove,
}

impl ActionKind {
    pub fn verb(self) -> &'static str {
        match self {
            ActionKind::Start => "Start",
            ActionKind::Stop => "Stop",
            ActionKind::Restart => "Restart",
            ActionKind::Remove => "Remove",
        }
    }

    pub fn pending_label(self) -> &'static str {
        match self {
            ActionKind::Start => "Starting…",
            ActionKind::Stop => "Stopping…",
            ActionKind::Restart => "Restarting…",
            ActionKind::Remove => "Removing…",
        }
    }

    pub fn for_container(action: ContainerAction) -> Self {
        match action {
            ContainerAction::Start => ActionKind::Start,
            ContainerAction::Stop => ActionKind::Stop,
            ContainerAction::Restart => ActionKind::Restart,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: ResourceKind,
    pub id: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContainerActions {
    pub start: bool,
    pub stop: bool,
    pub restart: bool,
    pub remove: bool,
}

pub fn container_actions(state: ContainerState) -> ContainerActions {
    match state {
        ContainerState::Running | ContainerState::Paused | ContainerState::Restarting => {
            ContainerActions {
                start: false,
                stop: true,
                restart: true,
                remove: true,
            }
        }
        ContainerState::Exited | ContainerState::Created => ContainerActions {
            start: true,
            stop: false,
            restart: false,
            remove: true,
        },
        ContainerState::Dead => ContainerActions {
            start: false,
            stop: false,
            restart: false,
            remove: true,
        },
        ContainerState::Unknown => ContainerActions {
            start: true,
            stop: true,
            restart: true,
            remove: true,
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunError {
    pub message: String,
    pub missing_image: bool,
    pub created: Option<String>,
}

pub fn clean_error(err: &bollard::errors::Error) -> String {
    match err {
        bollard::errors::Error::DockerResponseServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

pub fn classify_run_error(err: &bollard::errors::Error, created: Option<String>) -> RunError {
    let missing_image = match err {
        bollard::errors::Error::DockerResponseServerError {
            status_code,
            message,
        } => *status_code == 404 || message.contains("No such image"),
        _ => false,
    };
    RunError {
        message: clean_error(err),
        missing_image,
        created,
    }
}

/// Splits `repo[:tag]` at the last colon after the final slash, so a registry
/// port is never mistaken for a tag. Digests keep the whole reference and no tag.
pub fn split_image_reference(reference: &str) -> (String, Option<String>) {
    let reference = reference.trim();
    if reference.is_empty() {
        return (String::new(), None);
    }
    if reference.contains('@') {
        return (reference.to_string(), None);
    }

    let after_slash = reference.rfind('/').map(|index| index + 1).unwrap_or(0);
    match reference[after_slash..].rfind(':') {
        Some(offset) => {
            let colon = after_slash + offset;
            let tag = &reference[colon + 1..];
            let tag = if tag.is_empty() { "latest" } else { tag };
            (reference[..colon].to_string(), Some(tag.to_string()))
        }
        None => (reference.to_string(), Some("latest".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::ContainerState;

    #[test]
    fn availability_follows_the_state_table() {
        let running = container_actions(ContainerState::Running);
        assert!(running.stop && running.restart && running.remove && !running.start);
        let exited = container_actions(ContainerState::Exited);
        assert!(exited.start && exited.remove && !exited.stop && !exited.restart);
        let created = container_actions(ContainerState::Created);
        assert!(created.start && created.remove && !created.stop);
        let dead = container_actions(ContainerState::Dead);
        assert!(dead.remove && !dead.start && !dead.stop && !dead.restart);
        let unknown = container_actions(ContainerState::Unknown);
        assert!(unknown.start && unknown.stop && unknown.restart && unknown.remove);
        assert!(container_actions(ContainerState::Paused).stop);
        assert!(container_actions(ContainerState::Restarting).restart);
    }

    #[test]
    fn clean_error_returns_the_daemon_message_without_the_status_prefix() {
        let err = bollard::errors::Error::DockerResponseServerError {
            status_code: 409,
            message: "container is running: stop it before removing".into(),
        };
        assert_eq!(
            clean_error(&err),
            "container is running: stop it before removing"
        );

        let io = bollard::errors::Error::IOError {
            err: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"),
        };
        assert!(clean_error(&io).contains("refused"));
    }

    #[test]
    fn classify_run_error_flags_a_missing_image_and_keeps_the_created_id() {
        let missing = bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            message: "No such image: postgres:19".into(),
        };
        let out = classify_run_error(&missing, None);
        assert!(out.missing_image);
        assert!(out.created.is_none());

        let started = bollard::errors::Error::DockerResponseServerError {
            status_code: 409,
            message: "port is already allocated".into(),
        };
        let out = classify_run_error(&started, Some("abc".into()));
        assert!(!out.missing_image);
        assert_eq!(out.created.as_deref(), Some("abc"));
        assert!(out.message.contains("port is already allocated"));
    }

    #[test]
    fn splits_image_references_into_repo_and_tag() {
        assert_eq!(
            split_image_reference("nginx"),
            ("nginx".to_string(), Some("latest".to_string()))
        );
        assert_eq!(
            split_image_reference("nginx:alpine"),
            ("nginx".to_string(), Some("alpine".to_string()))
        );
        assert_eq!(
            split_image_reference("registry:5000/team/app:1.2"),
            ("registry:5000/team/app".to_string(), Some("1.2".to_string()))
        );
        assert_eq!(
            split_image_reference("registry:5000/team/app"),
            ("registry:5000/team/app".to_string(), Some("latest".to_string()))
        );
        assert_eq!(
            split_image_reference("app@sha256:abc"),
            ("app@sha256:abc".to_string(), None)
        );
    }
}
