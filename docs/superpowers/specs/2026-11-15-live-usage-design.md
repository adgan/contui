# contUI — live usage overview and container detail — design

Date: 2026-11-15
Status: awaiting approval

## Intent

contUI can show every container, but not what any of them are costing. The README lists
"no live CPU/memory columns" as a known gap. The chosen shape is not columns in the
list:

- **Overview:** one host-wide usage number in the overview strip, so the dashboard
  answers "is the engine busy?" without opening anything.
- **Detail:** the selected container's live CPU and memory on its detail page, so
  "which one is busy?" has an answer once you drill in.
- **List:** no new columns. It stays the fast, scannable index it is today.

This needs one `/stats` stream per running container, because Docker has no global
stats endpoint. The streams, their lifecycle and their failure behaviour are the
substance of this change; the two cards are the visible part.

## Goals

1. Show host-wide CPU and memory used by running containers in the overview strip,
   updated about once per second.
2. Show live CPU and memory for the open container on its detail page, including the
   memory limit when the daemon reports one.
3. Keep one `/stats` stream per running container, started and stopped by the Docker
   service, never by the render path.
4. Keep the list table, its columns, sorting, filtering and Compose grouping exactly as
   they are.
5. Stay honest when data is missing: `—` until a valid sample exists, no invented
   numbers, no animation.

## Non-goals

- No CPU/memory columns in the containers table.
- No history, sparklines, charts, averages or peak values.
- No per-container network or block I/O in this version. The same sample carries them;
  they can be added later without new plumbing.
- No stats for stopped or paused containers. No historical usage after a container stops.
- No new page, no new tab, no changes to inspect or logs.
- No changes to start/stop/restart/remove/run/pull.
- No polling timer and no `docker stats`-style one-shot requests.

## Anti-goals

A version that technically works but is a failure:

- Numbers that freeze while the app looks live, or values that silently stay after the
  container stopped.
- One stream leaking per snapshot refresh, or a stream left running for a removed
  container.
- A repaint per sample that makes the table stutter when ten containers are running.
- A host total that is obviously wrong because it double-counts or includes stopped
  containers.
- A detail page where the usage band shifts the layout or covers the inspect body.
- Adding CPU/memory columns back into the list.

## Constraints

- GPUI 0.2.2, Rust 2024, one binary crate.
- The UI thread never touches the network. Docker calls stay on the tokio runtime in
  `DockerService`.
- The service actor owns the Docker client and the lifetime of long-running work.
- `theme.rs` and `ui/style.rs` remain the source of truth for colour, spacing, radii and
  type.
- The overview strip keeps its authoritative height (`88px`, `76px` compact); content
  truncates rather than growing it.
- No new crates.

## Approaches considered

**1. Actor-owned long-lived streams (chosen).** The actor keeps a map of running
container id → stats task. On every snapshot it starts streams for newly running
containers, aborts streams for containers that stopped or disappeared, and leaves the
rest alone. Each task loops on `docker.stats(id, stream(true))`, converts each sample,
and sends one `Response::Usage`. A stream that errors retries with backoff until the
actor aborts it. This matches the API, gives true live values, and puts cancellation in
one place.

**2. One-shot polling.** Ask `/stats?stream=false&one_shot=true` for every running
container every second. No long-lived streams and no cancellation map, but it is no
longer the event-driven client the README promises, it multiplies requests, and the
one-shot response does not always carry a usable CPU delta. Rejected.

**3. UI-owned streams.** Let the view start and stop stats tasks when pages change.
This keeps the service simpler but ties network lifetime to render state, makes detail
and overview fight over the same stream, and risks leaks on page switches. Rejected.

There is no global stats endpoint, so some per-container work is unavoidable. The only
real choice is who owns it; the actor already owns every other long-running Docker call.

## Design

### Data model

Add to `src/docker/model.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContainerUsage {
    pub cpu_percent: Option<f32>,   // Docker's percentage; 100% = one full core
    pub memory_usage: Option<u64>,  // working set, cache/inactive_file subtracted
    pub memory_limit: Option<u64>,
    pub online_cpus: u32,
}
```

`ContainerUsage::from_sample(&ContainerStatsResponse) -> Option<ContainerUsage>` returns
`None` only when neither CPU nor memory is available. Partial samples are kept, so a
first sample without a usable CPU delta still gives the detail page its memory number.

