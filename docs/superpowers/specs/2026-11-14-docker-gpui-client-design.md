# contUI — a fast Docker client in GPUI — design

Date: 2026-11-14
Status: approved 2026-11-14

## Intent

A native macOS Docker client that replaces the Rancher Desktop UI, written in Rust on
Zed's GPUI. The complaint that started this: Rancher Desktop's UI is slow and ugly.
So speed and looks are not features to add later — they are the product. Every choice
below is judged against them.

The app navigates the way Docker Desktop does: an icon rail on the left, a table per
resource, clicking a row drills into a detail page with tabs, and a back arrow returns.

The product name is **contUI** (binary `contui`), settled 2026-11-14; it began as the
working name Dockyard and was renamed to contUI once the first implementation existed.

## Why now

Rancher Desktop's UI is a webview dashboard. It polls, it lags a beat behind reality,
and it spends seconds getting to a list that the daemon can answer in 20ms. A native
GPUI app has no webview, renders on the GPU, and can subscribe to the daemon's event
stream instead of guessing with a timer.

## Goals

1. Show **containers** (all, running and stopped), **images**, and **volumes** for the
   selected Docker host, live, with no visible refresh lag.
2. Navigate like Docker Desktop: rail → table → detail page → back. Nothing invented.
3. Work with **any Docker host**: `DOCKER_HOST`, `DOCKER_CONTEXT`, every Docker context
   in `~/.docker/contexts`, and auto-discovery of the common local sockets. Rancher
   Desktop is the first target and the one we test against. A host switcher in the
   header lists everything found. Scope: unix sockets, and `tcp://`/`http://` with and
   without TLS via `DOCKER_CERT_PATH`. `ssh://` hosts are listed but connect only when
   the SSH agent already holds a usable key (no passphrase prompt); full SSH support is
   v1.1.
4. Be a normal macOS app: native window, ⌘-key navigation, menu bar, no Xcode license
   requirement to build.
5. Look deliberate: dark-first theme, one accent, dense readable rows, no default-GPUI
   grey box energy.

## Non-goals for v1

- **No mutations.** No start/stop/restart/remove/prune/pull/run. Read-only. These are
  v2, and they change the architecture (confirmations, optimistic updates, error
  surfaces), so they get designed on their own.
- **No live stats.** Docker Desktop's CPU/memory columns need one `/stats` stream per
  running container. Deferred to v1.1; the container table shows Name, State, Image,
  Ports, Created instead. (Flagged for approval, see the end of this document.)
- **No log following.** The Logs tab shows a static tail (last 500 lines) fetched when
  the tab opens, with a refresh button. Continuous follow is v1.1.
- No Compose lifecycle control (Compose containers are grouped in the list, but
  start/stop/restart/remove act on one container at a time), no registry auth, no
  exec/shell, no builds, no Kubernetes, no extensions, no settings screen beyond
  the host switcher.
- No Windows or Linux support. GPUI runs there; we will not test or style for it.

## Anti-goals

A build that technically works but is a failure:

- Polling on a timer. That is the exact thing being replaced.
- A list that blocks the UI thread on a socket call.
- Default fonts, default colors, unstyled scrollbars.
- Any runtime dependency on the `docker` CLI binary.
- Shelling out to `docker ps` and parsing text.

## Constraints

- Machine: macOS 26.7.2, arm64. Rust 1.99.1 stable is installed; `cargo` is not on
  `PATH` (toolchain lives at `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin`).
- Engine: **Rancher Desktop** at `~/.rd/docker.sock`. `/var/run/docker.sock` does not
  exist. Verified live during design: 16 containers (15 stopped k8s leftovers, one
  running postgres), 45 images, 1 volume.
- Framework: `gpui-pre = "=0.3.12"` — Zed's own published snapshot of `main`, released
  weekly, pinned exactly. It has **no Metal build step**, so no Xcode license is
  needed. Entry point is `Application::new().run(|cx| ...)`.
