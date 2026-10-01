//! Phase 133A: immutable reusable definitions, never deployment/runtime authority.
use super::service_artifact_registry::{ResolvedImageIdentityV1, ServiceArtifact};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub const CATALOG_PATH_ENV: &str = "DATUM_SERVICE_CATALOG_DB_PATH";
// Serializes first bootstrap in this process, not a multi-process guarantee.
static BOOTSTRAP: Mutex<()> = Mutex::new(());
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub project_id: String,
    pub artifact_id: String,
    pub execution_generation: u64,
    pub execution_projection_digest: String,
}
/// A sealed, narrow snapshot of EXISTING artifact identity, not a second resolver.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSnapshot {
    pub source: ArtifactReference,
    pub runtime_kind: String,
    pub image: Option<String>,
    pub resolved_image_identity: Option<ResolvedImageIdentityV1>,
    pub executable_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tenancy {
    Dedicated,
    SharedTenantAware,
    SharedStateless,
    SystemGlobal,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TenantUnit {
    Domain,
    Namespace,
    Vhost,
    Schema,
    Organization,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    NetworkInterface,
    Device,
    P4Port,
    Gpu,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    pub kind: CapabilityKind,
    pub exclusive: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    pub cpu_millis: Option<u32>,
    pub memory_mb: Option<u32>,
    pub storage_mb: Option<u32>,
    #[serde(default)]
    pub capabilities: Vec<CapabilityRequirement>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MonitoringDefinition {
    pub subject_kind: String,
    pub cadence_seconds: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HealthDefinition {
    pub kind: String,
    pub target: String,
    pub port: u16,
    pub path: Option<String>,
    pub timeout_ms: u32,
}
/// Same reference/version/target vocabulary as ServiceArtifactSecretBinding;
/// strict decoding additionally rejects any secret-value property.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecretReference {
    pub secret_ref: String,
    pub version: String,
    pub target_environment_variable: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceShape {
    Atomic,
    Composite,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DefinitionReference {
    pub definition_id: String,
    pub revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComponentReference {
    pub key: String,
    pub definition: DefinitionReference,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Discovery {
    DockerCompose { compose_files: Vec<String> },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Composition {
    Static { components: Vec<ComponentReference> },
    Discovered { discovery: Discovery },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Provisioning {
    RepositoryCommand {
        repository_url: String,
        commit: String,
        command: Vec<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequirement {
    pub capability: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinitionDraft {
    pub service_definition_id: String,
    pub name: String,
    pub artifact: Option<ArtifactReference>,
    pub runtime_kind: Option<String>,
    pub service_shape: ServiceShape,
    pub composition: Option<Composition>,
    pub provisioning: Option<Provisioning>,
    pub tenancy: Tenancy,
    pub tenant_unit: Option<TenantUnit>,
    pub resources: Requirements,
    pub health_check: Option<HealthDefinition>,
    pub monitoring: Option<MonitoringDefinition>,
    #[serde(default)]
    pub secrets: Vec<SecretReference>,
    #[serde(default)]
    pub exact_dependencies: Vec<DefinitionReference>,
    pub requires: Vec<ServiceRequirement>,
    pub provides: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DefinitionRevision {
    pub definition: ServiceDefinitionDraft,
    pub revision: u64,
    pub digest: String,
    pub created_at_utc: String,
    pub artifact_snapshot_id: Option<String>,
    pub scope: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPage {
    pub items: Vec<DefinitionRevision>,
    pub has_more: bool,
    pub limit: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogDiscoveryPage {
    pub schema: String,
    pub items: Vec<DefinitionRevision>,
    pub limit: usize,
    pub offset: usize,
    pub total: usize,
    pub has_more: bool,
    pub search: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    Invalid,
    Conflict,
    NotFound,
    Unavailable,
    Corrupt,
}
pub type Result<T> = std::result::Result<T, CatalogError>;
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn bounded(s: &str) -> bool {
    !s.trim().is_empty() && s.trim() == s && s.len() <= 256 && !s.chars().any(char::is_control)
}
pub fn capture_artifact(
    a: &ServiceArtifact,
    reference: &ArtifactReference,
) -> Result<ArtifactSnapshot> {
    if a.artifact_id != reference.artifact_id
        || a.control_plane.project_id != reference.project_id
        || a.control_plane.execution_generation != reference.execution_generation
        || a.control_plane.execution_projection_digest != reference.execution_projection_digest
    {
        return Err(CatalogError::Conflict);
    }
    let snapshot = ArtifactSnapshot {
        source: reference.clone(),
        runtime_kind: a.runtime.kind.clone(),
        image: a.runtime.container.as_ref().map(|c| c.image.clone()),
        resolved_image_identity: a
            .runtime
            .container
            .as_ref()
            .and_then(|c| c.resolved_image_identity.clone()),
        executable_sha256: if a.runtime.kind == "native_process" {
            a.control_plane.acquisition.sha256.clone()
        } else {
            None
        },
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}
fn hex_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn validate_snapshot(a: &ArtifactSnapshot) -> Result<()> {
    if !bounded(&a.source.project_id)
        || !bounded(&a.source.artifact_id)
        || a.source.execution_generation == 0
        || !super::jcs::is_canonical_digest_format(&a.source.execution_projection_digest)
    {
        return Err(CatalogError::Invalid);
    }
    match a.runtime_kind.as_str() {
        "container" => {
            let p = a
                .resolved_image_identity
                .as_ref()
                .ok_or(CatalogError::Invalid)?;
            if a.image.as_ref().is_none_or(|s| !bounded(s))
                || a.executable_sha256.is_some()
                || p.platforms.is_empty()
            {
                return Err(CatalogError::Invalid);
            }
            super::oci_registry_resolver::validate_resolved_image_identity(
                a.image.as_ref().unwrap(),
                p,
            )
            .map_err(|_| CatalogError::Invalid)?;
        }
        "native_process"
            if a.image.is_none()
                && a.resolved_image_identity.is_none()
                && a.executable_sha256.as_ref().is_some_and(|s| hex_digest(s)) => {}
        _ => return Err(CatalogError::Invalid),
    }
    Ok(())
}
pub fn validate_definition<'a>(
    d: &ServiceDefinitionDraft,
    a: impl Into<Option<&'a ArtifactSnapshot>>,
) -> Result<()> {
    let a = a.into();
    match d.service_shape {
        ServiceShape::Atomic => {
            let a = a.ok_or(CatalogError::Invalid)?;
            validate_snapshot(a)?;
            if d.artifact.as_ref() != Some(&a.source)
                || d.runtime_kind.as_ref() != Some(&a.runtime_kind)
                || d.composition.is_some()
                || d.provisioning.is_some()
            {
                return Err(CatalogError::Invalid);
            }
        }
        ServiceShape::Composite => {
            if a.is_some()
                || d.artifact.is_some()
                || d.runtime_kind.is_some()
                || d.composition.is_none()
            {
                return Err(CatalogError::Invalid);
            }
        }
    }
    if !valid_id(&d.service_definition_id)
        || !bounded(&d.name)
        || (d.tenancy != Tenancy::SharedTenantAware && d.tenant_unit.is_some())
    {
        return Err(CatalogError::Invalid);
    }
    for (n, max) in [
        (d.resources.cpu_millis, 1_000_000),
        (d.resources.memory_mb, 16_777_216),
        (d.resources.storage_mb, 1_000_000_000),
    ] {
        if n.is_some_and(|n| n == 0 || n > max) {
            return Err(CatalogError::Invalid);
        }
    }
    let mut capabilities = BTreeSet::new();
    if d.resources.capabilities.len() > 32
        || d.resources
            .capabilities
            .iter()
            .any(|c| !capabilities.insert(format!("{:?}", c.kind)))
    {
        return Err(CatalogError::Invalid);
    }
    if let Some(m) = &d.monitoring {
        if m.subject_kind != "dserv_realization" || !(1..=86400).contains(&m.cadence_seconds) {
            return Err(CatalogError::Invalid);
        }
    }
    if let Some(h) = &d.health_check {
        if !["http", "tcp_connect"].contains(&h.kind.as_str())
            || !bounded(&h.target)
            || h.target.contains(['/', ':', '@', '?', '#'])
            || h.port == 0
            || !(1..=60000).contains(&h.timeout_ms)
            || (h.kind == "tcp_connect" && h.path.is_some())
            || (h.kind == "http"
                && h.path.as_ref().is_none_or(|p| {
                    !p.starts_with('/') || p.len() > 512 || p.chars().any(char::is_control)
                }))
        {
            return Err(CatalogError::Invalid);
        }
    }
    let mut secrets = BTreeSet::new();
    let mut targets = BTreeSet::new();
    if d.secrets.len() > 64
        || d.secrets.iter().any(|s| {
            !bounded(&s.secret_ref)
                || !bounded(&s.version)
                || !valid_id(&s.target_environment_variable)
                || !secrets.insert(&s.secret_ref)
                || !targets.insert(&s.target_environment_variable)
        })
    {
        return Err(CatalogError::Invalid);
    }
    validate_relations(d)?;
    Ok(())
}
fn valid_reference(r: &DefinitionReference, own: &str) -> bool {
    valid_id(&r.definition_id) && r.definition_id != own && (1..=1_000_000).contains(&r.revision)
}
fn capability_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.split('.').count() >= 2
        && s.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
fn relative_file(s: &str) -> bool {
    bounded(s)
        && !s.contains(['\\', ':'])
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
fn validate_relations(d: &ServiceDefinitionDraft) -> Result<()> {
    let mut deps = BTreeSet::new();
    if d.exact_dependencies.len() > 64
        || d.exact_dependencies.iter().any(|r| {
            !valid_reference(r, &d.service_definition_id)
                || !deps.insert((&r.definition_id, r.revision))
        })
    {
        return Err(CatalogError::Invalid);
    }
    for caps in [
        d.provides.clone(),
        d.requires.iter().map(|r| r.capability.clone()).collect(),
    ] {
        let mut seen = BTreeSet::new();
        if caps.len() > 64 || caps.iter().any(|s| !capability_id(s) || !seen.insert(s)) {
            return Err(CatalogError::Invalid);
        }
    }
    if let Some(composition) = &d.composition {
        match composition {
            Composition::Static { components } => {
                let mut keys = BTreeSet::new();
                if components.is_empty()
                    || components.len() > 64
                    || components.iter().any(|c| {
                        !valid_id(&c.key)
                            || !keys.insert(&c.key)
                            || !valid_reference(&c.definition, &d.service_definition_id)
                    })
                {
                    return Err(CatalogError::Invalid);
                }
                if d.provisioning.is_some() {
                    return Err(CatalogError::Invalid);
                }
            }
            Composition::Discovered {
                discovery: Discovery::DockerCompose { compose_files },
            } => {
                let mut files = BTreeSet::new();
                if d.provisioning.is_none()
                    || compose_files.is_empty()
                    || compose_files.len() > 16
                    || compose_files
                        .iter()
                        .any(|s| !relative_file(s) || !files.insert(s))
                {
                    return Err(CatalogError::Invalid);
                }
            }
        }
    }
    if let Some(Provisioning::RepositoryCommand {
        repository_url,
        commit,
        command,
    }) = &d.provisioning
    {
        // Inspect original source text before a URL parser can normalize it.
        if repository_url.is_empty()
            || repository_url.len() > 512
            || repository_url.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(CatalogError::Invalid);
        }
        let url = reqwest::Url::parse(repository_url).map_err(|_| CatalogError::Invalid)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || commit.len() != 40
            || !commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || command.is_empty()
            || command.len() > 32
            || command.iter().any(|s| !bounded(s))
        {
            return Err(CatalogError::Invalid);
        }
    }
    Ok(())
}
fn digest<T: Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value).map_err(|_| CatalogError::Invalid)?;
    super::jcs::canonical_digest_sha256_hex(&value).map_err(|_| CatalogError::Invalid)
}
#[derive(Clone)]
pub struct CatalogStore {
    path: PathBuf,
}
impl CatalogStore {
    /// Explicitly configured, standalone catalog database; no in-memory fallback.
    pub fn initialize(path: &Path) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().is_empty() {
            return Err(CatalogError::Unavailable);
        }
        let _guard = BOOTSTRAP.lock().map_err(|_| CatalogError::Unavailable)?;
        let store = Self { path: path.into() };
        if !path.try_exists().map_err(|_| CatalogError::Unavailable)? {
            // Only explicit first initialization may create persistence. create_new
            // also fails safely if another process wins; never repair its database.
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|_| CatalogError::Unavailable)?;
            let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(|_| CatalogError::Unavailable)?;
            c.execute_batch("BEGIN IMMEDIATE; CREATE TABLE catalog_artifacts(id TEXT PRIMARY KEY, body TEXT NOT NULL); CREATE TABLE catalog_definitions(id TEXT NOT NULL, revision INTEGER NOT NULL, body TEXT NOT NULL, PRIMARY KEY(id, revision)); PRAGMA user_version=133; COMMIT;")
                .map_err(|_| CatalogError::Unavailable)?;
        }
        store.connection()?;
        Ok(store)
    }
    fn connection(&self) -> Result<Connection> {
        self.open_existing(OpenFlags::SQLITE_OPEN_READ_ONLY)
    }
    fn writable_connection(&self) -> Result<Connection> {
        self.open_existing(OpenFlags::SQLITE_OPEN_READ_WRITE)
    }
    fn open_existing(&self, flags: OpenFlags) -> Result<Connection> {
        let c = Connection::open_with_flags(&self.path, flags)
            .map_err(|_| CatalogError::Unavailable)?;
        c.busy_timeout(Duration::from_secs(5))
            .map_err(|_| CatalogError::Unavailable)?;
        Self::validate_schema(&c)?;
        Ok(c)
    }
    fn validate_schema(c: &Connection) -> Result<()> {
        let version: i64 = c
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| CatalogError::Corrupt)?;
        if version != 133 {
            return Err(CatalogError::Corrupt);
        }
        for (table, expected) in [
            (
                "catalog_artifacts",
                vec![("id", "TEXT", 0, 1), ("body", "TEXT", 1, 0)],
            ),
            (
                "catalog_definitions",
                vec![
                    ("id", "TEXT", 1, 1),
                    ("revision", "INTEGER", 1, 2),
                    ("body", "TEXT", 1, 0),
                ],
            ),
        ] {
            let mut stmt = c
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(|_| CatalogError::Corrupt)?;
            let columns: Vec<(String, String, i64, i64)> = stmt
                .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(5)?)))
                .map_err(|_| CatalogError::Corrupt)?
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| CatalogError::Corrupt)?;
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(n, t, nn, pk)| (n.to_string(), t.to_string(), nn, pk))
                .collect();
            if columns != expected {
                return Err(CatalogError::Corrupt);
            }
            let mut stmt = c
                .prepare(&format!("PRAGMA index_list({table})"))
                .map_err(|_| CatalogError::Corrupt)?;
            let indexes: Vec<(i64, String)> = stmt
                .query_map([], |r| Ok((r.get(2)?, r.get(3)?)))
                .map_err(|_| CatalogError::Corrupt)?
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| CatalogError::Corrupt)?;
            if !indexes
                .iter()
                .any(|(unique, origin)| *unique == 1 && origin == "pk")
            {
                return Err(CatalogError::Corrupt);
            }
        }
        Ok(())
    }
    pub fn save(
        &self,
        d: ServiceDefinitionDraft,
        a: impl Into<Option<ArtifactSnapshot>>,
        previous: u64,
    ) -> Result<DefinitionRevision> {
        let a = a.into();
        validate_definition(&d, a.as_ref())?;
        if previous >= 1_000_000 {
            return Err(CatalogError::Invalid);
        }
        let artifact_snapshot_id = a.as_ref().map(digest).transpose()?;
        let component_refs: Vec<_> = match &d.composition {
            Some(Composition::Static { components }) => {
                components.iter().map(|c| &c.definition).collect()
            }
            _ => vec![],
        };
        for r in d.exact_dependencies.iter().chain(component_refs) {
            self.get(&r.definition_id, Some(r.revision)).map_err(|e| {
                if e == CatalogError::NotFound {
                    CatalogError::Invalid
                } else {
                    e
                }
            })?;
        }
        if previous > 0 {
            let current = self.get(&d.service_definition_id, None)?;
            if current.revision != previous {
                return Err(CatalogError::Conflict);
            }
        }
        if let Some(id) = &artifact_snapshot_id {
            match self.artifact(id) {
                Ok(_) | Err(CatalogError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        let record = DefinitionRevision {
            digest: digest(&d)?,
            definition: d,
            revision: previous + 1,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            artifact_snapshot_id: artifact_snapshot_id.clone(),
            scope: "global_catalog".into(),
        };
        let mut c = self.writable_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| CatalogError::Unavailable)?;
        let latest: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(revision),0) FROM catalog_definitions WHERE id=?",
                [&record.definition.service_definition_id],
                |r| r.get(0),
            )
            .map_err(|_| CatalogError::Unavailable)?;
        if latest != previous as i64 {
            return Err(CatalogError::Conflict);
        }
        if let (Some(id), Some(a)) = (&artifact_snapshot_id, &a) {
            tx.execute(
                "INSERT OR IGNORE INTO catalog_artifacts VALUES(?,?)",
                params![
                    id,
                    serde_json::to_string(&a).map_err(|_| CatalogError::Invalid)?
                ],
            )
            .map_err(|_| CatalogError::Unavailable)?;
        }
        tx.execute(
            "INSERT INTO catalog_definitions VALUES(?,?,?)",
            params![
                record.definition.service_definition_id,
                record.revision as i64,
                serde_json::to_string(&record).map_err(|_| CatalogError::Invalid)?
            ],
        )
        .map_err(|_| CatalogError::Unavailable)?;
        tx.commit().map_err(|_| CatalogError::Unavailable)?;
        Ok(record)
    }
    pub fn get(&self, id: &str, revision: Option<u64>) -> Result<DefinitionRevision> {
        if !valid_id(id) || revision.is_some_and(|r| r == 0 || r > 1_000_000) {
            return Err(CatalogError::Invalid);
        }
        let c = self.connection()?;
        let row: Option<(i64, String)> = c.query_row("SELECT revision,body FROM catalog_definitions WHERE id=?1 AND (?2 IS NULL OR revision=?2) ORDER BY revision DESC LIMIT 1", params![id, revision.map(|r| r as i64)], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(|_| CatalogError::Unavailable)?;
        let (stored_revision, body) = row.ok_or(CatalogError::NotFound)?;
        let d: DefinitionRevision =
            serde_json::from_str(&body).map_err(|_| CatalogError::Corrupt)?;
        if d.definition.service_definition_id != id
            || stored_revision != d.revision as i64
            || revision.is_some_and(|r| r != d.revision)
            || d.revision == 0
            || d.scope != "global_catalog"
            || digest(&d.definition)? != d.digest
        {
            return Err(CatalogError::Corrupt);
        }
        let a = d
            .artifact_snapshot_id
            .as_ref()
            .map(|id| self.artifact(id))
            .transpose()?;
        validate_definition(&d.definition, a.as_ref()).map_err(|_| CatalogError::Corrupt)?;
        Ok(d)
    }
    pub fn artifact(&self, id: &str) -> Result<ArtifactSnapshot> {
        let c = self.connection()?;
        let row: Option<String> = c
            .query_row("SELECT body FROM catalog_artifacts WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|_| CatalogError::Unavailable)?;
        let a: ArtifactSnapshot = serde_json::from_str(&row.ok_or(CatalogError::NotFound)?)
            .map_err(|_| CatalogError::Corrupt)?;
        if digest(&a)? != id {
            return Err(CatalogError::Corrupt);
        }
        validate_snapshot(&a).map_err(|_| CatalogError::Corrupt)?;
        Ok(a)
    }
    pub fn list(&self) -> Result<CatalogPage> {
        let c = self.connection()?;
        let mut stmt = c
            .prepare("SELECT DISTINCT id FROM catalog_definitions ORDER BY id LIMIT 101")
            .map_err(|_| CatalogError::Unavailable)?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .map_err(|_| CatalogError::Unavailable)?
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| CatalogError::Unavailable)?;
        let has_more = ids.len() > 100;
        Ok(CatalogPage {
            items: ids
                .into_iter()
                .take(100)
                .map(|id| self.get(&id, None))
                .collect::<Result<_>>()?,
            has_more,
            limit: 100,
        })
    }
    /// Bounded, read-only discovery over latest immutable revisions. No schema writes.
    pub fn discover(&self, search: &str, offset: usize, limit: usize) -> Result<CatalogDiscoveryPage> {
        if !(1..=100).contains(&limit) || offset > 1_000_000 || search.len() > 256
            || search.chars().any(char::is_control) { return Err(CatalogError::Invalid); }
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| CatalogError::Unavailable)?;
        // Literal substring matching (not LIKE patterns), ASCII case insensitive.
        // Aggregate and paged selection share one SQLite read snapshot. Exact row
        // reads below remain immutable even if a concurrent writer appends revisions.
        const FROM: &str = "FROM catalog_definitions d JOIN (SELECT id, MAX(revision) revision FROM catalog_definitions GROUP BY id) latest ON d.id=latest.id AND d.revision=latest.revision WHERE ?1='' OR instr(lower(d.id),lower(?1))>0 OR instr(lower(json_extract(d.body,'$.definition.name')),lower(?1))>0 OR EXISTS (SELECT 1 FROM json_each(d.body,'$.definition.provides') capability WHERE instr(lower(capability.value),lower(?1))>0)";
        let count: i64 = tx.query_row(&format!("SELECT COUNT(*) {FROM}"), params![search], |r| r.get(0)).map_err(|_| CatalogError::Corrupt)?;
        let total = usize::try_from(count).map_err(|_| CatalogError::Corrupt)?;
        if total as u128 > 9_007_199_254_740_991u128 { return Err(CatalogError::Unavailable); }
        let mut stmt = tx.prepare(&format!("SELECT d.id,d.revision {FROM} ORDER BY d.id COLLATE BINARY LIMIT ?2 OFFSET ?3")).map_err(|_| CatalogError::Corrupt)?;
        let rows: Vec<(String,i64)> = stmt.query_map(params![search,limit as i64,offset as i64], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|_| CatalogError::Corrupt)?
            .collect::<std::result::Result<_,_>>().map_err(|_| CatalogError::Corrupt)?;
        let items = rows.into_iter().map(|(id,revision)| self.get(&id,Some(u64::try_from(revision).map_err(|_| CatalogError::Corrupt)?))).collect::<Result<Vec<_>>>()?;
        let page = CatalogDiscoveryPage { schema:"datum.catalog-discovery/1".into(), items, limit, offset, total, has_more:offset.saturating_add(limit)<total, search:search.into() };
        if serde_json::to_vec(&page).map_err(|_| CatalogError::Corrupt)?.len() > 2_000_000 { return Err(CatalogError::Unavailable); }
        Ok(page)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[test]
    fn discovery_paginates_latest_revisions_and_searches_all_entries_read_only() {
        let s = store();
        for i in 0..100 {
            let mut d = draft();d.service_definition_id=format!("definition-{i:03}");d.name=format!("Discovery Service {i:03}");d.provides=vec![format!("example.provider-{i:03}")];
            s.save(d,snapshot(),0).unwrap();
        }
        let mut latest=s.get("definition-099",Some(1)).unwrap().definition;
        latest.provides=vec!["database.postgresql".into()];s.save(latest,snapshot(),1).unwrap();
        let before=std::fs::read(database_path(&s)).unwrap();
        let first=s.discover("",0,25).unwrap();assert_eq!((first.total,first.items.len(),first.has_more),(100,25,true));
        assert_eq!(first.items[0].definition.service_definition_id,"definition-000");
        let second=s.discover("",25,25).unwrap();assert_eq!(second.items[0].definition.service_definition_id,"definition-025");
        assert_eq!(s.discover("",0,25).unwrap().items[0].digest,first.items[0].digest);
        let last=s.discover("",75,25).unwrap();assert!(!last.has_more);assert_eq!(last.items.last().unwrap().revision,2);
        assert_eq!(s.discover("SERVICE 099",0,25).unwrap().items[0].definition.service_definition_id,"definition-099");
        assert_eq!(s.discover("DATABASE.POSTGRESQL",0,25).unwrap().items[0].revision,2);
        assert_eq!(s.discover("definition-099",0,25).unwrap().total,1);
        assert_eq!(s.discover("example.provider-099",0,25).unwrap().total,0);
        assert_eq!(s.discover("%",0,25).unwrap().total,0);
        assert_eq!(s.discover("no-results",0,25).unwrap().items.len(),0);
        assert_eq!(s.discover("",200,25).unwrap().items.len(),0);
        assert_eq!(s.discover("",0,100).unwrap().items.len(),100);
        for (q,offset,limit) in [("",0,0),("",0,101),("",1000001,25),("\n",0,25)] {assert!(matches!(s.discover(q,offset,limit),Err(CatalogError::Invalid)));}
        assert!(matches!(s.discover(&"x".repeat(257),0,25),Err(CatalogError::Invalid)));
        assert_eq!(std::fs::read(database_path(&s)).unwrap(),before);
    }
    pub fn snapshot() -> ArtifactSnapshot {
        ArtifactSnapshot {
            source: ArtifactReference {
                project_id: "artifact-source-project".into(),
                artifact_id: "p4-sniffer".into(),
                execution_generation: 1,
                execution_projection_digest: format!("sha256:{}", "a".repeat(64)),
            },
            runtime_kind: "native_process".into(),
            image: None,
            resolved_image_identity: None,
            executable_sha256: Some("b".repeat(64)),
        }
    }
    pub fn draft() -> ServiceDefinitionDraft {
        ServiceDefinitionDraft {
            service_definition_id: "p4-sniffer".into(),
            name: "P4 Sniffer".into(),
            artifact: Some(snapshot().source),
            runtime_kind: Some("native_process".into()),
            service_shape: ServiceShape::Atomic,
            composition: None,
            provisioning: None,
            tenancy: Tenancy::Dedicated,
            tenant_unit: None,
            resources: Requirements {
                cpu_millis: Some(1000),
                memory_mb: Some(256),
                storage_mb: None,
                capabilities: vec![CapabilityRequirement {
                    kind: CapabilityKind::NetworkInterface,
                    exclusive: true,
                }],
            },
            health_check: Some(HealthDefinition {
                kind: "tcp_connect".into(),
                target: "127.0.0.1".into(),
                port: 9000,
                path: None,
                timeout_ms: 5000,
            }),
            monitoring: Some(MonitoringDefinition {
                subject_kind: "dserv_realization".into(),
                cadence_seconds: 30,
            }),
            secrets: vec![],
            exact_dependencies: vec![],
            requires: vec![],
            provides: vec![],
        }
    }
    pub fn store() -> CatalogStore {
        let dir =
            std::env::temp_dir().join(format!("datum-service-catalog-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        CatalogStore::initialize(&dir.join("catalog.db")).unwrap()
    }
    #[test]
    fn create_and_history_survive_reopen_without_silent_overwrite() {
        let s = store();
        let one = s.save(draft(), snapshot(), 0).unwrap();
        let mut edit = draft();
        edit.name = "P4 Sniffer revision two".into();
        let two = s.save(edit, snapshot(), 1).unwrap();
        let reopened = CatalogStore::initialize(&s.path).unwrap();
        assert_eq!(reopened.get("p4-sniffer", Some(1)).unwrap(), one);
        assert_eq!(reopened.get("p4-sniffer", None).unwrap(), two);
        assert_eq!(reopened.list().unwrap().items.len(), 1);
        assert_eq!(
            reopened.save(draft(), snapshot(), 1),
            Err(CatalogError::Conflict)
        );
    }
    #[test]
    fn concurrent_revisions_have_one_winner() {
        let s = store();
        s.save(draft(), snapshot(), 0).unwrap();
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || s.save(draft(), snapshot(), 1))
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| **r == Err(CatalogError::Conflict))
                .count(),
            1
        );
    }
    #[test]
    fn invalid_runtime_artifact_combination_and_reference_rejected() {
        let mut d = draft();
        d.runtime_kind = Some("container".into());
        assert_eq!(
            validate_definition(&d, &snapshot()),
            Err(CatalogError::Invalid)
        );
        d = draft();
        d.artifact.as_mut().unwrap().execution_generation = 2;
        assert_eq!(
            validate_definition(&d, &snapshot()),
            Err(CatalogError::Invalid)
        );
        let mut a = snapshot();
        a.executable_sha256 = Some("bad".into());
        assert_eq!(
            validate_definition(&draft(), &a),
            Err(CatalogError::Invalid)
        );
    }
    #[test]
    fn monitoring_cadence_is_typed_and_bounded() {
        for seconds in [0, 86401, u32::MAX] {
            let mut d = draft();
            d.monitoring.as_mut().unwrap().cadence_seconds = seconds;
            assert_eq!(
                validate_definition(&d, &snapshot()),
                Err(CatalogError::Invalid)
            );
        }
        for seconds in [1, 30, 86400] {
            let mut d = draft();
            d.monitoring.as_mut().unwrap().cadence_seconds = seconds;
            assert!(validate_definition(&d, &snapshot()).is_ok());
        }
    }
    #[test]
    fn health_check_requires_supported_mechanism_and_valid_port_timeout() {
        for (kind, port, timeout) in [
            ("exec", 9000, 5000),
            ("tcp_connect", 0, 5000),
            ("http", 80, 0),
        ] {
            let mut d = draft();
            let h = d.health_check.as_mut().unwrap();
            h.kind = kind.into();
            h.port = port;
            h.timeout_ms = timeout;
            assert!(validate_definition(&d, &snapshot()).is_err());
        }
    }
    #[test]
    fn resource_requirements_are_not_allocations_and_invalid_ranges_fail() {
        let d = draft();
        assert!(validate_definition(&d, &snapshot()).is_ok());
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("node_id"));
        assert!(!json.contains("allocated"));
        for cpu in [0, 1000001] {
            let mut d = draft();
            d.resources.cpu_millis = Some(cpu);
            assert!(validate_definition(&d, &snapshot()).is_err());
        }
        let mut d = draft();
        d.resources
            .capabilities
            .push(d.resources.capabilities[0].clone());
        assert!(validate_definition(&d, &snapshot()).is_err());
    }
    #[test]
    fn dedicated_does_not_claim_physical_ownership() {
        let d = draft();
        assert_eq!(d.tenancy, Tenancy::Dedicated);
        assert!(!serde_json::to_string(&d).unwrap().contains("node"));
    }
    #[test]
    fn shared_tenant_aware_magistrala_can_exist_once_without_bindings() {
        let d = composite();
        let s = store();
        s.save(d, None, 0).unwrap();
        assert_eq!(s.list().unwrap().items.len(), 1);
        let c = s.connection().unwrap();
        let tables: i64=c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('tenant_bindings','service_instances','placements','dmonitor_state')", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 0);
    }
    #[test]
    fn shared_stateless_and_system_global_are_declarations_only() {
        for tenancy in [Tenancy::SharedStateless, Tenancy::SystemGlobal] {
            let mut d = draft();
            d.tenancy = tenancy;
            assert!(validate_definition(&d, &snapshot()).is_ok());
        }
    }
    #[test]
    fn tenant_unit_does_not_apply_to_dedicated_service() {
        let mut d = draft();
        d.tenant_unit = Some(TenantUnit::Domain);
        assert!(validate_definition(&d, &snapshot()).is_err());
    }
    #[test]
    fn secret_values_unknown_fields_and_duplicate_dependencies_fail_closed() {
        let mut json = serde_json::to_value(draft()).unwrap();
        json["secrets"] = serde_json::json!([{"secret_ref":"token","version":"v1","target_environment_variable":"TOKEN","value":"plaintext"}]);
        assert!(serde_json::from_value::<ServiceDefinitionDraft>(json).is_err());
        let mut d = draft();
        d.exact_dependencies = vec![
            DefinitionReference {
                definition_id: "mqtt".into(),
                revision: 1
            };
            2
        ];
        assert!(validate_definition(&d, &snapshot()).is_err());
    }
    #[test]
    fn corrupt_history_rejected_and_unrelated_database_not_initialized() {
        let s = store();
        let one = s.save(draft(), snapshot(), 0).unwrap();
        let mut corrupt = one.clone();
        corrupt.definition.name = "tampered".into();
        s.writable_connection()
            .unwrap()
            .execute(
                "UPDATE catalog_definitions SET body=?",
                [serde_json::to_string(&corrupt).unwrap()],
            )
            .unwrap();
        assert_eq!(s.get("p4-sniffer", None), Err(CatalogError::Corrupt));
        let dir =
            std::env::temp_dir().join(format!("datum-catalog-unrelated-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("other.db");
        Connection::open(&path)
            .unwrap()
            .execute("CREATE TABLE unrelated(id TEXT)", [])
            .unwrap();
        assert!(CatalogStore::initialize(&path).is_err());
    }
    #[test]
    fn missing_invalid_revisions_and_explicit_path_fail_safely() {
        let s = store();
        assert_eq!(s.get("unknown", None), Err(CatalogError::NotFound));
        assert_eq!(s.get("p4-sniffer", Some(0)), Err(CatalogError::Invalid));
        assert!(CatalogStore::initialize(Path::new("relative.db")).is_err());
    }
    pub fn database_path(s: &CatalogStore) -> &Path {
        &s.path
    }
    pub fn composite() -> ServiceDefinitionDraft {
        let mut d = draft();
        d.service_definition_id = "magistrala".into();
        d.name = "Magistrala".into();
        d.artifact = None;
        d.runtime_kind = None;
        d.service_shape = ServiceShape::Composite;
        d.resources = Requirements::default();
        d.health_check = None;
        d.tenancy = Tenancy::SharedTenantAware;
        d.tenant_unit = Some(TenantUnit::Domain);
        d.composition = Some(Composition::Discovered {
            discovery: Discovery::DockerCompose {
                compose_files: vec!["docker/docker-compose.yaml".into()],
            },
        });
        d.provisioning = Some(Provisioning::RepositoryCommand {
            repository_url: "https://github.com/absmach/magistrala.git".into(),
            commit: "a".repeat(40),
            command: vec!["make".into(), "run_latest".into()],
        });
        d
    }
    #[test]
    fn atomic_composite_and_discovery_contracts_are_strict() {
        assert!(validate_definition(&draft(), &snapshot()).is_ok());
        assert!(validate_definition(&composite(), None).is_ok());
        let mut d = draft();
        d.composition = composite().composition;
        assert!(validate_definition(&d, &snapshot()).is_err());
        let mut d = composite();
        d.composition = None;
        assert!(validate_definition(&d, None).is_err());
        let mut d = composite();
        d.provisioning = None;
        assert!(validate_definition(&d, None).is_err());
        for commit in ["main", "latest", "v1.0"] {
            let mut d = composite();
            if let Some(Provisioning::RepositoryCommand { commit: c, .. }) = &mut d.provisioning {
                *c = commit.into();
            }
            assert!(validate_definition(&d, None).is_err());
        }
        for file in ["../compose.yaml", "/compose.yaml", "C:\\compose.yaml"] {
            let mut d = composite();
            d.composition = Some(Composition::Discovered {
                discovery: Discovery::DockerCompose {
                    compose_files: vec![file.into()],
                },
            });
            assert!(validate_definition(&d, None).is_err());
        }
    }
    #[test]
    fn repository_urls_reject_raw_malformed_text_without_normalizing_or_persisting() {
        let valid = vec!["https://github.com/absmach/magistrala.git".to_string(), "https://github.com/org/repo".into(), "https://example.com/a/b/c/repo.git".into(), format!("https://example.com/{}", "a".repeat(490))];
        let mut invalid: Vec<String> = vec![" https://github.com/org/repo", "https://github.com/org/repo ", "https://github.com/org /repo", "https://user@example.com/repo", "https://user:token@example.com/repo", "http://example.com/repo", "file:///repo", "ssh://example.com/repo", "git@example.com:repo", "https://example.com/repo?q=1", "https://example.com/repo#x", "https://"].into_iter().map(String::from).collect();
        for c in ['\n', '\r', '\t', '\0', '\u{1}', '\u{7f}', '\u{85}', '\u{a0}'] { invalid.push(format!("https://github.com/org/{c}repo")); }
        invalid.push(format!("https://example.com/{}", "a".repeat(513)));
        for url in valid.iter().chain(invalid.iter()) {
            let mut d = composite();
            if let Some(Provisioning::RepositoryCommand { repository_url, .. }) = &mut d.provisioning { *repository_url = url.clone(); }
            let original = d.clone();
            assert_eq!(validate_definition(&d, None).is_ok(), valid.contains(url), "{url:?}");
            assert_eq!(d, original);
        }
        let s = store();
        for url in invalid {
            let mut d = composite();
            if let Some(Provisioning::RepositoryCommand { repository_url, .. }) = &mut d.provisioning { *repository_url = url; }
            assert_eq!(s.save(d, None, 0), Err(CatalogError::Invalid));
            assert!(s.list().unwrap().items.is_empty());
        }
    }
    #[test]
    fn static_components_and_dependencies_resolve_exact_existing_revisions() {
        let s = store();
        s.save(draft(), snapshot(), 0).unwrap();
        let mut d = composite();
        d.provisioning = None;
        d.composition = Some(Composition::Static {
            components: vec![ComponentReference {
                key: "sniffer".into(),
                definition: DefinitionReference {
                    definition_id: "p4-sniffer".into(),
                    revision: 1,
                },
            }],
        });
        d.requires = vec![ServiceRequirement {
            capability: "database.postgresql".into(),
        }];
        s.save(d.clone(), None, 0).unwrap();
        assert_eq!(s.get("magistrala", None).unwrap().definition, d);
        d.service_definition_id = "another-composite".into();
        if let Some(Composition::Static { components }) = &mut d.composition {
            components[0].definition.revision = 2;
        }
        assert_eq!(s.save(d, None, 0), Err(CatalogError::Invalid));
        let mut d = draft();
        d.service_definition_id = "api".into();
        d.exact_dependencies = vec![DefinitionReference {
            definition_id: "absent".into(),
            revision: 1,
        }];
        assert_eq!(s.save(d.clone(), snapshot(), 0), Err(CatalogError::Invalid));
        d.exact_dependencies = vec![DefinitionReference {
            definition_id: "api".into(),
            revision: 1,
        }];
        assert_eq!(s.save(d, snapshot(), 0), Err(CatalogError::Invalid));
    }
    #[test]
    fn exact_dependency_history_survives_target_new_revision() {
        let s = store();
        let mut b = draft();
        b.service_definition_id = "b".into();
        for previous in 0..4 {
            s.save(b.clone(), snapshot(), previous).unwrap();
        }
        let mut a = draft();
        a.service_definition_id = "a".into();
        s.save(a.clone(), snapshot(), 0).unwrap();
        a.exact_dependencies = vec![DefinitionReference {
            definition_id: "b".into(),
            revision: 4,
        }];
        let pinned = s.save(a.clone(), snapshot(), 1).unwrap();
        s.save(b, snapshot(), 4).unwrap();
        assert_eq!(s.get("a", Some(2)).unwrap(), pinned);
        assert_eq!(pinned.definition.exact_dependencies[0].revision, 4);
        a.exact_dependencies[0].revision = 5;
        s.save(a, snapshot(), 2).unwrap();
        assert_eq!(s.get("a", Some(2)).unwrap(), pinned);
    }
    #[test]
    fn peer_dependencies_do_not_imply_startup_order_or_latest() {
        let s = store();
        let mut a = draft();
        a.service_definition_id = "a".into();
        s.save(a.clone(), snapshot(), 0).unwrap();
        let mut b = draft();
        b.service_definition_id = "b".into();
        b.exact_dependencies = vec![DefinitionReference {
            definition_id: "a".into(),
            revision: 1,
        }];
        s.save(b, snapshot(), 0).unwrap();
        a.exact_dependencies = vec![DefinitionReference {
            definition_id: "b".into(),
            revision: 1,
        }];
        s.save(a, snapshot(), 1).unwrap();
        assert_eq!(
            s.get("b", Some(1)).unwrap().definition.exact_dependencies[0].revision,
            1
        );
    }
    #[test]
    fn software_capabilities_are_unresolved_and_separate_from_resources() {
        let s = store();
        let mut p = draft();
        p.service_definition_id = "postgresql".into();
        p.provides = vec!["database.postgresql".into()];
        s.save(p, snapshot(), 0).unwrap();
        let mut coffee = draft();
        coffee.service_definition_id = "smart-coffee-api".into();
        coffee.provides = vec!["api.smart-coffee".into()];
        coffee.requires = vec![
            ServiceRequirement {
                capability: "database.postgresql".into(),
            },
            ServiceRequirement {
                capability: "messaging.mqtt".into(),
            },
        ];
        let one = s.save(coffee.clone(), snapshot(), 0).unwrap();
        assert_eq!(
            one.definition.resources.capabilities[0].kind,
            CapabilityKind::NetworkInterface
        );
        assert!(one.definition.exact_dependencies.is_empty());
        // A gateway may consume and expose the same capability; no auto-binding.
        coffee.provides.push("database.postgresql".into());
        assert!(validate_definition(&coffee, &snapshot()).is_ok());
        for bad in [
            "postgres",
            "Database.postgresql",
            "database..postgresql",
            "database.postgres_ql",
        ] {
            let mut d = draft();
            d.provides = vec![bad.into()];
            assert!(validate_definition(&d, &snapshot()).is_err());
        }
        let mut d = coffee.clone();
        d.requires.push(d.requires[0].clone());
        assert!(validate_definition(&d, &snapshot()).is_err());
        let mut d = coffee;
        d.provides.push(d.provides[0].clone());
        assert!(validate_definition(&d, &snapshot()).is_err());
    }
    #[test]
    fn composite_history_digest_and_no_execution_authority() {
        let s = store();
        let mut d = draft();
        d.service_definition_id = "magistrala".into();
        let one = s.save(d, snapshot(), 0).unwrap();
        let two = s.save(composite(), None, 1).unwrap();
        assert_ne!(one.digest, two.digest);
        assert_eq!(s.get("magistrala", Some(1)).unwrap(), one);
        assert!(two.artifact_snapshot_id.is_none());
        let json = serde_json::to_value(&two).unwrap();
        assert_eq!(
            json["definition"]["provisioning"]["command"],
            serde_json::json!(["make", "run_latest"])
        );
        assert_eq!(
            json["definition"]["composition"]["discovery"]["kind"],
            "docker_compose"
        );
        let c = s.connection().unwrap();
        let tables: Vec<String> = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(tables, vec!["catalog_artifacts", "catalog_definitions"]);
        // A catalog save has no executor handle or side effects outside these rows.
        let source = include_str!("service_catalog.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for forbidden in [
            "Command::new",
            "docker compose",
            "ServiceInstance",
            "ComponentInstance",
            "RequirementBinding",
            "TenantBinding",
            "DServRealization",
        ] {
            assert!(!source.contains(forbidden));
        }
    }
    #[test]
    fn first_bootstrap_and_existing_open_do_not_mutate_schema() {
        let s = store();
        assert!(s.path.exists());
        assert!(s.list().unwrap().items.is_empty());
        let before = std::fs::read(&s.path).unwrap();
        CatalogStore::initialize(&s.path).unwrap().list().unwrap();
        assert_eq!(std::fs::read(&s.path).unwrap(), before);
    }
    #[test]
    fn missing_tables_fail_closed_without_repair_or_empty_success() {
        for table in ["catalog_definitions", "catalog_artifacts"] {
            let s = store();
            s.writable_connection()
                .unwrap()
                .execute(&format!("DROP TABLE {table}"), [])
                .unwrap();
            assert!(matches!(
                CatalogStore::initialize(&s.path),
                Err(CatalogError::Corrupt)
            ));
            assert!(matches!(s.list(), Err(CatalogError::Corrupt)));
            assert_eq!(s.save(draft(), snapshot(), 0), Err(CatalogError::Corrupt));
            let c = Connection::open_with_flags(&s.path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let found: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name=?",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 0);
        }
    }
    #[test]
    fn incompatible_version_columns_and_constraints_fail_closed() {
        for damage in ["PRAGMA user_version=0", "PRAGMA user_version=999", "DROP TABLE catalog_definitions; CREATE TABLE catalog_definitions(id TEXT,revision INTEGER,body TEXT)", "DROP TABLE catalog_artifacts; CREATE TABLE catalog_artifacts(id TEXT PRIMARY KEY,body INTEGER NOT NULL)"] {
            let s = store();
            s.writable_connection().unwrap().execute_batch(damage).unwrap();
            assert!(matches!(CatalogStore::initialize(&s.path), Err(CatalogError::Corrupt)));
            assert!(matches!(s.list(), Err(CatalogError::Corrupt)));
        }
    }
    #[test]
    fn removed_database_reads_and_writes_never_recreate_it() {
        let s = store();
        std::fs::remove_file(&s.path).unwrap();
        assert!(matches!(s.list(), Err(CatalogError::Unavailable)));
        assert_eq!(s.get("p4-sniffer", None), Err(CatalogError::Unavailable));
        assert_eq!(s.artifact("snapshot"), Err(CatalogError::Unavailable));
        assert_eq!(
            s.save(draft(), snapshot(), 0),
            Err(CatalogError::Unavailable)
        );
        assert!(!s.path.exists());
    }
    #[test]
    fn concurrent_same_process_bootstrap_has_one_complete_schema() {
        let s = store();
        std::fs::remove_file(&s.path).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = s.path.clone();
                std::thread::spawn(move || CatalogStore::initialize(&path))
            })
            .collect();
        for h in handles {
            assert!(h.join().unwrap().unwrap().list().unwrap().items.is_empty());
        }
    }
}
