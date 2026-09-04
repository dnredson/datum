use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::runtime_prerequisite::{
    detect_package_manager, probe_docker_runtime, DockerRuntimeProbe, DockerRuntimeState,
    PackageManager,
};

pub const RUNTIME_INSTALL_ARTIFACT_SCHEMA_VERSION: &str = "0.1.0";
const SMARTSENTINEL_USER_PLACEHOLDER: &str = "${SMARTSENTINEL_USER}";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeInstallCommand {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub privileged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeInstallerLifecycle {
    #[serde(default)]
    pub pre_install_commands: Vec<RuntimeInstallCommand>,
    pub install_command: RuntimeInstallCommand,
    #[serde(default)]
    pub post_install_commands: Vec<RuntimeInstallCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeInstallArtifact {
    pub schema_version: String,
    pub artifact_id: String,
    pub artifact_type: String,
    pub runtime: String,
    pub execution_user: String,
    pub installers: BTreeMap<String, RuntimeInstallerLifecycle>,
    #[serde(default)]
    pub notes: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeInstallCommandResult {
    pub phase: String,
    pub ordinal: usize,
    pub program: String,
    pub args: Vec<String>,
    pub privileged: bool,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeArtifactReconciliationReport {
    pub schema_version: String,
    pub artifact_id: String,
    pub runtime: String,
    pub execution_user: String,
    pub before: DockerRuntimeProbe,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<PackageManager>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_installer: Option<String>,
    pub execute_requested: bool,
    pub execution_authorized: bool,
    pub status: String,
    #[serde(default)]
    pub command_results: Vec<RuntimeInstallCommandResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<DockerRuntimeProbe>,
    pub side_effects_performed: bool,
    pub session_refresh_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub fn load_runtime_install_artifact(path: &Path) -> Result<RuntimeInstallArtifact> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read runtime installation artifact {}", path.display()))?;
    let artifact: RuntimeInstallArtifact = serde_json::from_str(&raw)
        .with_context(|| format!("parse runtime installation artifact {}", path.display()))?;
    validate_runtime_install_artifact(&artifact)?;
    Ok(artifact)
}

pub fn validate_runtime_install_artifact(artifact: &RuntimeInstallArtifact) -> Result<()> {
    if artifact.schema_version.trim() != RUNTIME_INSTALL_ARTIFACT_SCHEMA_VERSION {
        bail!(
            "runtime installation artifact schema_version must be {}",
            RUNTIME_INSTALL_ARTIFACT_SCHEMA_VERSION
        );
    }
    required("artifact_id", &artifact.artifact_id)?;
    if artifact.artifact_type.trim() != "runtime_installation" {
        bail!("runtime installation artifact_type must be runtime_installation");
    }
    if !artifact.runtime.trim().eq_ignore_ascii_case("docker") {
        bail!(
            "runtime installation artifact '{}' uses unsupported runtime '{}'",
            artifact.artifact_id,
            artifact.runtime
        );
    }
    required("execution_user", &artifact.execution_user)?;
    if artifact.installers.is_empty() {
        bail!("runtime installation artifact must contain at least one installer");
    }
    for (name, installer) in &artifact.installers {
        required("installer key", name)?;
        for command in &installer.pre_install_commands {
            validate_command(command)?;
        }
        validate_command(&installer.install_command)?;
        for command in &installer.post_install_commands {
            validate_command(command)?;
        }
    }
    Ok(())
}

pub fn reconcile_runtime_from_artifact(
    artifact: &RuntimeInstallArtifact,
    execution_user_override: Option<&str>,
    execute_requested: bool,
    execution_authorized: bool,
) -> Result<RuntimeArtifactReconciliationReport> {
    validate_runtime_install_artifact(artifact)?;
    let before = probe_docker_runtime()?;
    let execution_user = resolve_execution_user(artifact, execution_user_override)?;
    let package_manager = detect_package_manager();
    let selected_installer = package_manager.map(package_manager_key).map(str::to_string);

    match before.state {
        DockerRuntimeState::Available => Ok(base_report(
            artifact,
            execution_user,
            before,
            package_manager,
            selected_installer,
            execute_requested,
            execution_authorized,
            "available",
            Some("Docker runtime is already available to SmartSentinel".to_string()),
        )),
        DockerRuntimeState::InstalledButUnavailable => reconcile_installed_but_unavailable(
            artifact,
            execution_user,
            before,
            package_manager,
            execute_requested,
            execution_authorized,
        ),
        DockerRuntimeState::Missing => reconcile_missing_runtime(
            artifact,
            execution_user,
            before,
            package_manager,
            execute_requested,
            execution_authorized,
        ),
    }
}

fn reconcile_installed_but_unavailable(
    artifact: &RuntimeInstallArtifact,
    execution_user: String,
    before: DockerRuntimeProbe,
    package_manager: Option<PackageManager>,
    execute_requested: bool,
    execution_authorized: bool,
) -> Result<RuntimeArtifactReconciliationReport> {
    if !execute_requested {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            package_manager,
            package_manager.map(package_manager_key).map(str::to_string),
            execute_requested,
            execution_authorized,
            "remediation_suggested",
            Some("Docker is installed but unavailable; trusted post-install remediation is available but execution was not requested".to_string()),
        ));
    }
    if !execution_authorized {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            package_manager,
            package_manager.map(package_manager_key).map(str::to_string),
            execute_requested,
            execution_authorized,
            "blocked_governance",
            Some("Docker is installed but unavailable, and post-install remediation is not authorized by the active D-Graph revision and local policy".to_string()),
        ));
    }

    let Some(manager) = package_manager else {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            None,
            None,
            execute_requested,
            execution_authorized,
            "unsupported_remediation",
            Some("Docker is installed but unavailable and no supported package manager was detected to select trusted post-install commands".to_string()),
        ));
    };
    let installer_key = package_manager_key(manager);
    let Some(installer) = artifact.installers.get(installer_key) else {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            Some(manager),
            Some(installer_key.to_string()),
            execute_requested,
            execution_authorized,
            "missing_installer",
            Some(format!(
                "runtime artifact does not define installer '{}' for detected package manager",
                installer_key
            )),
        ));
    };
    if installer.post_install_commands.is_empty() {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            Some(manager),
            Some(installer_key.to_string()),
            execute_requested,
            execution_authorized,
            "no_post_install_remediation",
            Some("Docker is installed but unavailable and the selected runtime artifact defines no post-install remediation commands".to_string()),
        ));
    }
    if commands_require_privilege(&installer.post_install_commands) && !has_noninteractive_privilege()? {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            Some(manager),
            Some(installer_key.to_string()),
            execute_requested,
            execution_authorized,
            "privilege_required",
            Some("trusted post-install remediation requires root or passwordless sudo; SmartSentinel will not prompt for a password".to_string()),
        ));
    }

    let mut report = base_report(
        artifact,
        execution_user.clone(),
        before,
        Some(manager),
        Some(installer_key.to_string()),
        execute_requested,
        execution_authorized,
        "remediating",
        None,
    );
    if !run_command_phase(
        &mut report,
        "post_install_remediation",
        &installer.post_install_commands,
        &execution_user,
    )? {
        report.after = Some(probe_docker_runtime()?);
        return Ok(report);
    }

    let after = probe_docker_runtime()?;
    report.status = match after.state {
        DockerRuntimeState::Available => "remediated_available",
        DockerRuntimeState::InstalledButUnavailable if report.session_refresh_required => {
            "remediation_requires_session_refresh"
        }
        DockerRuntimeState::InstalledButUnavailable => "remediation_still_unavailable",
        DockerRuntimeState::Missing => "failed",
    }
    .to_string();
    report.detail = Some(match after.state {
        DockerRuntimeState::Available => {
            "trusted post-install remediation completed and Docker is available to SmartSentinel".to_string()
        }
        DockerRuntimeState::InstalledButUnavailable if report.session_refresh_required => {
            "trusted post-install remediation completed; Docker group membership changed and the SmartSentinel session/service must be refreshed before daemon access can change".to_string()
        }
        DockerRuntimeState::InstalledButUnavailable => {
            "trusted post-install remediation completed but Docker daemon access remains unavailable".to_string()
        }
        DockerRuntimeState::Missing => {
            "Docker CLI disappeared during post-install remediation".to_string()
        }
    });
    report.after = Some(after);
    Ok(report)
}