- Docker API client: `bollard` 0.21 (async, tokio, unix socket + tcp + ssh).
- No Docker Desktop app is installed on this machine.

## Approaches considered

The question was how the app gets its data.

**1. bollard + one background tokio runtime + the daemon's `/events` stream (chosen).**
A single actor task owns the `bollard::Docker` handle on a tokio runtime pinned to a
worker thread. It answers list/inspect requests and simultaneously forwards
`/events` messages. The UI holds a snapshot and re-renders when the snapshot changes.
Cost: one bridge to write between tokio and GPUI. Buys: sub-second truth, no polling,
and the same plumbing that logs/stats/exec will need.

**2. bollard + a 2-second refresh timer.** Half the plumbing, no event stream. Rejected:
this is what makes Rancher's UI feel slow, and it burns CPU forever. Kept as a
fallback only if Rancher's socket turns out to drop events in a way we cannot heal.

**3. Shell out to the `docker` CLI.** Free "any host" support, a day of work. Rejected:
a process spawn per refresh is the slowness being replaced, and text parsing is
brittle.

Framework choice, decided during recon:

**`gpui-pre` 0.3.12 (chosen)** over `gpui` 0.2.2, which is a year stale and diverges
from the docs; and over `gpui-component`, which is a 60-widget library built on
`gpui-pre` but drags in ~250 transitive crates (resvg, markdown, ropey, syntect,
lsp-types) and would make a bespoke look a fight with someone else's theme. We hand-roll
the four widgets we need (search field, host menu, state pill, scrollbar) from GPUI's
own examples. `gpui-component` stays the escape hatch if one of them turns painful.

## Architecture

One binary crate.

```
src/
  main.rs               bootstrap, window options, menu bar
  theme.rs              colors, spacing, type scale — single source of truth
  assets.rs             SVG icon + font AssetSource
  docker/
    endpoint.rs         host discovery and probing
    service.rs          tokio runtime, request/response actor, reconnect
    events.rs           /events stream → EventKind, debounce
    model.rs            ContainerRow, ImageRow, VolumeRow, HostInfo, Snapshot
  ui/
    root.rs             RootView: page, host, snapshot, selection, search
    rail.rs             left icon rail
    header.rs           title, counts, search, host chip, refresh
    table.rs            generic table: sticky header, sorted rows, uniform_list
    detail.rs           detail page shell: back, title, tabs
    inspect.rs          generic JSON → grouped tree renderer
    logs.rs             monospace tail viewer
    widgets/
      text_field.rs     single-line input, caret, selection, IME-lite
      menu.rs           host picker popup
      pill.rs           state badge
      scrollbar.rs      styled scrollbar (adapted from GPUI's data_table example)
      empty.rs          empty + error states
assets/
  icons/*.svg           containers, image, volume, refresh, back
```

### Data flow

- `DockerService` owns `tokio::runtime::Runtime` (multi-thread, 2 workers) and one
  long-lived connection actor per host.
- UI → service: `async_channel::Sender<Request>` (`ListContainers`, `ListImages`,
  `ListVolumes`, `Inspect{..}`, `Logs{..}`, `Subscribe`).
- service → UI: `cx.spawn` receiver task on the GPUI side calls
  `cx.update_entity(|view, cx| view.apply(Response))`, which mutates the snapshot and
  notifies. No UI state is touched from a tokio thread.
- Model: `Snapshot { containers: Vec<ContainerRow>, images: …, volumes: …, fetched_at }`
  plus `HostStatus::{Connected, Connecting, Disconnected{reason}}`.
- **Refresh triggers:** app start, host switch, manual ⌘R, window focus, and any
  debounced (250ms) `EventKind` from `/events`. Navigating between pages re-requests
  that page's list if it is older than 5s.
