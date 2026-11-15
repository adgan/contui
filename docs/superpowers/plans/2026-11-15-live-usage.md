# Live Usage Overview and Container Detail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show host-wide live CPU/memory in the overview strip and live CPU/memory for the open container on its detail page, fed by one `/stats` stream per running container.

**Architecture:** The Docker service actor owns one stats task per running container, reconciled on every snapshot. Samples arrive as `Response::Usage`; failures send `Response::UsageStale`. `RootView` keeps the latest samples in a map, computes a host-wide total for a fourth overview card, and formats the open container's numbers into a fixed-height band above the detail tabs. The list table never changes.

**Tech Stack:** Rust 2024, GPUI 0.2.2, bollard 0.21, tokio, async-channel.

**Spec:** `docs/superpowers/specs/2026-11-15-live-usage-design.md`

## Global Constraints

- GPUI 0.2.2, Rust 2024, one binary crate.
- No new crates.
- The UI thread never touches the network.
- No CPU/memory columns in the containers table.
- No stats for stopped or paused containers.
- No polling timer and no one-shot `/stats` requests.
- The overview strip keeps its `88px` / `76px` compact height; cards truncate.
- `theme.rs` and `ui/style.rs` stay the source of truth for colour, spacing, radii and type.

## Review Focus

- A first `/stats` sample with zeroed `precpu_stats` must not paint a lifetime-average CPU number; it shows `—` until a real delta exists. Tested in Task 1.
- A container that stops, restarts or is removed must not leave a stream or a stale usage entry. Tested in Tasks 2, 5 and 8.
- A disconnected host or stalled stream must not leave a live-looking number. Tested in Tasks 2, 3 and 5.
- Mixed samples, or memory-only samples, must not inflate the host CPU share. Tested in Task 3.
- Partial coverage must be visible (`2 of 3`, `· 2/3`), never claimed as complete. Tested in Task 3.

---

### Task 1: Container usage model and formatting

**Files:**
- Modify: `src/docker/model.rs` (imports around line 3, new type after the `ContainerRow` impl around line 77, tests around line 309)
- Test: `src/docker/model.rs` (`mod tests`)

**Interfaces:**
- Consumes: `bollard::models::ContainerStatsResponse`.
- Produces:
  - `pub struct ContainerUsage { pub cpu_percent: Option<f32>, pub memory_usage: Option<u64>, pub memory_limit: Option<u64>, pub online_cpus: u32 }`
  - `impl ContainerUsage { pub fn from_sample(sample: &ContainerStatsResponse) -> Option<Self> }`
  - `pub fn format_cpu_percent(percent: f32) -> String`
  - `pub fn format_usage_bytes(bytes: u64) -> String`
  - `pub fn format_memory_pair(usage: u64, limit: Option<u64>) -> String`

- [ ] **Step 1: Write the failing model tests**

Add `ContainerCpuStats`, `ContainerCpuUsage`, `ContainerMemoryStats` and `ContainerStatsResponse` to the test module's bollard imports, then add these tests:

```rust
fn stats_sample(total: u64, pre_total: u64, system: u64, pre_system: u64, online: u32)
    -> ContainerStatsResponse
{
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

fn memory_sample(usage: u64, limit: Option<u64>, stats: &[(&str, u64)])
    -> ContainerStatsResponse
{
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
        &[("total_inactive_file", 200), ("inactive_file", 150), ("cache", 100)],
    ))
    .unwrap();
    assert_eq!(v1.memory_usage, Some(800));
    assert_eq!(v1.memory_limit, Some(2_000));

    let v2 = ContainerUsage::from_sample(&memory_sample(
        1_000,
        Some(2_000),
        &[("inactive_file", 300)],
    ))
    .unwrap();
    assert_eq!(v2.memory_usage, Some(700));

    let cache = ContainerUsage::from_sample(&memory_sample(
        1_000,
        Some(2_000),
        &[("cache", 100)],
    ))
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib docker::model::tests -- --nocapture`
Expected: compile error, `ContainerUsage` and the format functions do not exist.

- [ ] **Step 3: Implement the model and formatting**

Add `ContainerStatsResponse`, `ContainerCpuStats`, `ContainerMemoryStats` to the bollard imports. Add `ContainerUsage` with `#[derive(Clone, Copy, Debug, Default, PartialEq)]`.

`from_sample` rules:

