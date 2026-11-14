use dockyard::docker::endpoint::discover;
use dockyard::docker::service::{DockerService, Request, Response};
use std::collections::HashMap;
use std::path::PathBuf;

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
