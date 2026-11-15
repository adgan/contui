# Live Container CPU and Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show live CPU and memory for running containers in the containers table and detail page, fed by one `/stats` stream per on-screen container and refreshed every 2 seconds.

**Architecture:** The Docker actor owns the stats tasks and diffs a desired id set sent by the UI. The UI keeps the latest sample per container, renders it beside the snapshot, and updates table rows in place so scroll, selection, filter and sort survive. The model parses bollard's stream frame into independent optional CPU and memory readings.

**Tech Stack:** Rust 2024, GPUI `=0.2.2`, bollard `0.21.1`, tokio, async-channel, chrono.

**Spec:** `docs/superpowers/specs/2026-11-15-live-container-stats-design.md`

## Global Constraints

- No new dependencies; use the existing `bollard`, `tokio`, `gpui` and `async-channel`.
- The UI thread never touches the network. Every Docker call goes through the service actor.
- Forward at most one sample per container every 2 seconds (`Duration::from_secs(2)`). The daemon stream itself stays at 1s.
- A missing reading is `None` and renders as `—`; never render a fabricated `0.0%`.
- Watch running containers only, and only while the Containers page or a container detail is on screen.
- Stats failures never raise the error banner.
- Do not batch samples in this version; one `Response::Stats` causes one row refresh.
- `DockerService::drop` must cap runtime shutdown at 500ms.
- Historical design/plan documents stay untouched; only `README.md` and the new spec describe the reversal.
- Run `cargo test --lib` after every task; do not claim success without the output.

## Review Focus

- The first `/stats` frame carries zeroed `precpu_stats`: CPU must show `—`, while a valid memory reading must still display.
- A sample arriving while the user scrolls, sorts by CPU/MEM, filters, or has a row selected must not move the viewport or lose the selection.
- Leaving Containers or opening a detail must stop the other streams; returning must restart them without showing minutes-old values.
- Switching hosts or quitting with many running containers must not hang the UI past the 500ms shutdown cap or leak streams.
- Memory with no limit (0) shows usage without a bar and is not treated as missing; memory at 90% or more turns amber.

---

### Task 1: Parse stats into independent CPU and memory readings

**Files:**
- Modify: `src/docker/model.rs` (model types near `ContainerRow`; helpers near `format_bytes`; tests in the existing `mod tests`)

**Interfaces:**
- Consumes: `bollard::models::{ContainerStatsResponse, ContainerMemoryStats}`.
- Produces:
  - `pub struct ContainerStats { pub cpu_percent: Option<f32>, pub memory: Option<MemoryUsage> }`
  - `pub struct MemoryUsage { pub used: u64, pub limit: u64, pub percent: Option<f32> }`
  - `impl ContainerStats { pub fn from_response(response: &ContainerStatsResponse) -> Option<Self> }`
  - `pub fn format_cpu(percent: Option<f32>) -> Option<String>`
  - `pub fn format_memory(memory: Option<MemoryUsage>) -> Option<String>`

- [ ] **Step 1: Write the failing tests**

Add to `src/docker/model.rs`'s test module. Build raw frames with struct literals and `..Default::default()`.

