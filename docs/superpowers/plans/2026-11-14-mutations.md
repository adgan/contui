# Dockyard Mutations (v2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add start, stop, restart, remove, pull, and a full run form to Dockyard without blocking the UI or inventing state the daemon did not report.

**Architecture:** Pure mutation domain in `docker/actions.rs` (availability, validation, request bodies), a service that spawns one tokio task per mutation and streams pull progress, a pure `ui/mutations.rs` that owns pending/confirm/run/pull state and matches every response to the surface that started it, and thin rendering in `root.rs`/`dialog.rs`. The event stream plus a refetch remains the only source of lifecycle truth.

**Tech Stack:** Rust 1.99, `gpui = "=0.2.2"`, `bollard = "0.21"`, tokio 1, async-channel 2, chrono.

**Spec:** `docs/superpowers/specs/2026-11-14-mutations-design.md`

## Global Constraints

- Cargo may not be on `PATH`: `export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"` in every shell that runs cargo.
- No `docker` CLI invocation anywhere in the program. All calls go through `bollard` on the tokio runtime.
- The GPUI thread never performs I/O. No polling loop; the only timers are the existing event debounce, reconnect backoff, and the request timeouts below.
- Timeouts: start/stop/restart/remove/create/start-after-create **30s**; stop uses Docker's `t(10)` grace; pull has no total cap but a **60s idle** cap between stream chunks.
- `remove_container` always sends `v(false)`: removing a container never deletes volumes. Image removal targets the **full image id**. Volume removal sends `force(false)`.
- Action availability: Running/Paused/Restarting → stop, restart, remove; Exited/Created → start, remove; Dead → remove; Unknown → all four.
- Exactly one pull at a time. While `pull.is_some()`, every pull entry point is disabled.
- Destructive actions confirm; start/stop/restart do not. Force is an explicit checkbox for containers and images only.
- Successful row actions are silent. Failures go to the confirm dialog when one started them, otherwise to a toast.
- The Run form is exactly image, name, command, ports, environment, volumes. No restart policy, `--rm`, labels, networks, user, platform, or resource limits.
- Action column widths: containers **124px**, images **84px**, volumes **44px**. Table row height stays 32px.
- Dialog widths: confirm **440px**, run **640px** (max height 80% of window), pull popover **360px**. Colors come from `Theme` only.
- A failed request never clears the last snapshot. Errors never panic.
- Response matching: `ActionDone` applies only when `(target.kind, target.id, action)` is the current pending entry; pull responses only for the current `(origin, reference)`; run responses only while the dialog is open and submitting. Late successes still refetch; late errors are dropped.

## Review Focus

1. **Late or stale responses.** A result that arrives after a disconnect, a host switch, or a closed dialog must not reopen a dialog, resurrect a pull strip, or clear a newer pending action. Tasks 5 and 10 pin this.
2. **Removing a container must not delete anonymous volumes.** The options builder is asserted in Task 3; the confirm copy promises it.
3. **A slow pull must not block list refreshes or other actions.** Task 3 spawns mutation tasks and adds an ignored live test; Task 8 is the manual check.
4. **Create succeeds, start fails.** The container must become visible rather than hiding behind a retry that collides on its name. Tasks 3, 5, and 9 pin this.
5. **Editing the form during a pull retry.** What runs must be the submitted spec, never later edits. Tasks 5 and 9 pin this.
6. **A second click on a pending row.** The second action must be ignored, not queued twice. Task 5 pins this.

---

### Task 1: Mutation domain primitives

**Files:**
- Create: `src/docker/actions.rs`
- Modify: `src/docker/mod.rs` (add `pub mod actions;` beside the existing modules)
- Test: inline `#[cfg(test)] mod tests` in `src/docker/actions.rs`

**Interfaces:**
- Consumes: `crate::docker::model::{ContainerState, ResourceKind}`, `bollard::errors::Error`.
- Produces:

```rust
pub enum ContainerAction { Start, Stop, Restart }
pub enum ActionKind { Start, Stop, Restart, Remove }
impl ActionKind {
    pub fn verb(self) -> &'static str;              // "Start" | "Stop" | "Restart" | "Remove"
    pub fn pending_label(self) -> &'static str;     // "Starting…" | "Stopping…" | "Restarting…" | "Removing…"
    pub fn for_container(action: ContainerAction) -> Self;
}
pub struct Target { pub kind: ResourceKind, pub id: String }   // Clone, Debug, PartialEq, Eq
pub struct ContainerActions { pub start: bool, pub stop: bool, pub restart: bool, pub remove: bool }
pub fn container_actions(state: ContainerState) -> ContainerActions;
pub struct RunError { pub message: String, pub missing_image: bool, pub created: Option<String> }
pub fn clean_error(err: &bollard::errors::Error) -> String;
pub fn classify_run_error(err: &bollard::errors::Error, created: Option<String>) -> RunError;
pub fn split_image_reference(reference: &str) -> (String, Option<String>); // (repo-or-digest, tag)
```

- [ ] **Step 1: Write the failing tests**

```rust
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
    assert_eq!(clean_error(&err), "container is running: stop it before removing");

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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib docker::actions`
Expected: compile error, `src/docker/actions.rs` not found.

- [ ] **Step 3: Implement the primitives**

`clean_error`: match `DockerResponseServerError { message, .. }` → `message.clone()`, everything else → `err.to_string()`.
`classify_run_error`: `clean_error` plus `missing_image = matches!(status_code, 404) || message.contains("No such image")` only for `DockerResponseServerError`; pass `created` through unchanged.
`split_image_reference`: if it contains `@`, return `(reference.to_string(), None)`; otherwise find the last `/`, look for a `:` **after** that slash; if present split there, else append `"latest"`. Trim whitespace first; an empty reference returns `(String::new(), None)`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib docker::actions`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/docker/actions.rs src/docker/mod.rs
git commit -m "feat: mutation primitives for availability and daemon errors"
```

---

### Task 2: Run form domain

**Files:**
- Modify: `src/docker/actions.rs`
- Test: extend the inline test module in `src/docker/actions.rs`

**Interfaces:**
- Consumes: `bollard::models::{ContainerCreateBody, HostConfig, PortBinding}`, `std::collections::HashMap`.
- Produces:

```rust
pub enum Proto { Tcp, Udp }
pub struct PortForm { pub host: String, pub container: String, pub proto: Proto }   // Default: empty, Tcp
pub struct EnvForm { pub key: String, pub value: String }                          // Default: empty
pub struct MountForm { pub source: String, pub target: String, pub read_only: bool }
pub struct RunForm {
    pub image: String, pub name: String, pub command: String,
    pub ports: Vec<PortForm>, pub env: Vec<EnvForm>, pub mounts: Vec<MountForm>,
}
pub struct PortSpec { pub host: Option<u16>, pub container: u16, pub udp: bool }
pub struct MountSpec { pub source: String, pub target: String, pub read_only: bool }
pub struct RunSpec {
    pub image: String, pub name: Option<String>, pub command: Option<Vec<String>>,
    pub ports: Vec<PortSpec>, pub env: Vec<(String, String)>, pub mounts: Vec<MountSpec>,
}
pub enum RunField {
    Image, Name, Command,
    PortHost(usize), PortContainer(usize),
    EnvKey(usize), EnvValue(usize),
    MountSource(usize), MountTarget(usize),
}
pub struct FieldError { pub field: RunField, pub message: String }
pub fn build_run_spec(form: &RunForm) -> Result<RunSpec, Vec<FieldError>>;
pub fn split_command(input: &str) -> Result<Vec<String>, String>;
pub fn run_body(spec: &RunSpec) -> ContainerCreateBody;
```

All form and spec types derive `Clone, Debug, Default, PartialEq, Eq`; `Proto` implements `Default` as `Tcp`; `RunField` derives `Clone, Copy, Debug, PartialEq, Eq`.

- [ ] **Step 1: Write the failing tests**

