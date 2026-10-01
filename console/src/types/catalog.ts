export interface ArtifactReference { project_id: string; artifact_id: string; execution_generation: number; execution_projection_digest: string }
export interface ArtifactSnapshot { source: ArtifactReference; runtime_kind: "container" | "native_process"; image: string | null; resolved_image_identity: { repository: string; index_digest?: string | null; platforms: { os: string; architecture: string; variant?: string | null; manifest_digest: string; config_digest: string; execution_reference: string }[] } | null; executable_sha256: string | null }
export type Tenancy = "dedicated" | "shared_tenant_aware" | "shared_stateless" | "system_global";
export interface DefinitionReference { definition_id: string; revision: number }
export type Composition = { mode: "static"; components: {key:string;definition:DefinitionReference}[] } | {mode:"discovered";discovery:{kind:"docker_compose";compose_files:string[]}};
export interface Provisioning {kind:"repository_command";repository_url:string;commit:string;command:string[]}
export interface ServiceDefinition {
  service_definition_id: string; name: string; artifact: ArtifactReference | null; runtime_kind: "container" | "native_process" | null;
  service_shape:"atomic"|"composite"; composition:Composition|null; provisioning:Provisioning|null;
  tenancy: Tenancy; tenant_unit: "domain" | "namespace" | "vhost" | "schema" | "organization" | null;
  resources: { cpu_millis: number | null; memory_mb: number | null; storage_mb: number | null; capabilities: { kind: "network_interface" | "device" | "p4_port" | "gpu"; exclusive: boolean }[] };
  health_check: { kind: "http" | "tcp_connect"; target: string; port: number; path: string | null; timeout_ms: number } | null;
  monitoring: { subject_kind: "dserv_realization"; cadence_seconds: number } | null;
  secrets: { secret_ref: string; version: string; target_environment_variable: string }[]; exact_dependencies:DefinitionReference[];requires:{capability:string}[];provides:string[];
}
export interface DefinitionRevision { definition: ServiceDefinition; revision: number; digest: string; created_at_utc: string; artifact_snapshot_id: string | null; scope: "global_catalog" }
export interface CatalogPage { items: DefinitionRevision[]; has_more: boolean; limit: number }