```rust
fn stats_frame(
    total: u64,
    system: u64,
    pre_total: u64,
    pre_system: u64,
    usage: u64,
    limit: u64,
    cache_key: Option<(&str, u64)>,
) -> bollard::models::ContainerStatsResponse {
    use bollard::models::{
        ContainerCpuStats, ContainerCpuUsage, ContainerMemoryStats, ContainerStatsResponse,
    };
    let cpu = ContainerCpuStats {
        cpu_usage: Some(ContainerCpuUsage {
            total_usage: Some(total),
            percpu_usage: Some(vec![total / 4; 4]),
            ..Default::default()
        }),
        system_cpu_usage: Some(system),
        online_cpus: Some(4),
        ..Default::default()
    };
    let pre = ContainerCpuStats {
        cpu_usage: Some(ContainerCpuUsage {
            total_usage: Some(pre_total),
            percpu_usage: Some(vec![pre_total / 4; 4]),
            ..Default::default()
        }),
        system_cpu_usage: Some(pre_system),
        online_cpus: Some(4),
        ..Default::default()
    };
    ContainerStatsResponse {
        cpu_stats: Some(cpu),
        precpu_stats: Some(pre),
        memory_stats: Some(ContainerMemoryStats {
            usage: Some(usage),
            limit: Some(limit),
            stats: cache_key.map(|(key, value)| {
                std::collections::HashMap::from([(key.to_string(), value)])
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn parses_multi_core_cpu_and_cache_adjusted_memory() {
    let frame = stats_frame(
        2_000_000_000,
        8_000_000_000,
        1_000_000_000,
        6_000_000_000,
        300 * 1_048_576,
        1_024 * 1_048_576,
        Some(("inactive_file", 50 * 1_048_576)),
    );
    let stats = ContainerStats::from_response(&frame).unwrap();
    let cpu = stats.cpu_percent.unwrap();
    assert!((cpu - 200.0).abs() < 0.01, "got {cpu}");
    let memory = stats.memory.unwrap();
    assert_eq!(memory.used, 250 * 1_048_576);
    assert_eq!(memory.limit, 1_024 * 1_048_576);
    assert!((memory.percent.unwrap() - 24.414).abs() < 0.01);
}

#[test]
fn first_frame_has_no_cpu_but_keeps_memory() {
    let frame = stats_frame(
        2_000_000_000,
        8_000_000_000,
        0,
        0,
        300 * 1_048_576,
        1_024 * 1_048_576,
        Some(("total_inactive_file", 50 * 1_048_576)),
    );
    let stats = ContainerStats::from_response(&frame).unwrap();
    assert!(stats.cpu_percent.is_none(), "zeroed precpu must not be 0%");
    assert_eq!(stats.memory.unwrap().used, 250 * 1_048_576);
}

#[test]
fn formats_cpu_and_memory_labels() {
    assert_eq!(format_cpu(Some(12.34)).as_deref(), Some("12.3%"));
    assert_eq!(format_cpu(None), None);
    let memory = MemoryUsage {
        used: 432_013_312,
        limit: 2_147_483_648,
        percent: Some(20.1),
    };
    assert_eq!(
        format_memory(Some(memory)).as_deref(),
        Some("412 MB / 2.0 GB")
    );
    assert_eq!(
        format_memory(Some(MemoryUsage { limit: 0, ..memory })).as_deref(),
        Some("412 MB")
    );
    assert_eq!(format_memory(None), None);
}

#[test]
fn empty_stats_frame_is_missing() {
    assert!(ContainerStats::from_response(&Default::default()).is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib model::tests`

Expected: compile failure, `ContainerStats`/`MemoryUsage`/`format_cpu` not found.

- [ ] **Step 3: Implement the model**

Add the structs and `impl ContainerStats` in `src/docker/model.rs`. Import `ContainerStatsResponse` and `ContainerMemoryStats` from `bollard::models`.

Algorithm details:

- CPU: require `cpu_stats`, `precpu_stats`, both `cpu_usage.total_usage`, both `system_cpu_usage`. Return `None` if `pre_total == 0`, `pre_system == 0`, or `system <= pre_system`. Use `(system - pre_system)` as the delta, `online_cpus` or `percpu_usage.len()` or 1 for the core count, then `cpu_delta / system_delta * cores * 100.0`, clamped to `>= 0.0`.
- Memory: require `usage`; subtract `stats["total_inactive_file"]` or `stats["inactive_file"]` with `saturating_sub`; `limit` defaults to 0; `percent` is `None` when `limit == 0`, else used/limit × 100.
- `from_response` returns `Some` if either reading exists, `None` otherwise.
- `format_cpu` uses one decimal (`format!("{:.1}%", p.max(0.0))`).
- `format_memory` uses `format_bytes` for used and limit; limit 0 shows usage alone.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib model::tests`

Expected: all model tests pass, including the new four.

- [ ] **Step 5: Commit**

```bash
git add src/docker/model.rs
git commit -m "feat: parse container CPU and memory stats"
```

---

### Task 2: Stream stats through the Docker actor

**Files:**
- Modify: `src/docker/service.rs` (request/response enums, actor loop, drop implementation, tests)

**Interfaces:**
- Consumes: `crate::docker::model::ContainerStats` from Task 1.
- Produces:
  - `Request::WatchStats { ids: Vec<String> }`
  - `Response::Stats { id: String, stats: ContainerStats }`
  - `pub fn stats_options() -> bollard::query_parameters::StatsOptions` (stream=true, one_shot=false)
  - `fn stats_sample_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool`

- [ ] **Step 1: Write the failing tests**

Add to `src/docker/service.rs`'s test module:

```rust
#[test]
fn stats_options_stream_and_do_not_one_shot() {
    let options = stats_options();
    assert!(options.stream);
    assert!(!options.one_shot);
}