- **Failure:** a failed request sets `HostStatus::Disconnected` with the reason; the
  header shows an amber chip and a retry affordance. Errors never panic and never clear
  the last good snapshot — stale data with a warning beats an empty screen.
- Reconnect: event stream end → mark disconnected, retry `ping` with 1s/2s/4s backoff
  to 10s cap, resume and resubscribe on success.

### Host discovery (`endpoint.rs`)

Deliverable: a list of `HostInfo { id, name, endpoint_kind, socket_or_url, status }`
shown in the switcher, plus one `active` host.

Resolution order sets the **default active host**; the switcher lists everything found:

1. `DOCKER_HOST` if set (`unix://`, `tcp://`, `http://`, `ssh://`).
2. `DOCKER_CONTEXT` if set, else `~/.docker/config.json` → `currentContext`.
3. Every directory under `~/.docker/contexts/meta/*/meta.json` — this is the CLI's own
   model, and it is how Rancher Desktop and Docker Desktop register themselves. Each
   becomes a switcher entry with its context `Name` and `Endpoints.docker.Host`, and its
   TLS material from `Storage.TLSPath` when present. A context with a non-empty
   `TLSMaterial` is connected with TLS; an unreachable or auth-failing one shows a
   distinct `AuthFailed` status, not "unreachable".
4. Socket candidates probed in parallel with a 300ms `docker.ping()` timeout:
   `~/.rd/docker.sock` (Rancher), `~/.docker/run/docker.sock` (Docker Desktop),
   `~/.colima/default/docker.sock`, `~/.orbstack/run/docker.sock`,
   `/var/run/docker.sock`. Listed only if they answer, so we never offer a dead path.

**Status dots are not a background job.** They are computed twice: once at launch for
the default host, and again each time the switcher menu opens (parallel pings, 300ms
timeout). Only the active host holds a live connection actor; there is no fan-out of
connections and no polling loop. A dot therefore means "answered a ping when this menu
opened", and the active host's chip additionally reflects live connection state from its
actor.

Switching hosts tears down the actor and rebuilds it against the chosen endpoint; the UI
paints a skeleton until the first response lands.

### Time labels

`Created` is stored as `chrono::DateTime<Utc>` and formatted to `2m ago` at fetch time
into a pre-formatted `SharedString`. Relative labels therefore go stale between
refreshes — that is deliberate: relative time is not allowed to introduce a repaint
timer. They update whenever a refresh happens (any event, focus, ⌘R), which on a live
host is often enough that staleness is never visible.

### UI

Window 1280×820, minimum 960×600. Transparent titlebar with inset traffic lights, the
product name and current page in the titlebar strip. If GPUI's transparent-titlebar
handling misbehaves, fall back to a standard titlebar — builder's call, not a blocker.

```
┌──────────────────────────────────────────────────────────────┐
│  ⌘ contUI · Containers          ⌕ search…      ⟳   runner ▾ │
├────┬─────────────────────────────────────────────────────────┤
│ ▣  │ Containers  16     3 running · 13 stopped                │
│ ▤  │                                                          │
│ ◧  │ Name              State     Image        Ports    Created │
│    │ ● sploney-db-1    running   postgres:18  5432    2m ago  │
│    │ ○ k8s_traefik…    exited    fcdef599     —       3mo ago │
│    │ ○ k8s_POD_svclb…  exited    pause:3.6    —       3mo ago │
└────┴─────────────────────────────────────────────────────────┘
```

- **Rail** (56px): Containers, Images, Volumes; bottom: host status dot. Selected item
  gets an accent bar, not a filled block.
- **Header**: page title, live counts, search field (⌘F), refresh button, host chip.
- **Table**: sticky header row, 32px rows, hover fill, selection fill, click to open.
  Columns are sortable by clicking a header (asc → desc → default). Default order:
  containers running-first then name; images newest first; volumes by name.
  Rows are `uniform_list` (virtualized) so thousands of rows and 45 images scroll
  identically.
  - Containers: Name · State · Image · Ports · Created
  - Images: Repository:Tag · ID · Size · Created · In Use (derived from container list,
    no extra request)
  - Volumes: Name · Driver · Mountpoint · Created
