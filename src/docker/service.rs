use crate::docker::actions::{
    classify_run_error, clean_error, run_body, split_image_reference, ActionKind, ContainerAction,
    RunSpec, Target,
};
use crate::docker::endpoint::{
    discover, EndpointKind, HostInfo, HostStatus, TlsFiles,
};
use crate::docker::events::Debouncer;
use crate::docker::model::{
    parse_logs, status_from_error, ContainerRow, ContainerStats, ImageRow, LogLine, ResourceKind,
    Snapshot, VolumeRow,
};
use async_channel::{Receiver, Sender};
use bollard::Docker;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptions, CreateImageOptionsBuilder,
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, ListVolumesOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptions, RemoveContainerOptionsBuilder,
    RemoveImageOptionsBuilder, RemoveVolumeOptionsBuilder, StatsOptionsBuilder,
    StopContainerOptions, StopContainerOptionsBuilder,
};
use chrono::Utc;
use futures_util::{join, StreamExt};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// Bollard's connection timeout is a per-read/write socket timeout, not a
/// connect timeout. It must outlast Docker's 10s stop grace and the 60s pull
/// idle cap, or a quiet-but-healthy operation is reported as transport failure.
const CLIENT_TIMEOUT_SECS: u64 = 65;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullOrigin {
    Header,
    RunRetry,
}

#[derive(Clone, Debug)]
pub enum Request {
    FetchAll,
    ProbeHosts,
    Inspect {
        kind: ResourceKind,
        id: String,
    },
    Logs {
        id: String,
    },
    ContainerAction {
        id: String,
        action: ContainerAction,
        epoch: u64,
    },
    RemoveContainer {
        id: String,
        force: bool,
        epoch: u64,
    },
    RemoveImage {
        id: String,
        force: bool,
        epoch: u64,
    },
    RemoveVolume {
        name: String,
        epoch: u64,
    },
    PullImage {
        reference: String,
        origin: PullOrigin,
        epoch: u64,
    },
    RunContainer {
        spec: RunSpec,
        epoch: u64,
    },
    /// The exact set of containers whose stats the UI can currently show.
    WatchStats {
        ids: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub enum Response {
    Snapshot(Snapshot),
    Status(HostStatus),
    HostStatuses(Vec<(String, HostStatus)>),
    Inspect {
        kind: ResourceKind,
        id: String,
        json: serde_json::Value,
    },
    Logs {
        id: String,
        lines: Vec<LogLine>,
    },
    Dirty {
        ids: Vec<(ResourceKind, String)>,
    },
    Failed {
        what: String,
        message: String,
    },
    ActionDone {
        target: Target,
        action: ActionKind,
        error: Option<String>,
        epoch: u64,
    },
    PullProgress {
        origin: PullOrigin,
        reference: String,
        layer: Option<String>,
        status: String,
        detail: Option<(u64, u64)>,
        epoch: u64,
    },
    PullDone {
        origin: PullOrigin,
        reference: String,
        error: Option<String>,
        epoch: u64,
    },
    ContainerCreated {
        id: String,
        warnings: Vec<String>,
        epoch: u64,
    },
    RunFailed {
        message: String,
        missing_image: bool,
        created: Option<String>,
        epoch: u64,
    },
    Stats {
        id: String,
        stats: ContainerStats,
    },
    /// The stream stalled or errored; the last reading is no longer live.
    StatsStale {
        id: String,
    },
}

pub fn remove_container_options(force: bool) -> RemoveContainerOptions {
    RemoveContainerOptionsBuilder::new()
        .force(force)
        .v(false)
        .build()
}

pub fn stop_options() -> StopContainerOptions {
    StopContainerOptionsBuilder::new().t(10).build()
}

/// Docker's stats stream emits once a second; the UI only needs a reading
/// every two. The stream stays open either way.
const STATS_UI_INTERVAL: Duration = Duration::from_secs(2);

pub fn stats_options() -> bollard::query_parameters::StatsOptions {
    StatsOptionsBuilder::new().stream(true).one_shot(false).build()
}

fn stats_sample_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|last| now.duration_since(last) >= STATS_UI_INTERVAL)
}

