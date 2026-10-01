import type {ModelingRequest,ModelingResponse} from "../types/service-modeling.ts";
import {validCatalogRevision,validRepositoryUrl,CatalogError} from "./catalog.ts";
const obj=(v:unknown):v is Record<string,unknown>=>!!v&&typeof v==="object"&&!Array.isArray(v);
const text=(v:unknown,max=512):v is string=>typeof v==="string"&&v.length>0&&v.length<=max&&!v.includes("\0");
const texts=(v:unknown)=>Array.isArray(v)&&v.length<=32&&v.every(s=>text(s));
export interface ValidationFinding {path:string;code:string;message:string}
export class ModelingValidationError extends CatalogError {
 readonly findings:ValidationFinding[];
 constructor(findings:ValidationFinding[]){super("provider_invalid_candidate","AI draft rejected by deterministic validator. No draft was applied.");this.findings=findings;}
}
function safeFindings(v:unknown):v is ValidationFinding[]{return Array.isArray(v)&&v.length>0&&v.length<=16&&v.every(f=>obj(f)&&text(f.path,128)&&text(f.code,64)&&text(f.message,256)&&/^[A-Za-z0-9_.$\[\]]+$/u.test(f.path)&&/^[a-z_]+$/u.test(f.code)&&!/[\u0000-\u001f\u007f]/u.test(f.message));}
export function validModelingResponse(v:unknown):v is ModelingResponse {
 if(!obj(v)||v.schema_version!=="datum.service-modeling-candidate/1"||v.prompt_version!=="datum.service-modeling-assistant/1"||v.side_effects_performed!==false||!text(v.provider,128)||!text(v.model_ref,128)||!text(v.generated_at)||!Number.isFinite(Date.parse(v.generated_at))||!(v.base===null||validCatalogRevision(v.base))||!obj(v.validation)||!["valid","incomplete"].includes(String(v.validation.state))||!texts(v.validation.unresolved_fields)||!texts(v.validation.findings)||!obj(v.candidate))return false;
 const c=v.candidate;if(!obj(c.partial_definition)||Object.keys(c.partial_definition).some(k=>!["service_definition_id","name","artifact","runtime_kind","service_shape","composition","provisioning","tenancy","tenant_unit","resources","health_check","monitoring","secrets","exact_dependencies","requires","provides"].includes(k))||![c.assumptions,c.warnings,c.unsupported_requests,c.rationale].every(texts)||!Array.isArray(c.unresolved_questions)||c.unresolved_questions.length>32||!c.unresolved_questions.every(q=>obj(q)&&text(q.field,128)&&text(q.question))||!Array.isArray(c.provenance)||c.provenance.length>64||!c.provenance.every(p=>obj(p)&&text(p.field,128)&&text(p.reference,256)&&["operator_input","existing_catalog","catalog_context","selected_artifact","system_default","model_inference"].includes(String(p.source))))return false;
 const d=c.partial_definition;
 if(!c.unresolved_questions.every(q=>obj(q)&&text(q.question_id,128))||new Set(c.unresolved_questions.map(q=>(q as Record<string,unknown>).question_id)).size!==c.unresolved_questions.length)return false;
 if(d.service_shape!=null&&!["atomic","composite"].includes(String(d.service_shape))||d.tenancy!=null&&!["dedicated","shared_tenant_aware","shared_stateless","system_global"].includes(String(d.tenancy)))return false;
 if(!validPartial(d))return false;
 // Bounded JSON data only; renderer uses textContent, never provider HTML.
 return JSON.stringify(c).length<=65536;
}
function validPartial(d:Record<string,unknown>):boolean{
 const optional=(v:unknown,guard:(v:unknown)=>boolean)=>v==null||guard(v);
 const id=(v:unknown)=>text(v,128)&&/^[A-Za-z0-9._-]+$/u.test(v)&&v!=="."&&v!=="..";
 const cap=(v:unknown)=>text(v,128)&&/^[a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)+$/u.test(v);
 const ref=(v:unknown)=>obj(v)&&optional(v.definition_id,id)&&optional(v.revision,r=>typeof r==="number"&&Number.isSafeInteger(r)&&r>=1&&r<=1000000);
 if(!optional(d.service_definition_id,id)||!optional(d.name,v=>text(v,256))||!optional(d.runtime_kind,v=>["container","native_process"].includes(String(v))))return false;
 if(d.service_shape==="atomic"&&(d.composition!=null||d.provisioning!=null)||d.service_shape==="composite"&&(d.artifact!=null||d.runtime_kind!=null))return false;
 if(!optional(d.artifact,v=>obj(v)&&text(v.project_id,256)&&text(v.artifact_id,256)&&typeof v.execution_generation==="number"&&Number.isSafeInteger(v.execution_generation)&&v.execution_generation>0&&text(v.execution_projection_digest)&&/^sha256:[a-f0-9]{64}$/u.test(v.execution_projection_digest)))return false;
 if(!optional(d.resources,v=>obj(v)&&Array.isArray(v.capabilities)&&v.capabilities.length<=32&&v.capabilities.every(c=>obj(c)&&["network_interface","device","p4_port","gpu"].includes(String(c.kind))&&typeof c.exclusive==="boolean")&&["cpu_millis","memory_mb","storage_mb"].every((k,i)=>v[k]===null||typeof v[k]==="number"&&Number.isSafeInteger(v[k])&&v[k]>0&&v[k]<=[1000000,16777216,1000000000][i]!)))return false;
 if(!optional(d.requires,v=>Array.isArray(v)&&v.length<=64&&v.every(r=>obj(r)&&cap(r.capability)))||!optional(d.provides,v=>Array.isArray(v)&&v.length<=64&&v.every(cap))||!optional(d.exact_dependencies,v=>Array.isArray(v)&&v.length<=64&&v.every(ref)))return false;
 if(!optional(d.secrets,v=>Array.isArray(v)&&v.length<=64&&v.every(s=>obj(s)&&Object.keys(s).every(k=>["secret_ref","version","target_environment_variable"].includes(k))&&optional(s.secret_ref,v=>text(v,256))&&optional(s.version,v=>text(v,256))&&optional(s.target_environment_variable,id))))return false;
 if(!optional(d.monitoring,v=>obj(v)&&v.subject_kind==="dserv_realization"&&typeof v.cadence_seconds==="number"&&Number.isInteger(v.cadence_seconds)&&v.cadence_seconds>=1&&v.cadence_seconds<=86400)||!optional(d.health_check,v=>obj(v)&&["http","tcp_connect"].includes(String(v.kind))&&text(v.target,256)&&typeof v.port==="number"&&v.port>=1&&v.port<=65535&&typeof v.timeout_ms==="number"&&v.timeout_ms>=1&&v.timeout_ms<=60000))return false;
 if(d.composition!=null){const v=d.composition;if(!obj(v))return false;if(v.mode==="static"){if(!Array.isArray(v.components)||!v.components.length||v.components.length>64||!v.components.every(m=>obj(m)&&optional(m.key,id)&&ref(m.definition)))return false;}else if(v.mode==="discovered"){if(v.discovery!=null&&(!obj(v.discovery)||v.discovery.kind!=="docker_compose"||!Array.isArray(v.discovery.compose_files)||!v.discovery.compose_files.length||v.discovery.compose_files.length>16||!v.discovery.compose_files.every(s=>text(s,256)&&!s.includes("\\")&&!s.includes(":")&&s.split("/").every(p=>p&&p!=="."&&p!==".."))))return false;}else return false;}
 if(d.provisioning!=null){const p=d.provisioning;if(!obj(p)||p.kind!=="repository_command"||!optional(p.repository_url,validRepositoryUrl)||!optional(p.commit,v=>text(v,40)&&/^[a-f0-9]{40}$/u.test(v))||!optional(p.command,v=>Array.isArray(v)&&v.length>0&&v.length<=32&&v.every(s=>text(s,256)&&!/(?:password|token|secret|api_key)=/iu.test(s))))return false;}
 return true;
}
export class ModelingClient {
 public providerId?:string;
 public modelRef?:string;
 private navigation?:AbortSignal;
 constructor(navigation?:AbortSignal){this.navigation=navigation;}
 private async request(path:string,request?:ModelingRequest,signal?:AbortSignal):Promise<unknown>{
  let response:Response;try{response=await fetch(`/api/v1/catalog/modeling-assistant${path}`,{method:request?"POST":"GET",headers:request?{"Content-Type":"application/json"}:{},body:request?JSON.stringify(request):undefined,signal:signal&&this.navigation?AbortSignal.any([signal,this.navigation]):signal??this.navigation});}catch(e){if(signal?.aborted||this.navigation?.aborted)throw e;throw new CatalogError("unavailable","Modeling assistant could not be reached. Manual editing remains available.");}
  const raw=await response.text();if(raw.length>100000)throw new CatalogError("invalid_response","Modeling response could not be verified.");let body:unknown;try{body=JSON.parse(raw);}catch{throw new CatalogError("invalid_response","Modeling response could not be verified.");}
  if(!response.ok&&obj(body)&&obj(body.error)&&body.error.code==="provider_invalid_candidate"){
   if(!safeFindings(body.error.findings))throw new CatalogError("invalid_response","Modeling error findings could not be verified. No draft was applied.");
   throw new ModelingValidationError(body.error.findings.map(f=>({path:f.path,code:f.code,message:f.message})));
  }
  if(!response.ok){const code=obj(body)&&obj(body.error)&&text(body.error.code,64)?body.error.code:"unavailable";const messages:Record<string,string>={not_configured:"Assistant not configured. Use the manual editor; no simulated AI is enabled.",timeout:"Assistant timed out. No draft was applied.",invalid_response:"Provider output failed deterministic validation. No draft was applied.",fixture_no_scenario:"Deterministic test provider has no scenario for this input. Choose an exact preset key; no real AI call was made.",rate_limited:"Assistant capacity is unavailable. Try later."};throw new CatalogError(code,messages[code]??"Assistant unavailable. Manual editing remains available.");}return body;
 }
 async status():Promise<"available"|"not_configured">{const b=await this.request("");if(!obj(b)||b.schema_version!=="datum.service-modeling-status/1"||!["available","not_configured"].includes(String(b.state))||b.repository_analysis!=="not_included"||b.provider!==undefined&&!text(b.provider,128)||b.model_ref!==undefined&&!text(b.model_ref,128))throw new CatalogError("invalid_response","Assistant status could not be verified.");this.providerId=typeof b.provider==="string"?b.provider:undefined;this.modelRef=typeof b.model_ref==="string"?b.model_ref:undefined;return b.state as "available"|"not_configured";}
 async generate(request:ModelingRequest,signal?:AbortSignal):Promise<ModelingResponse>{const b=await this.request("/draft",request,signal);if(!validModelingResponse(b))throw new CatalogError("invalid_response","Modeling response could not be verified.");return b;}
}
