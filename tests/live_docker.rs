use contui::docker::actions::{ContainerAction, PortSpec, RunSpec};
use contui::docker::endpoint::discover;
use contui::docker::service::{DockerService, PullOrigin, Request, Response};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn connect() -> (DockerService, async_channel::Receiver<Response>) {
    let env: HashMap<String, String> = std::env::vars().collect();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap();
    let discovery = discover(&env, &home);
    let active = discovery.active.clone().expect("an active host");
    let host = discovery
        .hosts
        .into_iter()
        .find(|h| h.id == active)
        .expect("active host in list");
    DockerService::connect(host).expect("connect to docker")
}

/// Skips status, event and detail traffic and returns the first matching
/// mutation or snapshot response.
fn wait_for(
    responses: &async_channel::Receiver<Response>,
    what: &str,
    pred: impl Fn(&Response) -> bool,
) -> Response {
    loop {
        let response = responses
            .recv_blocking()
            .unwrap_or_else(|err| panic!("channel closed while waiting for {what}: {err}"));
        if pred(&response) {
            return response;
        }
        match &response {
            Response::Status(_)
            | Response::HostStatuses(_)
            | Response::Dirty { .. }
            | Response::PullProgress { .. }
            | Response::Inspect { .. }
            | Response::Logs { .. }
            | Response::Stats { .. }
            | Response::StatsStale { .. } => continue,
            _ => {}
        }
        panic!("while waiting for {what}, got {response:?}");
    }
}

fn action_error(responses: &async_channel::Receiver<Response>, what: &str) -> Option<String> {
    match wait_for(responses, what, |response| {
        matches!(response, Response::ActionDone { .. })
    }) {
        Response::ActionDone { error, .. } => error,
        _ => unreachable!(),
    }
}

fn run_spec(name: &str, host_port: Option<u16>) -> RunSpec {
    RunSpec {
        image: "alpine:3.20".into(),
        name: Some(name.into()),
        command: Some(vec!["sh".into(), "-c".into(), "sleep 60".into()]),
        ports: host_port
            .map(|port| PortSpec {
                host: Some(port),
                container: 80,
                udp: false,
            })
            .into_iter()
            .collect(),
        ..Default::default()
    }
}

fn cleanup_test_containers(service: &DockerService, responses: &async_channel::Receiver<Response>) {
    service.request(Request::FetchAll);
    let snapshot = match wait_for(responses, "snapshot for cleanup", |response| {
        matches!(response, Response::Snapshot(_))
    }) {
        Response::Snapshot(snapshot) => snapshot,
        _ => return,
    };
    for container in snapshot.containers {
        let name = container.name.to_string();
        if name.starts_with("dockyard-live-") || name.starts_with("dockyard-port-") {
            service.request(Request::RemoveContainer {
                id: container.id,
                force: true,
                epoch: 0,
            });
            let _ = action_error(responses, "cleanup remove");
        }
    }
}

#[test]
#[ignore = "requires a running docker daemon"]
fn pull_run_stop_restart_and_force_remove_round_trip() {
    let (service, responses) = connect();
    cleanup_test_containers(&service, &responses);

    service.request(Request::PullImage {
        reference: "alpine:3.20".into(),
        origin: PullOrigin::Header,
        epoch: 0,
    });
    let pulled = wait_for(&responses, "pull alpine:3.20", |response| {
        matches!(response, Response::PullDone { .. })
    });
    if let Response::PullDone {
        error: Some(error), ..
    } = pulled
    {
        eprintln!("skipping live run test: pull failed: {error}");
        return;
    }

    let name = format!("dockyard-live-{}", std::process::id());
    service.request(Request::RunContainer {
        spec: run_spec(&name, None),
        epoch: 0,
    });
    let created = wait_for(&responses, "run", |response| {
        matches!(
            response,
            Response::ContainerCreated { .. } | Response::RunFailed { .. }
        )
    });
    let id = match created {
        Response::ContainerCreated { id, .. } => id,
        Response::RunFailed { message, .. } => panic!("run failed: {message}"),
        _ => unreachable!(),
    };

    service.request(Request::ContainerAction {
        id: id.clone(),
        action: ContainerAction::Stop,
        epoch: 0,
    });
    assert_eq!(action_error(&responses, "stop"), None);

    service.request(Request::ContainerAction {
        id: id.clone(),
        action: ContainerAction::Restart,
        epoch: 0,
    });
    assert_eq!(action_error(&responses, "restart"), None);

    service.request(Request::RemoveContainer {
        id: id.clone(),
        force: false,
        epoch: 0,
    });
    let blocked = action_error(&responses, "remove running container")
        .expect("a running container refuses a plain remove");
    assert!(
        blocked.to_lowercase().contains("running"),
        "unexpected removal error: {blocked}"
    );

    service.request(Request::RemoveContainer {
        id,
        force: true,
        epoch: 0,
    });
    assert_eq!(action_error(&responses, "force remove"), None);
}