/// A live stream should emit every second. Five seconds of silence means the
/// connection is wedged, so the reading is dropped rather than left frozen.
const STATS_STALL_TIMEOUT: Duration = Duration::from_secs(5);

async fn next_sample<S>(
    stream: &mut S,
    timeout: Duration,
) -> Option<Result<bollard::models::ContainerStatsResponse, bollard::errors::Error>>
where
    S: futures_util::Stream<
            Item = Result<bollard::models::ContainerStatsResponse, bollard::errors::Error>,
        > + Unpin,
{
    tokio::time::timeout(timeout, stream.next())
        .await
        .ok()
        .flatten()
}

pub fn pull_options(reference: &str) -> CreateImageOptions {
    let (repo, tag) = split_image_reference(reference);
    let builder = CreateImageOptionsBuilder::new().from_image(&repo);
    match tag {
        Some(tag) => builder.tag(&tag).build(),
        None => builder.build(),
    }
}

pub struct DockerService {
    cmd_tx: Sender<Request>,
    runtime: Option<Runtime>,
}

impl DockerService {
    pub fn connect(host: HostInfo) -> Result<(Self, Receiver<Response>), String> {
        let docker = build_docker(&host)?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| err.to_string())?;

        let (cmd_tx, cmd_rx) = async_channel::unbounded();
        let (resp_tx, resp_rx) = async_channel::unbounded();

        runtime.spawn(actor(docker.clone(), cmd_rx, resp_tx.clone()));
        runtime.spawn(event_loop(docker, resp_tx));

        Ok((
            Self {
                cmd_tx,
                runtime: Some(runtime),
            },
            resp_rx,
        ))
    }

    pub fn request(&self, req: Request) {
        let _ = self.cmd_tx.try_send(req);
    }
}

impl Drop for DockerService {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            // Cancels the actor, the event loop and every stats stream. The
            // timeout bounds a host switch or quit when a stream is stuck.
            runtime.shutdown_timeout(Duration::from_millis(500));
        }
    }
}

fn build_docker(host: &HostInfo) -> Result<Docker, String> {
    let version = bollard::API_DEFAULT_VERSION;
    match host.endpoint.kind {
        EndpointKind::Unix => Docker::connect_with_unix(&host.endpoint.address, CLIENT_TIMEOUT_SECS, version),
        EndpointKind::Http => Docker::connect_with_http(&host.endpoint.address, CLIENT_TIMEOUT_SECS, version),
        EndpointKind::Tls => {
            let tls = host
                .tls
                .as_ref()
                .ok_or_else(|| "TLS context without certificate files".to_string())?;
            connect_tls(&host.endpoint.address, tls)
        }
        EndpointKind::Ssh => return Err("SSH hosts are not supported in this version".to_string()),
    }
    .map_err(|err| err.to_string())
}

fn connect_tls(address: &str, tls: &TlsFiles) -> Result<Docker, bollard::errors::Error> {
    Docker::connect_with_ssl(
        address,
        &tls.key,
        &tls.cert,
        &tls.ca,
        CLIENT_TIMEOUT_SECS,
        bollard::API_DEFAULT_VERSION,
    )
}

fn mutation_error(
    result: Result<Result<(), bollard::errors::Error>, tokio::time::error::Elapsed>,
) -> Option<String> {
    match result {
        Ok(Ok(())) => None,
        Ok(Err(err)) => Some(clean_error(&err)),
        Err(_) => Some("timed out after 30s".to_string()),
    }
}