- Return `None` only when both CPU and memory are absent.
- CPU: require `precpu_stats.system_cpu_usage` present and `> 0`. Compute `cpu_delta` from `cpu_usage.total_usage` (saturating), `system_delta = cpu.system_cpu_usage - pre_system` (return `None` if zero), then `(cpu_delta / system_delta) * online_cpus * 100`.
- `online_cpus`: `cpu_stats.online_cpus` when `> 0`, else `percpu_usage.len()`, else `1`. Read it from `cpu_stats` even when the CPU delta is not yet valid, so a partial first sample still records the core count.
- Memory: `memory_stats.usage` minus the first present of `total_inactive_file`, `inactive_file`, `cache`, saturating; limit is `Some` only when `memory_stats.limit > 0`.
- With a memory-only sample (no `cpu_stats`), `cpu_percent` is `None` and `online_cpus` is `1`.

Formatting:

- `format_cpu_percent`: `format!("{percent:.2}%")`.
- `format_usage_bytes`: `>= 1 GiB` → `{:.1} GB`; `>= 100 MiB` → `{:.0} MB`; `>= 1 MiB` → `{:.1} MB`; `>= 1 KiB` → `{:.0} KB`; else `{n} B`.
- `format_memory_pair`: `usage / limit` via `format_usage_bytes` when the limit is `Some` and greater than zero, else usage alone.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib docker::model::tests`
Expected: all model tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/docker/model.rs
git commit -m "feat: parse container usage samples"
```

---

### Task 2: Service-owned stats streams

**Files:**
- Modify: `src/docker/service.rs` (imports around lines 9-21, `Response` around line 81, actor around line 275, `FetchAll` around line 285, tests around line 683)
- Test: `src/docker/service.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ContainerUsage::from_sample`.
- Produces:
  - `Response::Usage { id: String, usage: ContainerUsage }`
  - `Response::UsageStale { id: String }`
  - `fn running_container_ids(snapshot: &Snapshot) -> HashSet<String>`
  - `fn stats_plan(running: &HashSet<String>, active: &HashSet<String>) -> (Vec<String>, Vec<String>)`
  - `async fn next_sample<S>(stream: &mut S, timeout: Duration) -> Option<Result<ContainerStatsResponse, bollard::errors::Error>>` where `S: futures_util::Stream<Item = Result<ContainerStatsResponse, bollard::errors::Error>> + Unpin`

- [ ] **Step 1: Write the failing service tests**

Add to the service test module:

```rust
fn container(id: &str, state: ContainerState) -> ContainerRow {
    ContainerRow {
        id: id.to_string(),
        name: gpui::SharedString::from(id.to_string()),
        state,
        status: gpui::SharedString::from("Up"),
        image: gpui::SharedString::from("alpine:3.20"),
        ports: gpui::SharedString::from("—"),
        created: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        created_label: gpui::SharedString::from("2m ago"),
        compose_project: None,
    }
}

#[test]
fn running_ids_exclude_paused_and_restarting() {
    let snapshot = Snapshot {
        containers: vec![
            container("a", ContainerState::Running),
            container("b", ContainerState::Paused),
            container("c", ContainerState::Restarting),
            container("d", ContainerState::Exited),
        ],
        ..Snapshot::empty()
    };
    assert_eq!(
        running_container_ids(&snapshot),
        HashSet::from(["a".to_string()])
    );
}

#[test]
fn stats_plan_starts_new_ids_and_stops_gone_ids() {
    let running = HashSet::from(["a".to_string(), "b".to_string()]);
    let active = HashSet::from(["b".to_string(), "c".to_string()]);
    assert_eq!(
        stats_plan(&running, &active),
        (vec!["a".to_string()], vec!["c".to_string()])
    );
}

#[test]
fn next_sample_returns_none_when_the_stream_stalls() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut stream =
        futures_util::stream::pending::<Result<ContainerStatsResponse, bollard::errors::Error>>();
    let sample = runtime.block_on(next_sample(&mut stream, Duration::from_millis(1)));
    assert!(sample.is_none());
}
```

Add `ContainerStatsResponse` to the service test imports.

