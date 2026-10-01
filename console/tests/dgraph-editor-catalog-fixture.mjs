// TEST ONLY: valid-shaped definitions and pinned snapshots held in fixture memory.
import {createHash} from "node:crypto";
import {catalogFixture,compositeFixture,softwareCapabilityFixtures,containerSnapshot} from "./catalog-fixture.mjs";
const ordered=v=>Array.isArray(v)?v.map(ordered):v&&typeof v==="object"?Object.fromEntries(Object.keys(v).sort().map(k=>[k,ordered(v[k])])):v;
export const fixtureDigest=v=>`sha256:${createHash("sha256").update(JSON.stringify(ordered(v))).digest("hex")}`;
export function editorCatalogFixture(){
 const p4=catalogFixture(),magistrala=compositeFixture(),{coffee}=softwareCapabilityFixtures();
 const snapshot=containerSnapshot();snapshot.source.artifact_id="smart-coffee-api";
 coffee.record.definition.artifact=snapshot.source;coffee.record.definition.runtime_kind="container";
 coffee.record.definition.resources={cpu_millis:null,memory_mb:null,storage_mb:null,capabilities:[]};
 coffee.record.definition.monitoring=null;coffee.record.revision=3;
 const snapshots=new Map();
 for(const [record,artifact] of [[p4.record,p4.artifacts.items[0]],[coffee.record,snapshot],[magistrala.record,null]]){
  record.digest=fixtureDigest(record.definition);record.artifact_snapshot_id=artifact?fixtureDigest(artifact):null;
  if(artifact)snapshots.set(record.artifact_snapshot_id,artifact);
 }
 const items=[coffee.record,magistrala.record,p4.record];
 return {page:{items,has_more:false,limit:100},snapshots,get:(id,revision)=>items.find(r=>r.definition.service_definition_id===id&&r.revision===revision)};
}
export function largeEditorCatalogFixture(count=100){
 if(!Number.isInteger(count)||count<3||count>1000)throw new Error("Invalid fixture count");
 const seed=editorCatalogFixture(),items=[...seed.page.items];
 for(let i=0;items.length<count;i++){
  const template=seed.page.items[i%3],record=structuredClone(template);
  record.definition.service_definition_id=`service-${String(i).padStart(3,"0")}`;
  record.definition.name=`Discovery Service ${String(i).padStart(3,"0")}`;
  record.definition.provides=[`example.discovery-${String(i).padStart(3,"0")}`];
  record.digest=fixtureDigest(record.definition);items.push(record);
 }
 return {page:{items,has_more:false,limit:100},snapshots:seed.snapshots,get:(id,revision)=>items.find(r=>r.definition.service_definition_id===id&&r.revision===revision)};
}
const asciiLower=s=>s.replace(/[A-Z]/gu,c=>c.toLowerCase());
export function discoveryFixturePage(items,search="",offset=0,limit=25){
 if(!Number.isInteger(limit)||limit<1||limit>100||!Number.isInteger(offset)||offset<0||offset>1000000||new TextEncoder().encode(search).length>256||/[\u0000-\u001f\u007f-\u009f]/u.test(search))throw new Error("Invalid fixture discovery query");
 const matches=items.filter(r=>[r.definition.service_definition_id,r.definition.name,...r.definition.provides].some(s=>asciiLower(s).includes(asciiLower(search)))).sort((a,b)=>a.definition.service_definition_id<b.definition.service_definition_id?-1:1);
 return {schema:"datum.catalog-discovery/1",items:matches.slice(offset,offset+limit),limit,offset,total:matches.length,has_more:offset+limit<matches.length,search};
}