#[test]
fn stats_ui_samples_wait_two_seconds() {
    let now = std::time::Instant::now();
    assert!(stats_sample_due(None, now));
    assert!(!stats_sample_due(
        Some(now),
        now + std::time::Duration::from_millis(1_999)
    ));
    assert!(stats_sample_due(
        Some(now),
        now + std::time::Duration::from_secs(2)
    ));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib stats_`

Expected: compile failure, `stats_options`/`stats_sample_due` not found.

- [ ] **Step 3: Implement the streaming path**

In `src/docker/service.rs`:

1. Import `StatsOptionsBuilder` and `crate::docker::model::ContainerStats`; extend the existing `std::collections` import with `HashMap`. Import `tokio::task::JoinHandle`.
2. Add the request/response variants exactly as in Interfaces.
3. Add:
   ```rust
   const STATS_UI_INTERVAL: Duration = Duration::from_secs(2);

   pub fn stats_options() -> bollard::query_parameters::StatsOptions {
       StatsOptionsBuilder::new().stream(true).one_shot(false).build()
   }

   fn stats_sample_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
       last.is_none_or(|last| now.duration_since(last) >= STATS_UI_INTERVAL)
   }
   ```
4. In `actor`, add `let mut stats_tasks: HashMap<String, JoinHandle<()>> = HashMap::new();` and handle `Request::WatchStats { ids }`: build a `HashSet` of desired ids; for each existing entry, keep it only when desired and `!handle.is_finished()`, otherwise abort it; spawn `stats_task(docker.clone(), id, resp_tx.clone())` for each newly desired id. Import `tokio::task::JoinHandle` and `std::collections::HashSet`.
5. Write `async fn stats_task(docker: Docker, id: String, resp_tx: Sender<Response>)`:
   - loop: create the stream with `docker.stats(&id, Some(stats_options()))`; keep `last_sent: Option<Instant>`;
   - on each `Ok(frame)`, `ContainerStats::from_response(&frame)`, and when `stats_sample_due(last_sent, Instant::now())` send `Response::Stats { id: id.clone(), stats }` and update `last_sent`;
   - on stream end or error, sleep 1s and retry, backing off to 5s; exit only when the task is aborted.
6. In `DockerService::drop`, replace `self.runtime.take();` with:
   ```rust
   if let Some(runtime) = self.runtime.take() {
       runtime.shutdown_timeout(Duration::from_millis(500));
   }
   ```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib service::tests`

Expected: pass. `cargo build` must still compile (no GPUI changes yet).

- [ ] **Step 5: Commit**

```bash
git add src/docker/service.rs
git commit -m "feat: stream per-container stats from the docker actor"
```

---

### Task 3: Add CPU and MEM columns and preserve table state

**Files:**
- Modify: `src/ui/table.rs` (`CellStyle`, `TableState::adopt_state`, new `replace_rows`, tests)
- Modify: `src/ui/tables.rs` (`containers_table`, `tables_from`, tests)
- Modify: `src/ui/root.rs` (temporary call-site arguments only)
- Modify: `src/ui/root/view.rs` (temporary exhaustive `CellStyle` arms only)

**Interfaces:**
- Consumes: `ContainerStats`, `format_cpu`, `format_memory` from Task 1.
- Produces:
  - `CellStyle::Cpu` and `CellStyle::Memory`
  - `pub fn containers_table(snapshot: &Snapshot, stats: &HashMap<String, ContainerStats>) -> TableState`
  - `pub fn tables_from(snapshot: &Snapshot, stats: &HashMap<String, ContainerStats>, previous: &[TableState; 3]) -> [TableState; 3]`
  - `impl TableState { pub fn replace_rows(&mut self, rows: Vec<TableRow>) }`

- [ ] **Step 1: Write the failing tests**

In `src/ui/table.rs`'s test module:

