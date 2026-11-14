# Dockyard — mutations (v2) — design

Date: 2026-11-14
Status: draft for approval
Supersedes: the "no mutations" non-goal in `2026-11-14-docker-gpui-client-design.md`

## Intent

v1 made Dockyard a fast, live read-only view. It is not yet a replacement for Rancher
Desktop: the moment you want to stop a container, delete an image, or start one, you
leave for a terminal. v2 closes that gap with the everyday verbs — **start, stop,
restart, remove, pull, run** — without giving up the two things that make the app worth
using: it never blocks, and the daemon's event stream stays the source of truth.

Success looks like: a session where you stop a stack, pull a tag, and run a fresh
container without opening a terminal, and the list reflects every action within ~1s
without a manual refresh.

## Goals

1. **Containers:** start, stop, restart, and remove (with an explicit force option)
   from the list row, the right-click menu, and the detail page.
2. **Images:** remove (with an explicit force option), run (opens the Run form
   pre-filled), and pull an arbitrary `repo[:tag]` from the Images header with visible
   progress.
3. **Volumes:** remove, with the daemon's "in use" error surfaced verbatim.
4. **Run:** a real create-and-start form — image, name, command, ports, environment,
   volumes — that reports validation errors before the daemon and the daemon's own
   errors after it.
5. **Never lie about state.** A pending action is shown as pending; success is confirmed
   by the event stream plus a refetch, not by a guessed state.
6. **One implementation.** Every action is reachable from the table, the context menu,
   and the detail page without duplicating behavior.

## Non-goals for v2

