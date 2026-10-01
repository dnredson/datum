import { parseGraph,canonicalJson } from "../pages/dgraph-editor-model.ts";
import type {DGraph,Base,Finding} from "../pages/dgraph-editor-model.ts";
export interface GraphEntry extends Base { graph:DGraph }
export class EditorError extends Error {code:string;constructor(code:string){super(code==="base_revision_conflict"?"Accepted base changed or draft revision mismatches. Local draft retained; no rebase.":"Graph request failed. Local draft retained.");this.code=code;}}
const obj=(v:unknown):v is Record<string,unknown>=>!!v&&typeof v==="object"&&!Array.isArray(v);
export class DGraphEditorClient {
 private project:string;private signal:AbortSignal|undefined;
 constructor(project:string,signal?:AbortSignal){this.project=project;this.signal=signal;}
 private async request(suffix:string,body?:unknown):Promise<unknown>{
  const controller=new AbortController();
  let timedOut=false;
  const abort=()=>controller.abort();
  const timer=setTimeout(()=>{timedOut=true;controller.abort();},15000);
  this.signal?.addEventListener("abort",abort,{once:true});
  if(this.signal?.aborted)controller.abort();
  try {
   const response=await fetch(`/api/v1/operations/projects/${encodeURIComponent(this.project)}/dgraph-editor${suffix}`,{
    method:body?"POST":"GET",headers:body?{"Content-Type":"application/json"}:{},
    body:body?JSON.stringify(body):undefined,signal:controller.signal
   });
   const raw=await response.text();
   if(raw.length>2097152)throw new EditorError("invalid_response");
   let v:unknown;
   try{v=JSON.parse(raw);}catch{throw new EditorError("invalid_response");}
   if(!response.ok){
    const code=obj(v)&&obj(v.error)&&typeof v.error.code==="string"&&/^[a-z_]{1,64}$/u.test(v.error.code)?v.error.code:"unavailable";
    throw new EditorError(code);
   }
   return v;
  }catch(e){
   if(this.signal?.aborted)throw e;
   if(e instanceof EditorError)throw e;
   throw new EditorError(timedOut?"timeout":"unavailable");
  }finally{
   clearTimeout(timer);
   this.signal?.removeEventListener("abort",abort);
  }
 }
 async load():Promise<GraphEntry[]>{const v=await this.request("");if(!obj(v)||v.schema!=="datum.dgraph-editor-base/1"||v.project_id!==this.project||!Array.isArray(v.graphs)||v.graphs.length>100)throw new EditorError("invalid_response");const seen=new Set<string>();return v.graphs.map(e=>{if(!obj(e)||typeof e.graph_json!=="string"||typeof e.digest!=="string"||!/^sha256:[a-f0-9]{64}$/u.test(e.digest))throw new EditorError("invalid_response");let graph:DGraph;try{graph=parseGraph(e.graph_json);}catch{throw new EditorError("invalid_response");}if(e.application_id!==graph.application_id||e.dgraph_id!==graph.dgraph_id||e.revision!==graph.revision||seen.has(graph.application_id))throw new EditorError("invalid_response");seen.add(graph.application_id);return {application_id:graph.application_id,dgraph_id:graph.dgraph_id,revision:graph.revision,digest:e.digest,graph};});}
 async validate(graph:DGraph,base:Base|null):Promise<{valid:boolean;findings:Finding[];digest:string|null}>{const v=await this.request("/validate",{graph_json:canonicalJson(graph),base});if(!obj(v)||v.schema!=="datum.dgraph-editor-validation/1"||typeof v.valid!=="boolean"||!Array.isArray(v.findings)||v.findings.length>32||!(v.digest===null||(typeof v.digest==="string"&&/^sha256:[a-f0-9]{64}$/u.test(v.digest)))||v.valid!== (v.findings.length===0&&v.digest!==null))throw new EditorError("invalid_response");for(const f of v.findings){if(!obj(f)||typeof f.path!=="string"||!/^[A-Za-z0-9_.$\[\]]{1,128}$/u.test(f.path)||typeof f.code!=="string"||!/^[a-z_]{1,64}$/u.test(f.code)||typeof f.message!=="string"||f.message.length>256||/[\u0000-\u001f]/u.test(f.message))throw new EditorError("invalid_response");}return v as unknown as {valid:boolean;findings:Finding[];digest:string|null};}
}