/// Runs one daemon mutation off the actor loop and reports exactly one
/// `ActionDone`, whether the call succeeded, failed or timed out.
fn spawn_mutation<F>(
    resp_tx: Sender<Response>,
    target: Target,
    action: ActionKind,
    epoch: u64,
    call: F,
) where
    F: std::future::Future<Output = Result<(), bollard::errors::Error>> + Send + 'static,
{
    tokio::spawn(async move {
        let error = mutation_error(tokio::time::timeout(Duration::from_secs(30), call).await);
        let _ = resp_tx
            .send(Response::ActionDone {
                target,
                action,
                error,
                epoch,
            })
            .await;
    });
}

async fn send_run_failed(
    resp_tx: &Sender<Response>,
    message: String,
    missing_image: bool,
    created: Option<String>,
    epoch: u64,
) {
    let _ = resp_tx
        .send(Response::RunFailed {
            message,
            missing_image,
            created,
            epoch,
        })
        .await;
}

async fn actor(docker: Docker, cmd_rx: Receiver<Request>, resp_tx: Sender<Response>) {
    let status = match tokio::time::timeout(Duration::from_secs(3), docker.ping()).await {
        Ok(Ok(_)) => HostStatus::Connected,
        Ok(Err(err)) => status_from_error(&err),
        Err(_) => HostStatus::Unreachable("no answer within 3s".to_string()),
    };
    let _ = resp_tx.send(Response::Status(status)).await;

    let mut stats_tasks: HashMap<String, JoinHandle<()>> = HashMap::new();

    while let Ok(req) = cmd_rx.recv().await {
        match req {
            Request::FetchAll => match fetch_all(&docker).await {
                Ok(snapshot) => {
                    let _ = resp_tx.send(Response::Snapshot(snapshot)).await;
                }
                Err(err) => {
                    let _ = resp_tx.send(Response::Status(status_from_error(&err))).await;
                    let _ = resp_tx
                        .send(Response::Failed {
                            what: "fetch".to_string(),
                            message: err.to_string(),
                        })
                        .await;
                }
            },
            Request::ProbeHosts => {
                let statuses = probe_hosts().await;
                let _ = resp_tx.send(Response::HostStatuses(statuses)).await;
            }
            Request::WatchStats { ids } => {
                let desired: HashSet<String> = ids.into_iter().collect();
                stats_tasks.retain(|id, handle| {
                    let keep = desired.contains(id) && !handle.is_finished();
                    if !keep {
                        handle.abort();
                    }
                    keep
                });
                for id in desired {
                    if stats_tasks.contains_key(&id) {
                        continue;
                    }
                    let task = tokio::spawn(stats_task(docker.clone(), id.clone(), resp_tx.clone()));
                    stats_tasks.insert(id, task);
                }
            }
            Request::Inspect { kind, id } => match inspect(&docker, kind, &id).await {
                Ok(json) => {
                    let _ = resp_tx
                        .send(Response::Inspect { kind, id, json })
                        .await;
                }
                Err(err) => {
                    let _ = resp_tx
                        .send(Response::Failed {
                            what: format!("inspect {id}"),
                            message: err.to_string(),
                        })
                        .await;
                }
            },
            Request::Logs { id } => match fetch_logs(&docker, &id).await {
                Ok(lines) => {
                    let _ = resp_tx.send(Response::Logs { id, lines }).await;
                }
                Err(err) => {
                    let _ = resp_tx
                        .send(Response::Failed {
                            what: format!("logs {id}"),
                            message: err.to_string(),
                        })
                        .await;
                }
            },
            Request::ContainerAction { id, action, epoch } => {
                let docker = docker.clone();
                let kind = ActionKind::for_container(action);
                let target = Target {
                    kind: ResourceKind::Container,
                    id: id.clone(),
                };
                let call = async move {
                    match action {
                        ContainerAction::Start => docker.start_container(&id, None).await,
                        ContainerAction::Stop => {
                            docker.stop_container(&id, Some(stop_options())).await
                        }
                        ContainerAction::Restart => docker.restart_container(&id, None).await,
                    }
                };
                spawn_mutation(resp_tx.clone(), target, kind, epoch, call);
            }
            Request::RemoveContainer { id, force, epoch } => {
                let docker = docker.clone();
                let target = Target {
                    kind: ResourceKind::Container,
                    id: id.clone(),
                };
                let call = async move {
                    docker
                        .remove_container(&id, Some(remove_container_options(force)))
                        .await
                };
                spawn_mutation(resp_tx.clone(), target, ActionKind::Remove, epoch, call);
            }
            Request::RemoveImage { id, force, epoch } => {
                let docker = docker.clone();
                let target = Target {
                    kind: ResourceKind::Image,
                    id: id.clone(),
                };
                let call = async move {
                    let options = RemoveImageOptionsBuilder::new()
                        .force(force)
                        .noprune(false)
                        .build();
                    docker.remove_image(&id, Some(options), None).await.map(|_| ())
                };
                spawn_mutation(resp_tx.clone(), target, ActionKind::Remove, epoch, call);
            }
            Request::RemoveVolume { name, epoch } => {
                let docker = docker.clone();
                let target = Target {
                    kind: ResourceKind::Volume,
                    id: name.clone(),
                };
                let call = async move {
                    let options = RemoveVolumeOptionsBuilder::new().force(false).build();
                    docker.remove_volume(&name, Some(options)).await
                };
                spawn_mutation(resp_tx.clone(), target, ActionKind::Remove, epoch, call);
            }
            Request::PullImage { reference, origin, epoch } => {
                let docker = docker.clone();
                let resp_tx = resp_tx.clone();
                tokio::spawn(async move {
                    let mut stream =
                        docker.create_image(Some(pull_options(&reference)), None, None);
                    let error = loop {
                        match tokio::time::timeout(Duration::from_secs(60), stream.next()).await {
                            Err(_) => break Some("pull stalled for 60s".to_string()),
                            Ok(None) => break None,
                            Ok(Some(Err(err))) => break Some(clean_error(&err)),
                            Ok(Some(Ok(info))) => {
                                let detail = info.progress_detail.and_then(|progress| {
                                    match (progress.current, progress.total) {
                                        (Some(current), Some(total)) if total > 0 => {
                                            Some((current as u64, total as u64))
                                        }
                                        _ => None,
                                    }
                                });
                                let _ = resp_tx
                                    .send(Response::PullProgress {
                                        origin,
                                        reference: reference.clone(),
                                        layer: info.id,
                                        status: info.status.unwrap_or_default(),
                                        detail,
                                        epoch,
                                    })
                                    .await;
                            }
                        }
                    };
                    let _ = resp_tx
                        .send(Response::PullDone {
                            origin,
                            reference,
                            error,
                            epoch,
                        })
                        .await;
                });
            }
            Request::RunContainer { spec, epoch } => {
                let docker = docker.clone();
                let resp_tx = resp_tx.clone();
                tokio::spawn(async move {
                    let options = spec.name.as_deref().map(|name| {
                        CreateContainerOptionsBuilder::new().name(name).build()
                    });
                    let body = run_body(&spec);
                    let created = match tokio::time::timeout(
                        Duration::from_secs(30),
                        docker.create_container(options, body),
                    )
                    .await
                    {
                        Ok(Ok(response)) => response,
                        Ok(Err(err)) => {
                            let failure = classify_run_error(&err, None);
                            send_run_failed(
                                &resp_tx,
                                failure.message,
                                failure.missing_image,
                                None,
                                epoch,
                            )
                            .await;
                            return;
                        }
                        Err(_) => {
                            send_run_failed(
                                &resp_tx,
                                "timed out after 30s".to_string(),
                                false,
                                None,
                                epoch,
                            )
                            .await;
                            return;
                        }
                    };

                    let id = created.id;
                    let warnings = created.warnings;
                    match tokio::time::timeout(
                        Duration::from_secs(30),
                        docker.start_container(&id, None),
                    )
                    .await
                    {
                        Ok(Ok(())) => {
                            let _ = resp_tx
                                .send(Response::ContainerCreated { id, warnings, epoch })
                                .await;
                        }
                        Ok(Err(err)) => {
                            let failure = classify_run_error(&err, Some(id.clone()));
                            send_run_failed(
                                &resp_tx,
                                failure.message,
                                failure.missing_image,
                                Some(id),
                                epoch,
                            )
                            .await;
                        }
                        Err(_) => {
                            send_run_failed(
                                &resp_tx,
                                "created but did not start within 30s".to_string(),
                                false,
                                Some(id),
                                epoch,
                            )
                            .await;
                        }
                    }
                });
            }
        }
    }
}

