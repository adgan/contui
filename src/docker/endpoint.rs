use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointKind {
    Unix,
    Http,
    Tls,
    Ssh,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub kind: EndpointKind,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsFiles {
    pub key: PathBuf,
    pub cert: PathBuf,
    pub ca: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostStatus {
    Unknown,
    Connected,
    Unreachable(String),
    AuthFailed(String),
    Unsupported(String),
}

#[derive(Clone, Debug)]
pub struct HostInfo {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub endpoint: Endpoint,
    pub tls: Option<TlsFiles>,
    pub status: HostStatus,
}

#[derive(Clone, Debug, Default)]
pub struct Discovery {
    pub hosts: Vec<HostInfo>,
    pub active: Option<String>,
}

#[derive(Deserialize)]
struct ContextMeta {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Endpoints")]
    endpoints: ContextEndpoints,
    #[serde(rename = "TLSMaterial", default)]
    tls_material: HashMap<String, Vec<String>>,
    #[serde(rename = "Storage", default)]
    storage: Option<ContextStorage>,
}

#[derive(Deserialize)]
struct ContextEndpoints {
    #[serde(rename = "docker")]
    docker: Option<ContextDockerEndpoint>,
}

#[derive(Deserialize)]
struct ContextDockerEndpoint {
    #[serde(rename = "Host")]
    host: String,
}

#[derive(Deserialize)]
struct ContextStorage {
    #[serde(rename = "TLSPath", default)]
    tls_path: Option<String>,
}

#[derive(Deserialize)]
struct DockerConfig {
    #[serde(rename = "currentContext")]
    current_context: Option<String>,
}

fn strip_scheme(raw: &str) -> &str {
    raw.split_once("://").map(|(_, rest)| rest).unwrap_or(raw)
}

pub fn parse_host(raw: &str) -> Option<Endpoint> {
    let raw = raw.trim();
    if let Some(path) = raw.strip_prefix("unix://") {
        return Some(Endpoint {
            kind: EndpointKind::Unix,
            address: path.to_string(),
        });
    }
    if raw.starts_with("tcp://") || raw.starts_with("http://") || raw.starts_with("https://") {
        return Some(Endpoint {
            kind: EndpointKind::Http,
            address: raw.to_string(),
        });
    }
    if raw.starts_with("ssh://") {
        return Some(Endpoint {
            kind: EndpointKind::Ssh,
            address: raw.to_string(),
        });
    }
    None
}

pub fn candidate_sockets(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".rd/docker.sock"),
        home.join(".docker/run/docker.sock"),
        home.join(".colima/default/docker.sock"),
        home.join(".orbstack/run/docker.sock"),
        PathBuf::from("/var/run/docker.sock"),
    ]
}

fn socket_app_name(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.contains("/.rd/") {
        "Rancher Desktop".to_string()
    } else if s.contains("/.docker/run/") {
        "Docker Desktop".to_string()
    } else if s.contains("/.colima/") {
        "Colima".to_string()
    } else if s.contains("/.orbstack/") {
        "OrbStack".to_string()
    } else {
        "Docker".to_string()
    }
}

fn status_for(endpoint: &Endpoint) -> HostStatus {
    match endpoint.kind {
        EndpointKind::Ssh => HostStatus::Unsupported(
            "SSH hosts are listed but not connected in this version".to_string(),
        ),
        _ => HostStatus::Unknown,
    }
}

/// `DOCKER_CERT_PATH` is how the CLI is pointed at client certificates. When
/// all three files are present, a tcp host is spoken to over TLS.
fn apply_env_tls(
    endpoint: Endpoint,
    env: &HashMap<String, String>,
) -> (Endpoint, Option<TlsFiles>) {
    let Some(cert_path) = env.get("DOCKER_CERT_PATH").filter(|p| !p.is_empty()) else {
        return (endpoint, None);
    };
    if endpoint.kind != EndpointKind::Http {
        return (endpoint, None);
    }
    let dir = PathBuf::from(cert_path);
    let ca = dir.join("ca.pem");
    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");
    if !(ca.exists() && cert.exists() && key.exists()) {
        return (endpoint, None);
    }
    (
        Endpoint {
            kind: EndpointKind::Tls,
            address: endpoint.address,
        },
        Some(TlsFiles { key, cert, ca }),
    )
}