- No prune (containers/images/volumes/build cache), no bulk or multi-select actions.
- No tag, push, build, commit, rename, pause/unpause, kill (stop's timeout is fixed), or
  container create-without-start.
- No Compose, no exec/shell, no Kubernetes, no checks/log stream.
- **No registry authentication.** Public images pull; private ones fail with the daemon's
  auth error, shown as-is.
- **No pull cancel.** The Docker API offers no cancel; the strip has no Cancel button.
- No restart policy, `--rm`, labels, networks, resource limits, user/group, TTY/stdin,
  or platform override in the Run form. The six fields above are the v2 surface.
- No optimistic state flips or local state machine for container lifecycle.
- No Windows or Linux work.

## Anti-goals

- A pull that freezes list refreshes, another action, or the UI. (The current actor is
  sequential; this is the main reason v2 changes the service.)
- Removing a running container or an in-use image without the user ticking Force.
- Double-firing an action because the button was clicked twice.
- Losing the current selection, filter, or scroll position because an action refreshed
  the list.
- A silently swallowed failure. Every failed action has a visible surface.
- Blocking the GPUI thread on any daemon call.

## Constraints

Inherited from v1, still binding:

- `gpui = "=0.2.2"`, `bollard = "0.21"` (features `http`, `pipe`, `ssl`, `chrono`),
  edition 2024, lib + thin bin, no `docker` CLI invocation.
- The GPUI thread never performs I/O; all daemon calls run on the tokio runtime.
- No polling loop. The only timers remain the event debounce, the reconnect backoff,
  and finite per-request timeouts.
- Errors never panic and never clear the last good snapshot.

New constraints discovered while designing:

- **The actor loop is sequential** (`while let Ok(req) = cmd_rx.recv().await`). A pull can
  take minutes, so mutations must not run inside it.
- `Response::Failed` carries only a `what` string; actions need a typed target so a
  failure can be routed to the right row, dialog, or toast.
- The app has no real text field yet (the search box handles only printable keys and
  backspace) and no dialog layer. The Run form needs both.
- `assets.rs` currently serves only fonts; the rail uses letters. v2 adds SVG icons and
  an `AssetSource` arm for them.
- GPUI 0.2.2 provides `window.focus_next()` / `focus_prev()`, `FocusHandle::tab_stop`,
  `svg()`, `anchored()`, `on_mouse_down(MouseButton::Right, …)`, `tooltip()`, `opacity()`
  and `cx.read_from_clipboard()`. No new crate is needed.

## Approaches considered

### How mutations execute

1. **Run them in the existing actor loop.** Smallest diff. Rejected: a 2-minute pull
   would block every list refresh and every other action behind it, and one hung request
   would freeze the app's data path.
2. **Spawn one tokio task per mutation, answer through the existing response channel
   (chosen).** The actor keeps owning requests, but mutating requests hand off to a
   spawned task with a cloned `Docker` and sender. Slow pulls cannot starve lists, two
   actions can run at once, and every result is an ordinary `Response`.
3. **Optimistic state machine with rollback.** Flip the row immediately, reconcile with
   events. Rejected: it duplicates Docker's state rules, invents a second truth, and the
   event stream plus a 20ms refetch already gives immediate feedback.

### Where actions live

Chosen by the partner on 2026-11-14, option 1 of three: **Docker Desktop parity**. A
per-row Actions column on hover, right-click menu, and the same actions on the detail
page; global Run and Pull buttons in the header. The alternatives were a selection
toolbar (cleaner but slower and less discoverable) and detail-page-only actions (smallest
diff but every action costs a navigation).

### How deep the Run form goes

Chosen by the partner: the **broader** surface — image, name, command, ports, env,
volumes. The minimal alternative (image, name, ports) was rejected because it sends
people back to the terminal for env vars and mounts, which is exactly what v2 is for.

## Architecture

### Module layout

```
src/docker/
  actions.rs        RunSpec, form → spec validation, CreateContainerBody mapping,
                    shell-word splitting, image-reference splitting, action availability,
                    daemon-error to user-message mapping
  service.rs        new Request/Response variants, mutation tasks, pull stream, timeouts
src/ui/
  widgets/text_input.rs   reusable single-line input (focus, caret at end, backspace, ⌘V)
  dialog.rs               modal layer, confirm dialog, Run dialog, Pull popover, toast
  root.rs                 action state, routing, row actions column, context menu, headers
assets/icons/             play, stop, restart, trash, run, pull, plus, close (16px SVG)
```

### Service protocol

```rust
pub enum ContainerAction { Start, Stop, Restart }

pub enum ActionKind { Start, Stop, Restart, Remove }

pub struct Target { pub kind: ResourceKind, pub id: String }

pub enum PullOrigin { Header, RunRetry }

pub enum Request {
    // v1, unchanged
    FetchAll, ProbeHosts,
    Inspect { kind: ResourceKind, id: String },
    Logs { id: String },
    // v2
    ContainerAction { id: String, action: ContainerAction },
    RemoveContainer { id: String, force: bool },
    RemoveImage { id: String, force: bool },
    RemoveVolume { name: String },
    PullImage { reference: String, origin: PullOrigin },
    RunContainer { spec: RunSpec },
}

pub enum Response {
    // v1, unchanged
    Snapshot(Snapshot), Status(HostStatus), HostStatuses(Vec<(String, HostStatus)>),
    Inspect { kind: ResourceKind, id: String, json: serde_json::Value },
    Logs { id: String, lines: Vec<LogLine> },
    Dirty { kinds: Vec<ResourceKind>, ids: Vec<(ResourceKind, String)> },
    Failed { what: String, message: String },
    // v2
    ActionDone { target: Target, action: ActionKind, error: Option<String> },
    PullProgress { origin: PullOrigin, reference: String, layer: Option<String>,
                   status: String, detail: Option<(u64, u64)> },
    PullDone { origin: PullOrigin, reference: String, error: Option<String> },
    ContainerCreated { id: String, warnings: Vec<String> },
    RunFailed { message: String, missing_image: bool, created: Option<String> },
}
```

`ActionDone` is sent for start/stop/restart/remove, including transport failures; `error`
is the user-facing message, not a `Debug` dump. For volume targets, `Target.id` is the
volume name. For images it is the **full image id**, never a tag (see "Image identity").

`RunFailed.created` distinguishes the two failure shapes: `None` when creation failed,
`Some(id)` when the container was created but `start` failed (a taken host port, an
unwritable bind target). `PullProgress.layer` is `CreateImageInfo.id`; `None` means the
chunk carries only status text.

### Execution semantics

- The actor receives a mutating request and immediately
  `runtime.spawn(async move { … })` with cloned `Docker` and `Sender`. It does not await
  the operation.
- Timeouts, per operation: start/stop/restart/remove/create/start-after-create **30s**.
  Stop uses Docker's default 10s grace (`StopContainerOptionsBuilder::t(10)`), so the
  client cap is only a backstop. Pull has **no total cap** but a **60s idle** cap between
  stream chunks; either timeout produces `PullDone { error }`.
- Calls: `start_container(id, None)`, `stop_container(id, Some(t(10)))`,
  `restart_container(id, None)`, `remove_container(id, Some(force, v(false)))`,
  `remove_image(id, Some(force, noprune(false)))`,
  `remove_volume(name, Some(force(false)))`, `create_container(Some(name?), body)`
  followed by `start_container(new_id, None)`, and
  `create_image(Some(from_image(repo), tag(tag)))` streamed.
- `v(false)` is deliberate: the confirm copy promises volumes are kept, and Docker's own
  `rm` default keeps anonymous volumes. Removing volumes with a container is out of scope.
- Concurrency: any number of actions may be in flight. The UI refuses a second action
  for the same `(kind, id)` while one is pending.
- On a successful mutation, the service sends only `ActionDone`; **the UI** decides to
  refetch. This keeps the service free of UI policy.
- Pull progress: each chunk becomes `PullProgress { layer, status, detail }`. The UI
  keeps a `BTreeMap<layer_id, (u64, u64)>` and sums for the strip; chunks without a layer
  or without totals keep the strip in text-only mode.
- Every failure string is cleaned first: `DockerResponseServerError { message }` yields
  its message (no `409 Conflict` prefix); transport errors yield the IO description.

### Action availability

Pure function over `ContainerState` (image/volume: always removable; image also runnable).

| state | Start | Stop | Restart | Remove |
|---|---|---|---|---|
| Running | — | ✓ | ✓ | ✓ |
| Paused | — | ✓ | ✓ | ✓ |
| Restarting | — | ✓ | ✓ | ✓ |
| Exited | ✓ | — | — | ✓ |
| Created | ✓ | — | — | ✓ |
| Dead | — | — | — | ✓ |
| Unknown | ✓ | ✓ | ✓ | ✓ |

Unknown is the daemon's word, so we let the daemon arbitrate instead of showing an empty
row of buttons. Disabled icons render at 25% opacity and do not fire.

### Image identity

One row is one image, and the Images table shows only the first of that image's
`repo_tags`. Removal therefore targets the **full image id** (`Target.id` =
`ImageRow.id`), which removes every tag; the confirm title names the displayed
repository tag, and when the image has more than one tag the body adds `This removes
every tag for this image.` A successful removal always makes the row disappear.
`ImageRow` gains `tag_count` (from `repo_tags.len()`) purely for that copy. Removing a
single tag belongs to a future per-tag table; it is not folded into v2.

### Pull concurrency

At most **one pull runs at a time**. While `pull.is_some()`, the header Pull button, the
popover's Pull button, and the Run dialog's `Pull it` are all disabled (`A pull is
already running`). That is why `pull: Option<PullState>` is enough and the strip can
never show two references.

### UI state

```rust
pub struct PendingAction { pub action: ActionKind }

pub struct ConfirmState {
    pub kind: ResourceKind, pub id: String, pub title: String,
    pub force: bool, pub submitting: bool, pub error: Option<String>,
}

pub struct RunState {
    pub form: RunForm, pub field_errors: Vec<FieldError>,
    pub daemon_error: Option<String>, pub missing_image: bool,
    pub submitting: bool, pub submitted: Option<RunSpec>,
    pub pull_then_retry: bool,
}

pub struct PullState {
    pub origin: PullOrigin, pub reference: String, pub status: String,
    pub layers: BTreeMap<String, (u64, u64)>,
}

pub struct RowMenuState { pub kind: ResourceKind, pub id: String, pub at: Point<Pixels> }

pub struct Toast { pub text: String, pub tone: ToastTone }

// added to RootView
pending: BTreeMap<(ResourceKind, String), PendingAction>,
confirm: Option<ConfirmState>,
run: Option<RunState>,
pull: Option<PullState>,
pull_open: bool, pull_input: String, pull_error: Option<String>,
row_menu: Option<RowMenuState>,
toast: Option<Toast>,
hovered_row: Option<String>,
pending_select: Option<String>,   // select + open this container after the next snapshot
```

### UI composition

**Tables.** `TableSpec` gains `actions: Option<ActionsColumn>` (container/image/volume)
and `TableRow.cells` keeps exactly one entry per data column. The renderer draws the
header cell as empty and appends the action cluster after the cells. Widths: containers
124px (four 24px targets), images 84px (Run, Delete), volumes 44px (Delete). Icons sit
normally at 25% opacity; the hovered or selected row shows them at 100%. Search never
matches the action column because it has no cell text.

The State cell renders `Starting…`, `Stopping…`, `Restarting…`, or `Removing…` in the
paused color while that row has a pending action; every action icon on the row is
disabled until `ActionDone` arrives.

**Header.** Containers page: a primary `Run` button (icon + label) left of search.
Images page: a `Pull` button (icon + label) left of search. Volumes page: neither.
Buttons keep their width so the header does not jump.

**Row menu.** Right-click (`on_mouse_down(MouseButton::Right, …)` on the row) selects the
row and opens an overlay anchored at the cursor: `Open`, separator, Start / Stop /
Restart (per availability), separator, `Remove` in the error color. Clicking the backdrop
or pressing Esc closes it. The menu is a plain absolute overlay inside the root
`relative()` container; positions are window coordinates, clamped to the window.

**Confirm dialog.** 440px panel over a 50%-black backdrop. Title `Remove container
"web"?`, body `This permanently deletes the container and its writable layer. Volumes
are not removed.` Images say `This deletes the image from this host. Containers using it
keep running, but cannot be started again.`, plus the multi-tag sentence when
`tag_count > 1`. Volumes say `This permanently deletes the volume and its data.` For
containers and images a Force checkbox spells out its consequence in one line (`Force
remove while running — sends SIGKILL`, `Force remove even if a container uses it`).
Cancel / Remove. Esc and backdrop cancel; Esc is ignored while submitting. Remove
failures stay **inside** the dialog in the error color with the daemon message; the
dialog stays open so Force can be ticked and retried. Volumes have no Force checkbox.

**Run dialog.** 640px, max-height 80% of the window, fixed header/footer and a scrolling
body:

```
┌ Run a container ───────────────────────────────────────────┐
│ Image *   [ postgres:18                                  ] │
│ Name      [ sploney-db-1                                 ] │
│ Command   [ postgres -c max_connections=200              ] │
│                                                             │
│ Ports                                       + Add port      │
│  Host        Container        Proto                        │
│  [ 5432 ]    [ 5432 ]         ( tcp | udp )           [×]   │
│                                                             │
│ Environment                                 + Add variable │
│  [ POSTGRES_PASSWORD ]  [ secret ]                     [×]  │
│                                                             │
│ Volumes                                     + Add volume   │
│  [ sploney_db-data ]  [ /var/lib/postgresql/data ]  ro [×]  │
│                                                             │
│ ⚠ Image "postgres:19" is not available locally.  [Pull it] │
│                                        Cancel      [ Run ]  │
└─────────────────────────────────────────────────────────────┘
```

- Image is required; Run is disabled until it is non-empty.
- Name, Command, Ports, Environment, Volumes are optional and start empty (a prefilled
  image name from an image row is the only prefill).
- Add/remove rows for ports, env, volumes. Removing the last row is allowed; the section
  then shows only its Add link.
- Tab / Shift-Tab move between fields via `window.focus_next()` / `focus_prev()`; Enter
  submits; Esc cancels unless submitting.
- Field-level errors render under the offending input in the error color and the input
  border turns error-colored; the first invalid field gets focus.
- Submitting takes a snapshot: `submitted = Some(spec)`, then the form is frozen
  (inputs read-only, Add/Remove disabled) until the result. Re-validation never runs on
  edited fields after the fact; what is visible is what was sent.
- The daemon error renders above the footer. For a missing image, that block also shows
  a `Pull it` button: it starts a pull with `PullOrigin::RunRetry` (disabled while another
  pull runs), shows progress in the footer, and on success re-sends the stored
  `submitted` spec without revalidating. The dialog stays open the whole time.
- If creation succeeded but the start failed (`RunFailed { created: Some(id) }`), the
  dialog closes, the list refetches, the created row is selected and opened, and a toast
  names the start error. Retrying inside the dialog would collide with the name the
  server already took.
- On success: close, switch to Containers, refetch, then select and open the new
  container once the snapshot names its id (this gives its Logs tab immediately).
- Esc cancels only while not submitting. Cancelling during a `RunRetry` pull drops the
  retry and the dialog; the pull keeps running as an ordinary pull (strip, then refetch
  or toast). A closed dialog never reopens because of a later response.

**Pull popover.** Anchored under the Pull button: one input `image[:tag]` and a Pull
button. Enter pulls. Empty input disables the button. While any pull is running the
button is disabled and reads `Pulling…`. The popover closes on submit and on Esc/backdrop;
closing it never cancels the pull.

**Pull strip.** 32px strip under the header while `pull.is_some()`: reference, last
status, `12.4 MB / 45.6 MB` when totals exist, and a 3px progress bar on a track. With no
totals it is text-only (no fake animation). On success: clear, refetch, quiet (no toast).
On failure: clear the strip, keep the reference in `pull_input`, and show the daemon's
message as a toast — never reopen a surface the user closed. A failure that belongs to a
still-open Run dialog is shown there instead.

**Toast.** Bottom-center, 420px, panel color, error-colored left border for failures,
text plus a `×`. One at a time, replaced by the next; a new one never auto-dismisses
(no timer). Used for action failures and unexpected transport errors; successful row
actions are quiet because the row itself changes.

**Detail page.** The same action cluster sits in the detail header, using the same
pending map. Removing the open resource closes the detail page after success; if it
disappears for another reason, v1's "no longer exists" state still applies.

### Flows

- **Start.** Click ▶ → insert `pending[(Container, id)] = Start` → notify →
  `Request::ContainerAction` → task → `ActionDone` → remove pending → request `FetchAll`
  → new snapshot replaces rows, `adopt_state` keeps query/sort/selection by id → the
  state cell flips. The event stream may deliver the same change first; a duplicate
  refetch is harmless and not worth deduplicating.
- **Remove.** Trash → `ConfirmState` → Remove → `submitting` → `Request::RemoveContainer`
  → `ActionDone` → on success close dialog, clear pending, close detail if it was the
  target, refetch. On failure keep the dialog, set its error, clear `submitting`.
- **Pull.** Pull button → popover → submit → `pull = PullState` → progress responses →
  `PullDone` → clear; on success refetch, on failure toast (or the open Run dialog).
- **Run.** Run → form → validate locally → `submitted = Some(spec)`, freeze →
  `Request::RunContainer` → `ContainerCreated { id }` → close, switch page, refetch,
  `pending_select = id` → on the next snapshot select and open that row. A start failure
  after creation (`RunFailed { created: Some(id) }`) takes the same path, minus the
  success; the toast carries the error.

### Failure and disconnect rules

- A failed start/stop/restart/remove shows a toast with the daemon's message; the row
  keeps its last-known state and its buttons re-enable.
- A failed remove that originated in the confirm dialog is shown in the dialog instead.
- A created-but-not-started Run is not a dialog failure: the container exists, so the
  dialog closes, the row is opened, and a toast carries the start error.
- If the host disconnects (status leaves `Connected`) while anything is pending: clear
  `pending`, `pull`, `confirm.submitting`, and `run.submitting`, close dialogs, and show
  one toast `Host disconnected — action interrupted`. Whether the daemon applied it is
  unknowable; reconnecting refetches, and the list tells the truth.
- Switching hosts clears all v2 state without toasts, and the old runtime is dropped
  with its response channel, so nothing from the previous host can arrive.
- `ActionDone` for a resource that no longer exists is harmless: pending is cleared and
  the refetch drops the row.

### Stale responses

Spawned tasks outlive the surface that started them, so every response is matched before
it may change state:

- `ActionDone` clears `pending` only when its `(target.kind, target.id, action)` still
  matches the pending entry. A match with the wrong action (a late Stop after a
  disconnect, then a new Start) never clears the new pending. Late successes still
  request `FetchAll`; late errors are dropped, because their row, dialog, or toast
  surface is gone.
- A confirm dialog accepts only a `Remove` result for its own target. If it has closed,
  the error is dropped (a success still refetches).
- `PullProgress` and `PullDone` are applied only while `pull` exists and both `origin`
  and `reference` match; anything else is stale. Progress can never resurrect a cleared
  strip.
- Run results are applied only while `run` is open and `submitting`. A late
  `ContainerCreated` still refetches but never selects or reopens anything; a late
  `RunFailed` is dropped.
- A pull the user walked away from (Esc on the Run dialog) is ordinary: the strip stays,
  success refetches, failure toasts. Nothing is retried into a closed dialog.

No epoch field is needed: a host switch replaces the whole runtime and channel, and
within a host the match keys above are unique for the lifetime of a single action.

## Run form: parsing and validation

```rust
pub struct RunForm {
    pub image: String, pub name: String, pub command: String,
    pub ports: Vec<PortForm>,    // host: String, container: String, proto: Proto
    pub env: Vec<EnvForm>,       // key: String, value: String
    pub mounts: Vec<MountForm>,  // source: String, target: String, read_only: bool
}

pub struct RunSpec {
    pub image: String, pub name: Option<String>, pub command: Option<Vec<String>>,
    pub ports: Vec<PortSpec>,  // host: Option<u16>, container: u16, udp: bool
    pub env: Vec<(String, String)>,
    pub mounts: Vec<MountSpec>, // source: String, target: String, read_only: bool
}

pub fn build_run_spec(form: &RunForm) -> Result<RunSpec, Vec<FieldError>>;
pub fn run_body(spec: &RunSpec) -> ContainerCreateBody;
```

Rules, pinned so tests are unambiguous:

- **Image:** trimmed, non-empty.
- **Name:** optional; when present must match `[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}`.
- **Command:** whitespace-split with `'…'`, `"…"`, and `\` escapes; adjacent segments
  concatenate (`a"b c"d` → `ab cd`); an unterminated quote is an error; blank → `None`.
  No variable expansion, globbing, or shell.
- **Ports:** container required and 1–65535; host empty or 1–65535; duplicate
  `(host, proto)` across rows and duplicate `(container, proto)` are errors. Host empty
  becomes `HostPort: Some("")` (ephemeral).
- **Environment:** key non-empty, no `=`, no duplicates; value may be empty.
- **Volumes:** source required and must not contain `:`; target must start with `/` and
  must not contain `:`; binds are `source:target` plus `:ro` when read-only.
- Body mapping: `image`, `cmd`, `env = ["K=V", …]`, `exposed_ports = ["5432/tcp", …]`,
  `host_config.port_bindings["5432/tcp"] = [PortBinding { host_ip: None, host_port }]`,
  `host_config.binds = ["src:dst[:ro]", …]`. Name is a create query parameter, not a body
  field. `CreateContainerBody`/`HostConfig` are `Default`, so unset fields stay `None`.
- `utils::split_image_reference("registry:5000/team/app:1.2")` → `("registry:5000/team/app",
  "1.2")`; no tag → `"latest"`; `@sha256:…` is passed as `from_image` with an empty tag
  (the daemon ignores tag for digests).

## Testing

### Unit (no daemon)

- `split_command` handles quotes, escapes, adjacency, blank input, unterminated quotes.
- `build_run_spec` maps ports/env/mounts into the body; defaults tag to latest; keeps a
  registry port; passes digests through.
- `build_run_spec` rejects: empty image, bad name, 0/70000 ports, duplicate host and
  container ports, empty env key, env key containing `=`, duplicate env keys, empty
  mount source, relative mount target, `:` inside a source or target.
- `container_actions(state)` matches the availability table.
- `pending` guard: a second action for the same target is ignored while the first is
  pending; a different target is allowed; clearing on disconnect empties the map.
- `clean_error` strips the HTTP status prefix from `DockerResponseServerError` and keeps
  transport text.
- `classify_run_error` flags `404` / "No such image" as `missing_image`, and reports
  `created: Some(id)` when create succeeded and start failed.
- `routing`: an `ActionDone { Remove }` error goes to the matching confirm dialog; a
  start error goes to the toast; a success closes the dialog and requests a refetch.
- Stale-response rules: a late `ActionDone { Stop }` does not clear a newer `Start`
  pending; a `PullDone` for a reference that is not in the slot is ignored; a run result
  with no open dialog is ignored except for the refetch.
- The Run retry re-sends the submitted spec, not edits made after submission.
- Pull aggregation sums per-layer totals keyed by `layer`, ignoring `None` layers and
  not double counting a repeated id.
- Image removal targets the full image id and counts every tag in the confirm copy.
- Row building keeps the action column out of `cells`, so search cannot hit it.

### Integration (`#[ignore]`, live Rancher socket)

- `pull_run_stop_restart_remove_round_trip`: pull `alpine:3.20` (skip if the pull fails
  for network reasons), create `sh -c "sleep 5"`, start, see `ActionDone` ok, stop,
  restart, force-remove, and assert every response arrives within its timeout.
- `remove_volume_in_use_reports_daemon_message`: create a volume, run a container
  mounting it, remove the volume (no force) → error mentions "in use"; clean up.
- `force_removes_a_running_container`: run one, `RemoveContainer { force: true }`,
  assert removal and that a non-force attempt first returns an error.
- `run_with_a_taken_host_port_reports_the_created_container`: occupy a host port with one
  container, run another on the same port with a name, assert `RunFailed { created:
  Some(id) }`, then remove both and confirm no retry is needed.

### Manual acceptance

1. Hover a stopped container → ▶: state flips to running within ~1s with no refresh.
   Stop and restart behave the same.
2. Right-click a row → menu with the same actions; Esc and backdrop close it.
3. Click Stop twice fast: only one request fires; the second is disabled.
4. Remove a running container with Force off → inline 409 with the daemon message; tick
   Force → Remove succeeds and the row disappears, selection moves sanely.
5. Remove an in-use image without Force → 409 inline; with Force → gone, running
   containers unaffected.
6. Remove an in-use volume → 409 inline, dialog still open.
7. Pull `nginx:alpine` from Images: strip shows progress, button disabled, image appears
   without a manual refresh.
8. Pull `not-a-real/repo:nope` → error toast, the reference is still in the pull
   input, and the strip is gone.
9. Run a container with a name, command, two ports, two env vars and a named volume:
   it appears running and its detail opens. Reuse the name → daemon error inline.
10. Run with an image that is not local → the inline error offers `Pull it`; after the
    pull the container is created from the submitted spec without retyping anything.
    While that pull runs, the Images Pull button and `Pull it` are disabled.
11. Run with a host port that is already taken → create succeeds, start fails: the
    dialog closes, the new row is opened, and the toast names the start error. No name
    conflict appears on a retry.
12. Tab / Shift-Tab traverse the form; ⌘V pastes; Esc cancels when not submitting; ⌘N
    opens the form.
13. Kill Rancher during a pull and during a stop: the status chip flips, the strip and
    pending labels clear, one toast explains, and no closed dialog reappears when the
    late response arrives. Restarting the engine recovers.

## Acceptance

v2 is done when:

1. Every verb in the title works from the list row, the row menu, and the detail page
   (Pull and Run excepted, which live in the header and dialogs).
2. No mutation blocks a list refresh: start a 60MB pull, then scroll and switch pages;
   containers and volumes stay live.
3. Every destructive action requires confirmation and makes Force an explicit choice.
4. Every failure is visible in the surface that started it, with the daemon's wording.
5. `cargo test --lib` is green, and the ignored live tests pass against Rancher Desktop.
6. `cargo run` stays responsive during all of the above.

## Risks

- **Hand-rolled text input.** Caret-at-end only; no selection or arrow navigation. If
  focus/Tab in GPUI 0.2.2 misbehaves under the overlay, Tab is dropped (click-only)
  rather than blocking the feature.
- **Right-click coordinates.** GPUI reports window-space positions; the root is a
  `relative()` child at (0,0), so absolute placement is direct. If it drifts, use
  `anchored().position(...)`.
- **Pull progress varies by daemon.** Some chunks carry only `status`; the strip must
  degrade to text. No percentage is claimed unless totals exist.
- **Force semantics differ** between Docker engines (tag vs image removal). We surface
  the daemon's message and test against Rancher's engine.
- **Duplicate refetches** (action + event) are accepted. If they ever show, debounce the
  UI's post-action fetch, not the event stream.
- **Dialog focus traps** can eat global keys. Actions bound globally (⌘1/2/3, ⌘R, ⌘F)
  keep working; Esc is handled by the topmost dialog first.

## What's left to the builder

- Icon drawings, exact dialog spacing, and the action-column widths within the stated
  budgets.
- Whether row icons get GPUI tooltips (preferred) or rely on the context menu for labels.
- Exact copy for toasts and validation messages beyond the required wording above.
- Whether Enter submits the Run form (assumed yes) and whether the first field is
  focused on open (assumed yes).
- Task ordering and commit granularity.

## Decisions to check

Partner, 2026-11-14: Docker Desktop parity for where actions live; broader Run form.

Mine, made while writing this, any of which you can veto:

1. Mutations are spawned tasks, not queued through the actor.
2. No optimistic state flips; pending labels plus the event stream.
3. Destructive actions confirm; start/stop/restart do not.
4. Force is an explicit checkbox for containers and images only.
5. Successful row actions are silent; errors go to a toast (or the dialog that started
   them). Pull and Run report success quietly too.
6. The Run form is exactly image, name, command, ports, env, volumes; no restart policy,
   `--rm`, labels, networks, or resource limits.
7. Pulls cannot be cancelled and private registries are out of scope.
8. No prune in v2.
9. Removing a container keeps its anonymous volumes (`v = false`), matching the dialog's
   promise and Docker's own `rm` default.
10. Image removal targets the whole image (all tags), because the table shows one row per
    image; the confirm copy says so when there is more than one tag.
11. One pull at a time; every pull entry point is disabled while one runs.
12. A container created but not started is reported by closing the dialog, opening the
    new row, and toasting the start error — not by leaving a retry that collides.
13. Late responses are matched against the surface that started them and never reopen a
    closed dialog or clear a newer pending action.
14. The Run form freezes while submitting and a `Pull it` retry sends the submitted spec,
    not later edits.