/// One stats stream per watched container. Docker sends a frame per second;
/// only every other usable frame reaches the UI. A broken or stalled stream
/// tells the UI to drop its last reading, then retries with backoff until the
/// actor aborts the task.
async fn stats_task(docker: Docker, id: String, resp_tx: Sender<Response>) {
    let mut backoff_secs: u64 = 1;
    loop {
        let mut stream = Box::pin(docker.stats(&id, Some(stats_options())));
        let mut last_sent: Option<std::time::Instant> = None;
        loop {
            let Some(item) = next_sample(&mut stream, STATS_STALL_TIMEOUT).await else {
                break;
            };
            let Ok(frame) = item else {
                break;
            };
            let Some(stats) = ContainerStats::from_response(&frame) else {
                continue;
            };
            let now = std::time::Instant::now();
            if !stats_sample_due(last_sent, now) {
                continue;
            }
            if resp_tx
                .send(Response::Stats {
                    id: id.clone(),
                    stats,
                })
                .await
                .is_err()
            {
                return;
            }
            last_sent = Some(now);
            backoff_secs = 1;
        }
        if resp_tx
            .send(Response::StatsStale { id: id.clone() })
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(5);
    }
}

async fn fetch_all(docker: &Docker) -> Result<Snapshot, bollard::errors::Error> {
    let containers_options = ListContainersOptionsBuilder::new().all(true).build();
    let images_options = ListImagesOptionsBuilder::new().build();
    let volumes_options = ListVolumesOptionsBuilder::new().build();

    let (containers, images, volumes) = join!(
        docker.list_containers(Some(containers_options)),
        docker.list_images(Some(images_options)),
        docker.list_volumes(Some(volumes_options)),
    );

    let containers_raw = containers?;
    let in_use: HashSet<String> = containers_raw
        .iter()
        .filter_map(|c| c.image_id.clone())
        .collect();

    let now = Utc::now();
    let containers: Vec<ContainerRow> = containers_raw
        .into_iter()
        .map(|c| ContainerRow::from_summary(c, now))
        .collect();
    let images: Vec<ImageRow> = images?
        .into_iter()
        .map(|image| {
            let used = in_use.contains(&image.id);
            ImageRow::from_summary(image, used, now)
        })
        .collect();
    let volumes: Vec<VolumeRow> = volumes?
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|volume| VolumeRow::from_volume(volume, now))
        .collect();

    Ok(Snapshot {
        containers,
        images,
        volumes,
        fetched_at: Some(now),
    })
}