```rust
#[test]
fn replace_rows_keeps_selection_by_row_id_and_scroll_handle() {
    let mut t = TableState::new(spec());
    t.set_rows(vec![row("a", "alpha", "img"), row("b", "beta", "img")]);
    t.selected = Some(1);
    t.click_header(0);
    let scroll = t.scroll.clone();

    t.replace_rows(vec![row("b", "beta", "img"), row("a", "alpha", "img")]);
    assert_eq!(t.selected, Some(0), "selection follows b");
    assert_eq!(t.rows[0].id, "b");
    assert_eq!(t.sort, Some((0, true)));
    assert!(std::rc::Rc::ptr_eq(&t.scroll.0, &scroll.0));
}

#[test]
fn adopt_state_keeps_the_same_scroll_handle() {
    let previous = TableState::new(spec());
    let mut fresh = TableState::new(spec());
    fresh.adopt_state(&previous);
    assert!(std::rc::Rc::ptr_eq(&fresh.scroll.0, &previous.scroll.0));
}
```

In `src/ui/tables.rs`'s test module, update every `containers_table(&snapshot)` to `containers_table(&snapshot, &HashMap::new())` and every `tables_from(&snapshot, &previous)` to `tables_from(&snapshot, &HashMap::new(), &previous)`. Add:

```rust
#[test]
fn containers_table_has_cpu_and_memory_columns() {
    let table = containers_table(&Snapshot::empty(), &HashMap::new());
    assert_eq!(
        table.spec.columns,
        vec!["Name", "State", "CPU", "MEM", "Image", "Ports", "Created"]
    );
}

#[test]
fn running_rows_show_stats_and_stopped_rows_show_dashes() {
    let snapshot = Snapshot {
        containers: vec![
            container("web", ContainerState::Running),
            container("job", ContainerState::Exited),
        ],
        ..Snapshot::empty()
    };
    let stats = HashMap::from([(
        "web-id".to_string(),
        ContainerStats {
            cpu_percent: Some(12.34),
            memory: Some(MemoryUsage {
                used: 432_013_312,
                limit: 2_147_483_648,
                percent: Some(20.1),
            }),
        },
    )]);
    let table = containers_table(&snapshot, &stats);
    let web = table.rows.iter().find(|r| r.cells[0].as_ref() == "web").unwrap();
    let job = table.rows.iter().find(|r| r.cells[0].as_ref() == "job").unwrap();
    assert_eq!(web.cells[2].as_ref(), "12.3%");
    assert_eq!(web.cells[3].as_ref(), "412 MB / 2.0 GB");
    assert!(matches!(web.sort_values[2], SortValue::Number(123)));
    assert!(matches!(web.sort_values[3], SortValue::Number(432_013_312)));
    assert_eq!(job.cells[2].as_ref(), "—");
    assert_eq!(job.cells[3].as_ref(), "—");
    assert!(matches!(job.sort_values[2], SortValue::Number(-1)));
}

#[test]
fn memory_only_rows_leave_cpu_missing() {
    let snapshot = Snapshot {
        containers: vec![container("web", ContainerState::Running)],
        ..Snapshot::empty()
    };
    let stats = HashMap::from([(
        "web-id".to_string(),
        ContainerStats {
            cpu_percent: None,
            memory: Some(MemoryUsage { used: 1_048_576, limit: 0, percent: None }),
        },
    )]);
    let table = containers_table(&snapshot, &stats);
    assert_eq!(table.rows[0].cells[2].as_ref(), "—");
    assert_eq!(table.rows[0].cells[3].as_ref(), "1.0 MB");
}
```

`container(name, state)` returns rows with id `{name}-id` (the existing test helper).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib ui::table`

Expected: compile failure on the new `CellStyle` variants, signatures, and `replace_rows`.

- [ ] **Step 3: Implement the columns and state preservation**

In `src/ui/table.rs`:

- Add `Cpu` and `Memory` to `CellStyle`.
- In `adopt_state`, add `self.scroll = previous.scroll.clone();` before recompute.
- Add `replace_rows` that stores `self.selected_row().map(|row| row.id.clone())`, assigns rows, calls `recompute()`, then reselects by id via `position`.

In `src/ui/tables.rs`:

- `containers_table(snapshot, stats)` uses columns `Name, State, CPU, MEM, Image, Ports, Created`; widths `Flex(2.0), Fixed(100.0), Fixed(62.0), Fixed(120.0), Flex(1.8), Fixed(120.0), Fixed(96.0)`; styles `Text, Pill, Cpu, Memory, Mono, Mono, Timestamp`.
- For each row, look up `stats.get(&c.id)` and build cells:
  - CPU: `format_cpu(stats.and_then(|s| s.cpu_percent)).unwrap_or_else(|| "—".into())`
  - MEM: `format_memory(stats.and_then(|s| s.memory)).unwrap_or_else(|| "—".into())`
- Sort values: CPU `SortValue::Number(cpu.map(|p| (p * 10.0).round() as i64).unwrap_or(-1))`; MEM `SortValue::Number(memory.map(|m| m.used as i64).unwrap_or(-1))`.
- `tables_from` takes `stats` and passes it to `containers_table`.

Keep the build green before committing:

- In `src/ui/root.rs`, pass `&HashMap::new()` at every `containers_table`/`tables_from` call (`new`, `rebuild_tables`, and test call sites). Task 4 replaces these with the live map.
- In `src/ui/root/view.rs`, add `CellStyle::Cpu | CellStyle::Memory` arms to the cell `match`, styling them like `Mono` text with no bar for now. Task 5 replaces those arms with meters.
- In `src/ui/tables.rs`'s test module, import `crate::docker::model::{ContainerStats, MemoryUsage}` and `std::collections::HashMap`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::table`

