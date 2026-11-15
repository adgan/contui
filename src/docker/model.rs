use crate::docker::endpoint::HostStatus;
use bollard::container::LogOutput;
use bollard::models::{
    ContainerSummary, ContainerSummaryStateEnum, ImageSummary, PortSummary, Volume,
};
use chrono::{DateTime, Utc};
use gpui::SharedString;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContainerState {
    Running,
    Paused,
    Restarting,
    Exited,
    Created,
    Dead,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    Container,
    Image,
    Volume,
}

#[derive(Clone, Debug)]
pub struct ContainerRow {
    pub id: String,
    pub name: SharedString,
    pub state: ContainerState,
    pub status: SharedString,
    pub image: SharedString,
    pub ports: SharedString,
    pub created: DateTime<Utc>,
    pub created_label: SharedString,
    /// Docker Compose project name from the container labels, when present.
    pub compose_project: Option<SharedString>,
}

impl ContainerRow {
    pub fn from_summary(summary: ContainerSummary, now: DateTime<Utc>) -> Self {
        let id = summary.id.unwrap_or_default();
        let short_id: String = id.chars().take(12).collect();

        let name = summary
            .names
            .and_then(|names| names.into_iter().next())
            .map(|name| name.trim_start_matches('/').to_string())
            .unwrap_or_else(|| short_id.clone());

        let created = summary
            .created
            .and_then(|secs| DateTime::from_timestamp(secs, 0))
            .unwrap_or(now);

        let compose_project = summary
            .labels
            .as_ref()
            .and_then(|labels| labels.get("com.docker.compose.project"))
            .map(|project| project.trim())
            .filter(|project| !project.is_empty())
            .map(|project| SharedString::from(project.to_string()));

        Self {
            id,
            name: truncate(&name, 48).into(),
            state: container_state(summary.state.unwrap_or(ContainerSummaryStateEnum::EMPTY)),
            status: summary.status.unwrap_or_default().into(),
            image: truncate(&summary.image.unwrap_or_default(), 48).into(),
            ports: format_ports(&summary.ports.unwrap_or_default()).into(),
            created,
            created_label: format_ago(created, now).into(),
            compose_project,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImageRow {
    pub id: String,
    pub short_id: SharedString,
    pub repository_tag: SharedString,
    pub size: i64,
    pub size_label: SharedString,
    pub created: DateTime<Utc>,
    pub created_label: SharedString,
    pub in_use: bool,
    pub tag_count: usize,
}

impl ImageRow {
    pub fn from_summary(image: ImageSummary, in_use: bool, now: DateTime<Utc>) -> Self {
        let tag_count = image.repo_tags.len();
        let repository_tag = image
            .repo_tags
            .first()
            .cloned()
            .unwrap_or_else(|| "<none>".to_string());
        let created = DateTime::from_timestamp(image.created, 0).unwrap_or(now);
        Self {
            short_id: image.id.chars().take(12).collect::<String>().into(),
            id: image.id,
            repository_tag: truncate(&repository_tag, 48).into(),
            size: image.size,
            size_label: format_bytes(image.size).into(),
            created,
            created_label: format_ago(created, now).into(),
            in_use,
            tag_count,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VolumeRow {
    pub name: SharedString,
    pub driver: SharedString,
    pub mountpoint: SharedString,
    pub created_label: SharedString,
}

impl VolumeRow {
    pub fn from_volume(volume: Volume, now: DateTime<Utc>) -> Self {
        let created_label = volume
            .created_at
            .map(|ts| format_ago(ts, now))
            .unwrap_or_else(|| "—".to_string());
        Self {
            name: truncate(&volume.name, 40).into(),
            driver: volume.driver.into(),
            mountpoint: truncate(&volume.mountpoint, 58).into(),
            created_label: created_label.into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub containers: Vec<ContainerRow>,
    pub images: Vec<ImageRow>,
    pub volumes: Vec<VolumeRow>,
    pub fetched_at: Option<DateTime<Utc>>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub stderr: bool,
    pub timestamp: Option<String>,
    pub text: String,
}

pub fn container_state(state: ContainerSummaryStateEnum) -> ContainerState {
    match state {
        ContainerSummaryStateEnum::RUNNING => ContainerState::Running,
        ContainerSummaryStateEnum::PAUSED => ContainerState::Paused,
        ContainerSummaryStateEnum::RESTARTING => ContainerState::Restarting,
        ContainerSummaryStateEnum::EXITED => ContainerState::Exited,
        ContainerSummaryStateEnum::CREATED => ContainerState::Created,
        ContainerSummaryStateEnum::DEAD => ContainerState::Dead,
        _ => ContainerState::Unknown,
    }
}

pub fn format_ago(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3_600)
    } else if secs < 2_592_000 {
        format!("{}d ago", secs / 86_400)
    } else if secs < 31_536_000 {
        format!("{}mo ago", secs / 2_592_000)
    } else {
        format!("{}y ago", secs / 31_536_000)
    }
}

pub fn format_bytes(n: i64) -> String {
    let bytes = n.max(0) as f64;
    let mib = 1_048_576f64;
    if bytes >= 1024.0 * mib {
        format!("{:.1} GB", bytes / (1024.0 * mib))
    } else if bytes >= 100.0 * mib {
        format!("{:.0} MB", bytes / mib)
    } else {
        format!("{:.1} MB", bytes / mib)
    }
}

pub fn format_ports(ports: &[PortSummary]) -> String {
    if ports.is_empty() {
        return "—".to_string();
    }
    let mut parts: Vec<String> = ports
        .iter()
        .map(|p| match p.public_port {
            Some(public) => format!("{}→{}", public, p.private_port),
            None => format!("{}", p.private_port),
        })
        .collect();
    parts.sort();
    parts.dedup();
    parts.join(", ")
}

pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                continue;
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn parse_logs(chunks: &[LogOutput]) -> Vec<LogLine> {
    let mut lines = Vec::new();
    for chunk in chunks {
        let (message, stderr) = match chunk {
            LogOutput::StdOut { message } => (message, false),
            LogOutput::StdErr { message } => (message, true),
            LogOutput::StdIn { message } => (message, false),
            LogOutput::Console { message } => (message, false),
        };
        let text = strip_ansi(&String::from_utf8_lossy(message));
        let mut parts: Vec<&str> = text.split('\n').collect();
        if parts.last() == Some(&"") {
            parts.pop();
        }
        for part in parts {
            let (timestamp, text) = split_timestamp(part);
            lines.push(LogLine {
                stderr,
                timestamp,
                text: text.to_string(),
            });
        }
    }
    lines
}

/// Docker prefixes each line with an RFC3339 stamp when `timestamps(true)` is
/// requested; peel it off so the renderer can dim it separately.
fn split_timestamp(line: &str) -> (Option<String>, &str) {
    if let Some((first, rest)) = line.split_once(' ')
        && first.len() >= 20
        && first.contains('T')
        && first.ends_with('Z')
    {
        return (Some(first.to_string()), rest);
    }
    (None, line)
}

pub fn status_from_error(error: &bollard::errors::Error) -> HostStatus {
    use bollard::errors::Error;
    match error {
        Error::DockerResponseServerError {
            status_code: 401 | 403,
            message,
        } => HostStatus::AuthFailed(message.clone()),
        Error::SocketNotFoundError(path) => {
            HostStatus::Unreachable(format!("no such socket: {path}"))
        }
        Error::IOError { err } => HostStatus::Unreachable(err.to_string()),
        other => HostStatus::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{
        ContainerSummary, ContainerSummaryStateEnum, PortSummary, PortSummaryTypeEnum,
    };
    use chrono::{DateTime, Duration};

    #[test]
    fn formats_relative_times() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        assert_eq!(format_ago(now - Duration::seconds(5), now), "just now");
        assert_eq!(format_ago(now - Duration::minutes(2), now), "2m ago");
        assert_eq!(format_ago(now - Duration::hours(5), now), "5h ago");
        assert_eq!(format_ago(now - Duration::days(3), now), "3d ago");
        assert_eq!(format_ago(now - Duration::days(95), now), "3mo ago");
    }

    #[test]
    fn formats_image_sizes() {
        assert_eq!(format_bytes(456_654_848), "436 MB");
        assert_eq!(format_bytes(98_353_152), "93.8 MB");
        assert_eq!(format_bytes(2_147_483_648), "2.0 GB");
    }

    #[test]
    fn truncates_long_names() {
        assert_eq!(
            truncate("k8s_traefik_traefik-6cd8c7cd89-dlbpm_kube-system_4ddd", 20),
            "k8s_traefik_traefik…"
        );
        assert_eq!(truncate("short", 20), "short");
    }

    #[test]
    fn truncate_keeps_long_names_inside_their_column() {
        assert_eq!(
            truncate("k8s_traefik_traefik-6cd8c7cd89-dlbpm_kube-system", 26),
            "k8s_traefik_traefik-6cd8c…"
        );
    }

    #[test]
    fn maps_state_enum_and_defaults_unknown() {
        assert_eq!(
            container_state(ContainerSummaryStateEnum::RUNNING),
            ContainerState::Running
        );
        assert_eq!(
            container_state(ContainerSummaryStateEnum::EXITED),
            ContainerState::Exited
        );
        assert_eq!(
            container_state(ContainerSummaryStateEnum::EMPTY),
            ContainerState::Unknown
        );
    }

    #[test]
    fn image_row_counts_every_tag() {
        let summary = bollard::models::ImageSummary {
            id: "sha256:abc".into(),
            repo_tags: vec!["postgres:18".into(), "postgres:latest".into()],
            created: 1_699_999_880,
            size: 1,
            ..Default::default()
        };
        let row = ImageRow::from_summary(
            summary,
            false,
            DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        );
        assert_eq!(row.tag_count, 2);
        assert_eq!(row.repository_tag.as_ref(), "postgres:18");
    }

    #[test]
    fn maps_container_summary_fields() {
        let summary = ContainerSummary {
            id: Some("abc123def4567890".into()),
            names: Some(vec!["/web".into()]),
            image: Some("nginx:1".into()),
            state: Some(ContainerSummaryStateEnum::RUNNING),
            status: Some("Up 2 minutes".into()),
            created: Some(1_699_999_880),
            ..Default::default()
        };
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let row = ContainerRow::from_summary(summary, now);
        assert_eq!(row.name.as_ref(), "web");
        assert_eq!(row.state, ContainerState::Running);
        assert_eq!(row.image.as_ref(), "nginx:1");
        assert_eq!(row.status.as_ref(), "Up 2 minutes");
        assert_eq!(row.created_label.as_ref(), "2m ago");
    }

    #[test]
    fn reads_compose_project_from_labels() {
        let mut labels = std::collections::HashMap::new();
        labels.insert(
            "com.docker.compose.project".to_string(),
            "sploney".to_string(),
        );
        let summary = ContainerSummary {
            id: Some("abc".into()),
            labels: Some(labels),
            ..Default::default()
        };
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let row = ContainerRow::from_summary(summary, now);
        assert_eq!(
            row.compose_project.as_ref().map(|p| p.as_ref()),
            Some("sploney")
        );

        let standalone = ContainerRow::from_summary(
            ContainerSummary {
                id: Some("def".into()),
                ..Default::default()
            },
            now,
        );
        assert!(standalone.compose_project.is_none());
    }

    #[test]
    fn formats_ports_with_and_without_mapping() {
        let mapped = PortSummary {
            public_port: Some(5432),
            private_port: 5432,
            typ: Some(PortSummaryTypeEnum::TCP),
            ..Default::default()
        };
        let exposed = PortSummary {
            public_port: None,
            private_port: 80,
            typ: Some(PortSummaryTypeEnum::TCP),
            ..Default::default()
        };
        assert_eq!(format_ports(&[mapped]), "5432→5432");
        assert_eq!(format_ports(&[exposed]), "80");
        assert_eq!(format_ports(&[]), "—");
    }

    #[test]
    fn strips_ansi_and_carriage_returns() {
        assert_eq!(strip_ansi("\x1b[32mok\x1b[0m\r\n"), "ok\n");
    }

    #[test]
    fn parses_stderr_and_invalid_utf8_without_panicking() {
        let chunks = vec![
            LogOutput::StdErr {
                message: b"boom\n".as_slice().into(),
            },
            LogOutput::StdOut {
                message: b"\xff\xfe hi\n".as_slice().into(),
            },
        ];
        let lines = parse_logs(&chunks);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].stderr);
        assert_eq!(lines[0].text, "boom");
        assert!(!lines[1].stderr);
        assert!(lines[1].text.ends_with(" hi"));
    }

    #[test]
    fn splits_docker_timestamps_from_log_text() {
        let chunks = vec![LogOutput::StdOut {
            message: b"2026-11-14T21:00:00.123456789Z hello world\n"
                .as_slice()
                .into(),
        }];
        let lines = parse_logs(&chunks);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].timestamp.as_deref(),
            Some("2026-11-14T21:00:00.123456789Z")
        );
        assert_eq!(lines[0].text, "hello world");
    }

    #[test]
    fn plain_log_lines_have_no_timestamp() {
        let chunks = vec![LogOutput::StdOut {
            message: b"no timestamp here\n".as_slice().into(),
        }];
        assert!(parse_logs(&chunks)[0].timestamp.is_none());
    }

    #[test]
    fn maps_ping_errors_to_host_status() {
        use bollard::errors::Error;
        let forbidden = Error::DockerResponseServerError {
            status_code: 403,
            message: "forbidden".into(),
        };
        assert!(matches!(
            status_from_error(&forbidden),
            HostStatus::AuthFailed(_)
        ));

        let missing = Error::IOError {
            err: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert!(matches!(
            status_from_error(&missing),
            HostStatus::Unreachable(_)
        ));
    }
}
