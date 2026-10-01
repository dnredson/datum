// Explicit TEST-ONLY HTTP fixture. No provider, catalog DB or execution authority.
import http from "node:http";
import {catalogFixture} from "./catalog-fixture.mjs";
import {modelingFixture,fixtureScenario} from "./service-modeling-fixture.mjs";
import {validCatalogRevision} from "../src/api/catalog.ts";
const f=catalogFixture(),history=new Map([["p4-sniffer",[structuredClone(f.record)]]]);
const send=(res,status,body)=>{res.writeHead(status,{"Content-Type":"application/json"});res.end(JSON.stringify(body));};
const notConfigured=process.argv.includes("--not-configured");
const server=http.createServer(async(req,res)=>{
 const path=new URL(req.url,"http://127.0.0.1").pathname;
 let raw="";for await(const chunk of req){raw+=chunk;if(raw.length>32768){send(res,413,{error:{code:"invalid_request"}});return;}}
 let body;try{body=raw?JSON.parse(raw):null;}catch{send(res,400,{error:{code:"invalid_request"}});return;}
 if(path==="/api/v1/catalog/modeling-assistant"){send(res,200,{schema_version:"datum.service-modeling-status/1",state:notConfigured?"not_configured":"available",provider:notConfigured?"not_configured":"test_only",model_ref:notConfigured?"not_configured":"deterministic_fixture",repository_analysis:"not_included"});return;}
 if(path.endsWith("/modeling-assistant/draft")){
  if(notConfigured||body.intent==="not-configured"){send(res,503,{error:{code:"not_configured"}});return;}
  if(body.intent==="invalid"){send(res,502,{error:{code:"invalid_response"}});return;}
  const kind=body.intent==="unsupported-lifecycle"?"coffee":fixtureScenario(body.intent);
  if(!kind){send(res,422,{error:{code:"fixture_no_scenario"}});return;}
  const result=modelingFixture(kind);
  if(body.intent==="unsupported-lifecycle"){result.candidate.partial_definition.name="P4 Container lifecycle request";result.candidate.unsupported_requests=["Post-start docker exec/sysctl/simple_switch/P4 CLI programming: current ServiceDefinition has no RuntimeLifecycle/PostStartAction contract. No commands execute."];}
  if(kind==="magistrala"){const answer=body.answers.find(a=>a.field==="provisioning.commit");if(answer&&/^[a-f0-9]{40}$/u.test(answer.value)){result.candidate.partial_definition.provisioning.commit=answer.value;result.validation={state:"valid",unresolved_fields:[],findings:[]};result.candidate.unresolved_questions=[];}}
  if(body.base){result.base=history.get(body.base.definition_id)?.find(r=>r.revision===body.base.revision)??null;}
  send(res,200,result);return;
 }
 if(path==="/api/v1/catalog/artifacts"){send(res,200,f.artifacts);return;}
 if(path.startsWith("/api/v1/catalog/artifacts/")){send(res,200,f.artifacts.items[0]);return;}
 if(path==="/api/v1/catalog/services"&&req.method==="GET"){send(res,200,{items:[...history.values()].map(rows=>rows.at(-1)),has_more:false,limit:100});return;}
 if(req.method==="POST"&&path.startsWith("/api/v1/catalog/services")){
  const rows=history.get(body.definition.service_definition_id)??[];
  if(body.expected_revision!==(rows.at(-1)?.revision??0)){send(res,409,{error:{code:"revision_conflict"}});return;}
  const record={...f.record,definition:body.definition,revision:body.expected_revision+1,artifact_snapshot_id:body.definition.service_shape==="composite"?null:f.record.artifact_snapshot_id};
  if(!validCatalogRevision(record)){send(res,400,{error:{code:"invalid_definition"}});return;}
  rows.push(record);history.set(body.definition.service_definition_id,rows);send(res,200,record);return;
 }
 const match=path.match(/^\/api\/v1\/catalog\/services\/([^/]+)(?:\/revisions\/(\d+))?$/u);
 if(match){const rows=history.get(decodeURIComponent(match[1]))??[],record=match[2]?rows.find(r=>r.revision===Number(match[2])):rows.at(-1);send(res,record?200:404,record??{error:{code:"not_found"}});return;}
 send(res,404,{error:{code:"not_found"}});
});
const port=Number(process.env.DATUM_MODELING_FIXTURE_PORT??8134);
if(!Number.isInteger(port)||port<1024||port>65535)throw new Error("Invalid test fixture port");
server.listen(port,"127.0.0.1",()=>console.log(`TEST-ONLY modeling fixture http://127.0.0.1:${port} · mock output · no production persistence/execution`));
