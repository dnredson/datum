import test from "node:test";
import assert from "node:assert/strict";
import { CatalogClient, validCatalogRevision, validArtifactSnapshot, validRepositoryUrl } from "../src/api/catalog.ts";
import { parseRoute } from "../src/app/router.ts";
import { catalogFixture, containerSnapshot, compositeFixture, softwareCapabilityFixtures } from "./catalog-fixture.mjs";
test("repository URL validation rejects raw malformed input without normalization",()=>{
 const valid=["https://github.com/absmach/magistrala.git","https://github.com/org/repo","https://example.com/a/b/c/repo.git","https://example.com/"+"a".repeat(490)];
 const invalid=[" https://github.com/org/repo","https://github.com/org/repo ","https://github.com/org /repo",...['\n','\r','\t','\0','\u0001','\u007f','\u0085','\u00a0'].map(c=>`https://github.com/org/${c}repo`),"https://user@example.com/repo","https://user:token@example.com/repo","http://example.com/repo","file:///repo","ssh://example.com/repo","git@example.com:repo","https://example.com/repo?q=1","https://example.com/repo#x","https://","https://example.com/"+"a".repeat(513)];
 for(const url of [...valid,...invalid]){const expected=valid.includes(url);assert.equal(validRepositoryUrl(url),expected,JSON.stringify(url));const r=compositeFixture().record;r.definition.provisioning.repository_url=url;const before=JSON.stringify(r);assert.equal(validCatalogRevision(r),expected,JSON.stringify(url));assert.equal(JSON.stringify(r),before);}
});
test("catalog has global context and dedicated is not node ownership",()=>{
  assert.deepEqual(parseRoute("/catalog"),{kind:"catalog"}); const f=catalogFixture(); assert.ok(validCatalogRevision(f.record)); assert.equal(f.record.scope,"global_catalog"); assert.equal(JSON.stringify(f.record).includes("node_id"),false);
});
test("Atomic and Composite discovered contracts preserve distinct source semantics",()=>{
 assert.ok(validCatalogRevision(catalogFixture().record));const f=compositeFixture();assert.ok(validCatalogRevision(f.record));assert.equal(f.record.definition.artifact,null);assert.equal(f.record.artifact_snapshot_id,null);assert.deepEqual(f.record.definition.provisioning.command,["make","run_latest"]);
 for(const mutate of [r=>r.definition.composition=null,r=>r.definition.provisioning.commit="main",r=>r.definition.composition.discovery.compose_files=["../compose.yaml"],r=>r.definition.artifact=catalogFixture().record.definition.artifact]){const r=compositeFixture().record;mutate(r);assert.equal(validCatalogRevision(r),false);}
});
test("Static composition and exact dependencies use explicit revision identity",()=>{
 const r=compositeFixture().record;r.definition.provisioning=null;r.definition.composition={mode:"static",components:[{key:"database",definition:{definition_id:"postgresql",revision:7}}]};r.definition.exact_dependencies=[{definition_id:"mqtt",revision:4}];assert.ok(validCatalogRevision(r));r.definition.exact_dependencies[0].revision=0;assert.equal(validCatalogRevision(r),false);
});
test("Required/provided software capabilities do not resolve physical or service bindings",()=>{
 const {coffee,postgres}=softwareCapabilityFixtures();assert.ok(validCatalogRevision(coffee.record));assert.ok(validCatalogRevision(postgres.record));assert.equal(coffee.record.definition.requires[0].capability,postgres.record.definition.provides[0]);assert.deepEqual(coffee.record.definition.exact_dependencies,[]);
 for(const mutate of [r=>r.definition.requires.push(r.definition.requires[0]),r=>r.definition.provides.push(r.definition.provides[0]),r=>r.definition.provides=["Bad.Capability"],r=>r.definition.exact_dependencies=[{definition_id:"smart-coffee-api",revision:1}]]){const r=softwareCapabilityFixtures().coffee.record;mutate(r);assert.equal(validCatalogRevision(r),false);}
});
test("artifact pin validation accepts native and OCI typed platform pins",()=>{
 assert.ok(validArtifactSnapshot(catalogFixture().artifacts.items[0]));assert.ok(validArtifactSnapshot(containerSnapshot()));
});
test("malformed native/OCI digest platform and execution references fail closed",()=>{
 for(const mutate of [s=>s.executable_sha256="bad",s=>s.executable_sha256=`sha256:${"a".repeat(64)}`]){const s=catalogFixture().artifacts.items[0];mutate(s);assert.equal(validArtifactSnapshot(s),false);}
 for(const mutate of [s=>s.resolved_image_identity.platforms[0].manifest_digest="bad",s=>s.resolved_image_identity.platforms[0].config_digest="bad",s=>s.resolved_image_identity.index_digest="bad",s=>s.resolved_image_identity.platforms[0].os="",s=>s.resolved_image_identity.platforms[0].execution_reference="mutable:tag",s=>s.resolved_image_identity.platforms.push(s.resolved_image_identity.platforms[0])]){const s=containerSnapshot();mutate(s);assert.equal(validArtifactSnapshot(s),false);}
});
test("exact snapshot API uses stored snapshot ID rather than live artifact identity",async()=>{
 const original=globalThis.fetch,f=catalogFixture(),calls=[];globalThis.fetch=async url=>{calls.push(url);return new Response(JSON.stringify(f.artifacts.items[0]));};
 try{await new CatalogClient().snapshot(f.record.artifact_snapshot_id);assert.deepEqual(calls,[`/api/v1/catalog/artifacts/${encodeURIComponent(f.record.artifact_snapshot_id)}`]);}finally{globalThis.fetch=original;}
});
test("malformed artifact response returns safe invalid_response before pin presentation",async()=>{
 const original=globalThis.fetch,f=catalogFixture();f.artifacts.items[0].executable_sha256="bad";globalThis.fetch=async url=>new Response(JSON.stringify(url.endsWith("artifacts")?f.artifacts:f.artifacts.items[0]));
 try{const c=new CatalogClient();await assert.rejects(c.artifacts(),e=>e.code==="invalid_response");await assert.rejects(c.snapshot(f.record.artifact_snapshot_id),e=>e.code==="invalid_response");}finally{globalThis.fetch=original;}
});
test("catalog client create and revise use catalog-only POST and expected revision",async()=>{
  const original=globalThis.fetch, calls=[]; const f=catalogFixture();
  globalThis.fetch=async(url,init)=>{calls.push({url,init}); const previous=JSON.parse(init.body).expected_revision; return new Response(JSON.stringify({...f.record,revision:previous+1}));};
  try {const c=new CatalogClient(); await c.save(f.record.definition,0); await c.save(f.record.definition,1); assert.deepEqual(calls.map(c=>c.url),["/api/v1/catalog/services","/api/v1/catalog/services/p4-sniffer/revisions"]); assert.ok(calls.every(c=>c.init.method==="POST")); assert.ok(!JSON.stringify(calls).includes("deploy"));}finally{globalThis.fetch=original;}
});
test("list/latest/exact revision remain bounded and typed",async()=>{
 const original=globalThis.fetch,f=catalogFixture(),calls=[]; globalThis.fetch=async(url)=>{calls.push(url);return new Response(JSON.stringify(url.endsWith("services")?f.page:f.record));};
 try{const c=new CatalogClient();assert.equal((await c.list()).items.length,1);await c.get("p4-sniffer");await c.get("p4-sniffer",1);assert.equal(calls[2],"/api/v1/catalog/services/p4-sniffer/revisions/1");}finally{globalThis.fetch=original;}
});
test("safe errors never echo SQL/path/secret diagnostics",async()=>{
 const original=globalThis.fetch;globalThis.fetch=async()=>new Response(JSON.stringify({error:{code:"catalog_unavailable",safe_message:"SQL/path secret stack"}}),{status:503});
 try{await assert.rejects(new CatalogClient().list(),e=>e.code==="catalog_unavailable"&&!/SQL|path|secret|stack/.test(e.message));}finally{globalThis.fetch=original;}
});
test("malformed and oversize catalog pages are rejected without repair",async()=>{
 const original=globalThis.fetch,f=catalogFixture();f.page.items=Array(101).fill(f.record);globalThis.fetch=async()=>new Response(JSON.stringify(f.page));
 try{await assert.rejects(new CatalogClient().list(),e=>e.code==="invalid_response");}finally{globalThis.fetch=original;}
});
test("malformed nested catalog fields and plaintext secret properties fail validation",()=>{
 for(const mutate of [r=>r.definition.monitoring.cadence_seconds=0,r=>r.definition.resources.memory_mb="unknown",r=>r.definition.tenant_unit="domain",r=>r.definition.secrets=[{secret_ref:"token",version:"v1",target_environment_variable:"TOKEN",value:"plaintext"}],r=>r.definition.artifact.execution_generation=0]){
 const r=catalogFixture().record;mutate(r);assert.equal(validCatalogRevision(r),false);
 }
});