- **Search**: case-insensitive substring over every visible column, filtered as you
  type, Esc clears, ⌘F focuses. Empty result renders an empty state, not a blank table.
- **Detail** (containers, images, volumes): back arrow + `‹ Containers`, title, state
  pill where applicable, then tabs.
  - *Inspect*: a generic renderer over the inspect JSON — collapsible groups
    (Overview, Ports, Mounts, Environment, Labels, Config), key on the left in dim
    text, value monospace, long values truncated with a hover tooltip.
    Inspect is fetched when the page opens and re-fetched when a `/events` message
    names the same resource id (debounced with the rest). If the resource disappears,
    the page shows "no longer exists" with a back link rather than a stale payload.
    Inspect data lives in the detail view's own `Option<Inspect>` keyed by
    `(kind, id)` — it is not part of the list `Snapshot`.
  - *Logs* (containers only): monospace, last 500 lines, `stdout`/`stderr` merged,
    timestamps dimmed, horizontal scroll for long lines, refresh button.
  - One generic inspect renderer serves all three resource types; only the Logs tab is
    container-specific.
- **Keyboard**: ⌘1/⌘2/⌘3 switch pages, ↑/↓ move selection, Enter opens, Esc goes back
  (or clears search), ⌘R refreshes, ⌘F focuses search.
- **Empty/error states**: "No containers yet" / "Can't reach Rancher Desktop at
  ~/.rd/docker.sock — is the engine running?" with Retry.

### Theme

Dark-first, defined once in `theme.rs`:

| token | value |
|---|---|
| app background | `#0b0d10` |
| panel | `#12151a` |
| row hover | `#1a1f27` |
| row selected | `#1f2937` |
| border | `#232a34` |
| text primary | `#e6e9ef` |
| text secondary | `#8b93a1` |
| accent | `#2f7bf6` |
| running | `#3fb950` |
| exited | `#6e7681` |
| paused | `#d29922` |
| error | `#f85149` |

Bundled fonts: Inter (UI) and JetBrains Mono (IDs, logs, inspect values) as GPUI
`AssetSource` resources, so the app looks the same on any Mac. Light mode is v1.1.

### Performance

- `uniform_list` for every table; only visible rows are built.
- Sorted/filtered row order is recomputed when the snapshot or query changes, cached as
  `Vec<usize>` indices, never per frame.
- `SharedString` for all display text; rendering borrows from the snapshot.
- No allocations per row beyond the element tree; no `format!` in render paths that run
  every frame (pre-formatted labels live in the model).
- Startup budget: window painted with skeleton in <300ms, list populated as soon as the
  first response lands (expect <50ms locally).

## Deliverable and packaging

v1 is defined by `cargo run` working from the repo — that is how it is developed and how
it is accepted. On top of that, the last task adds `just bundle` (a short shell script,
no extra tooling): builds release, assembles `contUI.app` with an `Info.plist`
(`CFBundleIdentifier: dev.local.contui`, name, version, icon), ad-hoc codesigns it,
and drops it in `target/`. No notarization, no App Store.

Consequence for the menu bar: menus are registered through GPUI (`cx.set_menus`) and
must work in **both** an unbundled binary and a bundle. If the transparent titlebar or
menu behaves differently unbundled, the unbundled path gets a plain titlebar and we do
not spend time on parity; the bundle is the real artifact.

**Distribution, added 2026-11-14.** v1 still ships via `just bundle`, and distribution is
a public Homebrew tap at `github.com/adgan/homebrew-tap` (installed as `adgan/tap`). The
formula is HEAD-only and builds from source until `v0.1.0` is tagged; bottles and a cask
for `/Applications` follow in plan Task 10. See
`2026-11-14-homebrew-tap-design.md` for the tap's own design, including why there is no
cask yet and why `brew trust` is part of the install steps.

