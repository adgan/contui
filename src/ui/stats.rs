use crate::docker::model::{
    ContainerState, ContainerUsage, Snapshot, format_bytes, format_cpu_percent, format_usage_bytes,
};
use gpui::SharedString;
use std::collections::HashMap;

/// Numbers for the global overview strip. Computed once per snapshot so the
/// render path never walks the lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverviewStats {
    pub loaded: bool,
    pub containers_total: usize,
    pub containers_running: usize,
    pub containers_stopped: usize,
    pub images_total: usize,
    pub images_in_use: usize,
    pub images_bytes: i64,
    pub images_size_label: SharedString,
    pub volumes_total: usize,
    pub volumes_drivers: Vec<String>,
}

impl OverviewStats {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        let containers_total = snapshot.containers.len();
        let containers_running = snapshot
            .containers
            .iter()
            .filter(|container| container.state == ContainerState::Running)
            .count();
        let images_total = snapshot.images.len();
        let images_in_use = snapshot.images.iter().filter(|image| image.in_use).count();
        let images_bytes: i64 = snapshot.images.iter().map(|image| image.size).sum();

        let mut volumes_drivers: Vec<String> = snapshot
            .volumes
            .iter()
            .map(|volume| volume.driver.to_string())
            .collect();
        volumes_drivers.sort();
        volumes_drivers.dedup();

        Self {
            loaded: snapshot.fetched_at.is_some(),
            containers_total,
            containers_running,
            containers_stopped: containers_total.saturating_sub(containers_running),
            images_total,
            images_in_use,
            images_bytes,
            images_size_label: format_bytes(images_bytes).into(),
            volumes_total: snapshot.volumes.len(),
            volumes_drivers,
        }
    }

    pub fn containers_share(&self) -> f32 {
        if self.containers_total == 0 {
            0.0
        } else {
            (self.containers_running as f32 / self.containers_total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn volume_driver_label(&self) -> String {
        match self.volumes_drivers.len() {
            0 => "—".to_string(),
            1 => self.volumes_drivers[0].clone(),
            count => format!("{count} drivers"),
        }
    }

    pub fn volume_foot(&self) -> String {
        match self.volumes_drivers.len() {
            0 => "—".to_string(),
            1 => format!("{} driver", self.volumes_drivers[0]),
            count => format!("{count} drivers"),
        }
    }

    pub fn containers_subtitle(&self) -> String {
        if !self.loaded {
            return "connecting…".to_string();
        }
        format!(
            "{} running · {} stopped",
            self.containers_running, self.containers_stopped
        )
    }

    pub fn images_subtitle(&self) -> String {
        if !self.loaded {
            return "connecting…".to_string();
        }
        format!("{} images · {}", self.images_total, self.images_size_label)
    }

    pub fn volumes_subtitle(&self) -> String {
        if !self.loaded {
            return "connecting…".to_string();
        }
        let noun = if self.volumes_total == 1 {
            "volume"
        } else {
            "volumes"
        };
        let drivers = match self.volumes_drivers.len() {
            0 => "—".to_string(),
            1 => format!("{} driver", self.volumes_drivers[0]),
            count => format!("{count} drivers"),
        };
        format!("{} {} · {}", self.volumes_total, noun, drivers)
    }

    /// True when the card should show placeholders instead of numbers.
    pub fn is_loading(&self) -> bool {
        !self.loaded
    }
}

/// Host-wide live usage for the fourth overview card. CPU is normalized per
/// sample, so containers with different core counts cannot inflate the total.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UsageTotals {
    pub cpu_percent: Option<f32>,
    pub memory_bytes: Option<u64>,
    pub cpu_reporting: usize,
    pub memory_reporting: usize,
    pub running: usize,
}

impl UsageTotals {
    pub fn from_snapshot(snapshot: &Snapshot, usage: &HashMap<String, ContainerUsage>) -> Self {
        let mut totals = Self::default();

        for container in &snapshot.containers {
            if container.state != ContainerState::Running {
                continue;
            }
            totals.running += 1;
            let Some(sample) = usage.get(&container.id) else {
                continue;
            };
            if let Some(cpu) = sample.cpu_percent {
                totals.cpu_percent = Some(
                    totals.cpu_percent.unwrap_or(0.0)
                        + cpu / sample.online_cpus.max(1) as f32,
                );
                totals.cpu_reporting += 1;
            }
            if let Some(memory) = sample.memory_usage {
                totals.memory_bytes = Some(totals.memory_bytes.unwrap_or(0) + memory);
                totals.memory_reporting += 1;
            }
        }

        totals
    }
}

/// The text of the overview usage card, with every placeholder decided here so
/// the render path stays declarative.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageCardLabels {
    pub value: String,
    pub right: String,
    pub foot: String,
}