fn read_contexts(home: &Path) -> Vec<HostInfo> {
    let mut out = Vec::new();
    let meta_root = home.join(".docker/contexts/meta");
    let Ok(entries) = std::fs::read_dir(&meta_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path().join("meta.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<ContextMeta>(&text) else {
            continue;
        };
        let Some(docker) = meta.endpoints.docker else {
            continue;
        };
        let Some(endpoint) = parse_host(&docker.host) else {
            continue;
        };

        let tls = if meta.tls_material.contains_key("docker") {
            meta.storage
                .as_ref()
                .and_then(|s| s.tls_path.clone())
                .map(|dir| {
                    let dir = PathBuf::from(dir);
                    TlsFiles {
                        key: dir.join("key.pem"),
                        cert: dir.join("cert.pem"),
                        ca: dir.join("ca.pem"),
                    }
                })
        } else {
            None
        };

        let endpoint = if tls.is_some() && endpoint.kind == EndpointKind::Http {
            Endpoint {
                kind: EndpointKind::Tls,
                address: endpoint.address,
            }
        } else {
            endpoint
        };

        out.push(HostInfo {
            id: format!("context:{}", meta.name),
            name: meta.name,
            detail: docker.host,
            status: status_for(&endpoint),
            endpoint,
            tls,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn config_current_context(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(".docker/config.json")).ok()?;
    serde_json::from_str::<DockerConfig>(&text)
        .ok()?
        .current_context
}

pub fn discover(env: &HashMap<String, String>, home: &Path) -> Discovery {
    let mut hosts = read_contexts(home);

    if let Some(raw) = env.get("DOCKER_HOST").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if let Some(endpoint) = parse_host(raw) {
            let id = format!("env:{}", strip_scheme(raw));
            let detail = raw.to_string();
            let name = socket_app_name(Path::new(strip_scheme(raw)));
            let (endpoint, tls) = apply_env_tls(endpoint, env);
            hosts.insert(
                0,
                HostInfo {
                    id,
                    name,
                    detail,
                    status: status_for(&endpoint),
                    endpoint,
                    tls,
                },
            );
        }
    }

    for socket in candidate_sockets(home) {
        let address = socket.to_string_lossy().to_string();
        if !socket.exists() {
            continue;
        }
        let already = hosts
            .iter()
            .any(|h| h.endpoint.address == address);
        if already {
            continue;
        }
        hosts.push(HostInfo {
            id: format!("socket:{address}"),
            name: socket_app_name(&socket),
            detail: format!("unix://{address}"),
            status: HostStatus::Unknown,
            endpoint: Endpoint {
                kind: EndpointKind::Unix,
                address,
            },
            tls: None,
        });
    }

    let active = resolve_active(env, home, &hosts);
    Discovery { hosts, active }
}

fn resolve_active(env: &HashMap<String, String>, home: &Path, hosts: &[HostInfo]) -> Option<String> {
    if let Some(raw) = env.get("DOCKER_HOST").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let id = format!("env:{}", strip_scheme(raw));
        if hosts.iter().any(|h| h.id == id) {
            return Some(id);
        }
    }

    let from_env = env.get("DOCKER_CONTEXT").cloned();
    let from_config = config_current_context(home);
    for name in [from_env, from_config].into_iter().flatten() {
        let id = format!("context:{name}");
        if hosts.iter().any(|h| h.id == id) {
            return Some(id);
        }
    }

    if let Some(host) = hosts.iter().find(|h| h.id.starts_with("socket:")) {
        return Some(host.id.clone());
    }

    hosts
        .iter()
        .find(|h| h.id.starts_with("context:"))
        .or_else(|| hosts.first())
        .map(|h| h.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::path::Path;

    fn write_context(home: &Path, name: &str, host: &str) {
        let digest = hex(&Sha256::digest(name.as_bytes()));
        let meta = home.join(".docker/contexts/meta").join(digest);
        std::fs::create_dir_all(&meta).unwrap();
        let json = format!(
            r#"{{"Name":"{name}","Metadata":{{}},"Endpoints":{{"docker":{{"Host":"{host}","SkipTLSVerify":false}}}},"TLSMaterial":{{}},"Storage":{{"MetadataPath":"x","TLSPath":""}}}}"#
        );
        std::fs::write(meta.join("meta.json"), json).unwrap();
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn write_tls_context(home: &Path, name: &str, host: &str, tls_dir: &Path) {
        let digest = hex(&Sha256::digest(name.as_bytes()));
        let meta = home.join(".docker/contexts/meta").join(digest);
        std::fs::create_dir_all(&meta).unwrap();
        let json = format!(
            r#"{{"Name":"{name}","Metadata":{{}},"Endpoints":{{"docker":{{"Host":"{host}","SkipTLSVerify":false}}}},"TLSMaterial":{{"docker":["ca.pem","cert.pem","key.pem"]}},"Storage":{{"MetadataPath":"x","TLSPath":"{}"}}}}"#,
            tls_dir.to_string_lossy()
        );
        std::fs::write(meta.join("meta.json"), json).unwrap();
    }

    #[test]
    fn tls_context_produces_tls_endpoint_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let tls_dir = dir.path().join("tls");
        std::fs::create_dir_all(&tls_dir).unwrap();
        write_tls_context(dir.path(), "remote", "tcp://10.0.0.5:2376", &tls_dir);

        let d = discover(&HashMap::new(), dir.path());
        assert_eq!(d.hosts.len(), 1);
        let host = &d.hosts[0];
        assert_eq!(host.endpoint.kind, EndpointKind::Tls);
        let tls = host.tls.as_ref().expect("tls files");
        assert_eq!(tls.ca, tls_dir.join("ca.pem"));
        assert_eq!(tls.cert, tls_dir.join("cert.pem"));
        assert_eq!(tls.key, tls_dir.join("key.pem"));
    }

    #[test]
    fn docker_host_env_wins_and_sets_active() {
        let env = HashMap::from([("DOCKER_HOST".to_string(), "unix:///tmp/x.sock".to_string())]);
        let d = discover(&env, Path::new("/nonexistent"));
        assert_eq!(d.active.as_deref(), Some("env:/tmp/x.sock"));
        assert_eq!(d.hosts[0].endpoint.kind, EndpointKind::Unix);
    }

    #[test]
    fn docker_context_env_beats_config_current_context() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_context(home, "rancher-desktop", "unix:///Users/me/.rd/docker.sock");
        write_context(home, "default", "unix:///var/run/docker.sock");
        std::fs::create_dir_all(home.join(".docker")).unwrap();
        std::fs::write(
            home.join(".docker/config.json"),
            r#"{"currentContext":"default"}"#,
        )
        .unwrap();
        let env = HashMap::from([("DOCKER_CONTEXT".to_string(), "rancher-desktop".to_string())]);
        let d = discover(&env, home);
        assert_eq!(d.active.as_deref(), Some("context:rancher-desktop"));
    }

    #[test]
    fn every_context_is_listed_with_its_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        write_context(
            dir.path(),
            "rancher-desktop",
            "unix:///Users/me/.rd/docker.sock",
        );
        write_context(dir.path(), "remote", "tcp://10.0.0.5:2376");
        let d = discover(&HashMap::new(), dir.path());
        assert_eq!(d.hosts.len(), 2);
        assert!(
            d.hosts
                .iter()
                .any(|h| h.name == "remote" && h.endpoint.kind == EndpointKind::Http)
        );
    }

    #[test]
    fn malformed_context_meta_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join(".docker/contexts/meta/deadbeef");
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(meta.join("meta.json"), "{\"Name\":\"broken\"").unwrap();
        write_context(
            dir.path(),
            "rancher-desktop",
            "unix:///Users/me/.rd/docker.sock",
        );
        let d = discover(&HashMap::new(), dir.path());
        assert_eq!(d.hosts.len(), 1);
        assert_eq!(d.hosts[0].name, "rancher-desktop");
    }

    #[test]
    fn docker_cert_path_turns_a_tcp_host_into_a_tls_host() {
        let dir = tempfile::tempdir().unwrap();
        let certs = dir.path().join("certs");
        std::fs::create_dir_all(&certs).unwrap();
        for name in ["ca.pem", "cert.pem", "key.pem"] {
            std::fs::write(certs.join(name), "x").unwrap();
        }
        let env = HashMap::from([
            (
                "DOCKER_HOST".to_string(),
                "tcp://10.0.0.5:2376".to_string(),
            ),
            (
                "DOCKER_CERT_PATH".to_string(),
                certs.to_string_lossy().to_string(),
            ),
        ]);
        let d = discover(&env, Path::new("/nonexistent"));
        assert_eq!(d.hosts[0].endpoint.kind, EndpointKind::Tls);
        let tls = d.hosts[0].tls.as_ref().expect("tls files");
        assert_eq!(tls.ca, certs.join("ca.pem"));
    }

    #[test]
    fn ssh_endpoint_is_listed_as_unsupported() {
        let env = HashMap::from([("DOCKER_HOST".to_string(), "ssh://me@host".to_string())]);
        let d = discover(&env, Path::new("/nonexistent"));
        assert!(matches!(d.hosts[0].status, HostStatus::Unsupported(_)));
    }

    #[test]
    fn candidate_sockets_follow_spec_order() {
        let paths = candidate_sockets(Path::new("/Users/me"));
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        assert_eq!(names[0], "/Users/me/.rd/docker.sock");
        assert_eq!(names.last().unwrap(), "/var/run/docker.sock");
    }

    #[test]
    fn parses_endpoint_schemes() {
        assert_eq!(
            parse_host("unix:///tmp/d.sock").unwrap().kind,
            EndpointKind::Unix
        );
        assert_eq!(
            parse_host("tcp://10.0.0.5:2376").unwrap().kind,
            EndpointKind::Http
        );
        assert_eq!(parse_host("ssh://me@host").unwrap().kind, EndpointKind::Ssh);
        assert!(parse_host("garbage").is_none());
    }
}