fn reconcile_missing_runtime(
    artifact: &RuntimeInstallArtifact,
    execution_user: String,
    before: DockerRuntimeProbe,
    package_manager: Option<PackageManager>,
    execute_requested: bool,
    execution_authorized: bool,
) -> Result<RuntimeArtifactReconciliationReport> {
    if !execute_requested {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            package_manager,
            package_manager.map(package_manager_key).map(str::to_string),
            execute_requested,
            execution_authorized,
            "suggested",
            Some("Docker runtime is missing; trusted runtime artifact is available but execution was not requested".to_string()),
        ));
    }
    if !execution_authorized {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            package_manager,
            package_manager.map(package_manager_key).map(str::to_string),
            execute_requested,
            execution_authorized,
            "blocked_governance",
            Some("Docker runtime is missing, but host mutation is not authorized by the active D-Graph revision and local policy".to_string()),
        ));
    }

    let Some(manager) = package_manager else {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            None,
            None,
            execute_requested,
            execution_authorized,
            "unsupported",
            Some("Docker runtime is missing and no supported package manager was detected".to_string()),
        ));
    };
    let installer_key = package_manager_key(manager);
    let Some(installer) = artifact.installers.get(installer_key) else {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            Some(manager),
            Some(installer_key.to_string()),
            execute_requested,
            execution_authorized,
            "missing_installer",
            Some(format!(
                "runtime artifact does not define installer '{}' for detected package manager",
                installer_key
            )),
        ));
    };
    if installer_requires_privilege(installer) && !has_noninteractive_privilege()? {
        return Ok(base_report(
            artifact,
            execution_user,
            before,
            Some(manager),
            Some(installer_key.to_string()),
            execute_requested,
            execution_authorized,
            "privilege_required",
            Some("runtime artifact requires root or passwordless sudo; SmartSentinel will not prompt for a password".to_string()),
        ));
    }

    let mut report = base_report(
        artifact,
        execution_user.clone(),
        before,
        Some(manager),
        Some(installer_key.to_string()),
        execute_requested,
        execution_authorized,
        "executing",
        None,
    );

    if !run_command_phase(
        &mut report,
        "pre_install",
        &installer.pre_install_commands,
        &execution_user,
    )? {
        report.after = Some(probe_docker_runtime()?);
        return Ok(report);
    }
    if !run_command_phase(
        &mut report,
        "install",
        std::slice::from_ref(&installer.install_command),
        &execution_user,
    )? {
        report.after = Some(probe_docker_runtime()?);
        return Ok(report);
    }
    if !run_command_phase(
        &mut report,
        "post_install",
        &installer.post_install_commands,
        &execution_user,
    )? {
        report.after = Some(probe_docker_runtime()?);
        return Ok(report);
    }

    let after = probe_docker_runtime()?;
    report.status = match after.state {
        DockerRuntimeState::Available => "installed_available",
        DockerRuntimeState::InstalledButUnavailable if report.session_refresh_required => {
            "installed_requires_session_refresh"
        }
        DockerRuntimeState::InstalledButUnavailable => "installed_but_unavailable",
        DockerRuntimeState::Missing => "failed",
    }
    .to_string();
    report.detail = Some(match after.state {
        DockerRuntimeState::Available => {
            "runtime artifact completed and Docker is available to SmartSentinel".to_string()
        }
        DockerRuntimeState::InstalledButUnavailable if report.session_refresh_required => {
            "runtime artifact completed; Docker group membership changed and the SmartSentinel session/service must be refreshed before daemon access can change".to_string()
        }
        DockerRuntimeState::InstalledButUnavailable => {
            "runtime artifact completed, but Docker daemon access is still unavailable".to_string()
        }
        DockerRuntimeState::Missing => {
            "runtime artifact completed but Docker CLI is still missing".to_string()
        }
    });
    report.after = Some(after);
    Ok(report)
}

