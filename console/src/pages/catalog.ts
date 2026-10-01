import { el, badge } from "../components/presentation.ts";
import { CatalogClient, CatalogError, validRepositoryUrl } from "../api/catalog.ts";
import type { DefinitionRevision, ServiceDefinition, ArtifactSnapshot, Tenancy } from "../types/catalog.ts";
const tenancyNames: Record<Tenancy,string> = { dedicated: "Dedicated", shared_tenant_aware: "Shared tenant-aware", shared_stateless: "Shared stateless", system_global: "System/global" };
function artifactDetails(snapshot: ArtifactSnapshot, snapshotId?: string): HTMLElement {
  const details = el("div", "catalog-pin-details"); details.dataset.artifactPins = "true";
  details.append(el("h3", "section-title", "Pinned artifact identity · read-only"));
  const values = el("dl", "small");
  const add = (label: string, value: string) => { const full = el("code", "entity-identity", value); values.append(el("dt", "", label), el("dd")); values.lastElementChild!.append(full); };
  add("Artifact snapshot ID", snapshotId ?? "Captured on save; selected source pin shown below");
  add("Source project (provenance only)", snapshot.source.project_id); add("Source artifact", snapshot.source.artifact_id);
  add("Source execution generation", String(snapshot.source.execution_generation)); add("Execution projection digest", snapshot.source.execution_projection_digest);
  add("Runtime kind", snapshot.runtime_kind);
  if (snapshot.runtime_kind === "native_process") add("Native executable SHA-256", snapshot.executable_sha256!);
  else {
    const p = snapshot.resolved_image_identity!;
    add("Image reference (metadata)", snapshot.image!); add("Repository", p.repository);
    if (p.index_digest) add("OCI index digest", p.index_digest);
    for (const platform of p.platforms) {
      add("Platform", [platform.os, platform.architecture, platform.variant].filter(Boolean).join("/"));
      add("Execution reference", platform.execution_reference); add("Manifest digest", platform.manifest_digest); add("Config digest", platform.config_digest);
    }
  }
  details.append(values); return details;
}
export function catalogView(client: CatalogClient, state?: (ready: boolean) => void): HTMLElement {
  const page = el("div", "catalog-workspace");
  page.append(el("h1", "page-title", "Artifact & Service Catalog"), el("p", "text-secondary", "Global reusable definitions · not project usage, running instances or deployment authority."));
  const notice = el("p", "small", "Only catalog revisions can be saved here. Artifact pinning remains with the existing registry. No deployment, placement or tenant provisioning occurs.");
  const status = el("div", "catalog-status"); status.setAttribute("role", "status");
  const list = el("section", "console-card"), editor = el("section", "console-card");
  const layout = el("div", "catalog-grid"); layout.append(list, editor); page.append(notice, status, layout);
  const message = (error: unknown) => error instanceof CatalogError ? error.message : "Catalog could not be displayed.";
  let artifacts: ArtifactSnapshot[] = [], busy = false, formGeneration = 0;
  const formView = (existing?: DefinitionRevision, historical = false) => {
    const generation = ++formGeneration;
    editor.replaceChildren(el("h2", "section-title", historical ? `Historical revision ${existing?.revision} · read-only` : existing ? `New revision after ${existing.revision}` : "Create service definition"));
    const form = el("form", "catalog-form"); form.dataset.catalogForm = "true";
    const error = el("div", "catalog-error"); error.setAttribute("role", "alert"); error.id = "catalog-validation-error"; error.tabIndex = -1;
    const fields = el("fieldset"); fields.disabled = historical;
    function field(label: string, name: string, value = "", type = "text", required = false): HTMLInputElement {
      const wrap = el("div", "mb-3"), input = el("input", "form-control"); input.type = type; input.name = name; input.id = `catalog-${name}`; input.value = value; input.required = required;
      input.setAttribute("aria-describedby", error.id); input.maxLength = 256;
      const text = el("label", "form-label", label); text.htmlFor = input.id; wrap.append(text, input); fields.append(wrap); return input;
    }
    function select(label: string, name: string, options: [string,string][], value = ""): HTMLSelectElement {
      const wrap = el("div", "mb-3"), input = el("select", "form-select"); input.name = name; input.id = `catalog-${name}`; input.setAttribute("aria-describedby", error.id);
      const text = el("label", "form-label", label); text.htmlFor = input.id;
      for (const [id, title] of options) { const o = el("option", "", title); o.value = id; input.append(o); }
      input.value = value; wrap.append(text, input); fields.append(wrap); return input;
    }
    const id = field("Definition ID", "id", existing?.definition.service_definition_id ?? "", "text", true); id.pattern = "[A-Za-z0-9._-]+"; id.maxLength = 128; id.readOnly = !!existing;
    field("Name", "name", existing?.definition.name ?? "", "text", true);
    fields.append(el("h3", "section-title", "Service structure"));
    const shape=select("Service shape", "shape", [["atomic","Atomic"],["composite","Composite"]],existing?.definition.service_shape??"atomic");
    const composition=existing?.definition.composition;
    const mode=select("Composition mode", "composition-mode", [["static","Static"],["discovered","Discovered"]],composition?.mode??"static");
    const member=composition?.mode==="static"?composition.components[0]:undefined;
    const key=field("Static component key", "component-key",member?.key??"");
    const memberId=field("Component definition ID", "component-id",member?.definition.definition_id??"");
    const memberRevision=field("Exact component revision", "component-revision",String(member?.definition.revision??""),"number");memberRevision.min="1";memberRevision.max="1000000";
    const composeFiles=field("Docker Compose files (JSON array, repository-relative)","compose-files",JSON.stringify(composition?.mode==="discovered"?composition.discovery.compose_files:["docker/docker-compose.yaml"]));composeFiles.maxLength=8192;
    const provisioning=existing?.definition.provisioning;
    const repository=field("Repository URL (HTTPS; modeling only)","repository",provisioning?.repository_url??"");repository.maxLength=512;
    const commit=field("Immutable source commit (40 lowercase hex; no branch/latest)","commit",provisioning?.commit??"");
    const command=field("Repository-root command argv (JSON array; NOT executed)","command",JSON.stringify(provisioning?.command??["make","run_latest"]));command.maxLength=8192;
    fields.append(el("p","small text-secondary","Composition is expected structure, not observed components. Docker Compose discovery and repository commands are declarations only; no cloning, parsing or execution occurs."));
    const choices = artifacts.map((a, i): [string,string] => [String(i), `${a.source.project_id} / ${a.source.artifact_id} · ${a.runtime_kind} · generation ${a.source.execution_generation}`]);
    let initial = existing?.definition.artifact ? artifacts.findIndex(a => JSON.stringify(a.source) === JSON.stringify(existing.definition.artifact)) : 0;
    if (existing && initial < 0) {
      // Retain identity, not a guessed replacement. Save will conflict if source changed.
      choices.push(["historical", "Referenced historical artifact · reload/reselect before a new save"]);
    }
    const artifact = historical ? el("select") : select("Pinned artifact reference (source scope ≠ catalog ownership)", "artifact", choices, initial < 0 ? "historical" : String(initial));
    artifact.required = true;
    const runtime = field("Runtime kind · inherited from artifact", "runtime", existing?.definition.runtime_kind ?? artifacts[initial]?.runtime_kind ?? "Not available"); runtime.readOnly = true;
    const pins = el("section", "catalog-pin-details"); fields.append(pins);
    let pinGeneration = 0;
    const selectedPins = () => { ++pinGeneration; const source = artifacts[Number(artifact.value)]; pins.replaceChildren(source ? artifactDetails(source) : el("p", "small", "Select a currently pinned artifact before a new save.")); };
    artifact.addEventListener("change", () => { runtime.value = artifacts[Number(artifact.value)]?.runtime_kind ?? existing?.definition.runtime_kind ?? "Not available"; selectedPins(); });
    if (existing?.artifact_snapshot_id) {
      const requestedPin = ++pinGeneration;
      pins.append(el("p", "small", "Loading exact captured artifact snapshot…"));
      void client.snapshot(existing.artifact_snapshot_id).then(snapshot => {
        const expected = existing.definition.artifact!, actual = snapshot.source;
        if (actual.project_id !== expected.project_id || actual.artifact_id !== expected.artifact_id || actual.execution_generation !== expected.execution_generation || actual.execution_projection_digest !== expected.execution_projection_digest || snapshot.runtime_kind !== existing.definition.runtime_kind) throw new CatalogError("invalid_response", "Captured artifact scope could not be verified.");
        if (generation === formGeneration && requestedPin === pinGeneration) pins.replaceChildren(artifactDetails(snapshot, existing.artifact_snapshot_id!));
      }).catch(e => { if (generation === formGeneration && requestedPin === pinGeneration) pins.replaceChildren(el("p", "catalog-error", message(e))); });
    } else selectedPins();
    fields.append(el("p", "small text-secondary", "Atomic definitions import pinned container/native artifacts. Composite definitions declare component structure instead of one root runtime. D-Code and Management WASM remain separate contracts. Atomic commands, ports, mounts and configuration stay in their existing artifact contract."));
    const tenancy = select("Tenancy capability", "tenancy", Object.entries(tenancyNames), existing?.definition.tenancy ?? "dedicated");
    const unit = select("Optional tenant unit", "tenant-unit", [["", "Not included"], ...["domain", "namespace", "vhost", "schema", "organization"].map(v => [v,v] as [string,string])], existing?.definition.tenant_unit ?? "");
    const updateUnit = () => { unit.disabled = historical || tenancy.value !== "shared_tenant_aware"; if (tenancy.value !== "shared_tenant_aware") unit.value = ""; }; tenancy.addEventListener("change", updateUnit); updateUnit();
    fields.append(el("p", "small text-secondary", "Dedicated does not reserve a node. Sharing modes declare capability only; no shared instance or tenant binding is created."));
    for (const [key, title, max] of [["cpu_millis", "CPU requirement (millicores)", 1000000], ["memory_mb", "Memory requirement (MB)", 16777216], ["storage_mb", "Storage requirement (MB)", 1000000000]] as const) {
      const input = field(title, key, String(existing?.definition.resources[key] ?? ""), "number"); input.min = "1"; input.max = String(max); input.step = "1";
    }
    const capability = select("Capability requirement", "capability", [["", "None declared"], ...["network_interface", "device", "p4_port", "gpu"].map(v => [v,v] as [string,string])], existing?.definition.resources.capabilities[0]?.kind ?? "");
    const exclusive = select("Requirement exclusivity (not allocation)", "exclusive", [["false", "No exclusivity declared"], ["true", "Exclusive capability required"]], String(existing?.definition.resources.capabilities[0]?.exclusive ?? false));
    const health = select("Health-check mechanism", "health", [["", "Not configured"], ["http", "HTTP probe"], ["tcp_connect", "TCP connection probe"]], existing?.definition.health_check?.kind ?? "");
    field("Health target host (no URL/credentials)", "health-target", existing?.definition.health_check?.target ?? "");
    const port = field("Health port", "health-port", String(existing?.definition.health_check?.port ?? ""), "number"); port.min = "1"; port.max = "65535";
    field("HTTP probe path", "health-path", existing?.definition.health_check?.path ?? "/");
    const timeout = field("Health timeout (milliseconds)", "health-timeout", String(existing?.definition.health_check?.timeout_ms ?? 5000), "number"); timeout.min = "1"; timeout.max = "60000";
    const cadence = field("DMonitor realization evidence cadence (seconds; blank = not configured)", "cadence", String(existing?.definition.monitoring?.cadence_seconds ?? ""), "number"); cadence.min = "1"; cadence.max = "86400"; cadence.step = "1";
    fields.append(el("p", "small text-secondary", "Health and monitoring are separate declarative intent. Saving does not wire a collector, scheduler or health probe."));
    fields.append(el("h3","section-title","Exact dependencies"));
    const dep=existing?.definition.exact_dependencies[0];
    const depId=field("Dependency definition ID", "dependency-id",dep?.definition_id??"");
    const depRevision=field("Exact dependency revision", "dependency-revision",String(dep?.revision??""),"number");depRevision.min="1";depRevision.max="1000000";
    fields.append(el("p","small text-secondary","Exact dependencies stay pinned to this revision. Additional stored component/dependency entries are preserved; no startup order is implied."));
    fields.append(el("h3","section-title","Service capabilities · separate from hardware requirements"));
    const requires=field("Required service capabilities (space-separated)","requires",existing?.definition.requires.map(r=>r.capability).join(" ")??"");requires.maxLength=8192;
    const provides=field("Provided service capabilities (space-separated)","provides",existing?.definition.provides.join(" ")??"");provides.maxLength=8192;
    fields.append(el("p","small text-secondary","Requirements declare unresolved capabilities only. No provider, node or instance is selected or deployed."));
    field("Secret reference (never its value)", "secret-ref", existing?.definition.secrets[0]?.secret_ref ?? "");
    field("Secret version", "secret-version", existing?.definition.secrets[0]?.version ?? "");
    field("Secret target environment variable", "secret-target", existing?.definition.secrets[0]?.target_environment_variable ?? "");
    const review = el("p", "catalog-review small"); review.setAttribute("role", "status");
    const reviewButton = el("button", "btn btn-outline-secondary", "Review definition"); reviewButton.type = "button";
    const save = el("button", "btn btn-primary", existing ? "Save new immutable revision" : "Save definition"); save.type = "submit";
    const value = (name: string) => (form.elements.namedItem(name) as HTMLInputElement).value;
    const number = (name: string) => value(name) === "" ? null : Number(value(name));
    const draft = (): ServiceDefinition => {
      if (shape.value === "composite" && mode.value === "discovered" && !validRepositoryUrl(repository.value)) throw new CatalogError("invalid_definition", "Repository URL must be credential-free HTTPS without whitespace or control characters.");
      const source = artifacts[Number(artifact.value)];
      if (shape.value==="atomic"&&!source) throw new CatalogError("invalid_definition", "Select a currently pinned artifact reference before saving.");
      const base = existing?.definition;
      const staticTail=base?.composition?.mode==="static"?base.composition.components.slice(1):[];
      return { service_definition_id: value("id"), name: value("name"), artifact: shape.value==="atomic"?source!.source:null, runtime_kind: shape.value==="atomic"?source!.runtime_kind:null,
        service_shape:shape.value as ServiceDefinition["service_shape"],composition:shape.value==="atomic"?null:mode.value==="static"?{mode:"static",components:[{key:key.value,definition:{definition_id:memberId.value,revision:Number(memberRevision.value)}},...staticTail]}:{mode:"discovered",discovery:{kind:"docker_compose",compose_files:JSON.parse(composeFiles.value)}},
        provisioning:shape.value==="composite"&&mode.value==="discovered"?{kind:"repository_command",repository_url:repository.value,commit:commit.value,command:JSON.parse(command.value)}:null,
        tenancy: tenancy.value as Tenancy, tenant_unit: (unit.value || null) as ServiceDefinition["tenant_unit"],
        resources: { cpu_millis: number("cpu_millis"), memory_mb: number("memory_mb"), storage_mb: number("storage_mb"),
          capabilities: capability.value ? [{ kind: capability.value as ServiceDefinition["resources"]["capabilities"][number]["kind"], exclusive: exclusive.value === "true" }, ...(base?.resources.capabilities.slice(1) ?? [])] : (base?.resources.capabilities.slice(1) ?? []) },
        health_check: health.value ? { kind: health.value as "http" | "tcp_connect", target: value("health-target"), port: number("health-port") ?? 0, path: health.value === "http" ? value("health-path") : null, timeout_ms: number("health-timeout") ?? 0 } : null,
        monitoring: number("cadence") === null ? null : { subject_kind: "dserv_realization", cadence_seconds: number("cadence")! },
        secrets: value("secret-ref") ? [{ secret_ref: value("secret-ref"), version: value("secret-version"), target_environment_variable: value("secret-target") }, ...(base?.secrets.slice(1) ?? [])] : (base?.secrets.slice(1) ?? []),
        exact_dependencies:[...(depId.value?[{definition_id:depId.value,revision:Number(depRevision.value)}]:[]),...(base?.exact_dependencies.slice(1)??[])],requires:requires.value.trim()?requires.value.trim().split(/\s+/u).map(capability=>({capability})):[],provides:provides.value.trim()?provides.value.trim().split(/\s+/u):[] };
    };
    reviewButton.addEventListener("click", () => { try { const d = draft(); review.textContent = `${d.name} · ${d.service_shape} ${d.composition?.mode??""} · ${d.runtime_kind??"per-component runtime"} · ${tenancyNames[d.tenancy]} · ${d.artifact?.artifact_id??d.provisioning?.repository_url??"exact static components"} · dependencies ${d.exact_dependencies.map(r=>`${r.definition_id}@${r.revision}`).join(" ")||"none"} · requires ${d.requires.map(r=>r.capability).join(" ")||"none"} · provides ${d.provides.join(" ")||"none"} · monitoring ${d.monitoring?.cadence_seconds ?? "not configured"} · catalog only`; } catch(e) { error.textContent = message(e); error.focus(); } });
    form.addEventListener("submit", async event => {
      event.preventDefault(); if (busy || historical) return;
      if (!form.reportValidity()) return;
      busy = true; save.disabled = true; error.textContent = "";
      try { const result = await client.save(draft(), existing?.revision ?? 0); status.textContent = `Saved ${result.definition.service_definition_id} revision ${result.revision}. No runtime/deployment action performed.`; await loadList(); formView(result); }
      catch(e) { error.textContent = message(e); error.focus(); }
      finally { busy = false; save.disabled = false; }
    });
    fields.append(reviewButton, review, save); form.append(error, fields); editor.append(form);
    const updateStructure=()=>{
      const composite=shape.value==="composite", discovered=composite&&mode.value==="discovered";
      for(const input of [mode,key,memberId,memberRevision,composeFiles,repository,commit,command]) {const show=input===mode?composite:[key,memberId,memberRevision].includes(input as HTMLInputElement)?composite&&!discovered:discovered;input.closest(".mb-3")!.toggleAttribute("hidden",!show);input.disabled=historical||!show;}
      if(!historical)artifact.closest(".mb-3")!.toggleAttribute("hidden",composite);
      artifact.required=!composite&&!historical;artifact.disabled=composite||historical;runtime.closest(".mb-3")!.toggleAttribute("hidden",composite);pins.hidden=composite;
    };shape.addEventListener("change",updateStructure);mode.addEventListener("change",updateStructure);updateStructure();
    if (historical) editor.append(el("p", "small", "This immutable revision cannot be overwritten."));
  };
  const loadList = async () => {
    const data = await client.list(); list.replaceChildren(el("h2", "section-title", "Reusable service definitions"));
    if (!data.items.length) list.append(el("p", "small", "No definitions in the configured catalog. This is not project/runtime inventory."));
    if (data.has_more) list.append(el("p", "small", "Showing 100 loaded definitions. More exist upstream; pagination is not yet exposed."));
    const create = el("button", "btn btn-sm btn-outline-primary", "Create definition"); create.type = "button"; create.addEventListener("click", () => formView()); list.append(create);
    for (const r of data.items) {
      const row = el("article", "project-row catalog-row");
      row.append(el("h3", "small fw-semibold entity-identity", `${r.definition.name} · ${r.definition.service_definition_id}`),
        badge(`Revision ${r.revision}`), el("p", "small", `${r.definition.service_shape} ${r.definition.composition?.mode??""} · ${r.definition.runtime_kind??"per-component runtime"} · ${tenancyNames[r.definition.tenancy]} · global catalog`),
        el("p", "small entity-identity", r.definition.artifact?`Artifact ${r.definition.artifact.project_id} / ${r.definition.artifact.artifact_id} · generation ${r.definition.artifact.execution_generation}`:`Composition ${r.definition.composition?.mode} · ${r.definition.provisioning?.repository_url??"exact component revisions"}`),
        el("p", "small", `Health ${r.definition.health_check?.kind ?? "not configured"} · monitoring ${r.definition.monitoring ? `${r.definition.monitoring.cadence_seconds}s` : "not configured"} · CPU ${r.definition.resources.cpu_millis ?? "not included"} millicores · memory ${r.definition.resources.memory_mb ?? "not included"} MB`));
      const edit = el("button", "btn btn-sm btn-outline-secondary", "Create new revision"); edit.type = "button"; edit.addEventListener("click", () => formView(r)); row.append(edit);
      const pin = el("button", "btn btn-sm btn-outline-secondary", r.definition.service_shape==="atomic"?"Inspect pinned artifact":"Inspect composite definition"); pin.type = "button"; pin.addEventListener("click", () => formView(r, true)); row.append(pin);
      const historical = el("input", "form-control form-control-sm"); historical.type = "number"; historical.min = "1"; historical.max = String(r.revision); historical.value = String(r.revision); historical.setAttribute("aria-label", `Historical revision for ${r.definition.service_definition_id}`);
      const inspect = el("button", "btn btn-sm btn-outline-secondary", "Inspect historical revision"); inspect.type = "button"; inspect.addEventListener("click", async () => { try { formView(await client.get(r.definition.service_definition_id, Number(historical.value)), true); } catch(e) { status.textContent = message(e); } }); row.append(historical, inspect); list.append(row);
    }
  };
  void (async () => { try { await loadList(); const references = await client.artifacts(); artifacts = references.items; formView(); state?.(true); if (references.has_more) status.textContent = "Additional pinned artifact references exist beyond this loaded preview; source pagination is not exposed here."; if (!artifacts.length) status.textContent = "No eligible pinned artifact references loaded. Register/pin artifacts through the existing governed artifact workflow first; this page does not perform deployment or image resolution."; } catch(e) { status.textContent = message(e); state?.(false); } })();
  return page;
}
