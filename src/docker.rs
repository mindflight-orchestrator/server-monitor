//! Docker container inspection via Bollard.

use bollard::container::{InspectContainerOptions, ListContainersOptions};
use bollard::Docker;
use std::path::Path;
use tracing::{debug, warn};

/// Container status from Docker.
#[derive(Debug, Clone, PartialEq)]
pub enum ContainerStatus {
    Running,
    Exited,
    Restarting,
    Paused,
    Dead,
    Other(String),
}

/// Health status for containers with healthcheck (e.g. Postgres).
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Starting,
    None, // No healthcheck defined
}

/// Container info for a monitored container.
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub status: ContainerStatus,
    pub health: HealthStatus,
}

impl ContainerInfo {
    fn from_docker_name(name: &str) -> String {
        name.trim_start_matches('/').to_string()
    }
}

/// Docker client wrapper.
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    pub fn connect(socket_path: &Path) -> Result<Self, bollard::errors::Error> {
        let path = socket_path.to_str().unwrap_or("/var/run/docker.sock");
        let docker = Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION)?;
        Ok(Self { docker })
    }

    /// List all running containers (and some non-running) and filter by names.
    pub async fn inspect_containers(
        &self,
        names: &[&str],
    ) -> Result<Vec<ContainerInfo>, bollard::errors::Error> {
        let mut result = Vec::with_capacity(names.len());

        for name in names {
            let inspect = self
                .docker
                .inspect_container(name, None::<InspectContainerOptions>)
                .await;

            match inspect {
                Ok(info) => {
                    let state = info.state.as_ref();
                    let status_str = state
                        .and_then(|s| s.status.as_ref())
                        .map(|s| format!("{:?}", s).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    let container_status = match status_str.as_str() {
                        "running" => ContainerStatus::Running,
                        "exited" => ContainerStatus::Exited,
                        "restarting" => ContainerStatus::Restarting,
                        "paused" => ContainerStatus::Paused,
                        "dead" => ContainerStatus::Dead,
                        s => ContainerStatus::Other(s.to_string()),
                    };

                    let health = state
                        .and_then(|s| s.health.as_ref())
                        .and_then(|h| h.status.as_ref())
                        .map(|s| {
                            let s = format!("{:?}", s).to_lowercase();
                            match s.as_str() {
                                "healthy" => HealthStatus::Healthy,
                                "unhealthy" => HealthStatus::Unhealthy,
                                "starting" => HealthStatus::Starting,
                                _ => HealthStatus::None,
                            }
                        })
                        .unwrap_or(HealthStatus::None);

                    result.push(ContainerInfo {
                        name: Self::docker_name_to_display(name),
                        status: container_status,
                        health,
                    });
                }
                Err(e) => {
                    warn!(container = %name, error = %e, "failed to inspect container");
                    result.push(ContainerInfo {
                        name: Self::docker_name_to_display(name),
                        status: ContainerStatus::Other("inspect_failed".to_string()),
                        health: HealthStatus::None,
                    });
                }
            }
        }

        Ok(result)
    }

    fn docker_name_to_display(name: &str) -> String {
        name.trim_start_matches('/').to_string()
    }

    /// List all containers matching a name prefix (e.g. "closlamartine", "clsmstaging").
    pub async fn list_container_names(&self) -> Result<Vec<String>, bollard::errors::Error> {
        let opts = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };
        let containers = self.docker.list_containers(Some(opts)).await?;
        let names: Vec<String> = containers
            .into_iter()
            .flat_map(|c| {
                c.names
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| ContainerInfo::from_docker_name(s.as_str()))
            })
            .collect();
        debug!(count = names.len(), "listed containers");
        Ok(names)
    }
}
