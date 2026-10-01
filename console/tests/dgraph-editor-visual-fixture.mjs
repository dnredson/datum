// TEST ONLY. Accepted-looking base is fixture data, never production persistence.
import http from "node:http";
import {editorFixture} from "./dgraph-editor-fixture.mjs";
import {largeEditorCatalogFixture,discoveryFixturePage} from "./dgraph-editor-catalog-fixture.mjs";
const catalog=largeEditorCatalogFixture(100);
const f=editorFixture();let stale=false;
const send=(res,status,body)=>{res.writeHead(status,{"Content-Type":"application/json"});res.end(JSON.stringify(body));};
const server=http.createServer(async(req,res)=>{
 const url=new URL(req.url,"http://127.0.0.1"),path=url.pathname;
 if(req.method==="GET"&&path==="/api/v1/catalog/services"){try{send(res,200,url.search?discoveryFixturePage(catalog.page.items,url.searchParams.get("search")??"",Number(url.searchParams.get("offset")??0),Number(url.searchParams.get("limit")??25)):catalog.page);}catch{send(res,400,{error:{code:"invalid_definition"}});}return;}
 const exact=path.match(/^\/api\/v1\/catalog\/services\/([^/]+)\/revisions\/(\d+)$/u);
 if(req.method==="GET"&&exact){const record=catalog.get(decodeURIComponent(exact[1]),Number(exact[2]));send(res,record?200:404,record??{error:{code:"not_found"}});return;}
 if(req.method==="GET"&&path.startsWith("/api/v1/catalog/artifacts/")){const snapshot=catalog.snapshots.get(decodeURIComponent(path.split("/").at(-1)));send(res,snapshot?200:404,snapshot??{error:{code:"not_found"}});return;}
 if(path==="/fixture/stale"){stale=!stale;send(res,200,{stale});return;}
 if(path.endsWith("/dgraph-editor")&&req.method==="GET"){const b=structuredClone(f.base);if(stale){b.graphs[0].revision="8";b.graphs[0].graph_json=b.graphs[0].graph_json.replace('"revision":7','"revision":8');}if(path.includes("/empty/")){b.project_id="empty";b.graphs=[];}send(res,200,b);return;}
 if(path.endsWith("/dgraph-editor/validate")&&req.method==="POST"){
  let raw="";for await(const chunk of req){raw+=chunk;if(raw.length>524288){send(res,413,{error:{code:"invalid_request"}});return;}}
  let body,g;try{body=JSON.parse(raw);g=JSON.parse(body.graph_json);}catch{send(res,400,{error:{code:"invalid_request"}});return;}
  if(body.base&&body.base.revision!==(stale?"8":"7")){send(res,409,{error:{code:"base_revision_conflict"}});return;}
  const findings=[];if(!g.application_id||!g.dgraph_id)findings.push({path:"$",code:"missing_field",message:"Application and graph IDs are required."});
  for(const [i,s] of g.services.entries())if(s.service_definition){const r=s.service_definition,stored=catalog.get(r.definition_id,r.revision);if(g.schema!=="datum.dgraph/2"||!stored||stored.digest!==r.digest)findings.push({path:`services[${i}].service_definition`,code:"definition_reference_invalid",message:"Exact fixture catalog reference did not match."});}
  // Deterministic visual failure case, explicitly fixture-only; production uses canonical validator.
  if(g.services.some(s=>s.service_id==="fixture-invalid"))findings.push({path:"services",code:"fixture_validation_failure",message:"TEST ONLY validation failure scenario."});
  send(res,200,{schema:"datum.dgraph-editor-validation/1",valid:!findings.length,findings,digest:findings.length?null:`sha256:${"b".repeat(64)}`});return;
 }
 send(res,404,{error:{code:"fixture_not_found"}});
});
const port=Number(process.env.DATUM_DGRAPH_EDITOR_FIXTURE_PORT??8136);
if(!Number.isInteger(port)||port<1024||port>65535)throw new Error("Invalid TEST ONLY fixture port.");
server.listen(port,"127.0.0.1",()=>console.log(`TEST ONLY Phase134B fixture on http://127.0.0.1:${port}; no server persistence or authority.`));