Add `ContainerState`, `ContainerRow` and `ContainerUsage` to the model import if they are not already there.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib docker::service::tests`
Expected: compile error, `running_container_ids` and `stats_plan` do not exist.

- [ ] **Step 3: Implement the responses, stream task and reconciliation**

1. Add `Response::Usage` and `Response::UsageStale`.
2. Add `HashMap` and `tokio::task::JoinHandle` to the imports, `ContainerStatsResponse` to the bollard model imports, and `StatsOptionsBuilder` to the bollard query imports.
3. Add `running_container_ids` and `stats_plan` exactly as tested; sort both returned vectors.
4. Add `next_sample` as tested: `tokio::time::timeout(timeout, stream.next()).await.ok().flatten()`.
5. Add `stats_loop(docker: Docker, id: String, resp_tx: Sender<Response>)`:
   - open `Box::pin(docker.stats(&id, Some(StatsOptionsBuilder::new().stream(true).build())))`;
   - read each item through `next_sample(&mut stream, Duration::from_secs(5)).await`;
   - on `Some(Ok(sample))`, call `ContainerUsage::from_sample`, send `Response::Usage` when it is `Some`, and reset the backoff to 1s;
   - on timeout, stream end or error, send `Response::UsageStale`, sleep with 1s → 2s → 4s … 10s backoff, then resubscribe;
   - return early if either send fails.
6. Add `reconcile_stats(docker, resp_tx, tasks, running)`:
   - drop and abort finished handles;
   - build the active id set from `tasks.keys()`;
   - call `stats_plan`;
   - abort and remove each stopped id;
   - spawn `stats_loop` for each started id.
7. In `actor`, keep `let mut stats_tasks: HashMap<String, JoinHandle<()>> = HashMap::new();` before the request loop.
8. In the `FetchAll` success arm, compute `running_container_ids(&snapshot)`, send `Response::Snapshot(snapshot)`, then call `reconcile_stats`. Send the snapshot first so the UI knows the running set before samples arrive.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib docker::service::tests`
Expected: all service tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/docker/service.rs
git commit -m "feat: stream stats for running containers"
```

---

### Task 3: Host usage totals and overview card labels

**Files:**
- Modify: `src/ui/stats.rs` (new types after `OverviewStats` around line 116, tests around line 119)
- Test: `src/ui/stats.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ContainerUsage`, `ContainerState`, `Snapshot`.
- Produces:
  - `pub struct UsageTotals { pub cpu_percent: Option<f32>, pub memory_bytes: Option<u64>, pub cpu_reporting: usize, pub memory_reporting: usize, pub running: usize }`
  - `impl UsageTotals { pub fn from_snapshot(snapshot: &Snapshot, usage: &HashMap<String, ContainerUsage>) -> Self }`
  - `pub struct UsageCardLabels { pub value: String, pub right: String, pub foot: String }`
  - `impl UsageCardLabels { pub fn from(totals: &UsageTotals, loaded: bool, connected: bool) -> Self }`

- [ ] **Step 1: Write the failing totals and label tests**

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib ui::stats::tests`
Expected: compile error, `UsageTotals` and `UsageCardLabels` do not exist.

- [ ] **Step 3: Implement totals and labels**

- Add `use std::collections::HashMap;` and `ContainerUsage` to the imports.
- Derive `Clone, Copy, Debug, Default, PartialEq` on `UsageTotals` and `Clone, Debug, Default, PartialEq, Eq` on `UsageCardLabels`.
- `UsageTotals::from_snapshot`: iterate only `ContainerState::Running` containers; for each id present in `usage`, add `cpu_percent / online_cpus.max(1)` when CPU is `Some` and increment `cpu_reporting`; add `memory_usage` when present and increment `memory_reporting`. `cpu_percent` and `memory_bytes` are `Some` only when their count is non-zero.
- `UsageCardLabels::from` order:
  1. `!loaded` → `—` / `—` / `connecting…`.
  2. `!connected` → `—` / `—` / `host disconnected`.
  3. `running == 0` → `0.00%` / `0 running` / `no running containers`.
  4. value is `format_cpu_percent(cpu)` or `—`.
  5. right is `"{cpu_reporting} of {running}"` when CPU coverage is partial, else `"{running} running"`.
  6. foot is `"{format_usage_bytes(mem)} memory"`, with `" · {memory_reporting}/{running}"` appended when memory coverage is partial; when no memory sample exists, `waiting for stats…`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::stats::tests`
Expected: all stats tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/stats.rs
git commit -m "feat: total live usage for the overview card"
```

---

### Task 4: Detail usage view model and band