Run: `cargo test --lib ui::tables`

Expected: both pass, and `cargo build` succeeds. The temporary empty maps and text fallbacks are replaced in Task 4/5.

- [ ] **Step 5: Commit**

```bash
git add src/ui/table.rs src/ui/tables.rs
git commit -m "feat: add CPU and memory table columns"
```

---

### Task 4: Wire stats targets and responses into the view

**Files:**
- Modify: `src/ui/root.rs` (fields, `new`, `connect`, `apply`, `switch_page`, `open_detail`, `close_detail`, helper + tests)

**Interfaces:**
- Consumes: `Request::WatchStats`, `Response::Stats` from Task 2; `containers_table` and `replace_rows` from Task 3.
- Produces:
  - `pub container_stats: HashMap<String, ContainerStats>` on `RootView`
  - `fn stats_targets_for(page: Page, detail: Option<&DetailState>, containers: &[ContainerRow]) -> HashSet<String>`
  - `fn sync_stats_targets(&mut self)`

- [ ] **Step 1: Write the failing test**

Add to `src/ui/root.rs`'s test module:

```rust
#[test]
fn stats_targets_follow_page_and_detail() {
    let containers = vec![
        container("web", ContainerState::Running),
        container("db", ContainerState::Running),
        container("job", ContainerState::Exited),
    ];
    assert!(stats_targets_for(Page::Images, None, &containers).is_empty());
    assert_eq!(
        stats_targets_for(Page::Containers, None, &containers),
        HashSet::from(["web-id".to_string(), "db-id".to_string()])
    );

    let web = DetailState::new(ResourceKind::Container, "web-id".into(), "web".into());
    assert_eq!(
        stats_targets_for(Page::Containers, Some(&web), &containers),
        HashSet::from(["web-id".to_string()])
    );

    let stopped = DetailState::new(ResourceKind::Container, "job-id".into(), "job".into());
    assert!(stats_targets_for(Page::Containers, Some(&stopped), &containers).is_empty());

    let image = DetailState::new(ResourceKind::Image, "web-id".into(), "web".into());
    assert!(stats_targets_for(Page::Containers, Some(&image), &containers).is_empty());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib stats_targets_follow`

Expected: compile failure, `stats_targets_for` not found.

- [ ] **Step 3: Implement targets and lifecycle**

In `src/ui/root.rs`:

1. Import `ContainerRow`, `ContainerStats`, `format_*` not needed here, and `Response::Stats`.
2. Add fields to `RootView`: `pub container_stats: HashMap<String, ContainerStats>` and `watched_stats: HashSet<String>`; initialize both empty in `new`.
3. Add the free function:
   ```rust
   fn stats_targets_for(
       page: Page,
       detail: Option<&DetailState>,
       containers: &[ContainerRow],
   ) -> HashSet<String> {
       if page != Page::Containers {
           return HashSet::new();
       }
       if let Some(detail) = detail {
           if detail.kind != ResourceKind::Container {
               return HashSet::new();
           }
           return containers
               .iter()
               .filter(|c| c.id == detail.id && c.state == ContainerState::Running)
               .map(|c| c.id.clone())
               .collect();
       }
       containers
           .iter()
           .filter(|c| c.state == ContainerState::Running)
           .map(|c| c.id.clone())
           .collect()
   }
   ```