- **CPU:** `(cpu_delta / system_delta) * online_cpus * 100`, using
  `cpu_stats.cpu_usage.total_usage` and `cpu_stats.system_cpu_usage` against
  `precpu_stats`. Docker's first streamed sample carries a present but zeroed
  `precpu_stats`, which would otherwise paint a lifetime average. CPU is `None` unless
  `precpu_stats.system_cpu_usage` is present and greater than zero; after that, a zero
  `cpu_delta` is a real `0.00%`, and a zero `system_delta` is `None`. `online_cpus`
  falls back to `percpu_usage.len()`, then `1`; it is the same divisor used to compute
  that sample's percentage.
- **Memory:** `usage - cache`, where cache is `total_inactive_file`, then
  `inactive_file`, then `cache` from `memory_stats.stats`, never below zero. Limit comes
  from `memory_stats.limit` when present and non-zero.
- **No percentages computed at render time.** The render path only formats.

Formatting helpers live beside the model:

- `format_cpu_percent(f32) -> String` always two decimals (`0.42%`, `12.50%`).
- `format_usage_bytes(u64) -> String` uses B/KB/MB/GB with one decimal above 1 KiB.
- `format_memory_pair(usage, limit) -> String` gives `184 MB / 2.0 GB` when a limit is
  present, otherwise `184 MB`.

### Collection and lifecycle

`src/docker/service.rs` gains:

```rust
Response::Usage { id: String, usage: ContainerUsage }
Response::UsageStale { id: String }
```

The actor gains `HashMap<String, tokio::task::JoinHandle<()>>` for stats tasks. On every
successful `FetchAll`:

1. Send the snapshot to the UI first, so the list and running set are current.
2. Compute the running ids (`ContainerState::Running` only; paused and restarting are
   not running).
3. Plan the difference against the task map:
   - running but absent → start a task;
   - running but the task finished → replace it;
   - no longer running or gone → abort and remove.
4. Drop every task when the service or host changes; the runtime owns them.

Each stats task:

- calls `docker.stats(id, Some(StatsOptionsBuilder::new().stream(true).build()))`;
- converts each `Ok(sample)` with `ContainerUsage::from_sample`; sends
  `Response::Usage` only for samples that yielded something;
- reads the next item through a 5s timeout. On timeout, stream end or error, sends
  `Response::UsageStale` for that id, then sleeps with 1s → 2s → 4s … 10s backoff and
  resubscribes, resetting the backoff after a good sample. The actor's abort is the stop
  signal;
- exits if the response channel closes.

The 5s timeout is per stream, not a UI polling timer. It is what stops a stalled stream
from leaving a frozen number on screen after the daemon stops sending samples.

The streams run for every running container while the host is connected, not only while
the Containers page is visible. The overview strip is global and must stay truthful on
Images and Volumes too, and starting/stopping streams on every page switch would make
the card flicker. The cost is one idle stream per running container while the app is
open, which is exactly the model the feature was scoped around.

### UI state

`RootView` gains `usage: HashMap<String, ContainerUsage>`, cleared on host change and
whenever the host status leaves `Connected` (so a red host chip never sits beside a
number that looks live).

- `Response::Usage` inserts/updates the entry.
- `Response::UsageStale` removes it.
- `Response::Snapshot` retains only ids that are still running.
- `UsageTotals::from(snapshot, usage)` sums only running containers and only the samples
  that exist:

  ```rust
  pub struct UsageTotals {
      pub cpu_percent: Option<f32>,   // host share, already normalized
      pub memory_bytes: Option<u64>,
      pub cpu_reporting: usize,
      pub memory_reporting: usize,
      pub running: usize,
  }
  ```

  Host CPU share is `sum(cpu_percent / online_cpus)` per sample, not a sum divided by one
  sample's core count: the divisor travels with the sample that produced the percentage,
  so mixed or memory-only samples cannot inflate the total.
- The UI pump drains queued responses before notifying, so a burst of samples from many
  containers causes one repaint rather than one per container.

Aggregate CPU is shown as **share of host capacity**, while the detail page keeps
Docker's per-container percentage (`100%` = one core). This is the one interpretation
worth objecting to. The alternative is to sum raw container percentages in the overview,
which can read `300%` on a three-core workload. The chosen version answers "how busy is
the host?"; the detail answers "how busy is this container?".