**Files:**
- Create: `src/ui/usage.rs`
- Modify: `src/ui/mod.rs` (add `pub mod usage;`)
- Test: `src/ui/usage.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ContainerUsage`, `ContainerState`, `format_cpu_percent`, `format_memory_pair`.
- Produces:
  - `pub struct DetailUsage { pub cpu: String, pub memory: String, pub cpu_sub: String, pub memory_sub: String, pub memory_fraction: Option<f32> }`
  - `impl DetailUsage { pub fn from(usage: Option<&ContainerUsage>, state: ContainerState, connected: bool) -> Self }`
  - `pub fn render_usage_band(usage: &DetailUsage, theme: &Theme, layout: &Layout) -> gpui::Div`

- [ ] **Step 1: Write the failing view-model tests**

```rust
fn usage(cpu: Option<f32>, memory: Option<u64>, limit: Option<u64>) -> ContainerUsage {
    ContainerUsage {
        cpu_percent: cpu,
        memory_usage: memory,
        memory_limit: limit,
        online_cpus: 1,
    }
}

#[test]
fn running_container_shows_cpu_memory_and_fraction() {
    let value = usage(Some(0.42), Some(1_048_576), Some(2_097_152));
    let detail = DetailUsage::from(Some(&value), ContainerState::Running, true);
    assert_eq!(detail.cpu, "0.42%");
    assert_eq!(detail.cpu_sub, "of one core");
    assert_eq!(detail.memory, "1.0 MB / 2.0 MB");
    assert_eq!(detail.memory_fraction, Some(0.5));
}

#[test]
fn stopped_container_has_no_usage_text() {
    let detail = DetailUsage::from(None, ContainerState::Exited, true);
    assert_eq!(detail.cpu, "—");
    assert_eq!(detail.memory, "—");
    assert_eq!(detail.cpu_sub, "container is not running");
    assert_eq!(detail.memory_sub, "container is not running");
    assert_eq!(detail.memory_fraction, None);
}

#[test]
fn disconnected_host_wins_over_running_state() {
    let value = usage(Some(50.0), Some(1_048_576), None);
    let detail = DetailUsage::from(Some(&value), ContainerState::Running, false);
    assert_eq!(detail.cpu, "—");
    assert_eq!(detail.cpu_sub, "host disconnected");
}

#[test]
fn partial_sample_shows_memory_without_cpu() {
    let value = usage(None, Some(1_048_576), None);
    let detail = DetailUsage::from(Some(&value), ContainerState::Running, true);
    assert_eq!(detail.cpu, "—");
    assert_eq!(detail.cpu_sub, "waiting for stats…");
    assert_eq!(detail.memory, "1.0 MB");
    assert_eq!(detail.memory_fraction, None);
}

#[test]
fn memory_fraction_clamps_to_one() {
    let value = usage(Some(1.0), Some(3_000), Some(1_000));
    let detail = DetailUsage::from(Some(&value), ContainerState::Running, true);
    assert_eq!(detail.memory_fraction, Some(1.0));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib ui::usage::tests`
Expected: compile error, the module and `DetailUsage` do not exist.

- [ ] **Step 3: Implement the view model and band**

- Derive `Clone, Debug, PartialEq` on `DetailUsage`.
- `DetailUsage::from` order: `!connected` → both cells `—`, both subs `host disconnected`; `state != Running` → both `—`, both subs `container is not running`; no usage → both `—`, both subs `waiting for stats…`; otherwise format each present metric and use `waiting for stats…` for whichever metric is absent. `memory_fraction` is `used / limit` clamped to 0–1 only when both exist and limit is `> 0`.
- `render_usage_band`: a `crate::ui::style::card` with fixed `88px` / `76px` compact height, horizontal padding `space::XXL` / `space::LG`, two equal cells and a 1px vertical divider. Each cell has a `10/600` uppercase kicker, a value at `22px` (`20px` compact), a `11px` sub-line that reserves its height even when empty, and a 3px bar row reserved on both cells. The memory cell's bar fills with `theme.accent` at the fraction; the CPU cell's row stays empty.
- Add `pub mod usage;` to `src/ui/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::usage::tests`
Expected: all usage view-model tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/usage.rs src/ui/mod.rs
git commit -m "feat: format live usage for the detail page"
```

---

### Task 5: Root usage state and response handling

**Files:**
- Modify: `src/ui/root.rs` (imports around lines 5-12, `reset_host_data` around line 108, `RootView` around line 122, `new` around line 157, `connect` around line 196, pump around line 212, `apply` around line 239, `Status` arm around line 277, tests around line 1019)
- Test: `src/ui/root.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Response::Usage`, `Response::UsageStale`, `ContainerUsage`.
- Produces: `RootView.usage: HashMap<String, ContainerUsage>`.

- [ ] **Step 1: Write the failing state tests**

Add to the root test module:

```rust
#[test]
fn retain_running_usage_drops_stopped_and_missing_containers() {
    let snapshot = Snapshot {
        containers: vec![
            container("alpha", ContainerState::Running),
            container("beta", ContainerState::Exited),
        ],
        ..Snapshot::empty()
    };
    let mut usage = HashMap::from([
        ("alpha-id".to_string(), ContainerUsage::default()),
        ("beta-id".to_string(), ContainerUsage::default()),
        ("gone-id".to_string(), ContainerUsage::default()),
    ]);

    retain_running_usage(&mut usage, &snapshot);

    assert!(usage.contains_key("alpha-id"));
    assert!(!usage.contains_key("beta-id"));
    assert!(!usage.contains_key("gone-id"));
}

