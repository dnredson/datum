export function catalogFixture() {
  const source = { project_id: "artifact-source-project", artifact_id: "p4-sniffer", execution_generation: 1, execution_projection_digest: `sha256:${"a".repeat(64)}` };
  const definition = { service_definition_id: "p4-sniffer", name: "P4 Sniffer", artifact: source, runtime_kind: "native_process", tenancy: "dedicated", tenant_unit: null,
    service_shape:"atomic",composition:null,provisioning:null,exact_dependencies:[],requires:[],provides:[],
    resources: { cpu_millis:1000, memory_mb:256, storage_mb:null, capabilities:[{kind:"network_interface",exclusive:true}] }, health_check:null,
    monitoring:{subject_kind:"dserv_realization",cadence_seconds:30},secrets:[] };
  const record={definition,revision:1,digest:`sha256:${"b".repeat(64)}`,created_at_utc:"2026-10-01T00:00:00Z",artifact_snapshot_id:`sha256:${"c".repeat(64)}`,scope:"global_catalog"};
  return { record, page:{items:[record],has_more:false,limit:100}, artifacts:{ items:[{source,runtime_kind:"native_process",image:null,resolved_image_identity:null,executable_sha256:"d".repeat(64)}],has_more:false,limit:100,source:"pinned_operational_artifact_references"} };
}
export function compositeFixture() {
 const f=catalogFixture();const d=f.record.definition;
 Object.assign(d,{service_definition_id:"magistrala",name:"Magistrala",artifact:null,runtime_kind:null,service_shape:"composite",tenancy:"shared_tenant_aware",tenant_unit:"domain",
 composition:{mode:"discovered",discovery:{kind:"docker_compose",compose_files:["docker/docker-compose.yaml"]}},provisioning:{kind:"repository_command",repository_url:"https://github.com/absmach/magistrala.git",commit:"a".repeat(40),command:["make","run_latest"]},resources:{cpu_millis:null,memory_mb:null,storage_mb:null,capabilities:[]}});
 f.record.artifact_snapshot_id=null;return f;
}
export function softwareCapabilityFixtures() {
 const coffee=catalogFixture(),postgres=catalogFixture();Object.assign(coffee.record.definition,{service_definition_id:"smart-coffee-api",name:"Smart Coffee API",requires:[{capability:"database.postgresql"},{capability:"messaging.mqtt"}],provides:["api.smart-coffee"]});
 Object.assign(postgres.record.definition,{service_definition_id:"postgresql",name:"PostgreSQL",provides:["database.postgresql"]});
 coffee.record.definition.artifact.artifact_id="smart-coffee-api";postgres.record.definition.artifact.artifact_id="postgresql";
 coffee.record.artifact_snapshot_id=`sha256:${"6".repeat(64)}`;postgres.record.artifact_snapshot_id=`sha256:${"7".repeat(64)}`;return {coffee,postgres};
}
export function containerSnapshot() {
  const source = { project_id:"artifact-source-project", artifact_id:"magistrala", execution_generation:2, execution_projection_digest:`sha256:${"e".repeat(64)}` };
  const repository="docker.io/example/magistrala", manifest=`sha256:${"f".repeat(64)}`;
  return {source,runtime_kind:"container",image:`${repository}:display-tag`,executable_sha256:null,
    resolved_image_identity:{repository,index_digest:null,platforms:[{os:"linux",architecture:"amd64",variant:null,manifest_digest:manifest,config_digest:`sha256:${"1".repeat(64)}`,execution_reference:`${repository}@${manifest}`}]} };
}