#[test]
#[ignore = "requires a running docker daemon"]
fn run_with_a_taken_host_port_keeps_the_created_container() {
    let (service, responses) = connect();
    cleanup_test_containers(&service, &responses);

    service.request(Request::PullImage {
        reference: "alpine:3.20".into(),
        origin: PullOrigin::Header,
        epoch: 0,
    });
    let pulled = wait_for(&responses, "pull alpine:3.20", |response| {
        matches!(response, Response::PullDone { .. })
    });
    if let Response::PullDone {
        error: Some(error), ..
    } = pulled
    {
        eprintln!("skipping live port test: pull failed: {error}");
        return;
    }

    let first_name = format!("dockyard-port-a-{}", std::process::id());
    service.request(Request::RunContainer {
        spec: run_spec(&first_name, Some(45999)),
        epoch: 0,
    });
    let first = wait_for(&responses, "first run", |response| {
        matches!(
            response,
            Response::ContainerCreated { .. } | Response::RunFailed { .. }
        )
    });
    let first_id = match first {
        Response::ContainerCreated { id, .. } => id,
        Response::RunFailed { message, .. } => {
            eprintln!("skipping live port test: host port 45999 unavailable: {message}");
            return;
        }
        _ => unreachable!(),
    };

    let second_name = format!("dockyard-port-b-{}", std::process::id());
    service.request(Request::RunContainer {
        spec: run_spec(&second_name, Some(45999)),
        epoch: 0,
    });
    let second = wait_for(&responses, "second run", |response| {
        matches!(
            response,
            Response::ContainerCreated { .. } | Response::RunFailed { .. }
        )
    });
    let second_id = match second {
        Response::RunFailed {
            message,
            created: Some(id),
            ..
        } => {
            assert!(
                message.contains("port is already allocated")
                    || message.contains("address already in use"),
                "unexpected start error: {message}"
            );
            id
        }
        Response::RunFailed { message, .. } => {
            panic!("create failed instead of start: {message}")
        }
        Response::ContainerCreated { .. } => panic!("second container should not start"),
        _ => unreachable!(),
    };

    service.request(Request::RemoveContainer {
        id: first_id,
        force: true,
        epoch: 0,
    });
    assert_eq!(action_error(&responses, "remove first"), None);
    service.request(Request::RemoveContainer {
        id: second_id,
        force: true,
        epoch: 0,
    });
    assert_eq!(action_error(&responses, "remove second"), None);
}