#[test]
fn usage_is_only_kept_while_the_host_is_connected() {
    assert!(keeps_usage(&HostStatus::Connected));
    assert!(!keeps_usage(&HostStatus::Unknown));
    assert!(!keeps_usage(&HostStatus::Unreachable("gone".into())));
    assert!(!keeps_usage(&HostStatus::AuthFailed("denied".into())));
}
```

Update `host_switch_resets_rows_and_detail` to create a usage map, pass it to `reset_host_data`, and assert it is empty.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib ui::root::tests`
Expected: compile error, `retain_running_usage`, `keeps_usage` and the new `reset_host_data` parameter do not exist.

- [ ] **Step 3: Implement the root state**

- Add `ContainerUsage` to the `docker::model` import.
- Add `pub usage: HashMap<String, ContainerUsage>` to `RootView`; initialize it to `HashMap::new()` in `new`.
- Add `usage: &mut HashMap<String, ContainerUsage>` to `reset_host_data`, clear it, and pass `&mut self.usage` from `connect`.
- Add:
  ```rust
  fn keeps_usage(status: &HostStatus) -> bool {
      matches!(status, HostStatus::Connected)
  }

  fn retain_running_usage(
      usage: &mut HashMap<String, ContainerUsage>,
      snapshot: &Snapshot,
  ) {
      let running: HashSet<&str> = snapshot
          .containers
          .iter()
          .filter(|container| container.state == ContainerState::Running)
          .map(|container| container.id.as_str())
          .collect();
      usage.retain(|id, _| running.contains(id.as_str()));
  }
  ```
- In `apply`:
  - after assigning a new snapshot, call `retain_running_usage(&mut self.usage, &self.snapshot)`;
  - `Response::Usage { id, usage }` inserts;
  - `Response::UsageStale { id }` removes;
  - in `Response::Status`, clear the map when `!keeps_usage(&status)` before assigning the status.
- In the response pump, drain queued responses inside the update before notifying:
  ```rust
  view.apply(response);
  while let Ok(next) = responses.try_recv() {
      view.apply(next);
  }
  cx.notify();
  ```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::root::tests`
Expected: all root tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/root.rs
git commit -m "feat: keep live usage in the root view"
```

---

### Task 6: Overview usage card

**Files:**
- Create: `assets/icons/activity.svg`
- Modify: `src/assets.rs` (icon table around line 8, tests around line 94)
- Modify: `src/ui/root/view.rs` (`render_overview` around line 1501)
- Modify: `src/ui/root.rs` (stats import around line 11)
- Test: `src/assets.rs` (`mod tests`)

**Interfaces:**
- Consumes: `UsageTotals`, `UsageCardLabels`, `StatCard`, `stat_card`.
- Produces: the fourth overview card.

- [ ] **Step 1: Write the failing icon test**

Add to the asset tests:

```rust
#[test]
fn activity_icon_is_embedded() {
    assert!(Assets.load("icons/activity.svg").unwrap().is_some());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib assets::tests::activity_icon_is_embedded`
Expected: FAIL because the icon is not registered.

- [ ] **Step 3: Add the icon and the card**

