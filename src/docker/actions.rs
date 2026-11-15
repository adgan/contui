use crate::docker::model::{ContainerState, ResourceKind};
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use std::collections::{HashMap, HashSet};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Proto {
    #[default]
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortForm {
    pub host: String,
    pub container: String,
    pub proto: Proto,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvForm {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountForm {
    pub source: String,
    pub target: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunForm {
    pub image: String,
    pub name: String,
    pub command: String,
    pub ports: Vec<PortForm>,
    pub env: Vec<EnvForm>,
    pub mounts: Vec<MountForm>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortSpec {
    pub host: Option<u16>,
    pub container: u16,
    pub udp: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountSpec {
    pub source: String,
    pub target: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunSpec {
    pub image: String,
    pub name: Option<String>,
    pub command: Option<Vec<String>>,
    pub ports: Vec<PortSpec>,
    pub env: Vec<(String, String)>,
    pub mounts: Vec<MountSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunField {
    Image,
    Name,
    Command,
    PortHost(usize),
    PortContainer(usize),
    EnvKey(usize),
    EnvValue(usize),
    MountSource(usize),
    MountTarget(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    pub field: RunField,
    pub message: String,
}

fn valid_name(name: &str) -> bool {
    if name.len() > 128 {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn parse_port(raw: &str) -> Option<u16> {
    raw.trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port >= 1)
}

fn port_key(container: u16, udp: bool) -> String {
    format!("{}/{}", container, if udp { "udp" } else { "tcp" })
}

/// Splits a command line the way `docker run` does for the surface we expose:
/// whitespace outside quotes, `'…'` and `"…"` grouping, `\` escaping, and
/// adjacent segments concatenating. No expansion, globbing, or shell.
pub fn split_command(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars();

    while let Some(c) = chars.next() {
        match quote {
            Some(open) => {
                if c == open {
                    quote = None;
                    started = true;
                } else if c == '\\' && open == '"' {
                    match chars.next() {
                        Some(escaped) => current.push(escaped),
                        None => current.push('\\'),
                    }
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    started = true;
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                c => {
                    current.push(c);
                    started = true;
                }
            },
        }
    }

    if quote.is_some() {
        return Err("Unterminated quote".to_string());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

pub fn build_run_spec(form: &RunForm) -> Result<RunSpec, Vec<FieldError>> {
    let mut errors = Vec::new();
    let spec = RunSpec {
        image: validate_image(form, &mut errors),
        name: validate_name(form, &mut errors),
        command: validate_command(form, &mut errors),
        ports: validate_ports(form, &mut errors),
        env: validate_env(form, &mut errors),
        mounts: validate_mounts(form, &mut errors),
    };

    if errors.is_empty() {
        Ok(spec)
    } else {
        Err(errors)
    }
}

fn validate_image(form: &RunForm, errors: &mut Vec<FieldError>) -> String {
    let image = form.image.trim();
    if image.is_empty() {
        errors.push(FieldError {
            field: RunField::Image,
            message: "Image is required".to_string(),
        });
    }
    image.to_string()
}

fn validate_name(form: &RunForm, errors: &mut Vec<FieldError>) -> Option<String> {
    let name = form.name.trim();
    if !name.is_empty() && !valid_name(name) {
        errors.push(FieldError {
            field: RunField::Name,
            message: "Name may use letters, digits, ., _, - and must start with a letter or digit"
                .to_string(),
        });
    }
    (!name.is_empty()).then(|| name.to_string())
}

fn validate_command(form: &RunForm, errors: &mut Vec<FieldError>) -> Option<Vec<String>> {
    if form.command.trim().is_empty() {
        return None;
    }
    match split_command(&form.command) {
        Ok(args) => Some(args),
        Err(message) => {
            errors.push(FieldError {
                field: RunField::Command,
                message,
            });
            None
        }
    }
}

fn validate_ports(form: &RunForm, errors: &mut Vec<FieldError>) -> Vec<PortSpec> {
    let mut ports = Vec::new();
    let mut host_ports = HashSet::new();
    let mut container_ports = HashSet::new();
    for (index, port) in form.ports.iter().enumerate() {
        let mut valid = true;
        let host = if port.host.trim().is_empty() {
            None
        } else {
            match parse_port(&port.host) {
                Some(value) => Some(value),
                None => {
                    errors.push(FieldError {
                        field: RunField::PortHost(index),
                        message: "Host port must be 1-65535".to_string(),
                    });
                    valid = false;
                    None
                }
            }
        };
        let container = match parse_port(&port.container) {
            Some(value) => value,
            None => {
                errors.push(FieldError {
                    field: RunField::PortContainer(index),
                    message: "Container port must be 1-65535".to_string(),
                });
                valid = false;
                0
            }
        };
        let udp = port.proto == Proto::Udp;

        if let Some(value) = host
            && !host_ports.insert((value, udp))
        {
            errors.push(FieldError {
                field: RunField::PortHost(index),
                message: "Host port is used twice".to_string(),
            });
            valid = false;
        }
        if container != 0 && !container_ports.insert((container, udp)) {
            errors.push(FieldError {
                field: RunField::PortContainer(index),
                message: "Container port is used twice".to_string(),
            });
            valid = false;
        }
        if valid {
            ports.push(PortSpec {
                host,
                container,
                udp,
            });
        }
    }
    ports
}

fn validate_env(form: &RunForm, errors: &mut Vec<FieldError>) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let mut env_keys = HashSet::new();
    for (index, variable) in form.env.iter().enumerate() {
        let key = variable.key.trim();
        let mut valid = true;
        if key.is_empty() {
            errors.push(FieldError {
                field: RunField::EnvKey(index),
                message: "Variable name is required".to_string(),
            });
            valid = false;
        } else {
            if key.contains('=') {
                errors.push(FieldError {
                    field: RunField::EnvKey(index),
                    message: "Variable name cannot contain =".to_string(),
                });
                valid = false;
            }
            if !env_keys.insert(key.to_string()) {
                errors.push(FieldError {
                    field: RunField::EnvKey(index),
                    message: "Variable name is used twice".to_string(),
                });
                valid = false;
            }
        }
        if valid {
            env.push((key.to_string(), variable.value.clone()));
        }
    }
    env
}

fn validate_mounts(form: &RunForm, errors: &mut Vec<FieldError>) -> Vec<MountSpec> {
    let mut mounts = Vec::new();
    for (index, mount) in form.mounts.iter().enumerate() {
        let source = mount.source.trim();
        let target = mount.target.trim();
        let mut valid = true;
        if source.is_empty() {
            errors.push(FieldError {
                field: RunField::MountSource(index),
                message: "Volume source is required".to_string(),
            });
            valid = false;
        } else if source.contains(':') {
            errors.push(FieldError {
                field: RunField::MountSource(index),
                message: "Volume source cannot contain :".to_string(),
            });
            valid = false;
        }

        if !target.starts_with('/') {
            errors.push(FieldError {
                field: RunField::MountTarget(index),
                message: "Container path must be absolute".to_string(),
            });
            valid = false;
        } else if target.contains(':') {
            errors.push(FieldError {
                field: RunField::MountTarget(index),
                message: "Container path cannot contain :".to_string(),
            });
            valid = false;
        }

        if valid {
            mounts.push(MountSpec {
                source: source.to_string(),
                target: target.to_string(),
                read_only: mount.read_only,
            });
        }
    }
    mounts
}

pub fn run_body(spec: &RunSpec) -> ContainerCreateBody {
    let mut host = HostConfig::default();

    if !spec.ports.is_empty() {
        let mut bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for port in &spec.ports {
            bindings.insert(
                port_key(port.container, port.udp),
                Some(vec![PortBinding {
                    host_ip: None,
                    host_port: Some(
                        port.host
                            .map(|value| value.to_string())
                            .unwrap_or_default(),
                    ),
                }]),
            );
        }
        host.port_bindings = Some(bindings);
    }

    if !spec.mounts.is_empty() {
        host.binds = Some(
            spec.mounts
                .iter()
                .map(|mount| {
                    format!(
                        "{}:{}{}",
                        mount.source,
                        mount.target,
                        if mount.read_only { ":ro" } else { "" }
                    )
                })
                .collect(),
        );
    }

    ContainerCreateBody {
        image: Some(spec.image.clone()),
        cmd: spec.command.clone(),
        env: (!spec.env.is_empty()).then(|| {
            spec.env
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect()
        }),
        exposed_ports: (!spec.ports.is_empty()).then(|| {
            spec.ports
                .iter()
                .map(|port| port_key(port.container, port.udp))
                .collect()
        }),
        host_config: Some(host),
        ..Default::default()
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

    #[test]
    fn splits_commands_with_quotes_escapes_and_adjacency() {
        assert_eq!(
            split_command(r#"postgres -c "max_connections=200" 'a b'"#).unwrap(),
            vec!["postgres", "-c", "max_connections=200", "a b"]
        );
        assert_eq!(split_command(r#"a"b c"d"#).unwrap(), vec!["ab cd"]);
        assert_eq!(split_command(r#"one\ two"#).unwrap(), vec!["one two"]);
        assert_eq!(split_command("   ").unwrap(), Vec::<String>::new());
        assert_eq!(split_command(r#""""#).unwrap(), vec![""]);
        assert_eq!(
            split_command(r#"echo "oops"#),
            Err("Unterminated quote".to_string())
        );
    }

    #[test]
    fn builds_a_spec_from_a_full_form() {
        let form = RunForm {
            image: " postgres:18 ".into(),
            name: "db".into(),
            command: r#"postgres -c "max_connections=200""#.into(),
            ports: vec![PortForm {
                host: "5432".into(),
                container: "5432".into(),
                proto: Proto::Tcp,
            }],
            env: vec![EnvForm {
                key: "POSTGRES_PASSWORD".into(),
                value: "secret".into(),
            }],
            mounts: vec![MountForm {
                source: "db-data".into(),
                target: "/var/lib/postgresql/data".into(),
                read_only: true,
            }],
        };
        let spec = build_run_spec(&form).unwrap();
        assert_eq!(spec.image, "postgres:18");
        assert_eq!(spec.name.as_deref(), Some("db"));
        assert_eq!(
            spec.command,
            Some(vec![
                "postgres".into(),
                "-c".into(),
                "max_connections=200".into()
            ])
        );
        assert_eq!(
            spec.ports,
            vec![PortSpec {
                host: Some(5432),
                container: 5432,
                udp: false
            }]
        );
        assert_eq!(
            spec.env,
            vec![("POSTGRES_PASSWORD".into(), "secret".into())]
        );
        assert!(spec.mounts[0].read_only);
    }

    #[test]
    fn reports_every_invalid_field_at_once() {
        let form = RunForm {
            image: " ".into(),
            name: "bad name".into(),
            command: "\"oops".into(),
            ports: vec![
                PortForm {
                    host: "0".into(),
                    container: "70000".into(),
                    proto: Proto::Tcp,
                },
                PortForm {
                    host: "8080".into(),
                    container: "80".into(),
                    proto: Proto::Tcp,
                },
                PortForm {
                    host: "8080".into(),
                    container: "80".into(),
                    proto: Proto::Tcp,
                },
            ],
            env: vec![
                EnvForm {
                    key: "".into(),
                    value: "x".into(),
                },
                EnvForm {
                    key: "A".into(),
                    value: "1".into(),
                },
                EnvForm {
                    key: "A".into(),
                    value: "2".into(),
                },
            ],
            mounts: vec![MountForm {
                source: "".into(),
                target: "relative".into(),
                read_only: false,
            }],
        };
        let fields: Vec<RunField> = build_run_spec(&form)
            .unwrap_err()
            .iter()
            .map(|e| e.field)
            .collect();
        for expected in [
            RunField::Image,
            RunField::Name,
            RunField::Command,
            RunField::PortHost(0),
            RunField::PortContainer(0),
            RunField::PortHost(2),
            RunField::PortContainer(2),
            RunField::EnvKey(0),
            RunField::EnvKey(2),
            RunField::MountSource(0),
            RunField::MountTarget(0),
        ] {
            assert!(
                fields.contains(&expected),
                "missing {expected:?} in {fields:?}"
            );
        }
    }

    #[test]
    fn maps_a_spec_into_a_create_body() {
        let spec = RunSpec {
            image: "postgres:18".into(),
            name: Some("db".into()),
            command: Some(vec!["postgres".into()]),
            ports: vec![
                PortSpec {
                    host: Some(5432),
                    container: 5432,
                    udp: false,
                },
                PortSpec {
                    host: None,
                    container: 53,
                    udp: true,
                },
            ],
            env: vec![("A".into(), "1".into())],
            mounts: vec![MountSpec {
                source: "db-data".into(),
                target: "/data".into(),
                read_only: true,
            }],
        };
        let body = run_body(&spec);
        assert_eq!(body.image.as_deref(), Some("postgres:18"));
        assert_eq!(body.cmd.as_ref().unwrap(), &vec!["postgres".to_string()]);
        assert_eq!(body.env.as_ref().unwrap(), &vec!["A=1".to_string()]);
        assert_eq!(
            body.exposed_ports.as_ref().unwrap(),
            &vec!["5432/tcp".to_string(), "53/udp".to_string()]
        );
        let host = body.host_config.expect("host config");
        let bindings = host.port_bindings.expect("port bindings");
        assert_eq!(
            bindings["5432/tcp"].as_ref().unwrap()[0]
                .host_port
                .as_deref(),
            Some("5432")
        );
        assert_eq!(
            bindings["53/udp"].as_ref().unwrap()[0].host_port.as_deref(),
            Some("")
        );
        assert_eq!(host.binds.unwrap(), vec!["db-data:/data:ro"]);
    }

    #[test]
    fn empty_sections_stay_none() {
        let spec = RunSpec {
            image: "alpine".into(),
            name: None,
            command: None,
            ports: vec![],
            env: vec![],
            mounts: vec![],
        };
        let body = run_body(&spec);
        assert!(body.env.is_none());
        assert!(body.exposed_ports.is_none());
        assert!(body.cmd.is_none());
        let host = body.host_config.expect("host config");
        assert!(host.port_bindings.is_none());
        assert!(host.binds.is_none());
    }
}