#[test]
#[ignore = "requires a running docker daemon"]
fn list_requests_are_answered_while_a_pull_runs() {
    let (service, responses) = connect();
    let started = Instant::now();

    service.request(Request::PullImage {
        reference: "debian:bookworm-slim".into(),
        origin: PullOrigin::Header,
        epoch: 0,
    });
    service.request(Request::FetchAll);

    let mut snapshot_at = None;
    let mut pull_done = false;
    let mut pull_error = None;
    let deadline = started + Duration::from_secs(120);

    while (snapshot_at.is_none() || !pull_done) && Instant::now() < deadline {
        while let Ok(response) = responses.try_recv() {
            match response {
                Response::Snapshot(_) => {
                    snapshot_at.get_or_insert_with(Instant::now);
                }
                Response::PullDone { error, .. } => {
                    pull_done = true;
                    pull_error = error;
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let snapshot_at = snapshot_at.expect("a snapshot while a pull runs");
    assert!(
        snapshot_at.duration_since(started) <= Duration::from_secs(2),
        "snapshot took {:?} while a pull was running",
        snapshot_at.duration_since(started)
    );
    assert!(pull_done, "pull did not finish; last error: {pull_error:?}");
    if let Some(error) = pull_error {
        eprintln!("pull reported an error after the snapshot: {error}");
    }
}

#[test]
#[ignore = "requires a running docker daemon"]
fn lists_three_resources_from_live_daemon() {
    let env: HashMap<String, String> = std::env::vars().collect();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap();
    let discovery = discover(&env, &home);
    let active = discovery.active.clone().expect("an active host");
    let host = discovery
        .hosts
        .into_iter()
        .find(|h| h.id == active)
        .expect("active host in list");

    let (service, responses) = DockerService::connect(host).expect("connect to docker");

    // The actor reports status first, then answers our request.
    let status = responses.recv_blocking().expect("status response");
    assert!(
        matches!(status, Response::Status(_)),
        "expected status first, got {status:?}"
    );

    service.request(Request::FetchAll);

    let mut snapshot = None;
    for _ in 0..5 {
        match responses.recv_blocking() {
            Ok(Response::Snapshot(s)) => {
                snapshot = Some(s);
                break;
            }
            Ok(Response::Failed { what, message }) => {
                panic!("request {what} failed: {message}")
            }
            Ok(_) => continue,
            Err(err) => panic!("channel closed: {err}"),
        }
    }

    let snapshot = snapshot.expect("snapshot within 5 responses");
    assert!(
        !snapshot.containers.is_empty(),
        "expected at least one container"
    );
    assert!(!snapshot.images.is_empty(), "expected at least one image");
    assert!(snapshot.fetched_at.is_some());
}

#[test]
#[ignore = "requires a running docker daemon"]
fn stats_stream_reports_live_usage_for_a_running_container() {
    let (service, responses) = connect();
    cleanup_test_containers(&service, &responses);

    service.request(Request::PullImage {
        reference: "alpine:3.20".into(),
        origin: PullOrigin::Header,
        epoch: 0,
    });
    let pulled = wait_for(&responses, "pull alpine:3.20", |response| {
        matches!(response, Response::PullDone { .. })
    });
    if let Response::PullDone {
        error: Some(error), ..
    } = pulled
    {
        eprintln!("skipping live stats test: pull failed: {error}");
        return;
    }

    let name = format!("dockyard-live-stats-{}", std::process::id());
    service.request(Request::RunContainer {
        spec: run_spec(&name, None),
        epoch: 0,
    });
    let created = wait_for(&responses, "run", |response| {
        matches!(
            response,
            Response::ContainerCreated { .. } | Response::RunFailed { .. }
        )
    });
    let id = match created {
        Response::ContainerCreated { id, .. } => id,
        Response::RunFailed { message, .. } => panic!("run failed: {message}"),
        _ => unreachable!(),
    };

    service.request(Request::FetchAll);
    let _ = wait_for(&responses, "snapshot for stats", |response| {
        matches!(response, Response::Snapshot(_))
    });
    service.request(Request::WatchStats {
        ids: vec![id.clone()],
    });
    let stats = wait_for(&responses, "container stats", |response| {
        matches!(
            response,
            Response::Stats { stats, .. }
                if stats.cpu_percent.is_some() && stats.memory.is_some()
        )
    });
    match stats {
        Response::Stats { stats, .. } => {
            assert!(stats.cpu_percent.unwrap() >= 0.0);
            assert!(
                stats.memory.unwrap().used > 0,
                "expected a non-zero memory working set"
            );
        }
        _ => unreachable!(),
    }

    service.request(Request::RemoveContainer {
        id,
        force: true,
        epoch: 0,
    });
    assert_eq!(action_error(&responses, "stats cleanup remove"), None);
}