async fn inspect(
    docker: &Docker,
    kind: ResourceKind,
    id: &str,
) -> Result<serde_json::Value, bollard::errors::Error> {
    let value = match kind {
        ResourceKind::Container => {
            serde_json::to_value(docker.inspect_container(id, None).await?)
                .unwrap_or(serde_json::Value::Null)
        }
        ResourceKind::Image => {
            serde_json::to_value(docker.inspect_image(id).await?).unwrap_or(serde_json::Value::Null)
        }
        ResourceKind::Volume => {
            serde_json::to_value(docker.inspect_volume(id).await?).unwrap_or(serde_json::Value::Null)
        }
    };
    Ok(value)
}

async fn fetch_logs(docker: &Docker, id: &str) -> Result<Vec<LogLine>, bollard::errors::Error> {
    let options = LogsOptionsBuilder::new()
        .stdout(true)
        .stderr(true)
        .tail("500")
        .timestamps(true)
        .build();
    let mut stream = docker.logs(id, Some(options));
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(output) => chunks.push(output),
            Err(err) => return Err(err),
        }
    }
    Ok(parse_logs(&chunks))
}

async fn probe_hosts() -> Vec<(String, HostStatus)> {
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_default();
    let discovery = discover(&env, &home);

    let mut futures = Vec::new();
    for host in discovery.hosts {
        futures.push(async move {
            let status = probe_one(&host).await;
            (host.id, status)
        });
    }
    futures_util::future::join_all(futures).await
}

