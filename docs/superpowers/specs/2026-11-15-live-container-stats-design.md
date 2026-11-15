# Live container CPU and memory — design

Supersedes the "deferred to v1.1" call in
`2026-11-14-docker-gpui-client-design.md` for live stats only.

## Intent

Docker Desktop's signature columns are CPU and memory. contUI deliberately
left them out because each running container needs its own `/stats` stream.
The product now accepts that cost: a running container's CPU and memory should
be visible in the containers list and on its detail page, updated about every
two seconds, without polling the daemon or freezing the list.

## Goals

- `CPU` and `MEM` columns in the containers table for running containers.
- The same two readings in the container detail hero.
- Values match `docker stats` closely enough to trust: CPU as a percentage of
  one core (can exceed 100%), memory as working set over limit (cache
  subtracted, like the CLI).
- No list scroll, selection, filter or sort resets because a sample arrived.
- Streams exist only while the containers list or a container detail is on
  screen, and go away when those containers stop, disappear, or the host
  switches.

## Non-goals

- History, sparklines, charts or peak markers. Only the latest sample.
- Stats for stopped or paused containers; they show `—`.
- Network, disk, PID or GPU stats.
- Host-level aggregate stats, or stats on the Images and Volumes pages.
- Changing the `/events`-driven refresh model for containers, images or
  volumes.
- A configurable sample interval or a settings toggle.
- A stale-value badge. A disconnected host already has its own header state.

## Constraints and current state

- Docker's `/stats` stream emits once per second. There is no interval
  parameter. To honour "two seconds is enough", the service keeps the 1s
  stream but forwards at most one sample per container every 2s. Each
  displayed CPU % is therefore a 1s average refreshed every 2s.
- `bollard 0.21.1` exposes `Docker::stats(id, StatsOptions)` as a stream.
  `stream=true, one_shot=false` yields `precpu_stats`, which the CPU formula
  needs. A one-shot sample omits `preread` and can report 0% for its first
  reading.
- The containers table is rebuilt from a `Snapshot` and already carries
  query/sort/selection across rebuilds, but not the `UniformListScrollHandle`.
  A stats refresh must not go through a full table rebuild.
- The UI thread never touches the network; the service actor owns the Docker
  client and sends `Response`s through an `async_channel`.
- Supported hosts are local unix sockets, `tcp://`/`http://` and TLS contexts,
  all Linux containers under a macOS client.

## Approaches considered

1. **One streaming `/stats` per visible running container** (chosen). Highest
   fidelity, correct CPU deltas, one held connection per container. The cost
   is bounded by watching only running containers on the containers page, or
   the single open container's detail page.
2. **One-shot samples on a 2s timer.** No held streams and trivial teardown,
   but every reading is up to 2s old, the daemon has to answer N requests per
   tick anyway, and the first CPU sample after a start is usually 0. Rejected
   because it is not cheaper in daemon work and is less accurate.
3. **Detail page only.** One stream for the open container, no table columns.
   Cheapest, but it does not deliver the requested columns. Rejected.

## Design