impl UsageCardLabels {
    pub fn from(totals: &UsageTotals, loaded: bool, connected: bool) -> Self {
        if !loaded {
            return Self {
                value: "—".into(),
                right: "—".into(),
                foot: "connecting…".into(),
            };
        }
        if !connected {
            return Self {
                value: "—".into(),
                right: "—".into(),
                foot: "host disconnected".into(),
            };
        }
        if totals.running == 0 {
            return Self {
                value: "0.00%".into(),
                right: "0 running".into(),
                foot: "no running containers".into(),
            };
        }

        let value = totals
            .cpu_percent
            .map(format_cpu_percent)
            .unwrap_or_else(|| "—".into());
        let right = if totals.cpu_reporting < totals.running {
            format!("{} of {}", totals.cpu_reporting, totals.running)
        } else {
            format!("{} running", totals.running)
        };
        let foot = match totals.memory_bytes {
            Some(memory) => {
                let base = format!("{} memory", format_usage_bytes(memory));
                if totals.memory_reporting < totals.running {
                    format!("{base} · {}/{}", totals.memory_reporting, totals.running)
                } else {
                    base
                }
            }
            None => "waiting for stats…".into(),
        };

        Self { value, right, foot }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::model::{ContainerRow, ImageRow, VolumeRow};
    use chrono::{DateTime, Utc};
    use gpui::SharedString;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn container(name: &str, state: ContainerState) -> ContainerRow {
        ContainerRow {
            id: format!("{name}-id"),
            name: SharedString::from(name.to_string()),
            state,
            status: "Up".into(),
            image: "postgres:18".into(),
            ports: "5432".into(),
            created: now(),
            created_label: "2m ago".into(),
            compose_project: None,
        }
    }

    fn image(tag: &str, size: i64, in_use: bool) -> ImageRow {
        ImageRow {
            id: format!("{tag}-id"),
            short_id: "abcdef123456".into(),
            repository_tag: SharedString::from(tag.to_string()),
            size,
            size_label: format_bytes(size).into(),
            created: now(),
            created_label: "2m ago".into(),
            in_use,
            tag_count: 1,
        }
    }

    fn volume(name: &str, driver: &str) -> VolumeRow {
        VolumeRow {
            name: SharedString::from(name.to_string()),
            driver: SharedString::from(driver.to_string()),
            mountpoint: "/var/lib/docker/volumes/x/_data".into(),
            created_label: "2m ago".into(),
        }
    }

    #[test]
    fn empty_snapshot_is_loading_and_zeroed() {
        let stats = OverviewStats::from_snapshot(&Snapshot::empty());
        assert!(!stats.loaded);
        assert_eq!(stats.containers_total, 0);
        assert_eq!(stats.containers_running, 0);
        assert_eq!(stats.containers_stopped, 0);
        assert_eq!(stats.containers_share(), 0.0);
        assert_eq!(stats.images_total, 0);
        assert_eq!(stats.volumes_total, 0);
        assert_eq!(stats.volume_driver_label(), "—");
        assert!(stats.is_loading());
    }

    #[test]
    fn mixed_containers_split_running_from_stopped() {
        let mut snapshot = Snapshot::empty();
        snapshot.fetched_at = Some(now());
        snapshot.containers = vec![
            container("a", ContainerState::Running),
            container("b", ContainerState::Running),
            container("c", ContainerState::Paused),
            container("d", ContainerState::Exited),
        ];
        let stats = OverviewStats::from_snapshot(&snapshot);
        assert!(stats.loaded);
        assert_eq!(stats.containers_total, 4);
        assert_eq!(stats.containers_running, 2);
        assert_eq!(stats.containers_stopped, 2);
        assert_eq!(stats.containers_share(), 0.5);
        assert_eq!(stats.containers_subtitle(), "2 running · 2 stopped");
    }

    #[test]
    fn paused_containers_are_not_running() {
        let mut snapshot = Snapshot::empty();
        snapshot.fetched_at = Some(now());
        snapshot.containers = vec![
            container("a", ContainerState::Paused),
            container("b", ContainerState::Restarting),
            container("c", ContainerState::Running),
        ];
        let stats = OverviewStats::from_snapshot(&snapshot);
        assert_eq!(stats.containers_running, 1);
        assert_eq!(stats.containers_stopped, 2);
    }

    #[test]
    fn image_bytes_sum_and_label_are_real() {
        let mut snapshot = Snapshot::empty();
        snapshot.fetched_at = Some(now());
        snapshot.images = vec![
            image("postgres:18", 500 * 1_048_576, true),
            image("redis:7", 100 * 1_048_576, false),
        ];
        let stats = OverviewStats::from_snapshot(&snapshot);
        assert_eq!(stats.images_bytes, 600 * 1_048_576);
        assert_eq!(stats.images_size_label.as_ref(), "600 MB");
        assert_eq!(stats.images_total, 2);
        assert_eq!(stats.images_in_use, 1);
        assert_eq!(stats.images_subtitle(), "2 images · 600 MB");
    }

    #[test]
    fn volume_drivers_are_sorted_and_deduped() {
        let mut snapshot = Snapshot::empty();
        snapshot.fetched_at = Some(now());
        snapshot.volumes = vec![
            volume("a", "local"),
            volume("b", "nfs"),
            volume("c", "local"),
        ];
        let stats = OverviewStats::from_snapshot(&snapshot);
        assert_eq!(
            stats.volumes_drivers,
            vec!["local".to_string(), "nfs".to_string()]
        );
        assert_eq!(stats.volume_driver_label(), "2 drivers");
        assert_eq!(stats.volume_foot(), "2 drivers");
        assert_eq!(stats.volumes_subtitle(), "3 volumes · 2 drivers");
    }

    #[test]
    fn single_volume_uses_singular_labels() {
        let mut snapshot = Snapshot::empty();
        snapshot.fetched_at = Some(now());
        snapshot.volumes = vec![volume("only", "local")];
        let stats = OverviewStats::from_snapshot(&snapshot);
        assert_eq!(stats.volume_driver_label(), "local");
        assert_eq!(stats.volume_foot(), "local driver");
        assert_eq!(stats.volumes_subtitle(), "1 volume · local driver");
    }

    #[test]
    fn share_never_exceeds_one() {
        let mut stats = OverviewStats::default();
        stats.containers_total = 1;
        stats.containers_running = 3;
        assert_eq!(stats.containers_share(), 1.0);
    }

    #[test]
    fn totals_ignore_stopped_containers_and_normalize_each_sample() {
        let snapshot = Snapshot {
            containers: vec![
                container("a", ContainerState::Running),
                container("b", ContainerState::Running),
                container("c", ContainerState::Exited),
            ],
            fetched_at: Some(now()),
            ..Snapshot::empty()
        };
        let usage = HashMap::from([
            (
                "a-id".to_string(),
                ContainerUsage {
                    cpu_percent: Some(100.0),
                    memory_usage: Some(1_000),
                    memory_limit: None,
                    online_cpus: 2,
                },
            ),
            (
                "b-id".to_string(),
                ContainerUsage {
                    cpu_percent: Some(200.0),
                    memory_usage: Some(2_000),
                    memory_limit: None,
                    online_cpus: 4,
                },
            ),
            (
                "c-id".to_string(),
                ContainerUsage {
                    cpu_percent: Some(999.0),
                    memory_usage: Some(9_999),
                    memory_limit: None,
                    online_cpus: 1,
                },
            ),
        ]);
        let totals = UsageTotals::from_snapshot(&snapshot, &usage);
        assert_eq!(totals.running, 2);
        assert_eq!(totals.cpu_reporting, 2);
        assert_eq!(totals.memory_reporting, 2);
        assert!((totals.cpu_percent.unwrap() - 100.0).abs() < 0.01);
        assert_eq!(totals.memory_bytes, Some(3_000));
    }

    #[test]
    fn card_labels_show_partial_cpu_and_memory_coverage() {
        let totals = UsageTotals {
            cpu_percent: Some(4.1),
            memory_bytes: Some(1_288_490_188),
            cpu_reporting: 2,
            memory_reporting: 3,
            running: 3,
        };
        let labels = UsageCardLabels::from(&totals, true, true);
        assert_eq!(labels.value, "4.10%");
        assert_eq!(labels.right, "2 of 3");
        assert_eq!(labels.foot, "1.2 GB memory");
    }

    #[test]
    fn card_labels_use_running_count_when_coverage_is_complete() {
        let totals = UsageTotals {
            cpu_percent: Some(4.1),
            memory_bytes: Some(1_288_490_188),
            cpu_reporting: 3,
            memory_reporting: 2,
            running: 3,
        };
        let labels = UsageCardLabels::from(&totals, true, true);
        assert_eq!(labels.right, "3 running");
        assert_eq!(labels.foot, "1.2 GB memory · 2/3");
    }

    #[test]
    fn card_labels_cover_loading_disconnected_empty_and_missing_samples() {
        let empty = UsageTotals::default();
        assert_eq!(
            UsageCardLabels::from(&empty, false, false),
            UsageCardLabels {
                value: "—".into(),
                right: "—".into(),
                foot: "connecting…".into()
            }
        );
        assert_eq!(
            UsageCardLabels::from(&empty, true, false),
            UsageCardLabels {
                value: "—".into(),
                right: "—".into(),
                foot: "host disconnected".into()
            }
        );
        assert_eq!(
            UsageCardLabels::from(&empty, true, true),
            UsageCardLabels {
                value: "0.00%".into(),
                right: "0 running".into(),
                foot: "no running containers".into()
            }
        );

        let running_no_samples = UsageTotals {
            running: 2,
            ..UsageTotals::default()
        };
        let labels = UsageCardLabels::from(&running_no_samples, true, true);
        assert_eq!(labels.value, "—");
        assert_eq!(labels.right, "0 of 2");
        assert_eq!(labels.foot, "waiting for stats…");
    }
}