## Testing

- **Unit tests** (no daemon needed): endpoint resolution against a temp `DOCKER_HOST`,
  a fixture `~/.docker/config.json`, fixture context dirs (`meta.json` with and without
  TLS material, one malformed), and a fake socket path; `DOCKER_CONTEXT` overriding
  `currentContext`; sort comparators; search filter; size/duration formatting; JSON→tree
  flattening for the inspect renderer; event→resource-id routing.
- **Integration tests** (`#[ignore]`, run manually against the live Rancher socket):
  list containers/images/volumes return non-error; inspect of a throwaway container;
  logs of a container that exits immediately.
- **No automated UI tests in v1.** Manual acceptance list below.

### Acceptance (v1 is done when)

1. `cargo run` opens on Rancher Desktop and lists all 16 containers with correct states.
2. Images (45) and volumes (1) tabs list without a visible pause.
3. `docker run -d --name contui-test alpine sleep 300` → row appears within ~1s with
   no manual refresh; `docker stop contui-test` → state flips in place.
4. Search "postgres" narrows to the one container; Esc restores; sort by Name reverses.
5. Opening sploney-db-1 shows Inspect (ports, mounts, env) and Logs with real output.
6. Killing Rancher Desktop shows the disconnected state instead of a frozen or empty
   list; restarting it recovers without an app restart.
7. Switching to `DOCKER_HOST`/another context reloads all three lists.
8. `just bundle` produces a `contUI.app` that launches from Finder with a menu bar and
   the packaged fonts.

## Risks

- **gpui-pre is a weekly-moving snapshot.** Mitigation: pin `=0.3.12`, vendor nothing,
  and make Task 1 a hello-world spike (window + uniform_list + tokio bridge) before any
  page work. Its published examples reference an unpublished `gpui-platform` crate; we
  use `Application::new()` from `gpui_pre` directly, which exists in the crate.
- **macOS feature resolution.** `gpui-pre`'s default features include `wayland`/`x11`
  (Linux). Start with `default-features = false, features = ["font-kit"]`; fall back to
  defaults if text or the renderer misbehaves.
- **bollard 0.21 API drift.** Options builders and models moved between 0.19 and 0.21
  (`query_parameters`, `models`). Confirm exact paths during the spike before writing
  the service.
- **bollard features.** Enable `ssl` (rustls) for `DOCKER_CERT_PATH` contexts; do not
  enable `ssh` in v1 — it pulls a libssh2 C dependency and, without a passphrase prompt,
  earns little. `ssh://` entries are listed but marked "needs SSH support (v1.1)".
- **Rancher's event stream** may drop on engine restarts. Handled by the reconnect loop;
  if it proves unreliable, the fallback is a 2s poll only while the window is focused.
- **Transparent titlebar** may fight GPUI's window layering; standard titlebar is the
  fallback.

## What's left to the builder

- Exact column widths, spacing scale, and icon drawings.
- Whether to bundle Inter/JetBrains Mono or fall back to system fonts (bundling is
  assumed; it is a few MB and makes the look deterministic).
- Choosing the concrete helper API for the table so all three pages share it.
- Task ordering and commit granularity.

## Flagged for approval

Two scope calls that would change the build if they are wrong:

1. **Live CPU/memory** (Docker Desktop's signature columns) needs one `/stats` stream per
   running container. Deferred to v1.1; v1 shows Name · State · Image · Ports · Created.
2. **Remote hosts.** v1 covers unix sockets and `tcp://`/`http://` including TLS via
   `DOCKER_CERT_PATH`. `ssh://` hosts are listed but not connected, and there is no
   passphrase prompt — full SSH is v1.1. **Confirmed 2026-11-14: no SSH host is needed
   in v1.**
