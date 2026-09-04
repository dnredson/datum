use std::io::ErrorKind;
use std::process::{Command, Output};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const RUNTIME_PREREQUISITE_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DockerRuntimeState {
    Available,
    InstalledButUnavailable,
    Missing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    AptGet,
    Dnf,
    Yum,
    Pacman,
    Zypper,
}

impl PackageManager {
    pub fn executable(self) -> &'static str {
        match self {
            Self::AptGet => "apt-get",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
        }
    }

    pub fn docker_package(self) -> &'static str {
        match self {
            Self::AptGet => "docker.io",
            Self::Dnf => "moby-engine",
            Self::Yum => "docker",
            Self::Pacman => "docker",
            Self::Zypper => "docker",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DockerRuntimeProbe {
    pub state: DockerRuntimeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePrerequisiteReport {
    pub schema_version: String,
    pub runtime: String,
    pub before: DockerRuntimeProbe,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<PackageManager>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer_program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    pub install_supported: bool,
    pub execute_requested: bool,
    pub installation_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installation_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<DockerRuntimeProbe>,
    pub side_effects_performed: bool,
}

pub fn reconcile_docker_runtime(execute_requested: bool) -> Result<RuntimePrerequisiteReport> {
    let before = probe_docker_runtime()?;
    let package_manager = detect_package_manager();
    let install_supported = cfg!(unix) && package_manager.is_some();
    let installer_program = package_manager.map(|manager| manager.executable().to_string());
    let package_name = package_manager.map(|manager| manager.docker_package().to_string());

    if before.state != DockerRuntimeState::Missing {
        return Ok(RuntimePrerequisiteReport {
            schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
            runtime: "docker".to_string(),
            before,
            package_manager,
            installer_program,
            package_name,
            install_supported,
            execute_requested,
            installation_status: "not_needed".to_string(),
            installation_detail: None,
            after: None,
            side_effects_performed: false,
        });
    }

    if !execute_requested {
        return Ok(RuntimePrerequisiteReport {
            schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
            runtime: "docker".to_string(),
            before,
            package_manager,
            installer_program,
            package_name,
            install_supported,
            execute_requested,
            installation_status: if install_supported {
                "suggested".to_string()
            } else {
                "unsupported".to_string()
            },
            installation_detail: if install_supported {
                Some("Docker is missing; typed package-manager installation is available but --execute was not provided".to_string())
            } else {
                Some("Docker is missing and no supported package manager was detected".to_string())
            },
            after: None,
            side_effects_performed: false,
        });
    }

    let Some(manager) = package_manager else {
        return Ok(RuntimePrerequisiteReport {
            schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
            runtime: "docker".to_string(),
            before,
            package_manager: None,
            installer_program: None,
            package_name: None,
            install_supported: false,
            execute_requested,
            installation_status: "unsupported".to_string(),
            installation_detail: Some(
                "Docker is missing and no supported package manager was detected".to_string(),
            ),
            after: None,
            side_effects_performed: false,
        });
    };

    if !has_noninteractive_privilege()? {
        return Ok(RuntimePrerequisiteReport {
            schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
            runtime: "docker".to_string(),
            before,
            package_manager: Some(manager),
            installer_program: Some(manager.executable().to_string()),
            package_name: Some(manager.docker_package().to_string()),
            install_supported: true,
            execute_requested,
            installation_status: "privilege_required".to_string(),
            installation_detail: Some(
                "Docker installation requires root or passwordless sudo; SmartSentinel will not prompt for a password"
                    .to_string(),
            ),
            after: None,
            side_effects_performed: false,
        });
    }

    let mut side_effects_performed = false;
    if manager == PackageManager::AptGet {
        let update = run_privileged(manager.executable(), &["update"])
            .context("run apt-get update for Docker prerequisite")?;
        if !update.status.success() {
            return Ok(failed_install_report(
                before,
                manager,
                execute_requested,
                side_effects_performed,
                format!("apt-get update failed: {}", output_detail(&update)),
            ));
        }
        side_effects_performed = true;
    }

    let install_args = installation_args(manager);
    let install = run_privileged(manager.executable(), &install_args)
        .with_context(|| format!("install Docker using {}", manager.executable()))?;
    if !install.status.success() {
        return Ok(failed_install_report(
            before,
            manager,
            execute_requested,
            side_effects_performed,
            format!(
                "{} Docker package installation failed: {}",
                manager.executable(),
                output_detail(&install)
            ),
        ));
    }
    side_effects_performed = true;

    if command_exists("systemctl") {
        let start = run_privileged("systemctl", &["enable", "--now", "docker"])
            .context("enable/start Docker service")?;
        if !start.status.success() {
            let after = probe_docker_runtime()?;
            return Ok(RuntimePrerequisiteReport {
                schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
                runtime: "docker".to_string(),
                before,
                package_manager: Some(manager),
                installer_program: Some(manager.executable().to_string()),
                package_name: Some(manager.docker_package().to_string()),
                install_supported: true,
                execute_requested,
                installation_status: "installed_service_unavailable".to_string(),
                installation_detail: Some(format!(
                    "Docker package installed but systemctl enable --now docker failed: {}",
                    output_detail(&start)
                )),
                after: Some(after),
                side_effects_performed,
            });
        }
    }

    let after = probe_docker_runtime()?;
    let (installation_status, installation_detail) = match after.state {
        DockerRuntimeState::Available => (
            "installed_available".to_string(),
            Some("Docker package installed and daemon is available to the current SmartSentinel user".to_string()),
        ),
        DockerRuntimeState::InstalledButUnavailable => (
            "installed_but_unavailable".to_string(),
            Some(
                "Docker package installed, but the current SmartSentinel user cannot use the daemon yet; service or socket permissions may require separate remediation"
                    .to_string(),
            ),
        ),
        DockerRuntimeState::Missing => (
            "failed".to_string(),
            Some("Docker package installation completed but Docker CLI is still missing".to_string()),
        ),
    };

    Ok(RuntimePrerequisiteReport {
        schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
        runtime: "docker".to_string(),
        before,
        package_manager: Some(manager),
        installer_program: Some(manager.executable().to_string()),
        package_name: Some(manager.docker_package().to_string()),
        install_supported: true,
        execute_requested,
        installation_status,
        installation_detail,
        after: Some(after),
        side_effects_performed,
    })
}

pub fn probe_docker_runtime() -> Result<DockerRuntimeProbe> {
    let version_output = match Command::new("docker").arg("--version").output() {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(DockerRuntimeProbe {
                state: DockerRuntimeState::Missing,
                cli_version: None,
                detail: Some("docker executable not found in PATH".to_string()),
            });
        }
        Err(error) => {
            return Ok(DockerRuntimeProbe {
                state: DockerRuntimeState::InstalledButUnavailable,
                cli_version: None,
                detail: Some(format!("failed to execute docker --version: {error}")),
            });
        }
    };

    let cli_version = first_nonempty_line(&version_output.stdout)
        .or_else(|| first_nonempty_line(&version_output.stderr));
    if !version_output.status.success() {
        return Ok(DockerRuntimeProbe {
            state: DockerRuntimeState::InstalledButUnavailable,
            cli_version,
            detail: Some(format!(
                "docker executable exists but docker --version failed: {}",
                output_detail(&version_output)
            )),
        });
    }

    let info = Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .context("execute docker info")?;
    if info.status.success() {
        return Ok(DockerRuntimeProbe {
            state: DockerRuntimeState::Available,
            cli_version,
            detail: first_nonempty_line(&info.stdout)
                .map(|server| format!("docker daemon available; server_version={server}")),
        });
    }

    Ok(DockerRuntimeProbe {
        state: DockerRuntimeState::InstalledButUnavailable,
        cli_version,
        detail: Some(format!(
            "docker CLI is installed but daemon access failed: {}",
            output_detail(&info)
        )),
    })
}

pub fn detect_package_manager() -> Option<PackageManager> {
    [
        PackageManager::AptGet,
        PackageManager::Dnf,
        PackageManager::Yum,
        PackageManager::Pacman,
        PackageManager::Zypper,
    ]
    .into_iter()
    .find(|manager| command_exists(manager.executable()))
}

pub fn installation_args(manager: PackageManager) -> Vec<&'static str> {
    match manager {
        PackageManager::AptGet => vec!["install", "-y", manager.docker_package()],
        PackageManager::Dnf | PackageManager::Yum => {
            vec!["install", "-y", manager.docker_package()]
        }
        PackageManager::Pacman => vec!["-S", "--noconfirm", manager.docker_package()],
        PackageManager::Zypper => {
            vec!["--non-interactive", "install", manager.docker_package()]
        }
    }
}

