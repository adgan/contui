use crate::docker::endpoint::HostStatus;
use bollard::container::LogOutput;
use bollard::models::{
    ContainerCpuStats, ContainerMemoryStats, ContainerStatsResponse, ContainerSummary,
    ContainerSummaryStateEnum, ImageSummary, PortSummary, Volume,
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContainerUsage {
    /// Docker's per-container percentage: 100% is one full core.
    pub cpu_percent: Option<f32>,
    /// Working set, with the cgroup cache/inactive file subtracted.
    pub memory_usage: Option<u64>,
    pub memory_limit: Option<u64>,
    /// The divisor used for `cpu_percent`, kept with the sample so host totals
    /// can normalize each container independently.
    pub online_cpus: u32,
}

impl ContainerUsage {
    pub fn from_sample(sample: &ContainerStatsResponse) -> Option<Self> {
        let cpu_stats = sample.cpu_stats.as_ref();
        let online_cpus = cpu_stats.map(online_cpus).unwrap_or(1);
        let cpu_percent = cpu_stats
            .and_then(|cpu| {
                sample
                    .precpu_stats
                    .as_ref()
                    .map(|previous| (cpu, previous))
            })
            .and_then(|(cpu, previous)| cpu_percent(cpu, previous, online_cpus));
        let memory = sample.memory_stats.as_ref().and_then(memory_usage);

        if cpu_percent.is_none() && memory.is_none() {
            return None;
        }

        Some(Self {
            cpu_percent,
            memory_usage: memory.map(|(usage, _)| usage),
            memory_limit: memory.and_then(|(_, limit)| limit),
            online_cpus,
        })
    }
}

fn online_cpus(stats: &ContainerCpuStats) -> u32 {
    stats
        .online_cpus
        .filter(|count| *count > 0)
        .or_else(|| {
            stats
                .cpu_usage
                .as_ref()
                .and_then(|usage| usage.percpu_usage.as_ref())
                .map(|cores| cores.len() as u32)
                .filter(|count| *count > 0)
        })
        .unwrap_or(1)
}

fn cpu_percent(
    stats: &ContainerCpuStats,
    previous: &ContainerCpuStats,
    online_cpus: u32,
) -> Option<f32> {
    let previous_system = previous.system_cpu_usage?;
    if previous_system == 0 {
        return None;
    }
    let system_delta = stats.system_cpu_usage?.checked_sub(previous_system)?;
    if system_delta == 0 {
        return None;
    }
    let total = stats.cpu_usage.as_ref()?.total_usage?;
    let previous_total = previous.cpu_usage.as_ref()?.total_usage?;
    let cpu_delta = total.saturating_sub(previous_total) as f64;
    Some(((cpu_delta / system_delta as f64) * online_cpus as f64 * 100.0) as f32)
}

fn memory_usage(stats: &ContainerMemoryStats) -> Option<(u64, Option<u64>)> {
    let usage = stats.usage?;
    let cache = stats
        .stats
        .as_ref()
        .and_then(|values| {
            values
                .get("total_inactive_file")
                .or_else(|| values.get("inactive_file"))
                .or_else(|| values.get("cache"))
                .copied()
        })
        .unwrap_or(0);
    Some((
        usage.saturating_sub(cache),
        stats.limit.filter(|limit| *limit > 0),
    ))
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

pub fn format_cpu_percent(percent: f32) -> String {
    format!("{percent:.2}%")
}

pub fn format_usage_bytes(bytes: u64) -> String {
    let bytes = bytes as f64;
    let kib = 1024.0;
    let mib = kib * 1024.0;
    let gib = mib * 1024.0;
    if bytes >= gib {
        format!("{:.1} GB", bytes / gib)
    } else if bytes >= 100.0 * mib {
        format!("{:.0} MB", bytes / mib)
    } else if bytes >= mib {
        format!("{:.1} MB", bytes / mib)
    } else if bytes >= kib {
        format!("{:.0} KB", bytes / kib)
    } else {
        format!("{bytes:.0} B")
    }
}

pub fn format_memory_pair(usage: u64, limit: Option<u64>) -> String {
    match limit {
        Some(limit) if limit > 0 => {
            format!("{} / {}", format_usage_bytes(usage), format_usage_bytes(limit))
        }
        _ => format_usage_bytes(usage),
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
        ContainerCpuStats, ContainerCpuUsage, ContainerMemoryStats, ContainerStatsResponse,
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

    fn stats_sample(
        total: u64,
        pre_total: u64,
        system: u64,
        pre_system: u64,
        online: u32,
    ) -> ContainerStatsResponse {
        ContainerStatsResponse {
            cpu_stats: Some(ContainerCpuStats {
                cpu_usage: Some(ContainerCpuUsage {
                    total_usage: Some(total),
                    ..Default::default()
                }),
                system_cpu_usage: Some(system),
                online_cpus: Some(online),
                ..Default::default()
            }),
            precpu_stats: Some(ContainerCpuStats {
                cpu_usage: Some(ContainerCpuUsage {
                    total_usage: Some(pre_total),
                    ..Default::default()
                }),
                system_cpu_usage: Some(pre_system),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn memory_sample(usage: u64, limit: Option<u64>, stats: &[(&str, u64)]) -> ContainerStatsResponse {
        ContainerStatsResponse {
            memory_stats: Some(ContainerMemoryStats {
                usage: Some(usage),
                limit,
                stats: Some(stats.iter().map(|(k, v)| (k.to_string(), *v)).collect()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn cpu_uses_the_delta_and_the_sample_core_count() {
        let usage = ContainerUsage::from_sample(&stats_sample(300, 100, 1_300, 500, 2)).unwrap();
        assert!((usage.cpu_percent.unwrap() - 50.0).abs() < 0.01);
    }

    #[test]
    fn zeroed_first_sample_keeps_memory_but_has_no_cpu() {
        let mut sample = stats_sample(100, 0, 1_000, 0, 8);
        sample.memory_stats = memory_sample(1_000, None, &[]).memory_stats;
        let usage = ContainerUsage::from_sample(&sample).unwrap();
        assert!(usage.cpu_percent.is_none());
        assert_eq!(usage.memory_usage, Some(1_000));
        assert_eq!(usage.online_cpus, 8);
    }

    #[test]
    fn idle_cpu_is_zero_not_missing() {
        let usage = ContainerUsage::from_sample(&stats_sample(100, 100, 1_000, 900, 4)).unwrap();
        assert_eq!(usage.cpu_percent, Some(0.0));
    }

    #[test]
    fn memory_prefers_total_inactive_file_then_inactive_file_then_cache() {
        let v1 = ContainerUsage::from_sample(&memory_sample(
            1_000,
            Some(2_000),
            &[
                ("total_inactive_file", 200),
                ("inactive_file", 150),
                ("cache", 100),
            ],
        ))
        .unwrap();
        assert_eq!(v1.memory_usage, Some(800));
        assert_eq!(v1.memory_limit, Some(2_000));

        let v2 =
            ContainerUsage::from_sample(&memory_sample(1_000, Some(2_000), &[("inactive_file", 300)]))
                .unwrap();
        assert_eq!(v2.memory_usage, Some(700));

        let cache =
            ContainerUsage::from_sample(&memory_sample(1_000, Some(2_000), &[("cache", 100)]))
                .unwrap();
        assert_eq!(cache.memory_usage, Some(900));
    }

    #[test]
    fn zero_memory_limit_is_none_and_memory_only_sample_keeps_memory() {
        let usage = ContainerUsage::from_sample(&memory_sample(1_000, Some(0), &[])).unwrap();
        assert_eq!(usage.memory_usage, Some(1_000));
        assert_eq!(usage.memory_limit, None);
        assert!(usage.cpu_percent.is_none());
        assert_eq!(usage.online_cpus, 1);
    }

    #[test]
    fn formats_usage_numbers() {
        assert_eq!(format_cpu_percent(0.0), "0.00%");
        assert_eq!(format_cpu_percent(12.3), "12.30%");
        assert_eq!(format_usage_bytes(0), "0 B");
        assert_eq!(format_usage_bytes(1_536), "2 KB");
        assert_eq!(format_usage_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_usage_bytes(1_610_612_736), "1.5 GB");
        assert_eq!(
            format_memory_pair(196_000_000, Some(2_147_483_648)),
            "187 MB / 2.0 GB"
        );
        assert_eq!(format_memory_pair(1_048_576, None), "1.0 MB");
        assert_eq!(format_memory_pair(1_048_576, Some(0)), "1.0 MB");
    }
}