/// One mapping for every liveness check, so the event loop cannot downgrade an
/// auth failure into a generic "not answering".
fn liveness_status(error: &bollard::errors::Error) -> HostStatus {
    status_from_error(error)
}

async fn probe_one(host: &HostInfo) -> HostStatus {
    let Ok(docker) = build_docker(host) else {
        return host.status.clone();
    };
    match tokio::time::timeout(std::time::Duration::from_millis(300), docker.ping()).await {
        Ok(Ok(_)) => HostStatus::Connected,
        Ok(Err(err)) => status_from_error(&err),
        Err(_) => HostStatus::Unreachable("timed out after 300ms".to_string()),
    }
}


/// Subscribes to `/events` and pushes debounced dirty notifications into the UI.
/// The daemon restarting ends the stream; we mark the host unreachable and
/// resubscribe with exponential backoff (1s doubling to a 10s cap).
async fn event_loop(docker: Docker, resp_tx: Sender<Response>) {
    let mut backoff_secs: u64 = 1;

    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(3), docker.ping()).await {
            Ok(Ok(_)) => {
                backoff_secs = 1;
                let _ = resp_tx.try_send(Response::Status(HostStatus::Connected));
            }
            Ok(Err(err)) => {
                let _ = resp_tx.try_send(Response::Status(liveness_status(&err)));
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(10);
                continue;
            }
            Err(_) => {
                let _ = resp_tx.try_send(Response::Status(HostStatus::Unreachable(
                    "no answer within 3s".to_string(),
                )));
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs * 2).min(10);
                continue;
            }
        }

        let mut debouncer = Debouncer::new();
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(100));
        let mut stream = Box::pin(docker.events(None));

        loop {
            tokio::select! {
                item = stream.next() => match item {
                    Some(Ok(event)) => debouncer.record(&event, std::time::Instant::now()),
                    Some(Err(_)) | None => break,
                },
                _ = ticker.tick() => {
                    if let Some(ids) = debouncer.take_if_due(std::time::Instant::now()) {
                        let _ = resp_tx.try_send(Response::Dirty { ids });
                    }
                }
            }
        }

        let _ = resp_tx.try_send(Response::Status(HostStatus::Unreachable(
            "docker event stream ended".to_string(),
        )));
        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_options_keep_anonymous_volumes() {
        let forced = remove_container_options(true);
        assert!(forced.force);
        assert!(
            !forced.v,
            "v(true) would delete anonymous volumes behind the dialog's back"
        );
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

    #[test]
    fn liveness_failure_keeps_auth_failed_and_the_reason() {
        let forbidden = bollard::errors::Error::DockerResponseServerError {
            status_code: 403,
            message: "permission denied while trying to connect".into(),
        };
        match liveness_status(&forbidden) {
            HostStatus::AuthFailed(reason) => {
                assert!(reason.contains("permission denied"), "got {reason}")
            }
            other => panic!("expected AuthFailed, got {other:?}"),
        }
    }

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

    #[test]
    fn next_sample_returns_none_when_the_stream_stalls() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut stream = futures_util::stream::pending::<
            Result<bollard::models::ContainerStatsResponse, bollard::errors::Error>,
        >();
        let sample = runtime.block_on(next_sample(&mut stream, Duration::from_millis(1)));
        assert!(sample.is_none());
    }
}
