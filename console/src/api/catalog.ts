import type { ArtifactSnapshot, CatalogPage, DefinitionRevision, ServiceDefinition } from "../types/catalog.ts";
export class CatalogError extends Error { code: string; constructor(code: string, message: string) { super(message); this.code = code; } }
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && v.length <= 256 && v.trim() === v && !/[\u0000-\u001f\u007f]/u.test(v);
const integer = (v: unknown, max: number) => typeof v === "number" && Number.isSafeInteger(v) && v >= 1 && v <= max;
const digest = (v: unknown) => typeof v === "string" && /^sha256:[a-f0-9]{64}$/u.test(v);
const reference = (v: unknown) => object(v) && text(v.project_id) && text(v.artifact_id) && integer(v.execution_generation, Number.MAX_SAFE_INTEGER) && digest(v.execution_projection_digest);
export function validArtifactSnapshot(v: unknown): v is ArtifactSnapshot {
  if (!object(v) || !reference(v.source)) return false;
  if (v.runtime_kind === "native_process") return v.image === null && v.resolved_image_identity === null && typeof v.executable_sha256 === "string" && /^[a-f0-9]{64}$/u.test(v.executable_sha256);
  if (v.runtime_kind !== "container" || !text(v.image) || v.executable_sha256 !== null || !object(v.resolved_image_identity)) return false;
  const p = v.resolved_image_identity;
  if (!text(p.repository) || !(p.index_digest === undefined || p.index_digest === null || digest(p.index_digest)) || !Array.isArray(p.platforms) || !p.platforms.length) return false;
  const seen = new Set<string>();
  return p.platforms.every(platform => {
    if (!object(platform) || !text(platform.os) || !text(platform.architecture) || !(platform.variant === undefined || platform.variant === null || text(platform.variant)) || !digest(platform.manifest_digest) || !digest(platform.config_digest) || platform.execution_reference !== `${p.repository}@${platform.manifest_digest}`) return false;
    const key = JSON.stringify([platform.os, platform.architecture, platform.variant ?? null]);
    if (seen.has(key)) return false;
    seen.add(key); return true;
  });
}
function validDefinition(v: unknown): boolean {
  if (!object(v) || !text(v.service_definition_id) || !/^[A-Za-z0-9._-]{1,128}$/u.test(v.service_definition_id) || [".",".."].includes(v.service_definition_id) || !text(v.name) || !validStructure(v) ||
      !["dedicated","shared_tenant_aware","shared_stateless","system_global"].includes(String(v.tenancy)) ||
      !(v.tenant_unit === null || (v.tenancy === "shared_tenant_aware" && ["domain","namespace","vhost","schema","organization"].includes(String(v.tenant_unit)))) || !object(v.resources)) return false;
  const r=v.resources;
  if (!["cpu_millis","memory_mb","storage_mb"].every((key,i)=>r[key]===null || integer(r[key],[1000000,16777216,1000000000][i]!)) || !Array.isArray(r.capabilities) || r.capabilities.length>32 ||
      !r.capabilities.every(c=>object(c)&&["network_interface","device","p4_port","gpu"].includes(String(c.kind))&&typeof c.exclusive==="boolean")) return false;
  if (v.monitoring !== null && (!object(v.monitoring)||v.monitoring.subject_kind!=="dserv_realization"||!integer(v.monitoring.cadence_seconds,86400))) return false;
  if (v.health_check !== null) {
    const h=v.health_check;
    if (!object(h)||!["http","tcp_connect"].includes(String(h.kind))||!text(h.target)||!integer(h.port,65535)||!integer(h.timeout_ms,60000)||!(h.path===null||typeof h.path==="string")) return false;
  }
  return Array.isArray(v.secrets)&&v.secrets.length<=64&&v.secrets.every(s=>object(s)&&text(s.secret_ref)&&text(s.version)&&text(s.target_environment_variable)&&Object.keys(s).every(k=>["secret_ref","version","target_environment_variable"].includes(k)))&&
    true;
}
function validStructure(v: Record<string,unknown>): boolean {
  const id=(s:unknown)=>typeof s==="string"&&/^[A-Za-z0-9._-]{1,128}$/u.test(s)&&![".",".."].includes(s);
  const ref=(r:unknown)=>object(r)&&id(r.definition_id)&&r.definition_id!==v.service_definition_id&&integer(r.revision,1000000);
  const cap=(s:unknown)=>typeof s==="string"&&s.length<=128&&/^[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)+$/u.test(s);
  if (!Array.isArray(v.exact_dependencies)||v.exact_dependencies.length>64||!v.exact_dependencies.every(ref)||new Set(v.exact_dependencies.map(r=>JSON.stringify([r.definition_id,r.revision]))).size!==v.exact_dependencies.length||!Array.isArray(v.requires)||!Array.isArray(v.provides)||v.requires.length>64||v.provides.length>64||!v.requires.every(r=>object(r)&&cap(r.capability))||!v.provides.every(cap)||new Set(v.requires.map(r=>r.capability)).size!==v.requires.length||new Set(v.provides).size!==v.provides.length) return false;
  if (v.service_shape==="atomic") return reference(v.artifact)&&["container","native_process"].includes(String(v.runtime_kind))&&v.composition===null&&v.provisioning===null;
  if (v.service_shape!=="composite"||v.artifact!==null||v.runtime_kind!==null||!object(v.composition))return false;
  const c=v.composition;
  if(c.mode==="static")return v.provisioning===null&&Array.isArray(c.components)&&c.components.length>0&&c.components.length<=64&&c.components.every(m=>object(m)&&id(m.key)&&ref(m.definition))&&new Set(c.components.map(m=>m.key)).size===c.components.length;
  if(c.mode!=="discovered"||!object(c.discovery)||c.discovery.kind!=="docker_compose"||!Array.isArray(c.discovery.compose_files)||!c.discovery.compose_files.length||c.discovery.compose_files.length>16||!c.discovery.compose_files.every(s=>text(s)&&!s.includes("\\")&&!s.includes(":")&&s.split("/").every(p=>p!==""&&p!=="."&&p!==".."))||new Set(c.discovery.compose_files).size!==c.discovery.compose_files.length||!object(v.provisioning))return false;
  const p=v.provisioning;
  if(p.kind!=="repository_command"||!validRepositoryUrl(p.repository_url)||typeof p.commit!=="string"||!/^[a-f0-9]{40}$/u.test(p.commit)||!Array.isArray(p.command)||!p.command.length||p.command.length>32||!p.command.every(text))return false;
  try{const u=new URL(p.repository_url);return u.protocol==="https:"&&!u.username&&!u.password&&!u.search&&!u.hash;}catch{return false;}
}
export function validRepositoryUrl(value: unknown): value is string {
  if (typeof value !== "string" || !value.length || new TextEncoder().encode(value).length > 512 || /[\s\u0000-\u001f\u007f-\u009f]/u.test(value)) return false;
  try { const url = new URL(value); return url.protocol === "https:" && !!url.hostname && !url.username && !url.password && !url.search && !url.hash; } catch { return false; }
}
export function validCatalogRevision(v: unknown): v is DefinitionRevision {
  return object(v) && v.scope === "global_catalog" && Number.isSafeInteger(v.revision) && Number(v.revision) > 0 &&
    typeof v.digest === "string" && /^sha256:[a-f0-9]{64}$/u.test(v.digest) &&
    (v.artifact_snapshot_id===null ? object(v.definition)&&v.definition.service_shape==="composite" : digest(v.artifact_snapshot_id)&&object(v.definition)&&v.definition.service_shape==="atomic") &&
    typeof v.created_at_utc === "string" && Number.isFinite(Date.parse(v.created_at_utc)) &&
    validDefinition(v.definition);
}
export class CatalogClient {
  private signal?: AbortSignal;
  constructor(signal?: AbortSignal) { this.signal = signal; }
  private async request(path: string, definition?: ServiceDefinition, previous?: number): Promise<unknown> {
    let response: Response;
    try { response = await fetch(`/api/v1/catalog/${path}`, { signal: this.signal,
      method: definition ? "POST" : "GET", headers: definition ? {"Content-Type": "application/json"} : {},
      body: definition ? JSON.stringify({ definition, expected_revision: previous }) : undefined }); }
    catch (e) { if (this.signal?.aborted) throw e; throw new CatalogError("network_error", "Catalog could not be reached."); }
    const text = await response.text();
    if (text.length > 2_000_000) throw new CatalogError("invalid_response", "Catalog response could not be displayed.");
    let body: unknown;
    try { body = JSON.parse(text); } catch { throw new CatalogError("invalid_response", "Catalog response could not be displayed."); }
    if (!response.ok) {
      const code = object(body) && object(body.error) && typeof body.error.code === "string" ? body.error.code : "catalog_unavailable";
      const messages: Record<string,string> = { invalid_definition: "Definition is invalid. Check structure, exact revisions, capability IDs, source pins, resources, cadence, health and tenancy.", revision_conflict: "The definition or source artifact changed. Reload before saving.", not_found: "Catalog record is not available.", catalog_corrupt: "Catalog integrity cannot be verified." };
      throw new CatalogError(code, messages[code] ?? "Catalog persistence is not configured or cannot be accessed.");
    }
    return body;
  }
  async list(): Promise<CatalogPage> {
    const body = await this.request("services");
    if (!object(body) || !Array.isArray(body.items) || body.items.length > 100 || !body.items.every(validCatalogRevision) || typeof body.has_more !== "boolean" || body.limit !== 100) throw new CatalogError("invalid_response", "Catalog response could not be displayed.");
    return body as unknown as CatalogPage;
  }
  async artifacts(): Promise<{ items: ArtifactSnapshot[]; has_more: boolean }> {
    const body = await this.request("artifacts");
    if (!object(body) || body.source !== "pinned_operational_artifact_references" || body.limit !== 100 || typeof body.has_more !== "boolean" || !Array.isArray(body.items) || body.items.length > 100 || !body.items.every(validArtifactSnapshot)) throw new CatalogError("invalid_response", "Artifact references could not be displayed.");
    return { items: body.items as ArtifactSnapshot[], has_more: body.has_more as boolean };
  }
  async snapshot(id: string): Promise<ArtifactSnapshot> {
    if (!digest(id)) throw new CatalogError("invalid_response", "Artifact snapshot identity is invalid.");
    const body = await this.request(`artifacts/${encodeURIComponent(id)}`);
    if (!validArtifactSnapshot(body)) throw new CatalogError("invalid_response", "Artifact snapshot could not be displayed.");
    return body;
  }
  async get(id: string, revision?: number): Promise<DefinitionRevision> {
    const body = await this.request(`services/${encodeURIComponent(id)}${revision === undefined ? "" : `/revisions/${revision}`}`);
    if (!validCatalogRevision(body) || body.definition.service_definition_id !== id || (revision !== undefined && body.revision !== revision)) throw new CatalogError("invalid_response", "Catalog response could not be displayed.");
    return body;
  }
  async save(definition: ServiceDefinition, previous: number): Promise<DefinitionRevision> {
    const body = await this.request(previous ? `services/${encodeURIComponent(definition.service_definition_id)}/revisions` : "services", definition, previous);
    if (!validCatalogRevision(body) || body.definition.service_definition_id !== definition.service_definition_id || body.revision !== previous + 1) throw new CatalogError("invalid_response", "Saved revision could not be verified. Reload before retrying.");
    return body;
  }
}
