import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import {editorFixture} from "./dgraph-editor-fixture.mjs";
import {parseGraph,canonicalJson,newGraph,diff,removeService,draftElements} from "../src/pages/dgraph-editor-model.ts";
import {DGraphEditorClient} from "../src/api/dgraph-editor.ts";
import {parseRoute} from "../src/app/router.ts";
import {GRAPH_CATEGORIES,GRAPH_RELATIONS} from "../src/components/graph-visuals.ts";
import {editorCatalogFixture,largeEditorCatalogFixture,discoveryFixturePage} from "./dgraph-editor-catalog-fixture.mjs";
import {validCatalogRevision,CatalogClient} from "../src/api/catalog.ts";
test("Phase134B exact catalog reference survives canonical JSON independently of service ID",()=>{
 const record=editorCatalogFixture().get("smart-coffee-api",3),g=editorFixture().graph;
 g.schema="datum.dgraph/2";g.services[0].service_id="coffee-api-primary";g.calls=[];
 g.services[0].service_definition={definition_id:record.definition.service_definition_id,revision:record.revision,digest:record.digest};
 assert.deepEqual(parseGraph(canonicalJson(g)),g);assert.equal(g.services[0].service_definition.revision,3);
 const base=structuredClone(g);g.services[0].service_definition.revision=4;assert.deepEqual(diff(base,g).nodes.modified,["coffee-api-primary"]);
 for(const change of [r=>r.revision="latest",r=>r.digest="tag",r=>r.unknown=true]){const v=JSON.parse(canonicalJson(base));change(v.services[0].service_definition);assert.throws(()=>parseGraph(JSON.stringify(v)));}
 const v=JSON.parse(canonicalJson(base));v.schema="datum.dgraph/1";assert.throws(()=>parseGraph(JSON.stringify(v)));
});
test("Phase134B legacy service names never cause catalog linking",()=>{
 const g=editorFixture().graph;g.services[0].service_id="smart-coffee-api";g.calls=[];
 assert.equal(parseGraph(canonicalJson(g)).services[0].service_definition,undefined);
 assert.doesNotMatch(canonicalJson(g),/service_definition/);
});
test("Phase134B catalog fixture holds valid revision DTOs and exact read paths only",async()=>{
 const fixture=editorCatalogFixture();assert.ok(fixture.page.items.every(validCatalogRevision));
 const prior=globalThis.fetch,requests=[];try{globalThis.fetch=async(url,init)=>{requests.push({url,init});return new Response(JSON.stringify(url.endsWith('/services')?fixture.page:fixture.get("smart-coffee-api",3)));};
 const c=new CatalogClient();assert.equal((await c.list()).items.length,3);assert.equal((await c.get("smart-coffee-api",3)).revision,3);
 assert.match(requests[1].url,/smart-coffee-api\/revisions\/3$/);assert.ok(requests.every(r=>r.init.method==='GET'));
 }finally{globalThis.fetch=prior;}
});
test("Phase134C bounded client discovery pages and full catalog search are GET only",async()=>{
 const catalog=largeEditorCatalogFixture(100),prior=globalThis.fetch,requests=[];assert.ok(catalog.page.items.every(validCatalogRevision));
 try{globalThis.fetch=async(url,init)=>{requests.push({url,init});const u=new URL(url,"http://fixture");return new Response(JSON.stringify(discoveryFixturePage(catalog.page.items,u.searchParams.get("search")??"",Number(u.searchParams.get("offset")??0),Number(u.searchParams.get("limit")??25))));};
 const c=new CatalogClient(),first=await c.discover(),second=await c.discover("",25),previous=await c.discover(),last=await c.discover("",75);
 assert.equal(first.items.length,25);assert.equal(first.total,100);assert.equal(first.has_more,true);assert.equal(last.has_more,false);assert.equal(last.items.length,25);
 assert.deepEqual(previous,first);assert.ok(second.items.every(r=>!first.items.some(f=>f.definition.service_definition_id===r.definition.service_definition_id)));
 assert.ok(!first.items.some(r=>r.definition.service_definition_id==="service-090"));
 const search=await c.discover("EXAMPLE.DISCOVERY-090");assert.equal(search.total,1);assert.equal(search.offset,0);assert.equal(search.items[0].definition.service_definition_id,"service-090");
 assert.equal((await c.discover("absent")).total,0);assert.equal((await c.discover("",0,100)).items.length,100);
 assert.ok(requests.every(r=>r.init.method==="GET"&&!r.init.body));
 for(const args of [["",-1,25],["",0,0],["",0,101],["",1000001,25],["\n",0,25],["x".repeat(257),0,25]])await assert.rejects(c.discover(...args));
 }finally{globalThis.fetch=prior;}
});
test("Phase134C malformed metadata/order and provider errors fail safe",async()=>{
 const prior=globalThis.fetch,valid=discoveryFixturePage(largeEditorCatalogFixture().page.items);
 try{for(const mutate of [p=>p.limit=101,p=>p.offset=25,p=>p.search="unexpected",p=>p.total=-1,p=>p.has_more=false,p=>p.items.reverse(),p=>p.items.push(p.items[0])]){const page=structuredClone(valid);mutate(page);globalThis.fetch=async()=>new Response(JSON.stringify(page));await assert.rejects(new CatalogClient().discover(),e=>e.code==="invalid_response");}
 globalThis.fetch=async()=>new Response(JSON.stringify({error:{code:"catalog_unavailable",message:"RAW SQL SECRET"}}),{status:503});await assert.rejects(new CatalogClient().discover(),e=>!e.message.includes("RAW SQL SECRET"));
 }finally{globalThis.fetch=prior;}
});
test("Phase134 canonical wire types cannot be coerced by lossless integer parsing",()=>{
 const raw=editorFixture().base.graphs[0].graph_json;
 for(const changed of [raw.replace('"revision":7','"revision":"7"'),raw.replace('"application_id":"app:editor"','"application_id":123'),raw.replace('"service_id":"service-0"','"service_id":123'),raw.replace('"memory_bytes":9007199254740991','"memory_bytes":"9007199254740991"')])assert.throws(()=>parseGraph(changed));
});
test("Phase134 canonical lossless roundtrip and scoped editor route",()=>{const f=editorFixture(3,"18446744073709551615");const g=parseGraph(f.base.graphs[0].graph_json);assert.equal(g.services[0].resource_requirement.memory_bytes,"18446744073709551615");assert.deepEqual(parseGraph(canonicalJson(g)),g);assert.deepEqual(parseRoute("/projects/a%2Fb/graph/edit"),{kind:"project_graph_editor",projectId:"a/b"});});
test("Phase134 draft mutations never change immutable base and diff is deterministic",()=>{const base=editorFixture().graph,g=structuredClone(base);g.services.push({service_id:"new",execution_requirement:{dnode_abi:"datum-dnode/0"},resource_requirement:{cpu_cores:2,memory_bytes:"1"}});g.services[0].resource_requirement.cpu_cores=2;g.calls.push({call_id:"new-call",source_service_id:"new",target_service_id:"service-0"});g.calls[0].target_service_id="service-2";removeService(g,"service-1");const d=diff(base,g);assert.deepEqual(d.nodes,{added:["new"],modified:["service-0"],removed:["service-1"]});assert.deepEqual(d.edges,{added:["new-call"],modified:["call-0"],removed:["call-1"]});assert.equal(base.services.length,3);assert.equal(base.services[0].resource_requirement.cpu_cores,1);});
test("Phase134 malformed dangling duplicate unknown and oversized graphs reject",()=>{const f=editorFixture();for(const mutate of [g=>g.calls[0].target_service_id="no",g=>g.services.push(g.services[0]),g=>g.calls.push(g.calls[0]),g=>g.nodes=[],g=>g.services[0].resource_requirement.memory_bytes="18446744073709551616",g=>g.services[0].execution_requirement.dnode_abi="container"]){const g=JSON.parse(f.base.graphs[0].graph_json);mutate(g);assert.throws(()=>parseGraph(JSON.stringify(g)));}assert.throws(()=>parseGraph("x".repeat(262145)));});
test("Phase134 canonical self/reciprocal calls remain valid and IDs collision-safe",()=>{const g=editorFixture(2).graph;g.calls.push({call_id:"self",source_service_id:"service-0",target_service_id:"service-0"},{call_id:"back",source_service_id:"service-1",target_service_id:"service-0"});assert.equal(canonicalJson(parseGraph(canonicalJson(g))),canonicalJson(g));const elements=draftElements(g);assert.equal(new Set(elements.map(e=>e.data.id)).size,elements.length);assert.ok(elements.filter(e=>e.data.source).every(e=>e.data.visual_relation==="logical"));assert.equal(GRAPH_RELATIONS.logical.line,"solid");assert.ok(elements.filter(e=>e.position).every(e=>e.data.visual_category==="software"));assert.equal(GRAPH_CATEGORIES.software.shape,"rectangle");});
test("Phase134 50 services 100 calls layout and payload remain bounded",()=>{const g=editorFixture(50).graph;g.calls=Array.from({length:100},(_,i)=>({call_id:`c${i}`,source_service_id:`service-${i%50}`,target_service_id:`service-${(i+1)%50}`}));assert.equal(draftElements(g).length,150);assert.deepEqual(draftElements(g),draftElements(g));assert.equal(canonicalJson(parseGraph(canonicalJson(g))),canonicalJson(g));assert.ok(canonicalJson(g).length<262144);});
test("Phase134 client validates only explicit POST and preserves exact base",async()=>{const prior=globalThis.fetch,requests=[];const f=editorFixture();try{globalThis.fetch=async(url,init)=>{requests.push({url,init});return new Response(JSON.stringify(init.method==="GET"?f.base:{schema:"datum.dgraph-editor-validation/1",valid:true,findings:[],digest:`sha256:${"b".repeat(64)}`}));};const client=new DGraphEditorClient("foundation-demo");const [entry]=await client.load();const draft=structuredClone(entry.graph);draft.revision="8";const base={application_id:entry.application_id,dgraph_id:entry.dgraph_id,revision:entry.revision,digest:entry.digest};assert.equal((await client.validate(draft,base)).valid,true);assert.equal(requests.length,2);assert.match(requests[1].url,/dgraph-editor\/validate$/);const sent=JSON.parse(requests[1].init.body);assert.deepEqual(sent.base,base);assert.match(sent.graph_json,/"memory_bytes":9007199254740991/);assert.doesNotMatch(sent.graph_json,/position|layout|selected|project_id/);}finally{globalThis.fetch=prior;}});
test("Phase134 invalid transport/conflicts keep safe errors without raw echo",async()=>{const prior=globalThis.fetch;try{for(const response of [new Response(JSON.stringify({error:{code:"base_revision_conflict",message:"RAW SECRET"}}),{status:409}),new Response(JSON.stringify({schema:"wrong"}))]){globalThis.fetch=async()=>response;await assert.rejects(new DGraphEditorClient("p").validate(newGraph("a","g"),null),e=>!e.message.includes("RAW SECRET"));}}finally{globalThis.fetch=prior;}});
test("Phase134 editor imports centralized visuals and has no mutation endpoint",async()=>{const page=await readFile(new URL("../src/pages/dgraph-editor.ts",import.meta.url),"utf8"),api=await readFile(new URL("../src/api/dgraph-editor.ts",import.meta.url),"utf8");assert.match(page,/graphVisualStyles/);assert.match(page,/beforeunload/);assert.match(page,/confirmLeave/);assert.doesNotMatch(api,/\/deploy|\/accept|\/placement|\/catalog\/services/);});
