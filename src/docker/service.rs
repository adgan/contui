use crate::docker::endpoint::{
    discover, EndpointKind, HostInfo, HostStatus, TlsFiles,
};
use crate::docker::events::Debouncer;
use crate::docker::model::{
    parse_logs, status_from_error, ContainerRow, ImageRow, LogLine, ResourceKind, Snapshot,
    VolumeRow,
};
use async_channel::{Receiver, Sender};
use bollard::Docker;
use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, ListVolumesOptionsBuilder,
    LogsOptionsBuilder,
};
use chrono::Utc;
use futures_util::{join, StreamExt};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::runtime::Runtime;

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
        kinds: Vec<ResourceKind>,
        ids: Vec<(ResourceKind, String)>,
    },
    Failed {
        what: String,
        message: String,
    },
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

    pub fn shutdown(mut self) {
        self.runtime.take();
    }
}

impl Drop for DockerService {
    fn drop(&mut self) {
        self.runtime.take();
    }
}

fn build_docker(host: &HostInfo) -> Result<Docker, String> {
    let version = bollard::API_DEFAULT_VERSION;
    match host.endpoint.kind {
        EndpointKind::Unix => Docker::connect_with_unix(&host.endpoint.address, 5, version),
        EndpointKind::Http => Docker::connect_with_http(&host.endpoint.address, 5, version),
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
        5,
        bollard::API_DEFAULT_VERSION,
    )
}

async fn actor(docker: Docker, cmd_rx: Receiver<Request>, resp_tx: Sender<Response>) {
    let status = match docker.ping().await {
        Ok(_) => HostStatus::Connected,
        Err(err) => status_from_error(&err),
    };
    let _ = resp_tx.send(Response::Status(status)).await;

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
        }
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
        let healthy = matches!(
            tokio::time::timeout(std::time::Duration::from_secs(3), docker.ping()).await,
            Ok(Ok(_))
        );
        if healthy {
            backoff_secs = 1;
            let _ = resp_tx.try_send(Response::Status(HostStatus::Connected));
        } else {
            let _ = resp_tx.try_send(Response::Status(HostStatus::Unreachable(
                "docker engine is not answering".to_string(),
            )));
            tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(10);
            continue;
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
                    if let Some((kinds, ids)) = debouncer.take_if_due(std::time::Instant::now()) {
                        let _ = resp_tx.try_send(Response::Dirty { kinds, ids });
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