```rust
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
    assert_eq!(split_command(r#"echo "oops"#), Err("Unterminated quote".to_string()));
}

#[test]
fn builds_a_spec_from_a_full_form() {
    let form = RunForm {
        image: " postgres:18 ".into(),
        name: "db".into(),
        command: r#"postgres -c "max_connections=200""#.into(),
        ports: vec![PortForm { host: "5432".into(), container: "5432".into(), proto: Proto::Tcp }],
        env: vec![EnvForm { key: "POSTGRES_PASSWORD".into(), value: "secret".into() }],
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
        Some(vec!["postgres".into(), "-c".into(), "max_connections=200".into()])
    );
    assert_eq!(spec.ports, vec![PortSpec { host: Some(5432), container: 5432, udp: false }]);
    assert_eq!(spec.env, vec![("POSTGRES_PASSWORD".into(), "secret".into())]);
    assert_eq!(spec.mounts[0].read_only, true);
}

#[test]
fn reports_every_invalid_field_at_once() {
    let form = RunForm {
        image: " ".into(),
        name: "bad name".into(),
        command: "\"oops".into(),
        ports: vec![
            PortForm { host: "0".into(), container: "70000".into(), proto: Proto::Tcp },
            PortForm { host: "8080".into(), container: "80".into(), proto: Proto::Tcp },
            PortForm { host: "8080".into(), container: "80".into(), proto: Proto::Tcp },
        ],
        env: vec![
            EnvForm { key: "".into(), value: "x".into() },
            EnvForm { key: "A".into(), value: "1".into() },
            EnvForm { key: "A".into(), value: "2".into() },
        ],
        mounts: vec![MountForm { source: "".into(), target: "relative".into(), read_only: false }],
    };
    let fields: Vec<RunField> = build_run_spec(&form).unwrap_err().iter().map(|e| e.field).collect();
    for expected in [
        RunField::Image, RunField::Name, RunField::Command,
        RunField::PortHost(0), RunField::PortContainer(0),
        RunField::PortHost(2), RunField::PortContainer(2),
        RunField::EnvKey(0), RunField::EnvKey(2),
        RunField::MountSource(0), RunField::MountTarget(0),
    ] {
        assert!(fields.contains(&expected), "missing {expected:?} in {fields:?}");
    }
}

#[test]
fn maps_a_spec_into_a_create_body() {
    let spec = RunSpec {
        image: "postgres:18".into(),
        name: Some("db".into()),
        command: Some(vec!["postgres".into()]),
        ports: vec![
            PortSpec { host: Some(5432), container: 5432, udp: false },
            PortSpec { host: None, container: 53, udp: true },
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
    assert_eq!(bindings["5432/tcp"].as_ref().unwrap()[0].host_port.as_deref(), Some("5432"));
    assert_eq!(bindings["53/udp"].as_ref().unwrap()[0].host_port.as_deref(), Some(""));
    assert_eq!(host.binds.unwrap(), vec!["db-data:/data:ro"]);
}

#[test]
fn empty_sections_stay_none() {
    let spec = RunSpec {
        image: "alpine".into(), name: None, command: None,
        ports: vec![], env: vec![], mounts: vec![],
    };
    let body = run_body(&spec);
    assert!(body.env.is_none());
    assert!(body.exposed_ports.is_none());
    assert!(body.cmd.is_none());
    let host = body.host_config.expect("host config");
    assert!(host.port_bindings.is_none());
    assert!(host.binds.is_none());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib docker::actions`
Expected: compile errors for the missing types.

- [ ] **Step 3: Implement the form and body mapping**

Rules: trim image and name; empty name → `None`; blank command → `None`. Name must match `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}`. Ports: container required 1–65535, host empty or 1–65535; duplicate `(host, proto)` and `(container, proto)` are errors reported on the **later** row. Env: key non-empty, no `=`, no duplicate keys (reported on the later row). Mounts: source non-empty and no `:`, target must start with `/` and contain no `:`. Collect errors in this order: Image, Name, Command, ports, env, mounts.

Exact messages: `"Image is required"`, `"Name may use letters, digits, ., _, - and must start with a letter or digit"`, `"Unterminated quote"`, `"Host port must be 1-65535"`, `"Container port must be 1-65535"`, `"Host port is used twice"`, `"Container port is used twice"`, `"Variable name is required"`, `"Variable name cannot contain ="`, `"Variable name is used twice"`, `"Volume source is required"`, `"Volume source cannot contain :"`, `"Container path must be absolute"`, `"Container path cannot contain :"`.

`split_command`: whitespace separates tokens outside quotes; `'…'` and `"…"` group; `\` escapes the next character outside quotes and inside double quotes; adjacent segments concatenate; `""` is one empty argument; unterminated quotes return `Err("Unterminated quote")`.

`run_body`: always `host_config: Some(host)`; set `env`, `cmd`, `exposed_ports`, `port_bindings`, `binds` only when non-empty. Port key `"{container}/tcp"` or `"{container}/udp"`; `PortBinding { host_ip: None, host_port: Some(host.map(|p| p.to_string()).unwrap_or_default()) }`. Bind string `"{source}:{target}"` plus `":ro"`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib docker::actions`
Expected: 9 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/docker/actions.rs
git commit -m "feat: run form validation and container create body"
```

---

### Task 3: Service mutations, pull stream, run

**Files:**
- Modify: `src/docker/service.rs`
- Modify: `src/ui/root.rs` (temporary empty arms for the new `Response` variants)
- Modify: `tests/live_docker.rs`
- Test: inline test module in `src/docker/service.rs`; ignored live tests

**Interfaces:**
- Consumes: `crate::docker::actions::{ActionKind, ContainerAction, RunSpec, Target, clean_error, classify_run_error, run_body, split_image_reference}`, `crate::docker::actions::ContainerAction`.
- Produces:

```rust
pub enum PullOrigin { Header, RunRetry }          // Clone, Copy, Debug, PartialEq, Eq
pub enum Request {
    FetchAll, ProbeHosts,
    Inspect { kind: ResourceKind, id: String },
    Logs { id: String },
    ContainerAction { id: String, action: ContainerAction },
    RemoveContainer { id: String, force: bool },
    RemoveImage { id: String, force: bool },
    RemoveVolume { name: String },
    PullImage { reference: String, origin: PullOrigin },
    RunContainer { spec: RunSpec },
}
pub enum Response {
    Snapshot(Snapshot), Status(HostStatus), HostStatuses(Vec<(String, HostStatus)>),
    Inspect { kind: ResourceKind, id: String, json: serde_json::Value },
    Logs { id: String, lines: Vec<LogLine> },
    Dirty { kinds: Vec<ResourceKind>, ids: Vec<(ResourceKind, String)> },
    Failed { what: String, message: String },
    ActionDone { target: Target, action: ActionKind, error: Option<String> },
    PullProgress { origin: PullOrigin, reference: String, layer: Option<String>,
                   status: String, detail: Option<(u64, u64)> },
    PullDone { origin: PullOrigin, reference: String, error: Option<String> },
    ContainerCreated { id: String, warnings: Vec<String> },
    RunFailed { message: String, missing_image: bool, created: Option<String> },
}
pub fn remove_container_options(force: bool) -> bollard::query_parameters::RemoveContainerOptions;
pub fn stop_options() -> bollard::query_parameters::StopContainerOptions;
pub fn pull_options(reference: &str) -> bollard::query_parameters::CreateImageOptions;
```

- [ ] **Step 1: Write the failing unit tests**

```rust
#[test]
fn remove_options_keep_anonymous_volumes() {
    let forced = remove_container_options(true);
    assert!(forced.force);
    assert!(!forced.v, "v(true) would delete anonymous volumes behind the dialog's back");
    assert!(!remove_container_options(false).force);
}

#[test]
fn stop_options_use_a_ten_second_grace() {
    assert_eq!(stop_options().t, Some(10));
}