- Add a 16×16 stroked pulse SVG at `assets/icons/activity.svg` matching the existing icon style.
- Register it in `ICONS`.
- In `render_overview`, compute:
  ```rust
  let usage_totals = UsageTotals::from_snapshot(&self.snapshot, &self.usage);
  let usage_labels = UsageCardLabels::from(
      &usage_totals,
      self.snapshot.fetched_at.is_some(),
      matches!(&self.status, HostStatus::Connected),
  );
  ```
- Build a `StatCard` with `id: "overview-usage"`, `kicker: "Usage"`, `icon: "icons/activity.svg"`, the labels' `value`/`right`/`foot`, `unit: "CPU"`, `progress: None`, `active: false`; its click switches to `Page::Containers`.
- Insert it after the containers card and before images. The strip order becomes Containers, Usage, Images, Volumes.

- [ ] **Step 4: Run the test and the library tests**

Run: `cargo test --lib assets::tests`
Expected: pass.
Run: `cargo test --lib`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add assets/icons/activity.svg src/assets.rs src/ui/root.rs src/ui/root/view.rs
git commit -m "feat: show host usage in the overview strip"
```

---

### Task 7: Detail usage band

**Files:**
- Modify: `src/ui/root/view.rs` (imports around line 1, `render_detail` around line 332 and the layout chain around line 790)
- Test: `src/ui/usage.rs` (view-model tests from Task 4)

**Interfaces:**
- Consumes: `DetailUsage`, `render_usage_band`.
- Produces: the band between the detail hero and tabs.

- [ ] **Step 1: Add the band to `render_detail`**

After `let state = container.as_ref().map(|container| container.state);`, build:

```rust
let detail_usage = if detail.kind == ResourceKind::Container {
    let connected = matches!(&self.status, HostStatus::Connected);
    Some(DetailUsage::from(
        self.usage.get(&detail.id),
        state.unwrap_or(ContainerState::Unknown),
        connected,
    ))
} else {
    None
};
```

Import `DetailUsage` with `use crate::ui::usage::DetailUsage;`.

In the layout chain, insert between `hero` and `tab_strip`:

```rust
.when_some(detail_usage, |this, usage| {
    this.child(crate::ui::usage::render_usage_band(&usage, &theme, layout))
})
```

The band appears for container details only. Images, volumes and missing-container states are unchanged.

- [ ] **Step 2: Run the library tests**

Run: `cargo test --lib`
Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add src/ui/root/view.rs
git commit -m "feat: show live usage on container detail"
```

---

### Task 8: Live test, README and final verification

**Files:**
- Modify: `tests/live_docker.rs` (`wait_for` around line 23, new test at the end)
- Modify: `README.md` (What it does around line 7, Not in this version around line 25)

- [ ] **Step 1: Make `wait_for` match first, then ignore noise**

Change `wait_for` so it evaluates `pred(&response)` before the skip match, and add `Response::Usage { .. }` and `Response::UsageStale { .. }` to the skipped variants. Other live tests then ignore usage traffic, while the new test can still match it.

- [ ] **Step 2: Add the ignored live stats test**

Use the existing `connect`, `cleanup_test_containers`, `run_spec`, `wait_for`, `action_error` helpers. The test must:

1. clean up old test containers;
2. pull `alpine:3.20`, returning early with a log line if the pull fails;
3. run `dockyard-live-stats-{pid}` with `run_spec(&name, None)`;
4. request `FetchAll` and wait for the `Snapshot`;
5. wait for `Response::Usage` where `usage.cpu_percent.is_some()` and `usage.memory_usage.is_some()`;
6. assert the CPU is non-negative and memory is greater than zero;
7. force-remove the container and assert the removal error is `None`.

Mark it `#[ignore = "requires a running docker daemon"]`.

- [ ] **Step 3: Update the README**

- Remove the `No live CPU/memory columns` bullet from "Not in this version".
- Add under "What it does": "Shows host-wide CPU and memory for running containers in the overview strip, and live CPU/memory for the open container on its detail page. Each running container has one `/stats` stream; stopped containers have none."

- [ ] **Step 4: Run the full verification**

Run: `cargo fmt`
Run: `cargo fmt --check`
Expected: no diff.
Run: `cargo test --lib`
Expected: pass.
Run: `cargo test --test live_docker -- --ignored`
Expected: pass against the local Docker daemon; if the daemon is not available, report that instead of claiming success.

- [ ] **Step 5: Commit**

```bash
git add tests/live_docker.rs README.md
git commit -m "test: cover live stats and document the usage views"
```
