use std::{collections::HashSet, fs, io::Read, path::Path, time::Duration};

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, redirect::Policy as RedirectPolicy, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasmtime::{Config, Engine, ExternType, Module};

use crate::wasm_runtime::integrity::WasmArtifactIdentity;

const WASM_MAGIC: &[u8; 4] = b"\0asm";
const SHA256_HEX_LEN: usize = 64;
const DEFAULT_MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatumNodeDeploymentScope {
    Full,
    NodeSlice,
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatumDeploymentArtifact {
    pub artifact_id: String,
    pub artifact_uri: String,
    pub sha256: String,
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DatumDeploymentRuntimeLimits {
    pub fuel: Option<u64>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatumDeploymentAssignment {
    pub assignment_id: String,
    pub module_id: String,
    pub function_name: String,
    pub artifact: DatumDeploymentArtifact,
    pub target_node: String,
    #[serde(default)]
    pub limits: DatumDeploymentRuntimeLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatumDeploymentExecutionBoundary {
    pub explicit_invocation_required: bool,
    pub automatic_execution_allowed: bool,
    pub governed_execution_authorized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatumNodeScopedDeployment {
    pub manifest_version: String,
    pub manifest_id: String,
    pub project_id: String,
    pub node_id: String,
    pub delivery_scope: DatumNodeDeploymentScope,
    pub assignments: Vec<DatumDeploymentAssignment>,
    pub execution_boundary: DatumDeploymentExecutionBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactAcquisitionPolicy {
    pub allow_local_file: bool,
    pub allow_https: bool,
    pub allow_insecure_http: bool,
    pub max_artifact_bytes: usize,
    pub request_timeout_ms: u64,
}

impl Default for ArtifactAcquisitionPolicy {
    fn default() -> Self {
        Self {
            allow_local_file: true,
            allow_https: true,
            allow_insecure_http: false,
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            request_timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSourceKind {
    LocalFile,
    Https,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAcquisitionEvidence {
    pub source_kind: ArtifactSourceKind,
    pub bytes_received: usize,
    pub artifact_delivery_performed: bool,
    pub host_filesystem_read_performed: bool,
    pub host_network_fetch_performed: bool,
    pub persistent_write_performed: bool,
    pub host_mutation_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmArtifactPreflightEvidence {
    pub integrity_verified: bool,
    pub artifact_content_id: String,
    pub wasmtime_compiled: bool,
    pub wasmtime_instantiated: bool,
    pub guest_execution_performed: bool,
    pub wasm_guest_side_effects: bool,
    pub governed_execution_performed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquiredWasmArtifact {
    pub display_name: String,
    pub size_bytes: usize,
    bytes: Vec<u8>,
}

impl AcquiredWasmArtifact {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDeploymentArtifact {
    pub assignment_id: String,
    pub module_id: String,
    pub function_name: String,
    pub target_node: String,
    pub artifact_uri: String,
    pub identity: WasmArtifactIdentity,
    pub acquisition: ArtifactAcquisitionEvidence,
    pub preflight: WasmArtifactPreflightEvidence,
    pub artifact: AcquiredWasmArtifact,
}

pub fn prepare_node_scoped_deployment(
    view: &DatumNodeScopedDeployment,
    local_node_id: &str,
    policy: &ArtifactAcquisitionPolicy,
) -> Result<Vec<PreparedDeploymentArtifact>> {
    validate_node_scoped_deployment_for_local_node(view, local_node_id)?;
    validate_policy(policy)?;

    view.assignments
        .iter()
        .map(|assignment| prepare_assignment(assignment, policy))
        .collect()
}

pub fn validate_node_scoped_deployment_for_local_node(
    view: &DatumNodeScopedDeployment,
    local_node_id: &str,
) -> Result<()> {
    let local_node_id = local_node_id.trim();
    if local_node_id.is_empty() {
        return Err(anyhow!("local_node_id must not be empty"));
    }
    if view.node_id.trim() != local_node_id {
        return Err(anyhow!(
            "node-scoped deployment is addressed to '{}' but local DATUM node is '{}'",
            view.node_id,
            local_node_id
        ));
    }
    if view.manifest_version.trim().is_empty()
        || view.manifest_id.trim().is_empty()
        || view.project_id.trim().is_empty()
    {
        return Err(anyhow!(
            "manifest_version, manifest_id, and project_id must not be empty"
        ));
    }

    validate_execution_boundary(&view.execution_boundary)?;

    match &view.delivery_scope {
        DatumNodeDeploymentScope::Empty if !view.assignments.is_empty() => {
            return Err(anyhow!(
                "empty node-scoped deployment must not contain assignments"
            ));
        }
        DatumNodeDeploymentScope::Full | DatumNodeDeploymentScope::NodeSlice
            if view.assignments.is_empty() =>
        {
            return Err(anyhow!(
                "non-empty delivery scope must contain at least one assignment"
            ));
        }
        _ => {}
    }

    let mut seen = HashSet::new();
    for assignment in &view.assignments {
        validate_assignment(assignment, local_node_id)?;
        if !seen.insert(assignment.assignment_id.trim()) {
            return Err(anyhow!(
                "duplicate assignment_id is not allowed: {}",
                assignment.assignment_id
            ));
        }
    }
    Ok(())
}

fn prepare_assignment(
    assignment: &DatumDeploymentAssignment,
    policy: &ArtifactAcquisitionPolicy,
) -> Result<PreparedDeploymentArtifact> {
    let (artifact, acquisition) = acquire_artifact(assignment, policy)?;
    let identity = verify_sha256(&artifact, &assignment.artifact.sha256).with_context(|| {
        format!(
            "verify deployment artifact integrity for assignment '{}'",
            assignment.assignment_id
        )
    })?;

    if identity.content_id != assignment.artifact.content_id.trim().to_ascii_lowercase() {
        return Err(anyhow!(
            "verified artifact content identity '{}' does not match assignment content_id '{}'",
            identity.content_id,
            assignment.artifact.content_id
        ));
    }

    compile_preflight(&artifact, &assignment.function_name)?;

    Ok(PreparedDeploymentArtifact {
        assignment_id: assignment.assignment_id.clone(),
        module_id: assignment.module_id.clone(),
        function_name: assignment.function_name.clone(),
        target_node: assignment.target_node.clone(),
        artifact_uri: assignment.artifact.artifact_uri.clone(),
        preflight: WasmArtifactPreflightEvidence {
            integrity_verified: true,
            artifact_content_id: identity.content_id.clone(),
            wasmtime_compiled: true,
            wasmtime_instantiated: false,
            guest_execution_performed: false,
            wasm_guest_side_effects: false,
            governed_execution_performed: false,
        },
        identity,
        acquisition,
        artifact,
    })
}

fn acquire_artifact(
    assignment: &DatumDeploymentAssignment,
    policy: &ArtifactAcquisitionPolicy,
) -> Result<(AcquiredWasmArtifact, ArtifactAcquisitionEvidence)> {
    let url = Url::parse(assignment.artifact.artifact_uri.trim()).with_context(|| {
        format!("parse artifact_uri '{}'", assignment.artifact.artifact_uri)
    })?;

    match url.scheme() {
        "file" => acquire_file(&url, policy),
        "https" => acquire_http(assignment, &url, policy, ArtifactSourceKind::Https),
        "http" => acquire_http(assignment, &url, policy, ArtifactSourceKind::Http),
        scheme => Err(anyhow!(
            "artifact_uri scheme '{}' is not allowed; expected file, https, or explicitly enabled http",
            scheme
        )),
    }
}

fn acquire_file(
    url: &Url,
    policy: &ArtifactAcquisitionPolicy,
) -> Result<(AcquiredWasmArtifact, ArtifactAcquisitionEvidence)> {
    if !policy.allow_local_file {
        return Err(anyhow!("local file artifact acquisition is disabled by policy"));
    }

    let path = url
        .to_file_path()
        .map_err(|_| anyhow!("file artifact_uri is not a valid local file path: {url}"))?;
    ensure_wasm_extension(&path)?;
    let metadata = fs::metadata(&path)
        .with_context(|| format!("read deployment artifact metadata from {}", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!(
            "deployment artifact path is not a regular file: {}",
            path.display()
        ));
    }
    enforce_size(metadata.len() as usize, policy.max_artifact_bytes)?;
    let bytes = fs::read(&path)
        .with_context(|| format!("read deployment artifact from {}", path.display()))?;
    let artifact = build_artifact(path.display().to_string(), bytes, policy.max_artifact_bytes)?;

    Ok((
        artifact.clone(),
        ArtifactAcquisitionEvidence {
            source_kind: ArtifactSourceKind::LocalFile,
            bytes_received: artifact.size_bytes,
            artifact_delivery_performed: false,
            host_filesystem_read_performed: true,
            host_network_fetch_performed: false,
            persistent_write_performed: false,
            host_mutation_performed: false,
        },
    ))
}

fn acquire_http(
    assignment: &DatumDeploymentAssignment,
    url: &Url,
    policy: &ArtifactAcquisitionPolicy,
    source_kind: ArtifactSourceKind,
) -> Result<(AcquiredWasmArtifact, ArtifactAcquisitionEvidence)> {
    match &source_kind {
        ArtifactSourceKind::Https if !policy.allow_https => {
            return Err(anyhow!("HTTPS artifact acquisition is disabled by policy"));
        }
        ArtifactSourceKind::Http if !policy.allow_insecure_http => {
            return Err(anyhow!(
                "insecure HTTP artifact acquisition is disabled by default and must be explicitly enabled"
            ));
        }
        _ => {}
    }

    let is_wasm = url
        .path_segments()
        .and_then(|segments| segments.last())
        .map(|name| name.to_ascii_lowercase().ends_with(".wasm"))
        .unwrap_or(false);
    if !is_wasm {
        return Err(anyhow!("remote artifact URI must identify a .wasm resource"));
    }

    let client = Client::builder()
        .timeout(Duration::from_millis(policy.request_timeout_ms))
        .redirect(RedirectPolicy::none())
        .build()
        .context("build bounded artifact acquisition HTTP client")?;
    let response = client
        .get(url.clone())
        .send()
        .with_context(|| format!("fetch deployment artifact from {url}"))?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "artifact acquisition returned non-success HTTP status {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .map(|length| length > policy.max_artifact_bytes as u64)
        .unwrap_or(false)
    {
        return Err(anyhow!("artifact Content-Length exceeds configured maximum"));
    }

    let mut bytes = Vec::new();
    response
        .take(policy.max_artifact_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read deployment artifact body from {url}"))?;
    let artifact = build_artifact(
        format!(
            "acquired-{}.wasm",
            sanitize_artifact_id(&assignment.artifact.artifact_id)
        ),
        bytes,
        policy.max_artifact_bytes,
    )?;

    Ok((
        artifact.clone(),
        ArtifactAcquisitionEvidence {
            source_kind,
            bytes_received: artifact.size_bytes,
            artifact_delivery_performed: true,
            host_filesystem_read_performed: false,
            host_network_fetch_performed: true,
            persistent_write_performed: false,
            host_mutation_performed: false,
        },
    ))
}

fn build_artifact(
    display_name: String,
    bytes: Vec<u8>,
    max_artifact_bytes: usize,
) -> Result<AcquiredWasmArtifact> {
    enforce_size(bytes.len(), max_artifact_bytes)?;
    if !bytes.starts_with(WASM_MAGIC) {
        return Err(anyhow!(
            "deployment artifact does not contain the WebAssembly binary magic header"
        ));
    }
    Ok(AcquiredWasmArtifact {
        display_name,
        size_bytes: bytes.len(),
        bytes,
    })
}

fn verify_sha256(
    artifact: &AcquiredWasmArtifact,
    expected_sha256: &str,
) -> Result<WasmArtifactIdentity> {
    let expected = normalize_sha256(expected_sha256)?;
    let observed = sha256_hex(artifact.bytes());
    if observed != expected {
        return Err(anyhow!(
            "external WASM artifact SHA-256 mismatch: expected {expected}, calculated {observed}"
        ));
    }

    Ok(WasmArtifactIdentity {
        algorithm: "sha256".to_string(),
        sha256: observed.clone(),
        content_id: format!("sha256:{observed}"),
        size_bytes: artifact.size_bytes,
    })
}

fn compile_preflight(artifact: &AcquiredWasmArtifact, function_name: &str) -> Result<()> {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build Wasmtime engine for deployment preflight")?;
    let module = Module::new(&engine, artifact.bytes())
        .context("compile verified deployment artifact during Wasmtime preflight")?;

    if let Some(import) = module.imports().next() {
        return Err(anyhow!(
            "deployment artifact imports host item '{}::{}'; host imports are not allowed in the controlled sandbox",
            import.module(),
            import.name()
        ));
    }

    let export = module
        .exports()
        .find(|export| export.name() == function_name)
        .ok_or_else(|| anyhow!("deployment artifact does not export requested function '{function_name}'"))?;
    if !matches!(export.ty(), ExternType::Func(_)) {
        return Err(anyhow!(
            "requested deployment export '{function_name}' is not a function"
        ));
    }
    Ok(())
}

fn validate_execution_boundary(boundary: &DatumDeploymentExecutionBoundary) -> Result<()> {
    if !boundary.explicit_invocation_required {
        return Err(anyhow!("explicit_invocation_required must remain true"));
    }
    if boundary.automatic_execution_allowed {
        return Err(anyhow!("automatic_execution_allowed must remain false"));
    }
    if boundary.governed_execution_authorized {
        return Err(anyhow!("governed_execution_authorized must remain false"));
    }
    Ok(())
}

fn validate_assignment(assignment: &DatumDeploymentAssignment, local_node_id: &str) -> Result<()> {
    if assignment.assignment_id.trim().is_empty()
        || assignment.module_id.trim().is_empty()
        || assignment.function_name.trim().is_empty()
        || assignment.artifact.artifact_id.trim().is_empty()
        || assignment.artifact.artifact_uri.trim().is_empty()
    {
        return Err(anyhow!("assignment identifiers must not be empty"));
    }
    if assignment.target_node.trim() != local_node_id {
        return Err(anyhow!(
            "assignment '{}' targets '{}' but local node-scoped deployment is for '{}'",
            assignment.assignment_id,
            assignment.target_node,
            local_node_id
        ));
    }

    let digest = normalize_sha256(&assignment.artifact.sha256)?;
    let expected_content_id = format!("sha256:{digest}");
    if assignment.artifact.content_id.trim().to_ascii_lowercase() != expected_content_id {
        return Err(anyhow!(
            "assignment artifact content_id must match SHA-256 digest; expected {expected_content_id}"
        ));
    }
    if assignment.limits.fuel == Some(0) {
        return Err(anyhow!("assignment fuel must be positive when present"));
    }
    if assignment.limits.timeout_ms == Some(0) {
        return Err(anyhow!("assignment timeout_ms must be positive when present"));
    }
    Ok(())
}

fn validate_policy(policy: &ArtifactAcquisitionPolicy) -> Result<()> {
    if policy.max_artifact_bytes == 0 || policy.request_timeout_ms == 0 {
        return Err(anyhow!(
            "artifact size limit and request timeout must be positive"
        ));
    }
    Ok(())
}

fn normalize_sha256(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let digest = trimmed.strip_prefix("sha256:").unwrap_or(trimmed);
    if digest.len() != SHA256_HEX_LEN || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "artifact SHA-256 must contain exactly 64 hexadecimal characters, optionally prefixed with 'sha256:'"
        ));
    }
    Ok(digest.to_ascii_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ensure_wasm_extension(path: &Path) -> Result<()> {
    let valid = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("wasm"))
        .unwrap_or(false);
    if !valid {
        return Err(anyhow!("deployment artifact must use the .wasm extension"));
    }
    Ok(())
}

fn enforce_size(size_bytes: usize, max_artifact_bytes: usize) -> Result<()> {
    if size_bytes > max_artifact_bytes {
        return Err(anyhow!(
            "artifact size {} exceeds configured maximum {} bytes",
            size_bytes,
            max_artifact_bytes
        ));
    }
    Ok(())
}

fn sanitize_artifact_id(artifact_id: &str) -> String {
    artifact_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
    };

    use uuid::Uuid;

    use super::*;

    const ADD_WAT: &str = r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#;

    fn wasm(source: &str) -> Vec<u8> {
        wat::parse_str(source).expect("test WAT should compile")
    }

    fn write_artifact(bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("datum-c65-{}.wasm", Uuid::new_v4()));
        fs::write(&path, bytes).expect("test artifact should be written");
        path
    }

    fn assignment(uri: String, bytes: &[u8]) -> DatumDeploymentAssignment {
        let digest = sha256_hex(bytes);
        DatumDeploymentAssignment {
            assignment_id: "assignment-1".to_string(),
            module_id: "external-add".to_string(),
            function_name: "add".to_string(),
            artifact: DatumDeploymentArtifact {
                artifact_id: "artifact-1".to_string(),
                artifact_uri: uri,
                sha256: digest.clone(),
                content_id: format!("sha256:{digest}"),
            },
            target_node: "fog-01".to_string(),
            limits: DatumDeploymentRuntimeLimits {
                fuel: Some(1_000_000),
                timeout_ms: Some(1_000),
            },
        }
    }

    fn view(assignment: DatumDeploymentAssignment) -> DatumNodeScopedDeployment {
        DatumNodeScopedDeployment {
            manifest_version: "0.1.0".to_string(),
            manifest_id: "manifest-1".to_string(),
            project_id: "smart-farm".to_string(),
            node_id: "fog-01".to_string(),
            delivery_scope: DatumNodeDeploymentScope::NodeSlice,
            assignments: vec![assignment],
            execution_boundary: DatumDeploymentExecutionBoundary {
                explicit_invocation_required: true,
                automatic_execution_allowed: false,
                governed_execution_authorized: false,
            },
        }
    }

    fn file_uri(path: &Path) -> String {
        Url::from_file_path(path)
            .expect("temporary path should convert to file URL")
            .to_string()
    }

    #[test]
    fn local_file_is_verified_and_compiled_without_instantiation() {
        let bytes = wasm(ADD_WAT);
        let path = write_artifact(&bytes);
        let prepared = prepare_node_scoped_deployment(
            &view(assignment(file_uri(&path), &bytes)),
            "fog-01",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect("local artifact should prepare");

        assert_eq!(prepared.len(), 1);
        assert!(prepared[0].preflight.integrity_verified);
        assert!(prepared[0].preflight.wasmtime_compiled);
        assert!(!prepared[0].preflight.wasmtime_instantiated);
        assert!(!prepared[0].preflight.guest_execution_performed);
        assert!(prepared[0].acquisition.host_filesystem_read_performed);
        assert!(!prepared[0].acquisition.host_mutation_performed);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn wrong_node_is_rejected_before_acquisition() {
        let bytes = wasm(ADD_WAT);
        let path = write_artifact(&bytes);
        let error = prepare_node_scoped_deployment(
            &view(assignment(file_uri(&path), &bytes)),
            "fog-02",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect_err("wrong recipient must fail");
        assert!(error.to_string().contains("addressed to 'fog-01'"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cross_node_assignment_is_rejected() {
        let bytes = wasm(ADD_WAT);
        let path = write_artifact(&bytes);
        let mut leaked = assignment(file_uri(&path), &bytes);
        leaked.target_node = "fog-02".to_string();
        let error = validate_node_scoped_deployment_for_local_node(&view(leaked), "fog-01")
            .expect_err("cross-node assignment must fail");
        assert!(error.to_string().contains("targets 'fog-02'"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn empty_scope_is_a_valid_noop() {
        let empty = DatumNodeScopedDeployment {
            manifest_version: "0.1.0".to_string(),
            manifest_id: "manifest-empty".to_string(),
            project_id: "smart-farm".to_string(),
            node_id: "fog-01".to_string(),
            delivery_scope: DatumNodeDeploymentScope::Empty,
            assignments: Vec::new(),
            execution_boundary: DatumDeploymentExecutionBoundary {
                explicit_invocation_required: true,
                automatic_execution_allowed: false,
                governed_execution_authorized: false,
            },
        };
        assert!(prepare_node_scoped_deployment(
            &empty,
            "fog-01",
            &ArtifactAcquisitionPolicy::default()
        )
        .expect("empty view should prepare")
        .is_empty());
    }

    #[test]
    fn sha_mismatch_precedes_wasmtime_compilation() {
        let bytes = b"\0asminvalid-module".to_vec();
        let path = write_artifact(&bytes);
        let mut invalid = assignment(file_uri(&path), &bytes);
        invalid.artifact.sha256 = "0".repeat(64);
        invalid.artifact.content_id = format!("sha256:{}", "0".repeat(64));
        let error = prepare_node_scoped_deployment(
            &view(invalid),
            "fog-01",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect_err("hash mismatch must fail");
        let detail = format!("{error:#}");
        assert!(detail.contains("SHA-256 mismatch"));
        assert!(!detail.contains("compile verified deployment artifact"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn host_import_is_rejected_during_preflight() {
        let bytes = wasm(
            r#"
(module
  (import "host" "log" (func $log))
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#,
        );
        let path = write_artifact(&bytes);
        let error = prepare_node_scoped_deployment(
            &view(assignment(file_uri(&path), &bytes)),
            "fog-01",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect_err("host import must fail");
        assert!(error.to_string().contains("host imports are not allowed"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_function_is_rejected_during_preflight() {
        let bytes = wasm(ADD_WAT);
        let path = write_artifact(&bytes);
        let mut invalid = assignment(file_uri(&path), &bytes);
        invalid.function_name = "missing".to_string();
        let error = prepare_node_scoped_deployment(
            &view(invalid),
            "fog-01",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect_err("missing export must fail");
        assert!(error
            .to_string()
            .contains("does not export requested function"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn insecure_http_is_blocked_by_default() {
        let bytes = wasm(ADD_WAT);
        let error = prepare_node_scoped_deployment(
            &view(assignment(
                "http://127.0.0.1:1/artifact.wasm".to_string(),
                &bytes,
            )),
            "fog-01",
            &ArtifactAcquisitionPolicy::default(),
        )
        .expect_err("HTTP must be explicit");
        assert!(error.to_string().contains("explicitly enabled"));
    }

    #[test]
    fn lab_http_fetch_stays_in_memory() {
        let bytes = wasm(ADD_WAT);
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let body = bytes.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("headers should write");
            stream.write_all(&body).expect("body should write");
        });

        let mut policy = ArtifactAcquisitionPolicy::default();
        policy.allow_insecure_http = true;
        let prepared = prepare_node_scoped_deployment(
            &view(assignment(
                format!("http://{address}/artifact.wasm"),
                &bytes,
            )),
            "fog-01",
            &policy,
        )
        .expect("lab HTTP should prepare");
        server.join().expect("server should finish");

        assert!(prepared[0].acquisition.artifact_delivery_performed);
        assert!(prepared[0].acquisition.host_network_fetch_performed);
        assert!(!prepared[0].acquisition.persistent_write_performed);
        assert!(!prepared[0].acquisition.host_mutation_performed);
    }

    #[test]
    fn artifact_size_limit_is_enforced() {
        let bytes = wasm(ADD_WAT);
        let path = write_artifact(&bytes);
        let mut policy = ArtifactAcquisitionPolicy::default();
        policy.max_artifact_bytes = 4;
        let error = prepare_node_scoped_deployment(
            &view(assignment(file_uri(&path), &bytes)),
            "fog-01",
            &policy,
        )
        .expect_err("size limit must fail");
        assert!(error.to_string().contains("exceeds configured maximum"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn automatic_execution_remains_forbidden() {
        let bytes = wasm(ADD_WAT);
        let path = write_artifact(&bytes);
        let mut invalid = view(assignment(file_uri(&path), &bytes));
        invalid.execution_boundary.automatic_execution_allowed = true;
        let error = validate_node_scoped_deployment_for_local_node(&invalid, "fog-01")
            .expect_err("automatic execution must fail");
        assert!(error.to_string().contains("automatic_execution_allowed"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn c64_json_shape_deserializes_with_extra_fields() {
        let bytes = wasm(ADD_WAT);
        let digest = sha256_hex(&bytes);
        let value = serde_json::json!({
            "manifest_version": "0.1.0",
            "manifest_id": "manifest-1",
            "project_id": "smart-farm",
            "node_id": "fog-01",
            "strategy": "distributed",
            "graph_distribution": "node_slice",
            "delivery_scope": "node_slice",
            "source_dgraph_id": "dgraph-1",
            "source_assignment_count": 2,
            "selected_assignment_count": 1,
            "assignments": [{
                "assignment_id": "assignment-1",
                "module_id": "external-add",
                "function_name": "add",
                "artifact": {
                    "artifact_id": "artifact-1",
                    "artifact_uri": "https://example.test/artifact.wasm",
                    "sha256": digest.clone(),
                    "content_id": format!("sha256:{digest}")
                },
                "target_node": "fog-01",
                "limits": {"fuel": 1000000, "timeout_ms": 1000},
                "metadata": {"demo": true}
            }],
            "execution_boundary": {
                "explicit_invocation_required": true,
                "automatic_execution_allowed": false,
                "governed_execution_authorized": false
            }
        });

        let parsed: DatumNodeScopedDeployment =
            serde_json::from_value(value).expect("C64 JSON should deserialize");
        assert_eq!(parsed.node_id, "fog-01");
        assert_eq!(parsed.assignments.len(), 1);
        assert_eq!(parsed.delivery_scope, DatumNodeDeploymentScope::NodeSlice);
    }
}