#[test]
fn pull_options_split_registry_tag_and_digest() {
    let opts = pull_options("registry:5000/team/app:1.2");
    assert_eq!(opts.from_image.as_deref(), Some("registry:5000/team/app"));
    assert_eq!(opts.tag.as_deref(), Some("1.2"));

    let digest = pull_options("app@sha256:abc");
    assert_eq!(digest.from_image.as_deref(), Some("app@sha256:abc"));
    assert_eq!(digest.tag, None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib docker::service`
Expected: compile errors for the missing functions.

- [ ] **Step 3: Add the protocol and the pure option helpers**

Add the enums and variants. Build options with `RemoveContainerOptionsBuilder::new().force(force).v(false).build()`, `StopContainerOptionsBuilder::new().t(10).build()`, and `CreateImageOptionsBuilder::new().from_image(&repo)` plus `.tag(&tag)` only when `split_image_reference` returns `Some`.

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test --lib docker::service`
Expected: 4 tests pass (3 new plus the existing liveness test).

- [ ] **Step 5: Spawn mutation tasks from the actor**

In `actor`, every new request clones `docker` and `resp_tx` and calls `tokio::spawn`; the actor loop never awaits a mutation. Each task wraps its call in `tokio::time::timeout(Duration::from_secs(30), …)` and maps the result:

- start/stop/restart → `Response::ActionDone { target: Target { kind: Container, id }, action, error }`; a timeout message is `"timed out after 30s"`.
- remove container/image/volume → `ActionDone` with `ActionKind::Remove` and the matching `ResourceKind`; volume `Target.id` is the name.
- pull → loop over `docker.create_image(Some(options), None, None)`; each `Ok(info)` sends `PullProgress { layer: info.id, status: info.status.unwrap_or_default(), detail }` where `detail` is `Some((current as u64, total as u64))` only when both exist and `total > 0`; `Err` ends with `PullDone { error: Some(clean_error(&err)) }`; stream end with `None`; a 60s idle timeout ends with `Some("pull stalled for 60s")`.
- run → `create_container(options, run_body(&spec))`, then `start_container(id, None)`. Create error → `RunFailed { created: None }`; create ok + start error → `RunFailed { created: Some(id) }`; both ok → `ContainerCreated { id, warnings }`.

Because `RootView::apply` matches on `Response`, add temporary arms so the crate keeps compiling until Tasks 7–9 replace them:

```rust
Response::ActionDone { .. }
| Response::PullProgress { .. }
| Response::PullDone { .. }
| Response::ContainerCreated { .. }
| Response::RunFailed { .. } => {}
```

- [ ] **Step 6: Write the ignored live tests**

Add helpers in `tests/live_docker.rs`:

```rust
fn connect() -> (DockerService, async_channel::Receiver<Response>);
fn wait_for(responses: &async_channel::Receiver<Response>, what: &str,
            pred: impl Fn(&Response) -> bool) -> Response;
// wait_for skips Status, HostStatuses, Dirty, Inspect and Logs; returns the first
// ActionDone/PullDone/ContainerCreated/RunFailed/Snapshot.
```

Then three tests:

- `pull_run_stop_restart_and_force_remove_round_trip`: pull `alpine:3.20` (if `PullDone` carries an error, `eprintln!` and return — network-dependent); run `sh -c "sleep 60"` with a unique name; assert `ContainerCreated`; stop → ok; restart → ok; `RemoveContainer { force: false }` → error mentioning running; `RemoveContainer { force: true }` → ok.
- `run_with_a_taken_host_port_keeps_the_created_container`: run two containers on host port `45999` (`sleep 60`), assert the second gets `RunFailed { created: Some(id) }`; force-remove both.
- `list_requests_are_answered_while_a_pull_runs`: send `PullImage` for `debian:bookworm-slim`, immediately send `FetchAll`, then loop over responses until both a `Snapshot` and the matching `PullDone` have arrived; assert the `Snapshot` arrived within 2s of the pull starting. A cached pull may legitimately finish first, so record arrival order in the failure message rather than asserting it.

- [ ] **Step 7: Run the ignored live tests**

Run: `cargo test --test live_docker -- --ignored --test-threads=1`
Expected: PASS on this machine with Rancher Desktop running. If the network refuses the pull, the first and third tests print a skip and pass.

- [ ] **Step 8: Commit**

```bash
git add src/docker/service.rs tests/live_docker.rs
git commit -m "feat: spawn container, image, volume and pull mutations"
```

---

### Task 4: Image tag count and action columns in table specs

**Files:**
- Modify: `src/docker/model.rs`
- Modify: `src/ui/table.rs`
- Modify: `src/ui/root.rs` (the three table builders only)
- Test: inline tests in `src/docker/model.rs`, `src/ui/table.rs`, `src/ui/root.rs`

**Interfaces:**
- Consumes: `ImageSummary.repo_tags`, `ResourceKind`.
- Produces:

```rust
// model.rs
pub struct ImageRow { /* existing fields */ pub tag_count: usize }

// table.rs
pub enum ActionsColumn { Container, Image, Volume }
impl ActionsColumn {
    pub fn width(self) -> f32;        // 124.0 | 84.0 | 44.0
    pub fn kind(self) -> ResourceKind;
}
pub struct TableSpec { /* existing */ pub actions: Option<ActionsColumn> }
```

- [ ] **Step 1: Write the failing tests**

In `src/docker/model.rs`:

```rust
#[test]
fn image_row_counts_every_tag() {
    let summary = bollard::models::ImageSummary {
        id: "sha256:abc".into(),
        repo_tags: Some(vec!["postgres:18".into(), "postgres:latest".into()]),
        created: 1_699_999_880,
        size: 1,
        ..Default::default()
    };
    let row = ImageRow::from_summary(summary, false, chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap());
    assert_eq!(row.tag_count, 2);
    assert_eq!(row.repository_tag.as_ref(), "postgres:18");
}
```

In `src/ui/root.rs`:

```rust
#[test]
fn each_table_declares_its_action_column() {
    let snapshot = Snapshot::empty();
    assert_eq!(RootView::containers_table(&snapshot).spec.actions, Some(ActionsColumn::Container));
    assert_eq!(RootView::images_table(&snapshot).spec.actions, Some(ActionsColumn::Image));
    assert_eq!(RootView::volumes_table(&snapshot).spec.actions, Some(ActionsColumn::Volume));
    assert_eq!(ActionsColumn::Container.width(), 124.0);
    assert_eq!(ActionsColumn::Image.width(), 84.0);
    assert_eq!(ActionsColumn::Volume.width(), 44.0);
}
```

In `src/ui/table.rs`, update the existing `spec()` helper to set `actions: None` and add:

```rust
#[test]
fn action_columns_are_not_data_cells() {
    let mut s = spec();
    s.actions = Some(ActionsColumn::Container);
    assert_eq!(s.columns.len(), 2);
    assert!(s.actions.is_some());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib image_row_counts_every_tag each_table_declares_its_action_column`
Expected: compile errors for the missing field and enum.

- [ ] **Step 3: Implement**

`ImageRow::from_summary`: compute `let tag_count = image.repo_tags.as_ref().map_or(0, |tags| tags.len());` before moving `repo_tags`, and set the field.
Add `ActionsColumn` to `table.rs` with `kind()` returning the matching `ResourceKind`; add `actions: Option<ActionsColumn>` to `TableSpec`. Update every `TableSpec` literal in the repo to name the new field (the three builders and the `table.rs` test helper). The three builders set `actions: Some(...)`, and the `root.rs` test imports `ActionsColumn`. Update the `image()` test helper in `src/ui/root.rs` with `tag_count: 1`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: all passing.

- [ ] **Step 5: Commit**

```bash
git add src/docker/model.rs src/ui/table.rs src/ui/root.rs
git commit -m "feat: image tag counts and action column specs"
```

---

### Task 5: Pure mutation UI state

**Files:**
- Create: `src/ui/mutations.rs`
- Modify: `src/ui/mod.rs` (add `pub mod mutations;`)
- Test: inline `#[cfg(test)] mod tests` in `src/ui/mutations.rs`

**Interfaces:**
- Consumes: `crate::docker::actions::{ActionKind, FieldError, RunForm, RunSpec, Target}`, `crate::docker::model::ResourceKind`, `crate::docker::service::PullOrigin`, `std::collections::BTreeMap`.
- Produces:

```rust
pub struct Toast { pub text: String, pub error: bool }
pub struct ConfirmState {
    pub kind: ResourceKind, pub id: String, pub title: String, pub tag_count: usize,
    pub force: bool, pub submitting: bool, pub error: Option<String>,
}
pub struct RunState {
    pub form: RunForm, pub field_errors: Vec<FieldError>,
    pub daemon_error: Option<String>, pub missing_image: bool,
    pub submitting: bool, pub submitted: Option<RunSpec>, pub pull_then_retry: bool,
}   // Clone, Debug, Default
impl RunState {
    pub fn retry_spec(&self) -> Option<RunSpec>;   // submitted, only while pull_then_retry
}
pub struct PullState {
    pub origin: PullOrigin, pub reference: String, pub status: String,
    pub layers: BTreeMap<String, (u64, u64)>,
}
impl PullState {
    pub fn record(&mut self, layer: Option<String>, detail: Option<(u64, u64)>);
    pub fn bytes(&self) -> Option<(u64, u64)>;
    pub fn fraction(&self) -> Option<f32>;
}
pub struct RowMenuState { pub kind: ResourceKind, pub id: String, pub at: gpui::Point<gpui::Pixels> }
pub struct ActionOutcome { pub refetch: bool, pub close_detail: bool, pub toast: Option<Toast> }
pub struct PullOutcome { pub retry_run: bool, pub refetch: bool, pub toast: Option<Toast> }
pub struct RunOutcome { pub close_dialog: bool, pub select_container: Option<String>,
                        pub refetch: bool, pub toast: Option<Toast> }
pub struct Mutations {
    pub pending: BTreeMap<(ResourceKind, String), ActionKind>,
    pub confirm: Option<ConfirmState>,
    pub run: Option<RunState>,
    pub pull: Option<PullState>,
    pub pull_open: bool, pub pull_input: String, pub pull_error: Option<String>,
    pub toast: Option<Toast>,
    pub hovered_row: Option<String>,
    pub row_menu: Option<RowMenuState>,
    pub pending_select: Option<String>,
}
impl Mutations {
    pub fn new() -> Self;
    pub fn has_in_flight(&self) -> bool;
    pub fn clear_in_flight(&mut self);
    pub fn is_pending(&self, target: &Target) -> bool;
    pub fn begin(&mut self, target: &Target, action: ActionKind) -> bool;
    pub fn open_confirm(&mut self, target: Target, title: String, tag_count: usize);
    pub fn apply_action_done(&mut self, target: Target, action: ActionKind,
                             error: Option<String>) -> ActionOutcome;
    pub fn apply_pull_progress(&mut self, origin: PullOrigin, reference: &str,
                               layer: Option<String>, status: String,
                               detail: Option<(u64, u64)>) -> bool;
    pub fn apply_pull_done(&mut self, origin: PullOrigin, reference: &str,
                           error: Option<String>) -> PullOutcome;
    pub fn apply_run_created(&mut self, id: String, warnings: Vec<String>) -> RunOutcome;
    pub fn apply_run_failed(&mut self, message: String, missing_image: bool,
                            created: Option<String>) -> RunOutcome;
}
pub fn confirm_copy(kind: ResourceKind, title: &str, tag_count: usize) -> (String, String);
```

`Toast`, `ActionOutcome`, `PullOutcome`, `RunOutcome` derive `Clone, Debug, Default, PartialEq, Eq`; `PullState` derives `Clone, Debug`; `ConfirmState` and `Mutable` state structs derive `Clone, Debug` where practical.

- [ ] **Step 1: Write the failing tests**

```rust
fn target(kind: ResourceKind, id: &str) -> Target { Target { kind, id: id.into() } }

#[test]
fn begin_blocks_a_second_action_on_the_same_target() {
    let mut m = Mutations::new();
    assert!(m.begin(&target(ResourceKind::Container, "a"), ActionKind::Stop));
    assert!(!m.begin(&target(ResourceKind::Container, "a"), ActionKind::Stop));
    assert!(m.begin(&target(ResourceKind::Container, "b"), ActionKind::Stop));
    assert!(m.has_in_flight());
}

#[test]
fn a_late_action_never_clears_a_newer_pending_action() {
    let mut m = Mutations::new();
    assert!(m.begin(&target(ResourceKind::Container, "a"), ActionKind::Start));
    let late = m.apply_action_done(target(ResourceKind::Container, "a"), ActionKind::Stop, Some("gone".into()));
    assert!(!late.refetch && late.toast.is_none() && !m.pending.is_empty());
    assert!(m.confirm.is_none());
}

#[test]
fn remove_errors_stay_in_the_confirm_dialog() {
    let mut m = Mutations::new();
    m.open_confirm(target(ResourceKind::Container, "a"), "web".into(), 0);
    m.confirm.as_mut().unwrap().submitting = true;
    assert!(m.begin(&target(ResourceKind::Container, "a"), ActionKind::Remove));
    let out = m.apply_action_done(target(ResourceKind::Container, "a"), ActionKind::Remove, Some("is running".into()));
    assert!(out.toast.is_none());
    assert_eq!(m.confirm.as_ref().unwrap().error.as_deref(), Some("is running"));
    assert!(!m.confirm.as_ref().unwrap().submitting);
    assert!(m.pending.is_empty());
}

#[test]
fn stale_pull_updates_are_ignored() {
    let mut m = Mutations::new();
    m.pull = Some(PullState { origin: PullOrigin::Header, reference: "nginx".into(), status: "Starting…".into(), layers: Default::default() });
    assert!(m.apply_pull_progress(PullOrigin::Header, "nginx", Some("l1".into()), "Downloading".into(), Some((1, 10))));
    assert!(!m.apply_pull_progress(PullOrigin::Header, "redis", Some("l1".into()), "Downloading".into(), Some((1, 10))));
    assert!(!m.apply_pull_progress(PullOrigin::RunRetry, "nginx", Some("l1".into()), "Downloading".into(), Some((1, 10))));
    let stale_error = m.apply_pull_done(PullOrigin::Header, "redis", Some("boom".into()));
    assert!(!stale_error.refetch && stale_error.toast.is_none() && m.pull.is_some());
    let stale_success = m.apply_pull_done(PullOrigin::Header, "redis", None);
    assert!(stale_success.refetch && m.pull.is_some(), "a late success still refetches");
}

#[test]
fn pull_totals_dedupe_layers_and_ignore_missing_totals() {
    let mut p = PullState { origin: PullOrigin::Header, reference: "nginx".into(), status: String::new(), layers: Default::default() };
    p.record(Some("a".into()), Some((10, 100)));
    p.record(Some("a".into()), Some((40, 100)));
    p.record(Some("b".into()), Some((20, 50)));
    p.record(None, Some((999, 999)));
    p.record(Some("c".into()), None);
    assert_eq!(p.bytes(), Some((60, 150)));
    assert!((p.fraction().unwrap() - 0.4).abs() < 0.001);
}

#[test]
fn pull_failure_reopens_nothing_and_keeps_the_reference() {
    let mut m = Mutations::new();
    m.pull = Some(PullState { origin: PullOrigin::Header, reference: "bad:1".into(), status: String::new(), layers: Default::default() });
    let out = m.apply_pull_done(PullOrigin::Header, "bad:1", Some("manifest unknown".into()));
    assert!(out.refetch);
    assert!(!out.retry_run);
    assert_eq!(out.toast.unwrap().text, "manifest unknown");
    assert_eq!(m.pull_input, "bad:1", "the reference is kept for a one-click retry");
    assert!(m.pull.is_none());
}

#[test]
fn a_pull_retry_resumes_the_open_run_dialog() {
    let mut m = Mutations::new();
    m.run = Some(RunState { form: RunForm::default(), field_errors: vec![], daemon_error: Some("No such image".into()), missing_image: true, submitting: true, submitted: None, pull_then_retry: true });
    m.pull = Some(PullState { origin: PullOrigin::RunRetry, reference: "nginx".into(), status: String::new(), layers: Default::default() });
    let out = m.apply_pull_done(PullOrigin::RunRetry, "nginx", None);
    assert!(out.retry_run && !out.refetch);
    assert!(m.run.as_ref().unwrap().submitting);
    assert!(m.pull.is_none());

    let mut failed = Mutations::new();
    failed.run = Some(RunState { form: RunForm::default(), field_errors: vec![], daemon_error: None, missing_image: true, submitting: true, submitted: None, pull_then_retry: true });
    failed.pull = Some(PullState { origin: PullOrigin::RunRetry, reference: "nginx".into(), status: String::new(), layers: Default::default() });
    let out = failed.apply_pull_done(PullOrigin::RunRetry, "nginx", Some("manifest unknown".into()));
    assert!(!out.retry_run && !out.refetch && out.toast.is_none());
    let run = failed.run.as_ref().unwrap();
    assert!(!run.submitting && !run.pull_then_retry);
    assert_eq!(run.daemon_error.as_deref(), Some("manifest unknown"));
}

#[test]
fn run_outcomes_select_the_created_container() {
    let mut m = Mutations::new();
    m.run = Some(RunState { form: RunForm::default(), field_errors: vec![], daemon_error: None, missing_image: false, submitting: true, submitted: None, pull_then_retry: false });
    let out = m.apply_run_created("abc".into(), vec![]);
    assert!(out.close_dialog && out.refetch);
    assert_eq!(out.select_container.as_deref(), Some("abc"));
    assert!(m.run.is_none());

    let mut m = Mutations::new();
    m.run = Some(RunState { form: RunForm::default(), field_errors: vec![], daemon_error: None, missing_image: false, submitting: true, submitted: None, pull_then_retry: false });
    let out = m.apply_run_failed("port is allocated".into(), false, Some("abc".into()));
    assert!(out.close_dialog && out.refetch);
    assert_eq!(out.select_container.as_deref(), Some("abc"));
    assert_eq!(out.toast.unwrap().text, "port is allocated");
}

#[test]
fn a_pull_retry_reuses_the_submitted_spec_not_later_edits() {
    let submitted = RunSpec {
        image: "postgres:18".into(), name: None, command: None,
        ports: vec![], env: vec![], mounts: vec![],
    };
    let mut run = RunState {
        form: RunForm { image: "edited:latest".into(), ..Default::default() },
        submitted: Some(submitted.clone()),
        pull_then_retry: true,
        ..Default::default()
    };
    assert_eq!(run.retry_spec(), Some(submitted));
    run.pull_then_retry = false;
    assert_eq!(run.retry_spec(), None);
}

#[test]
fn a_missing_image_stays_in_the_dialog() {
    let mut m = Mutations::new();
    m.run = Some(RunState { form: RunForm::default(), field_errors: vec![], daemon_error: None, missing_image: false, submitting: true, submitted: None, pull_then_retry: false });
    let out = m.apply_run_failed("No such image".into(), true, None);
    assert!(!out.close_dialog && !out.refetch && out.toast.is_none());
    assert!(m.run.as_ref().unwrap().missing_image);
    assert!(!m.run.as_ref().unwrap().submitting);
}

#[test]
fn clear_in_flight_empties_everything_pending() {
    let mut m = Mutations::new();
    m.begin(&target(ResourceKind::Container, "a"), ActionKind::Stop);
    m.pull = Some(PullState { origin: PullOrigin::Header, reference: "x".into(), status: String::new(), layers: Default::default() });
    m.open_confirm(target(ResourceKind::Image, "i"), "img".into(), 1);
    m.confirm.as_mut().unwrap().submitting = true;
    m.clear_in_flight();
    assert!(!m.has_in_flight());
    assert!(m.pending.is_empty() && m.pull.is_none());
}

#[test]
fn confirm_copy_names_every_tag_for_a_multi_tag_image() {
    let (title, body) = confirm_copy(ResourceKind::Image, "postgres:18", 2);
    assert!(title.contains("postgres:18"));
    assert!(body.contains("every tag"));
    let (_, single) = confirm_copy(ResourceKind::Image, "postgres:18", 1);
    assert!(!single.contains("every tag"));
    let (_, volume) = confirm_copy(ResourceKind::Volume, "db-data", 0);
    assert!(volume.contains("data"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib ui::mutations`
Expected: compile error, module missing.

- [ ] **Step 3: Implement**

`apply_action_done`: match `pending.get(&key) == Some(&action)`. If it does not match: return `ActionOutcome { refetch: error.is_none(), ..Default::default() }`. If it matches: remove the entry. If `action == Remove` and `confirm` names the same target: clear/replace `submitting`, set `error` and return default, or on success clear `confirm`, `refetch: true`, `close_detail: true`. Otherwise on error return a toast; on success return `refetch: true` and `close_detail: action == Remove`.

`apply_pull_done`: require `pull` with the same origin and reference; otherwise return `{ refetch: error.is_none(), .. }`. When it matches: take the pull and set `refetch: true` either way. On error, if a run dialog is open with `pull_then_retry`, set its `daemon_error`, clear `submitting` and `pull_then_retry`, and return no toast; otherwise set `pull_input = reference.to_string()` and return an error toast. On success, return `retry_run: true` when a run dialog is open with `pull_then_retry` (leave `submitting` true), else just `refetch: true`.

`apply_run_created`: only when `run` is open and `submitting`; otherwise `RunOutcome { refetch: true, .. }`. When it applies: close the dialog, set `select_container`, refetch, and a non-error toast when warnings exist (`warnings.join(" ")`).
`apply_run_failed`: only when `run` is open and `submitting`; otherwise `{ refetch: created.is_some(), .. }`. With `created: Some(id)`: close, select, refetch, error toast. Without: keep the dialog, set `daemon_error`, `missing_image`, `submitting = false`.
`RunState::retry_spec`: `self.submitted.clone()` when `pull_then_retry`, else `None`. Root sends exactly this spec after a successful retry pull, so the frozen form and the running container can never disagree.

`confirm_copy`: container body `"This permanently deletes the container and its writable layer. Volumes are not removed."`; image body `"This deletes the image from this host. Containers using it keep running, but cannot be started again."` plus `" This removes every tag for this image."` when `tag_count > 1`; volume body `"This permanently deletes the volume and its data."`. Titles: `Remove container "{title}"?`, `Remove image "{title}"?`, `Remove volume "{title}"?`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::mutations`
Expected: 12 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/mutations.rs src/ui/mod.rs
git commit -m "feat: mutation UI state with stale-response matching"
```

---

### Task 6: Text input helper

**Files:**
- Create: `src/ui/widgets/mod.rs` (empty except `pub mod text_input;`)
- Create: `src/ui/widgets/text_input.rs`
- Modify: `src/ui/mod.rs` (add `pub mod widgets;`)
- Test: inline test module in `src/ui/widgets/text_input.rs`

**Interfaces:**
- Consumes: `crate::theme::Theme`, `gpui::{App, Div, FocusHandle, KeyDownEvent, Window, px, div}`.
- Produces:

```rust
pub fn apply_edit(text: &mut String, key: &str, platform: bool, control: bool,
                  pasted: Option<String>) -> bool;
pub fn edit_text(text: &mut String, event: &KeyDownEvent, cx: &mut App) -> bool;
pub fn field(id: impl Into<gpui::ElementId>, text: &str, placeholder: &str,
             focus: &FocusHandle, focused: bool, error: bool, read_only: bool,
             mono: bool, theme: &Theme) -> Div;
```

`field` renders a 28px-high input: `bg(theme.bg)`, 1px border in `theme.error` / `theme.accent` / `theme.border`, `text_sm`, optional `font_family("JetBrains Mono")`, dim placeholder when empty, a 1px accent caret when focused, and dim text when read-only. It calls `.track_focus(focus)` and focuses the field when clicked.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn edits_text_at_the_end() {
    let mut s = String::from("ngin");
    assert!(apply_edit(&mut s, "x", false, false, None));
    assert_eq!(s, "nginx");
    assert!(apply_edit(&mut s, "space", false, false, None));
    assert_eq!(s, "nginx ");
    assert!(apply_edit(&mut s, "backspace", false, false, None));
    assert_eq!(s, "nginx");
}

#[test]
fn ignores_navigation_and_shortcuts_but_pastes() {
    let mut s = String::new();
    assert!(!apply_edit(&mut s, "tab", false, false, None));
    assert!(!apply_edit(&mut s, "enter", false, false, None));
    assert!(!apply_edit(&mut s, "left", false, false, None));
    assert!(!apply_edit(&mut s, "a", true, false, None)); // cmd-a
    assert!(!apply_edit(&mut s, "a", false, true, None)); // ctrl-a
    assert!(apply_edit(&mut s, "v", true, false, Some("a\nb".into())));
    assert_eq!(s, "a b", "a pasted newline becomes a space in a single-line field");
}

#[test]
fn empty_text_ignores_backspace() {
    let mut s = String::new();
    assert!(!apply_edit(&mut s, "backspace", false, false, None));
    assert_eq!(s, "");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib ui::widgets::text_input`
Expected: compile error, module missing.

- [ ] **Step 3: Implement**

`apply_edit` order: platform+v with `pasted` → append with `\r`/`\n` replaced by spaces, return true. `backspace` → `pop`, return whether it removed anything. `platform || control` → false. `"space"` → push `' '`. Keys `tab`, `enter`, `escape`, and any key with `chars().count() != 1` → false. Otherwise append the character.
`edit_text` reads `event.keystroke.key`, `event.keystroke.modifiers.platform`, `.control`, and calls `cx.read_from_clipboard().and_then(|item| item.text())` for paste.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::widgets::text_input`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/ui/widgets src/ui/mod.rs
git commit -m "feat: text input editing helper and field renderer"
```

---

### Task 7: Action surface and confirm dialog

**Files:**
- Create: `assets/icons/play.svg`, `assets/icons/stop.svg`, `assets/icons/restart.svg`, `assets/icons/trash.svg`
- Modify: `src/assets.rs` (serve `icons/*.svg` in `load` and `list`)
- Create: `src/ui/dialog.rs` (confirm dialog only in this task)
- Modify: `src/ui/mod.rs` (add `pub mod dialog;`)
- Modify: `src/ui/root.rs`
- Test: manual verification; logic is covered by Task 5

**Interfaces:**
- Consumes: `Mutations`, `Target`, `ActionKind`, `container_actions`, `confirm_copy`.
- Produces:

```rust
// root.rs
pub struct RootView { /* existing */ pub mutations: Mutations }
fn icon_for(action: ActionKind) -> &'static str;
fn action_enabled(state: Option<ContainerState>, kind: ResourceKind, action: ActionKind) -> bool;
fn request_action(&mut self, kind: ResourceKind, id: &str, title: &str, action: ActionKind);
fn submit_confirm(&mut self);
fn cancel_confirm(&mut self);
fn open_detail_by_id(&mut self, kind: ResourceKind, id: &str);
fn render_action_buttons(&mut self, kind: ResourceKind, id: &str, title: &str,
                         state: Option<ContainerState>, emphasized: bool,
                         cx: &mut Context<Self>) -> Div;
fn render_row_menu(&mut self, cx: &mut Context<Self>) -> Option<Div>;
fn render_toast(&mut self, cx: &mut Context<Self>) -> Option<Div>;

// dialog.rs
pub fn confirm(confirm: &ConfirmState, theme: &Theme, cx: &mut Context<RootView>) -> AnyElement;
```

- [ ] **Step 1: Add the icons and asset arms**

Each SVG is 16×16, monochrome (`fill="#000"`), simple geometry: a filled triangle for play, a filled square for stop, a circular arrow for restart, a trash can for delete. `assets.rs` matches `"icons/play.svg"` etc. and includes the bytes with `include_bytes!`; `list()` includes the four names.

- [ ] **Step 2: Construct and route `Mutations` in `RootView`**

Initialize `mutations: Mutations::new()` in `RootView::new`. In `apply`, add:

```rust
Response::ActionDone { target, action, error } => {
    let outcome = self.mutations.apply_action_done(target.clone(), action, error);
    if outcome.close_detail {
        let open = self.detail.as_ref().map(|d| (d.kind, d.id.clone()));
        if open == Some((target.kind, target.id)) {
            self.close_detail();
        }
    }
    if let Some(toast) = outcome.toast { self.mutations.toast = Some(toast); }
    if outcome.refetch { self.refresh(); }
}
```

`request_action`: compute `let tag_count = if kind == ResourceKind::Image { self.snapshot.images.iter().find(|i| i.id == id).map(|i| i.tag_count).unwrap_or(1) } else { 0 };`. For Remove, `self.mutations.open_confirm(Target { kind, id: id.to_string() }, title.to_string(), tag_count)`. For start/stop/restart, require `self.service.is_some()`; if none, set a toast `"No host connection"` and return. Map `ActionKind` to `ContainerAction`, call `mutations.begin(&target, action)` and return if false, then `service.request(Request::ContainerAction { id: id.to_string(), action })`. Wrap image/volume actions so Start/Stop/Restart are no-ops for them.

`submit_confirm`: read the confirm target and force; set `submitting = true`; send `RemoveContainer`, `RemoveImage`, or `RemoveVolume` accordingly; on no service set the dialog error.

- [ ] **Step 3: Render the actions column**

In `render_table`, after the data cells, when `table.spec.actions` is set:

```rust
let kind = actions.kind();
let state = if kind == ResourceKind::Container {
    this.snapshot.containers.iter().find(|c| c.id == row.id).map(|c| c.state)
} else { None };
let cluster = this.render_action_buttons(kind, &row.id, row.cells[0].as_ref(), state, emphasized, cx);
```

`render_action_buttons` emits 24×24 targets with a 14px `svg()`, a 4px gap, `.opacity(if emphasized { 1.0 } else { 0.25 })`, `.cursor_pointer()` when enabled, and `.on_click(cx.listener(move |this, _, _, cx| { this.request_action(kind, &id, &title, action); cx.notify(); }))`. Disabled actions have no listener and `text_dim`. Containers show play/stop/restart/trash; images show trash (Run arrives in Task 9); volumes show trash. The empty header cell width is `actions.width()`.

In the row closure, compute `emphasized = this.mutations.hovered_row.as_deref() == Some(row.id.as_str()) || selected == Some(row_index)`. Add `.on_hover(cx.listener(...))` on the row as described in the design, clearing only when the id still matches.

Pending state: when `this.mutations.pending.get(&(kind, row.id.clone())).is_some()`, render the State cell (column 1) as `action.pending_label()` in `theme.paused` instead of the snapshot label.

- [ ] **Step 4: Render the context menu**

Add `.on_mouse_down(MouseButton::Right, cx.listener(move |this, event, _window, cx| { this.current_table_mut().selected = Some(row_index); this.mutations.row_menu = Some(RowMenuState { kind, id: row_id.clone(), at: event.position }); cx.notify(); }))` to each row.

`render_row_menu` returns `None` unless `row_menu` is set. It renders a full-size transparent backdrop that closes the menu on mousedown, plus an `absolute()` panel at `at`, `w(px(200.))`, panel/border/radius/shadow, with `Open`, a separator, the available container actions, and `Remove` in `theme.error`. Items call `open_detail_by_id`, `request_action`, or `open_confirm`. Append the menu to the root after content.

- [ ] **Step 5: Render the confirm dialog**

`dialog::confirm` draws a full-size backdrop (`black` at 50%) and a centered 440px panel: title, body from `confirm_copy`, optional Force checkbox (`14px` box, accent fill with a `✓` when ticked; label and consequence line per kind), the inline error in `theme.error`, and Cancel / Remove buttons. Remove uses `theme.error` as its background. Backdrop and Cancel call `cancel_confirm`; Esc is handled in Task 10.

Append the dialog in `RootView::render` after the row menu. Also render the toast there: when `mutations.toast` is set, a bottom-center 420px panel (`bg(theme.panel)`, `border_1`, error-colored left border when `toast.error`, text plus a `×` that clears it). One at a time, no timer.

- [ ] **Step 6: Detail-page toolbar**

In `render_detail`, after the title and state, render the same action cluster for `detail.kind`/`detail.id`, using `detail.title` for the confirm copy and the container state looked up in the snapshot. Show the pending label in `theme.paused` when one exists.

- [ ] **Step 7: Run tests and verify manually**

Run: `cargo test --lib` — PASS.
Run: `cargo run` — hover a stopped container and press play; it starts within ~1s. Stop and restart a running one. Delete a running container: the dialog appears, Remove without Force shows the 409 inline, Force succeeds and the row disappears. Right-click opens the same menu. Repeat Delete for an in-use image and for a volume.

- [ ] **Step 8: Commit**

```bash
git add assets/icons src/assets.rs src/ui/dialog.rs src/ui/mod.rs src/ui/root.rs
git commit -m "feat: row actions, context menu and remove confirmation"
```

---

### Task 8: Pull

**Files:**
- Create: `assets/icons/pull.svg`
- Modify: `src/assets.rs`
- Modify: `src/ui/root.rs`
- Modify: `src/ui/dialog.rs` (pull popover)
- Test: manual; logic is covered by Task 5

**Interfaces:**
- Consumes: `Mutations.pull`, `PullState`, `PullOrigin`, `apply_edit`.
- Produces:

```rust
// root.rs
fn start_pull(&mut self, reference: &str, origin: PullOrigin);
fn retry_run_after_pull(&mut self);
fn render_pull_popover(&mut self, cx: &mut Context<Self>) -> Option<Div>;
fn render_pull_strip(&mut self) -> Option<Div>;

// dialog.rs
pub fn pull_popover(reference: &str, error: Option<&str>, pulling: bool,
                    focused: bool, theme: &Theme,
                    cx: &mut Context<RootView>) -> AnyElement;
```

- [ ] **Step 1: Wire the Pull button and popover**

`RootView` gains `pull_focus: FocusHandle` created in `new`. On the Images page, the header renders a Pull button (icon + `"Pull"`, or `"Pulling…"` while `mutations.pull.is_some()`) right of the search, disabled while a pull is running. Clicking toggles `mutations.pull_open` and focuses `pull_focus` when opening.

The popover renders under the Images header, 360px wide: `text_input::field` for the reference, an inline error, and a Pull button disabled when the trimmed input is empty or a pull is running.

Key handling on the input: `edit_text(&mut mutations.pull_input, event, cx)`; Enter → `start_pull(input.trim(), PullOrigin::Header)`; Esc → `pull_open = false` (Task 10 also routes it through `go_back`).

- [ ] **Step 2: Send and route the pull**

`start_pull`: trim; return if empty or `pull.is_some()`; require a service (otherwise toast `"No host connection"`); set `pull = Some(PullState { origin, reference, status: "Starting…", layers: BTreeMap::new() })`, clear `pull_error`, close the popover, and send `Request::PullImage`.

In `apply`: `PullProgress` → `mutations.apply_pull_progress(...)`; `PullDone` → `apply_pull_done`; if `retry_run` call `retry_run_after_pull()`; if `refetch` refresh; if a toast, set it; copy the reference into `pull_input` so a retry is easy.

`retry_run_after_pull`: if `mutations.run` is open, take `run.retry_spec()`; when it is `Some(spec)`, send `Request::RunContainer { spec }` and set `pull_then_retry = false`. Nothing is sent when the dialog closed during the pull.

- [ ] **Step 3: Render the progress strip**

When `pull.is_some()`, render a 32px strip in the list column directly under the header (it stays visible on all three list pages; an open detail page omits it): the word `"Pulling"`, reference in mono, last status, `format_bytes(current)` / `format_bytes(total)` when `bytes()` is `Some`, and a 3px bar whose width is `fraction() * 100%` over a `theme.border` track. No totals → no bar.

- [ ] **Step 4: Run tests and verify manually**

Run: `cargo test --lib` — PASS.
Run: `cargo run`, Images → Pull `nginx:alpine`: the strip advances, the button reads `Pulling…`, and the image appears without a manual refresh. Start it again with `not-a-real/repo:nope`: a toast carries the daemon message, the strip clears, and the reference is still in the input. While a pull runs, scroll and switch pages to confirm lists stay live.

- [ ] **Step 5: Commit**

```bash
git add assets/icons/pull.svg src/assets.rs src/ui/dialog.rs src/ui/root.rs
git commit -m "feat: image pull with progress and retry-friendly errors"
```

---

### Task 9: Run dialog

**Files:**
- Create: `assets/icons/run.svg`, `assets/icons/plus.svg`, `assets/icons/close.svg`
- Modify: `src/assets.rs`
- Modify: `src/ui/dialog.rs`
- Modify: `src/ui/root.rs`
- Test: manual; validation is Task 2, routing is Task 5

**Interfaces:**
- Consumes: `RunForm`, `RunField`, `build_run_spec`, `RunState`, `RunOutcome`, `text_input`, `start_pull`.
- Produces:

```rust
// dialog.rs
pub struct RunFocus {
    pub image: FocusHandle, pub name: FocusHandle, pub command: FocusHandle,
    pub ports: Vec<(FocusHandle, FocusHandle)>,
    pub env: Vec<(FocusHandle, FocusHandle)>,
    pub mounts: Vec<(FocusHandle, FocusHandle)>,
}
impl RunFocus {
    pub fn new(cx: &mut Context<RootView>) -> Self;
    pub fn add_port(&mut self, cx: &mut Context<RootView>);
    pub fn remove_port(&mut self, index: usize);
    pub fn add_env(&mut self, cx: &mut Context<RootView>);
    pub fn remove_env(&mut self, index: usize);
    pub fn add_mount(&mut self, cx: &mut Context<RootView>);
    pub fn remove_mount(&mut self, index: usize);
    pub fn handle(&self, field: RunField) -> Option<FocusHandle>;
}
pub fn run_dialog(run: &RunState, focus: &RunFocus, theme: &Theme,
                  window: &mut Window, cx: &mut Context<RootView>) -> AnyElement;

// root.rs
fn open_run(&mut self, prefill: Option<String>, window: &mut Window, cx: &mut Context<Self>);
fn cancel_run(&mut self);
fn submit_run(&mut self, window: &mut Window, cx: &mut Context<Self>);
fn run_field_text_mut(&mut self, field: RunField) -> Option<&mut String>;
fn clear_run_field_error(&mut self, field: RunField);
fn add_run_port(&mut self, cx: &mut Context<Self>);
fn remove_run_port(&mut self, index: usize);
fn add_run_env(&mut self, cx: &mut Context<Self>);
fn remove_run_env(&mut self, index: usize);
fn add_run_mount(&mut self, cx: &mut Context<Self>);
fn remove_run_mount(&mut self, index: usize);
```

- [ ] **Step 1: Add the icons**

`run.svg` (play inside a rounded square), `plus.svg`, `close.svg`; extend `assets.rs` as in Task 7.

- [ ] **Step 2: Own the focus handles and open the dialog**

`RootView` gains `run_focus: Option<RunFocus>`. `open_run` sets `mutations.run = Some(RunState { form: RunForm { image: prefill.unwrap_or_default(), ..Default::default() }, ..Default::default() })`, builds `RunFocus::new(cx)`, and calls `window.focus(&run_focus.image)`. `RunState` derives `Default` (Task 5).

`RunFocus::new` creates every handle with `cx.focus_handle().tab_stop(true)` — GPUI defaults `tab_stop` to `false`, so Tab would otherwise skip every field — for image/name/command and no rows. Add/remove methods mirror the form vectors; every `open_run` and row change keeps `run_focus` in sync.

- [ ] **Step 3: Render the dialog**

`run_dialog` renders a backdrop and a 640px panel (`max_h` 80% of the window), fixed header (`Run a container`, close icon), scrolling body, fixed footer (Cancel, Run). Body sections:

- Image / Name / Command: `text_input::field` with ids `run-image`, `run-name`, `run-command`; error border when a `FieldError` names the field; message below.
- Ports: header + `+ Add port`; each row = host input, container input, a tcp/udp toggle (two clickable labels), and a 14px close icon that calls `remove_run_port`.
- Environment: `+ Add variable`; each row = key input, value input, remove icon.
- Volumes: `+ Add volume`; each row = source input, target input, a `ro` toggle, remove icon.

Wire each input with `track_focus`, `on_click` (field already does it), and:

```rust
.on_key_down(cx.listener(move |this, event, window, cx| {
    if event.keystroke.key == "tab" {
        if event.keystroke.modifiers.shift { window.focus_prev(); } else { window.focus_next(); }
        return;
    }
    if event.keystroke.key == "enter" {
        this.submit_run(window, cx);
        return;
    }
    let Some(text) = this.run_field_text_mut(field) else { return };
    if apply_edit(text, event, cx) {
        this.clear_run_field_error(field);
        cx.notify();
    }
}))
```

`run_dialog` is a free function holding `&RunState`; clone each field's small render values (text, placeholder, invalid flag, focus handle) into locals before calling `field`, so `cx` can be borrowed mutably for the listeners.

Inputs are `read_only = run.submitting`, and Add/Remove controls are disabled while submitting. The footer shows the daemon error, and when `missing_image` is true a `Pull it` button calls `start_pull(run.form.image.trim(), PullOrigin::RunRetry)` after setting `pull_then_retry = true`; the button is disabled while a pull runs. The Run button is disabled when the trimmed image is empty or `submitting`.

- [ ] **Step 4: Submit and route results**

`submit_run`: return if `submitting`; call `build_run_spec`. On `Err(errors)`: store them, clear `daemon_error`, focus the first error's handle. On `Ok(spec)`: clear field errors, set `submitted = Some(spec.clone())`, `submitting = true`, `pull_then_retry = false`, `daemon_error = None`, and send `Request::RunContainer { spec }`.

In `apply`: `ContainerCreated` → `apply_run_created`; `RunFailed` → `apply_run_failed`. Apply the outcome exactly: if `close_dialog`, set `run = None` and `run_focus = None`; if `select_container`, switch to `Page::Containers` and set `pending_select`; if `refetch`, refresh; if `toast`, set it. When the dialog stays open (missing image or a daemon create error), only its inline error and `submitting` change.

In `apply(Response::Snapshot)`: if `pending_select` is set, try to find that container id. If found, set `tables[0].selected` and call `open_detail`; clear `pending_select` either way (one snapshot attempt only).

- [ ] **Step 5: Add Run entry points**

Containers header: a `Run` button (icon + label) left of search, calling `open_run(None, window, cx)`. Images rows: add the Run icon to the image action cluster (the 84px width already reserves it), prefilled with `repository_tag` or the image id when the tag is `<none>`; it calls `open_run(Some(prefill), window, cx)`.

- [ ] **Step 6: Run tests and verify manually**

Run: `cargo test --lib` — PASS.
Run: `cargo run`, Run: fill image `postgres:18`, name `dockyard-run-test`, command `postgres -c "max_connections=200"`, one port pair, one env pair, one named volume. The container appears running and its detail opens. Reuse the name → the daemon error stays in the dialog. Submit an image that is not local → `Pull it` appears, the pull progress shows in the footer, and the container is created after it without retyping; try to edit the fields while that pull runs and confirm they are read-only and the created container matches what was submitted. Use a taken host port → the dialog closes, the created row opens, and the toast names the start error. Tab/Shift-Tab traverse, ⌘V pastes, Esc cancels when not submitting.

- [ ] **Step 7: Commit**

```bash
git add assets/icons src/assets.rs src/ui/dialog.rs src/ui/root.rs
git commit -m "feat: run container dialog with validation and pull retry"
```

---

### Task 10: Keyboard, menus, disconnects, README

**Files:**
- Modify: `src/actions.rs`
- Modify: `src/lib.rs`
- Modify: `src/ui/root.rs`
- Modify: `README.md`
- Test: manual acceptance list below

**Interfaces:**
- Consumes: everything above.
- Produces: `RunContainer` and `PullImage` actions; Esc precedence; disconnect clearing.

- [ ] **Step 1: Add the actions and keys**

`actions.rs` adds `RunContainer, PullImage`. `lib.rs` binds `cmd-n` → `RunContainer` and `cmd-shift-p` → `PullImage`, and adds a `Docker` menu with `Run Container…` and `Pull Image…` above the existing View menu.

`RootView` handles them: `RunContainer` → `open_run(None, window, cx)`; `PullImage` → `switch_page(Page::Images)`, `mutations.pull_open = true`, focus `pull_focus`. Ignore both while a modal (`confirm` or `run`) is open.

- [ ] **Step 2: Esc precedence**

In `go_back`, before the existing query/detail logic:

```rust
if let Some(confirm) = &self.mutations.confirm {
    if !confirm.submitting { self.cancel_confirm(); return; }
    return;
}
if let Some(run) = &self.mutations.run {
    if !run.submitting { self.cancel_run(); return; }
    return;
}
if self.mutations.row_menu.take().is_some() { return; }
if self.mutations.pull_open { self.mutations.pull_open = false; return; }
```

A cancelled run drops `pull_then_retry` but leaves an in-flight pull running; the pull then behaves like a header pull.

- [ ] **Step 3: Clear in-flight work on disconnect**

In `apply(Response::Status(status))`: before replacing the status, if the next status is not `Connected` and `mutations.has_in_flight()`, snapshot `had_work = true`. After setting the status, when `!matches!(self.status, Connected)` and `had_work`: `mutations.clear_in_flight()`, set `confirm = None`, `run = None`, `run_focus = None`, `row_menu = None`, and toast `"Host disconnected — action interrupted"`. Keep `pull_input` so a retry is one click away. A host switch goes through `connect`: reset `mutations = Mutations::new()` and `run_focus = None` there, beside `reset_host_data`.

- [ ] **Step 4: Update the README**

Replace the read-only bullet in "Not in this version" with the real limits: no prune/bulk actions, no registry auth, no pull cancel, no exec/build/compose. Add a "Doing things" section listing the six verbs, where they live, the confirmation/Force rule, and the `⌘N` / `⌘⇧P` keys.

- [ ] **Step 5: Run the full suite and the acceptance list**

Run: `cargo test --lib` — PASS.
Run: `cargo test --test live_docker -- --ignored --test-threads=1` — PASS (daemon running).
Run: `cargo run`, then walk the spec's manual acceptance list 1–13:
1. hover start/stop/restart flips within ~1s; 2. right-click menu; 3. double-click Stop fires once; 4. force-remove a running container; 5. force-remove an in-use image; 6. in-use volume error stays in the dialog; 7. pull with progress; 8. bad pull → toast, reference kept; 9. full run form; 10. missing image → Pull it; 11. taken port → created row opens with a toast; 12. Tab/⌘V/Esc/⌘N; 13. kill Rancher mid-pull and mid-stop → one toast, no reopened dialog, recovery on restart.

Record any deviation in the commit message.

- [ ] **Step 6: Commit**

```bash
git add src/actions.rs src/lib.rs src/ui/root.rs README.md
git commit -m "feat: keyboard, menu and disconnect handling for mutations"
```