### Overview strip

Add a fourth card, `USAGE`, to the existing three. It is informational: clicking it
switches to Containers, but it never becomes the active page card.

```
┌──────────────────────┐
│ USAGE             ◇  │
│ 4.10%  CPU    3 of 3 │
│ 1.2 GB memory        │
└──────────────────────┘
```

- Kicker `USAGE`, icon `icons/activity.svg`.
- Value: host CPU share, two decimals. Unit: `CPU`.
- Right: CPU coverage. `2 of 3` while coverage is incomplete, `3 running` when every
  running container has a CPU sample. It counts CPU samples, because CPU is the headline;
  a memory-only sample never makes it read `3 of 3`.
- Foot: `1.2 GB memory`. When memory coverage is partial, append
  ` · 2/3` so the foot does not over-claim either.
- States:
  - no snapshot yet → `—`, foot `connecting…`;
  - host not connected → `—`, foot `host disconnected`;
  - no running containers → `0.00%`, foot `no running containers`;
  - running but no samples yet → `—`, foot `waiting for stats…`;
  - samples but no CPU yet → CPU `—`, foot still shows memory when present.

Four cards still fit at the 960px minimum: content width is 864px, each card gets about
207px. Text truncates; the strip height does not change. The card uses the existing
`stat_card` primitive and adds no new layout mode.

### Detail page

For container details only, insert a usage band between the hero and the tab strip. It
stays visible on both Inspect and Logs. Images and volumes are unchanged.

```
┌──────────────────────────────────────────────────────────────┐
│ CPU                                MEMORY                    │
│ 0.42%                              184 MB / 2.0 GB           │
│ current                            [██████░░░░░░░░░░]        │
└──────────────────────────────────────────────────────────────┘
```

- One card, two equal cells, 16px padding, 14px radius; the same panel surface as the
  rest of the detail page.
- CPU cell: kicker `CPU`, value at hero size, sub `of one core`.
- Memory cell: kicker `MEMORY`, value `usage / limit` from `format_memory_pair` (so just
  `usage` when there is no limit), no sub label, plus a 3px usage bar when a limit
  exists. The bar is `usage / limit`, clamped to 0–1, and uses the existing
  running/accent palette.
- States:
  - running, no sample → `—` in both cells, sub `waiting for stats…`;
  - not running → `—` in both cells, sub `container is not running`;
  - host not connected → `—` in both cells, sub `host disconnected`; the host chip remains
    the primary error, but the band does not keep a frozen number.
- The band has a fixed height (`88px`, `76px` compact) so switching tabs never shifts
  the body. The inspect body keeps its own scroll.

## Testing

- `docker/model.rs`: CPU delta math (missing or zeroed `precpu_stats`, a real zero
  `cpu_delta`, a zero `system_delta`, `online_cpus` fallback), memory cache selection in
  v1/v2 order, limit handling, partial samples, CPU/memory/byte formatting.
- `docker/service.rs`: `running_ids` excludes paused/restarting; stats plan starts new
  ids, keeps existing ids, and stops disappeared ids.
- `ui/stats.rs`: aggregate totals ignore stopped containers and missing samples, sum
  per-sample `cpu_percent / online_cpus` across samples with different core counts,
  report CPU and memory coverage separately, and produce the card's labels for every
  state above (including disconnected and partial coverage).
- `ui/root.rs`: a stale or disconnected usage entry disappears rather than staying in
  the map.
- `tests/live_docker.rs` (ignored): start a short-lived container, request a snapshot,
  and wait for a `Response::Usage` whose `cpu_percent` is `Some` (the first sample is
  deliberately suppressed) and whose memory value is present. In `wait_for`, check the
  predicate before ignoring noise, then treat `Response::Usage` and
  `Response::UsageStale` as noise; other live tests ignore them, while the stats test can
  still match them.
- `cargo test --lib` and `cargo test --test live_docker -- --ignored` against the local
  engine.

## What is left to the builder

- Exact module split for the detail band (inline in `view.rs` or a small widget module);
  the visual spec above is the contract.
- Whether the activity icon is a new SVG or reuses the containers mark; a 14px stroke
  icon matching the existing set is expected.
- The precise GPU diffing needed to keep the detail band cheap; the data flow forbids
  rebuilding tables on a stats tick.