fn failed_install_report(
    before: DockerRuntimeProbe,
    manager: PackageManager,
    execute_requested: bool,
    side_effects_performed: bool,
    detail: String,
) -> RuntimePrerequisiteReport {
    RuntimePrerequisiteReport {
        schema_version: RUNTIME_PREREQUISITE_SCHEMA_VERSION.to_string(),
        runtime: "docker".to_string(),
        before,
        package_manager: Some(manager),
        installer_program: Some(manager.executable().to_string()),
        package_name: Some(manager.docker_package().to_string()),
        install_supported: true,
        execute_requested,
        installation_status: "failed".to_string(),
        installation_detail: Some(detail),
        after: None,
        side_effects_performed,
    }
}

fn command_exists(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .map(|_| true)
        .unwrap_or(false)
}

fn has_noninteractive_privilege() -> Result<bool> {
    if current_uid()? == Some(0) {
        return Ok(true);
    }

    let status = match Command::new("sudo").args(["-n", "true"]).status() {
        Ok(status) => status,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("probe passwordless sudo"),
    };
    Ok(status.success())
}

fn current_uid() -> Result<Option<u32>> {
    if !cfg!(unix) {
        return Ok(None);
    }
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("execute id -u")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok())
}

fn run_privileged(program: &str, args: &[&str]) -> Result<Output> {
    if current_uid()? == Some(0) {
        return Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("execute privileged program {program}"));
    }

    let mut sudo_args = vec!["-n", program];
    sudo_args.extend_from_slice(args);
    Command::new("sudo")
        .args(&sudo_args)
        .output()
        .with_context(|| format!("execute privileged program {program} through sudo -n"))
}

fn output_detail(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout={stdout}; stderr={stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => format!("exit_status={}", output.status),
    }
}

fn first_nonempty_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manager_install_specs_are_typed() {
        assert_eq!(PackageManager::AptGet.docker_package(), "docker.io");
        assert_eq!(
            installation_args(PackageManager::AptGet),
            vec!["install", "-y", "docker.io"]
        );
        assert_eq!(
            installation_args(PackageManager::Pacman),
            vec!["-S", "--noconfirm", "docker"]
        );
        assert_eq!(PackageManager::Dnf.docker_package(), "moby-engine");
    }

    #[test]
    fn package_manager_names_are_stable() {
        assert_eq!(PackageManager::AptGet.executable(), "apt-get");
        assert_eq!(PackageManager::Dnf.executable(), "dnf");
        assert_eq!(PackageManager::Yum.executable(), "yum");
        assert_eq!(PackageManager::Pacman.executable(), "pacman");
        assert_eq!(PackageManager::Zypper.executable(), "zypper");
    }
}