### Model (`src/docker/model.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContainerStats {
    /// `None` until a real previous sample exists; Docker's first stream frame
    /// carries zeros, not a usable delta.
    pub cpu_percent: Option<f32>,
    pub memory: Option<MemoryUsage>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemoryUsage {
    pub used: u64,
    pub limit: u64,
    /// `None` when the daemon reports no limit.
    pub percent: Option<f32>,
}
```

`ContainerStats::from_response(&ContainerStatsResponse) -> Option<Self>` returns
`None` only when neither reading is usable, so CPU and memory become ready
independently. A populated memory reading must not be hidden behind a missing
CPU delta.

- **CPU**: `(cpu_delta / system_delta) * online_cpus * 100`, using
  `cpu_stats.cpu_usage.total_usage` and `system_cpu_usage` minus their
  `precpu_stats` counterparts. `online_cpus` falls back to the length of
  `percpu_usage`, then 1. If the previous values are missing, or either
  previous total is zero (Docker's first frame sends zeros), or the system
  delta is zero, the result is `None` rather than a bogus burst. Clamp real
  values to `>= 0`. Percentages above 100 are kept: 100% means one full core.
- **Memory**: `usage - inactive_file`, where `inactive_file` is read from
  `memory_stats.stats` as `total_inactive_file` (cgroups v1) or
  `inactive_file` (v2), whichever exists; absent means no subtraction. Use
  `saturating_sub`. `percent` is used/limit, or `None` when the limit is 0 or
  missing.

Formatting helpers next to the existing `format_bytes`:

- `format_cpu(Option<f32>) -> Option<String>`, one decimal, no thousands
  separators (`"12.3%"`).
- `format_memory(Option<MemoryUsage>) -> Option<String>` via `format_bytes`:
  `"412 MB / 2.0 GB"`, or just the usage when the limit is 0.

`format_bytes` stays as is. The table does not need a percentage label.

### Service (`src/docker/service.rs`)

New request:

```rust
Request::WatchStats { ids: Vec<String> },
```

The actor keeps `HashMap<String, JoinHandle<()>>` of stats tasks. On
`WatchStats` it aborts and forgets every task whose id is not in the new set,
and spawns a task for each new id. A task whose handle has finished is
replaced even if its id is still wanted.

Each task:

- opens `docker.stats(&id, StatsOptionsBuilder::new().stream(true).one_shot(false).build())`;
- for each sample, builds `ContainerStats`; forwards `Response::Stats { id, stats }` at most once every 2 seconds (the first usable sample goes immediately);
- when the stream ends or errors, waits 1s (capped at 5s) and reopens. It exits only by being aborted: the next `WatchStats` aborts ids that left the set, and dropping the last service shuts the runtime down. `DockerService::drop` changes from dropping the `Runtime` bare to `shutdown_timeout(Duration::from_millis(500))`, so switching hosts or quitting cannot hang on a stuck stats connection while still cancelling every task and stream.

New response:

```rust
Response::Stats { id: String, stats: ContainerStats },
```

No error response for stats: a missing sample is not a host failure and must
not raise the error banner. Rows keep their last value while a reconnect runs;
the next snapshot and sample replace them.

### Targets and lifecycle (`src/ui/root.rs`)

`RootView` gains:

```rust
pub container_stats: HashMap<String, ContainerStats>,
watched_stats: HashSet<String>,
```

(`stats` is already the `OverviewStats` field; do not collide with it.)

`stats_targets()` is a pure helper over page, detail and snapshot:

- not on the containers page: empty;
- container detail open: just that container, and only while it is running;
- otherwise: every running container in the snapshot.

Collapsed Compose groups and the search query do **not** prune targets. "On
screen" means the containers list or a detail page, not each individual row;
this keeps targets a pure function of page/detail/snapshot and avoids
restarting streams on every keystroke. A collapsed group therefore keeps its
running containers' streams alive, exactly as Docker Desktop does.

`sync_stats()` replaces `watched_stats` with that set, drops stats for ids no
longer watched, rebuilds the containers rows from the snapshot plus the
remaining stats, and sends `Request::WatchStats`. It runs after a snapshot, a
page switch, opening or closing a detail, and after connect. `connect` and the
host reset clear both collections. A late `Response::Stats` for an id that is
no longer watched is ignored.

This makes the stream set follow the visible surface: leaving Containers stops
all streams; opening a container's detail stops the other containers' streams;
going back starts the list's streams again. The first sample after each switch
arrives within about a second, so values briefly read `—` rather than showing
minutes-old numbers.

### Table (`src/ui/tables.rs`, `src/ui/table.rs`, `src/ui/root/view.rs`)

Columns become:

| # | Header | Width | Style |
|---|--------|-------|-------|
| 0 | Name | Flex 2.0 | Text |
| 1 | State | 100 fixed | Pill |
| 2 | CPU | 62 fixed | Cpu |
| 3 | MEM | 120 fixed | Memory |
| 4 | Image | Flex 1.8 | Mono |
| 5 | Ports | 120 fixed | Mono |
| 6 | Created | 96 fixed | Timestamp |

`CellStyle` gains `Cpu` and `Memory`. Their row cells carry the formatted
label for filtering (`"12.3%"`, `"412 MB / 2.0 GB"`) and numeric sort values
(percent × 10; used bytes). A missing CPU reading and a missing memory reading
are independent: each stores `—` and `-1`, so they sort below real readings.

Rendering for those two styles is a compact stack: the label in mono 11px, and
a 3px bar under it. `Cpu` fills `theme.accent` at `percent / 100` (clamped to
1). `Memory` fills `theme.running`, turning `theme.paused` at 90% or more. With
no reading the cell is a dim mono `—` and no bar; a memory reading with no
limit shows its label without a bar. The pill/actions paths are untouched.

### Scale

This is built for tens of running containers: at 2s per container, a 30-row
list triggers about 15 row rebuilds a second, which is cheap. Nothing batches
samples in this version; each `Response::Stats` does a `replace_rows` and one
`notify`. A hundred running containers still works but re-sorts the list 50
times a second when sorted by CPU, which is visibly busy. The model and
response path leave room for a future coalescing layer without a redesign; it
is out of scope here.

`containers_table` takes `&HashMap<String, ContainerStats>`.
`tables_from` passes it through. `RootView` rebuilds only the containers table's
rows on a sample, not its spec:

- add `TableState::replace_rows(rows)`, which remembers the selected row id,
  swaps the rows, recomputes the filtered/sorted view, and reselects by id;
- `TableState::adopt_state` also clones `previous.scroll` so a snapshot rebuild
  keeps the user's scroll position. The handle is an `Rc`, so this is cheap.

The spec, scroll handle and sort stay put on a stats tick. Grouped Compose rows
are unaffected: they read row ids and container state from the same tables and
snapshot as today.

### Detail hero (`src/ui/root/view.rs`)

For a container, a compact metrics block sits between the title/meta column
and the action buttons, about 170px wide:

```
CPU 12.3%          MEM 412 MB / 2.0 GB
▂▂▂▂▂▂▂▂░░░░░░     ▂▂▂▂▂▂▂▂▂▂▂▂░░░░
```

Labels are 9px faint; values 13px semibold mono; bars 3px, same colours and
thresholds as the table. The block is omitted for non-container details. CPU
and MEM each show `—` independently when the container is not running or that
reading is not yet available. It shares the table's `ContainerStats`, so table
and detail never disagree.

### Docs

- `README.md`: add live CPU/memory to "What it does" (2s refresh, running
  containers, bars) and delete the "No live CPU/memory columns" line from
  "Not in this version".
- Historical designs and plans stay as written; this document is the record
  of the reversal.

## Tests

Unit tests (`cargo test --lib`):

- `ContainerStats::from_response`: normal CPU delta with multiple cores;
  missing or zeroed `precpu_stats` yields `cpu_percent == None` while a valid
  memory reading still comes through; zero system delta yields `None`;
  percentages over 100 survive; cgroup v1 `total_inactive_file` and v2
  `inactive_file` are both subtracted; a missing limit yields `None` percent
  and usage-only formatting; completely empty stats yield `None`.
- `format_cpu`, `format_memory` edge cases (0%, limit 0, missing reading).
- `containers_table`: column titles/order, a running row with stats has the
  formatted CPU/MEM cells and numeric sort values, a row with only memory has
  a CPU `—` and `-1` beside real memory values, a stopped row has both `—`
  and `-1`.
- `TableState::replace_rows`: selection follows the row id across a reorder;
  query/sort survive.
- `stats_targets`: other pages yield none; a detail page yields only that
  container; a stopped container is excluded; the list page yields every
  running container, including ones hidden by a collapsed group or the search
  query.
- `adopt_state` keeps the same scroll handle (pointer equality).
- Service: `StatsOptions` is a streaming, non-one-shot request.

Manual/live check against Rancher Desktop:

- `cargo run`; compare the two columns with `docker stats --no-stream`.
- Watch a busy container update about every 2s; scroll a long list while it
  updates; sort by CPU and MEM; filter; open a detail and come back.
- Stop and remove containers; start a new one; switch hosts; stop the daemon
  and restart it.
- Confirm CPU is sane on a multi-core host (a single busy loop ≈ 100%) and
  memory matches `docker stats` after cache subtraction.

## What is left to the builder

- Exact bar corner radius, cell padding inside the two new columns, and
  whether the detail block is two stacked rows or one row of two metrics at
  narrow widths.
- Whether to extract the label+bar element into `ui/widgets` for the table and
  detail to share, or duplicate the dozen lines.
- Task ordering and commit granularity.
- Whether the ignored live-Docker test grows a stats case (nice, not required).