4. Add `sync_stats_targets`: compute targets; set `watched_stats`; retain only `container_stats` in targets; if on Containers, rebuild `containers_table(&self.snapshot, &self.container_stats).rows` into `self.tables[0].replace_rows(...)`; send `Request::WatchStats { ids: targets.into_iter().collect() }` when a service exists.
5. In `connect`, after `reset_host_data`, clear `container_stats` and `watched_stats`.
6. In `apply`:
   - Snapshot arm: after `self.snapshot = snapshot`, collect the running ids into a local `HashSet` and retain only those entries in `container_stats`; after the existing pending-select/detail block, call `sync_stats_targets()`.
   - Add `Response::Stats { id, stats }`: if `watched_stats.contains(&id)`, insert into `container_stats`, and when on Containers rebuild rows via `containers_table(...).rows` + `replace_rows`.
7. Replace the temporary `&HashMap::new()` arguments in `new` and `rebuild_tables` with `&self.container_stats` (the tables' initial construction can keep an empty map).
8. Call `sync_stats_targets()` at the end of `switch_page`, `open_detail`, and `close_detail`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib stats_targets_follow`

Expected: pass. Then run `cargo build`; expect success.

- [ ] **Step 5: Commit**

```bash
git add src/ui/root.rs
git commit -m "feat: watch stats for the containers on screen"
```

---

### Task 5: Render meters in the table and detail hero

**Files:**
- Create: `src/ui/widgets/usage_meter.rs`
- Modify: `src/ui/widgets/mod.rs`
- Modify: `src/ui/root/view.rs` (table cell match, detail hero)

**Interfaces:**
- Consumes: `CellStyle::Cpu`/`Memory` from Task 3; `RootView.container_stats` and `format_cpu`/`format_memory` from Tasks 1 and 4.
- Produces:
  - `pub fn usage_meter(label: &str, fraction: Option<f32>, color: gpui::Rgba, theme: &crate::theme::Theme) -> gpui::Div`
  - `pub fn memory_color(percent: Option<f32>, theme: &crate::theme::Theme) -> gpui::Rgba`

- [ ] **Step 1: Write the failing widget test**

Create `src/ui/widgets/usage_meter.rs` with a test module:

```rust
#[test]
fn memory_color_turns_amber_at_ninety_percent() {
    let theme = crate::theme::Theme::dark();
    assert_eq!(memory_color(Some(89.9), &theme), theme.running);
    assert_eq!(memory_color(Some(90.0), &theme), theme.paused);
    assert_eq!(memory_color(None, &theme), theme.running);
}
```

Add `pub mod usage_meter;` to `src/ui/widgets/mod.rs`.

Run: `cargo test --lib memory_color_turns` → expected compile failure.

- [ ] **Step 2: Implement the widget**

`usage_meter` returns:

```rust
div()
    .w_full()
    .min_w(px(0.))
    .flex()
    .flex_col()
    .gap(px(3.))
    .text_size(px(11.))
    .line_height(px(14.))
    .font_family("JetBrains Mono")
    .text_color(if label == "—" { theme.text_faint } else { theme.text_dim })
    .child(div().truncate().child(label.to_string()))
    .when_some(fraction, |el, fraction| {
        let fraction = fraction.clamp(0.0, 1.0);
        let width = if fraction <= 0.0 {
            relative(0.)
        } else {
            relative(fraction.max(0.05))
        };
        el.child(
            div()
                .w_full()
                .h(px(3.))
                .rounded(px(2.))
                .bg(theme.border_subtle)
                .child(div().h_full().w(width).rounded(px(2.)).bg(color)),
        )
    })
```

Use a minimum visible width of 0.05 only for non-zero fractions; a zero fraction still draws nothing (use `relative(0.)`).

`memory_color` returns `theme.paused` when `percent >= 90.0`, else `theme.running`.

Run: `cargo test --lib memory_color_turns` → pass.

- [ ] **Step 3: Render the table cells**

In `src/ui/root/view.rs`, inside the row cell loop:

- After the `Pill` branch, add a branch for `CellStyle::Cpu | CellStyle::Memory` that:
  - looks up `this.container_stats.get(&row_id).copied()`;
  - CPU: fraction `cpu_percent.map(|p| p / 100.0)`, color `theme.accent`;
  - MEM: fraction `memory.and_then(|m| m.percent).map(|p| p / 100.0)`, color `memory_color(memory.and_then(|m| m.percent), &theme)`;
  - wraps `usage_meter(cell.as_ref(), fraction, color, &theme)` in `div().map_width(width).h_full().flex().flex_col().justify_center()`;
  - `continue`s.
- Keep the existing text match exhaustive by adding `CellStyle::Cpu | CellStyle::Memory => {}` (unreachable after the branch) or restructuring the match.

- [ ] **Step 4: Render the detail hero**

In `render_detail`, for a container only, build a metrics block:

```rust
let running = state == Some(ContainerState::Running);
let stats = running.then(|| self.container_stats.get(&detail.id).copied()).flatten();
let cpu_label = format_cpu(stats.and_then(|s| s.cpu_percent))
    .map(|v| format!("CPU {v}"))
    .unwrap_or_else(|| "CPU —".to_string());
let memory = stats.and_then(|s| s.memory);
let mem_label = format_memory(memory)
    .map(|v| format!("MEM {v}"))
    .unwrap_or_else(|| "MEM —".to_string());
let metrics = div()
    .w(px(180.))
    .flex_shrink_0()
    .flex()
    .flex_col()
    .gap(px(10.))
    .child(usage_meter(
        &cpu_label,
        stats.and_then(|s| s.cpu_percent).map(|p| p / 100.0),
        theme.accent,
        &theme,
    ))
    .child(usage_meter(
        &mem_label,
        memory.and_then(|m| m.percent).map(|p| p / 100.0),
        memory_color(memory.and_then(|m| m.percent), &theme),
        &theme,
    ));
```

Insert it into the hero only when `detail.kind == ResourceKind::Container`, between the title/meta column and `action_column`.

- [ ] **Step 5: Build and test**

Run: `cargo test --lib`

Expected: all tests pass.

Run: `cargo build`

Expected: success, no warnings about unused variants/imports.

- [ ] **Step 6: Commit**

```bash
git add src/ui/widgets/usage_meter.rs src/ui/widgets/mod.rs src/ui/root/view.rs
git commit -m "feat: render CPU and memory meters"
```

---

### Task 6: Update the README and verify the whole suite

**Files:**
- Modify: `README.md` ("What it does", "Not in this version", "Layout" if needed)

**Interfaces:**
- Consumes: everything above.
- Produces: product documentation that matches the shipped behavior.

- [ ] **Step 1: Update the README**

Add to "What it does":

```markdown
- Shows live CPU and memory for running containers: one `/stats` stream per
  on-screen container, refreshed every ~2s, with a bar per reading. Stopped
  containers show `—`; leaving the containers list stops the streams.
```

Delete the line `- No live CPU/memory columns (that needs one /stats stream per running container).`

If the "Layout" block lists UI modules, mention the meter only if it already lists individual widgets; do not expand it otherwise.

- [ ] **Step 2: Run the library tests**

Run: `cargo test --lib`

Expected: all tests pass; note the count.

- [ ] **Step 3: Run the full test suite and build**

Run: `cargo test`

Expected: all non-ignored tests pass (the live-Docker test remains ignored).

Run: `cargo build`

Expected: success.

- [ ] **Step 4: Check the manual surface**

Run `cargo run` against Rancher Desktop and confirm:

- a busy container's CPU updates about every 2s and matches `docker stats --no-stream`;
- scrolling, filtering, sorting by CPU/MEM and selecting a row are not reset by samples;
- a stopped container shows `—` in both columns;
- opening a detail leaves only that container's stream, and going back restores the list;
- switching between Containers, Images and Volumes does not leak streams or show stale values;
- memory at 90%+ uses the amber bar; a no-limit container shows usage with no bar.

Record the results in the final report. If no Docker daemon is available, say so and list the checks as unverified.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: document live container stats"
```

---

## Self-Review

- Spec coverage: model (Task 1), service streams/2s/teardown (Task 2), table columns/state (Task 3), targets/lifecycle/response (Task 4), rendering/colors/detail (Task 5), docs (Task 6). All spec sections have a task.
- Type consistency: `ContainerStats`, `MemoryUsage`, `format_cpu`, `format_memory`, `stats_targets_for`, `stats_options`, `stats_sample_due`, `containers_table(snapshot, stats)`, `tables_from(snapshot, stats, previous)`, `replace_rows`, `usage_meter`, `memory_color` are used with the same signatures in every task.
- Review Focus: each line has a pinning test in Tasks 1, 3, 4 or 5; host-switch shutdown and the restart-without-stale-values flow are manual checks because they need a live runtime.