fn run_command_phase(
    report: &mut RuntimeArtifactReconciliationReport,
    phase: &str,
    commands: &[RuntimeInstallCommand],
    execution_user: &str,
) -> Result<bool> {
    for (ordinal, command) in commands.iter().enumerate() {
        let resolved = resolve_command(command, execution_user);
        let output = execute_typed_command(&resolved)?;
        let success = output.status.success();
        report.command_results.push(RuntimeInstallCommandResult {
            phase: phase.to_string(),
            ordinal,
            program: resolved.program.clone(),
            args: resolved.args.clone(),
            privileged: resolved.privileged,
            success,
            detail: Some(output_detail(&output)),
        });
        report.side_effects_performed = true;
        if resolved.program == "usermod"
            && resolved.args.iter().any(|arg| arg == "docker")
            && success
        {
            report.session_refresh_required = true;
        }
        if !success {
            report.status = "failed".to_string();
            report.detail = Some(format!(
                "runtime artifact command failed during {}: {}",
                phase, resolved.program
            ));
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn current_execution_user() -> Result<String> {
    for key in ["SUDO_USER", "USER", "LOGNAME"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() && value != "root" {
                return Ok(value.to_string());
            }
        }
    }
    let output = Command::new("id")
        .arg("-un")
        .output()
        .context("resolve current SmartSentinel execution user")?;
    if !output.status.success() {
        bail!("unable to resolve current SmartSentinel execution user");
    }
    let user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    required("execution user", &user)?;
    Ok(user)
}

fn base_report(
    artifact: &RuntimeInstallArtifact,
    execution_user: String,
    before: DockerRuntimeProbe,
    package_manager: Option<PackageManager>,
    selected_installer: Option<String>,
    execute_requested: bool,
    execution_authorized: bool,
    status: &str,
    detail: Option<String>,
) -> RuntimeArtifactReconciliationReport {
    RuntimeArtifactReconciliationReport {
        schema_version: RUNTIME_INSTALL_ARTIFACT_SCHEMA_VERSION.to_string(),
        artifact_id: artifact.artifact_id.clone(),
        runtime: artifact.runtime.clone(),
        execution_user,
        before,
        package_manager,
        selected_installer,
        execute_requested,
        execution_authorized,
        status: status.to_string(),
        command_results: Vec::new(),
        after: None,
        side_effects_performed: false,
        session_refresh_required: false,
        detail,
    }
}

fn resolve_execution_user(
    artifact: &RuntimeInstallArtifact,
    execution_user_override: Option<&str>,
) -> Result<String> {
    if let Some(user) = execution_user_override.map(str::trim).filter(|v| !v.is_empty()) {
        return Ok(user.to_string());
    }
    let configured = artifact.execution_user.trim();
    if configured != SMARTSENTINEL_USER_PLACEHOLDER {
        required("execution_user", configured)?;
        return Ok(configured.to_string());
    }
    current_execution_user()
}

fn resolve_command(command: &RuntimeInstallCommand, execution_user: &str) -> RuntimeInstallCommand {
    RuntimeInstallCommand {
        program: command
            .program
            .replace(SMARTSENTINEL_USER_PLACEHOLDER, execution_user),
        args: command
            .args
            .iter()
            .map(|arg| arg.replace(SMARTSENTINEL_USER_PLACEHOLDER, execution_user))
            .collect(),
        privileged: command.privileged,
    }
}

fn validate_command(command: &RuntimeInstallCommand) -> Result<()> {
    required("command program", &command.program)?;
    if command.program.contains(char::is_whitespace) {
        bail!(
            "runtime artifact command program '{}' must be a single executable name/path",
            command.program
        );
    }
    Ok(())
}

fn package_manager_key(manager: PackageManager) -> &'static str {
    match manager {
        PackageManager::AptGet => "apt_get",
        PackageManager::Dnf => "dnf",
        PackageManager::Yum => "yum",
        PackageManager::Pacman => "pacman",
        PackageManager::Zypper => "zypper",
    }
}

fn installer_requires_privilege(installer: &RuntimeInstallerLifecycle) -> bool {
    installer
        .pre_install_commands
        .iter()
        .chain(std::iter::once(&installer.install_command))
        .chain(installer.post_install_commands.iter())
        .any(|command| command.privileged)
}

fn commands_require_privilege(commands: &[RuntimeInstallCommand]) -> bool {
    commands.iter().any(|command| command.privileged)
}

fn execute_typed_command(command: &RuntimeInstallCommand) -> Result<Output> {
    if command.privileged {
        return run_privileged(&command.program, &command.args);
    }
    Command::new(&command.program)
        .args(&command.args)
        .output()
        .with_context(|| format!("execute runtime artifact program {}", command.program))
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

fn run_privileged(program: &str, args: &[String]) -> Result<Output> {
    if current_uid()? == Some(0) {
        return Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("execute privileged runtime artifact program {program}"));
    }
    let mut command = Command::new("sudo");
    command.arg("-n").arg(program).args(args);
    command
        .output()
        .with_context(|| format!("execute privileged runtime artifact program {program} through sudo -n"))
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

fn required(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("runtime installation artifact {label} must not be empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> RuntimeInstallArtifact {
        serde_json::from_str(include_str!("../../../scripts/fixtures/o2-runtime-docker-install-artifact.json"))
            .unwrap()
    }

    #[test]
    fn docker_fixture_is_valid_and_typed() {
        let artifact = fixture();
        validate_runtime_install_artifact(&artifact).unwrap();
        let apt = artifact.installers.get("apt_get").unwrap();
        assert_eq!(apt.pre_install_commands[0].program, "apt-get");
        assert_eq!(apt.install_command.args, vec!["install", "-y", "docker.io"]);
        assert_eq!(apt.post_install_commands[1].program, "usermod");
    }

    #[test]
    fn execution_user_placeholder_is_resolved_without_shell_expansion() {
        let artifact = fixture();
        let command = &artifact.installers.get("apt_get").unwrap().post_install_commands[1];
        let resolved = resolve_command(command, "smartsentinel");
        assert_eq!(resolved.program, "usermod");
        assert_eq!(resolved.args, vec!["-aG", "docker", "smartsentinel"]);
    }

    #[test]
    fn manager_keys_match_artifact_contract() {
        assert_eq!(package_manager_key(PackageManager::AptGet), "apt_get");
        assert_eq!(package_manager_key(PackageManager::Dnf), "dnf");
        assert_eq!(package_manager_key(PackageManager::Zypper), "zypper");
    }

    #[test]
    fn docker_fixture_has_post_install_remediation_for_linux_managers() {
        let artifact = fixture();
        for key in ["apt_get", "dnf", "yum", "pacman", "zypper"] {
            let installer = artifact.installers.get(key).unwrap();
            assert!(installer
                .post_install_commands
                .iter()
                .any(|command| command.program == "systemctl"));
            assert!(installer
                .post_install_commands
                .iter()
                .any(|command| command.program == "usermod"));
        }
    }
}
