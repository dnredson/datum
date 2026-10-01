/** Lossless projection of datum.dgraph/1. Decimal strings are transport/UI only:
 * canonicalJson emits exact integer JSON, never canvas fields or authority state. */
import type { ElementDefinition } from "cytoscape";
export interface DefinitionRef { definition_id:string;revision:number;digest:string }
export interface DServ { service_id:string; service_definition?:DefinitionRef; execution_requirement:{dnode_abi:string}; resource_requirement:{cpu_cores:number;memory_bytes:string} }
export interface DCall { call_id:string; source_service_id:string; target_service_id:string }
export interface DGraph { schema:"datum.dgraph/1"|"datum.dgraph/2";application_id:string;dgraph_id:string;revision:string;services:DServ[];calls:DCall[] }
export interface Base { application_id:string;dgraph_id:string;revision:string;digest:string }
export interface Finding { path:string;code:string;message:string }
const u64=(s:unknown):s is string=>typeof s==="string"&&/^(0|[1-9]\d{0,19})$/u.test(s)&&BigInt(s)<=18446744073709551615n;
export const editorId=(s:string)=>s.length>0&&new TextEncoder().encode(s).length<=256&&s.trim()===s&&!/[\u0000-\u001f\u007f-\u009f]/u.test(s);
const keys=(v:unknown,expected:string[]):v is Record<string,unknown>=>v!==null&&typeof v==="object"&&!Array.isArray(v)&&Object.keys(v).sort().join(",")===expected.sort().join(",");
/** Quote integer tokens only outside JSON strings before JS parsing, avoiding u64 rounding. */
export function parseGraph(raw:string):DGraph {
  if(raw.length>262144)throw new Error("Graph exceeds editor bounds");
  let out="",i=0;
  while(i<raw.length){const ch=raw[i]!;if(ch==='"'){const start=i++;while(i<raw.length){if(raw[i]==="\\"){i+=2;continue;}if(raw[i++]==='"')break;}out+=raw.slice(start,i);}else if(/[0-9-]/u.test(ch)){const start=i++;while(i<raw.length&&/[0-9.eE+-]/u.test(raw[i]!))i++;const n=raw.slice(start,i);if(!/^(0|[1-9]\d*)$/u.test(n))throw new Error("Expected unsigned integer");out+=JSON.stringify(n);}else {out+=ch;i++;}}
  const v:unknown=JSON.parse(out);
  // Type-only check of the original wire shape: never use its rounded numeric values.
  // This prevents quoting integer tokens from accidentally coercing IDs or accepting
  // quoted quantities. All numeric identity/quantity values come from the lossless parse.
  const wire=JSON.parse(raw);
  if(typeof wire.application_id!=="string"||typeof wire.dgraph_id!=="string"||typeof wire.revision!=="number"||!Array.isArray(wire.services)||!Array.isArray(wire.calls)||wire.services.some((s:any)=>typeof s?.service_id!=="string"||typeof s?.execution_requirement?.dnode_abi!=="string"||typeof s?.resource_requirement?.cpu_cores!=="number"||typeof s?.resource_requirement?.memory_bytes!=="number")||wire.calls.some((c:any)=>typeof c?.call_id!=="string"||typeof c?.source_service_id!=="string"||typeof c?.target_service_id!=="string"))throw new Error("Incorrect canonical wire types");
  if(!keys(v,["schema","application_id","dgraph_id","revision","services","calls"])||!["datum.dgraph/1","datum.dgraph/2"].includes(String(v.schema))||typeof v.application_id!=="string"||!editorId(v.application_id)||typeof v.dgraph_id!=="string"||!editorId(v.dgraph_id)||!u64(v.revision)||BigInt(v.revision)<1n||!Array.isArray(v.services)||v.services.length>2000||!Array.isArray(v.calls)||v.calls.length>4000)throw new Error("Malformed canonical graph");
  const ids=new Set<string>(),calls=new Set<string>();
  for(const [index,s] of v.services.entries()){
    const hasRef=Object.prototype.hasOwnProperty.call(s,"service_definition");
    if(!keys(s,["service_id","execution_requirement","resource_requirement",...(hasRef?["service_definition"]:[])])||typeof s.service_id!=="string"||!editorId(s.service_id)||ids.has(s.service_id)||!keys(s.execution_requirement,["dnode_abi"])||s.execution_requirement.dnode_abi!=="datum-dnode/0"||!keys(s.resource_requirement,["cpu_cores","memory_bytes"])||!u64(s.resource_requirement.cpu_cores)||BigInt(s.resource_requirement.cpu_cores)<1n||BigInt(s.resource_requirement.cpu_cores)>4294967295n||!u64(s.resource_requirement.memory_bytes)||BigInt(s.resource_requirement.memory_bytes)<1n)throw new Error("Malformed DServ");
    if(hasRef){const r=s.service_definition;if(v.schema!=="datum.dgraph/2"||!keys(r,["definition_id","revision","digest"])||typeof r.definition_id!=="string"||!/^[A-Za-z0-9._-]{1,128}$/u.test(r.definition_id)||[".",".."].includes(r.definition_id)||typeof wire.services[index].service_definition?.revision!=="number"||!u64(r.revision)||BigInt(r.revision)<1n||BigInt(r.revision)>1000000n||typeof r.digest!=="string"||!/^sha256:[a-f0-9]{64}$/u.test(r.digest))throw new Error("Malformed exact definition reference");r.revision=Number(r.revision);}
    ids.add(s.service_id);s.resource_requirement.cpu_cores=Number(s.resource_requirement.cpu_cores);
  }
  for(const c of v.calls){if(!keys(c,["call_id","source_service_id","target_service_id"])||typeof c.call_id!=="string"||!editorId(c.call_id)||calls.has(c.call_id)||typeof c.source_service_id!=="string"||!ids.has(c.source_service_id)||typeof c.target_service_id!=="string"||!ids.has(c.target_service_id))throw new Error("Malformed DCall");calls.add(c.call_id);}
  return v as unknown as DGraph;
}
export function canonicalJson(g:DGraph):string {
  if(!u64(g.revision)||g.services.some(s=>!u64(s.resource_requirement.memory_bytes)))throw new Error("Invalid exact integer");
  const services=[...g.services].sort((a,b)=>a.service_id<b.service_id?-1:a.service_id>b.service_id?1:0).map(s=>`{"service_id":${JSON.stringify(s.service_id)}${s.service_definition?`,"service_definition":${JSON.stringify(s.service_definition)}`:""},"execution_requirement":${JSON.stringify(s.execution_requirement)},"resource_requirement":{"cpu_cores":${JSON.stringify(s.resource_requirement.cpu_cores)},"memory_bytes":${s.resource_requirement.memory_bytes}}}`);
  const calls=[...g.calls].sort((a,b)=>a.call_id<b.call_id?-1:a.call_id>b.call_id?1:0);
  return `{"schema":${JSON.stringify(g.schema)},"application_id":${JSON.stringify(g.application_id)},"dgraph_id":${JSON.stringify(g.dgraph_id)},"revision":${g.revision},"services":[${services.join(",")}],"calls":${JSON.stringify(calls)}}`;
}
export function newGraph(application="",id=""):DGraph {return {schema:"datum.dgraph/1",application_id:application,dgraph_id:id,revision:"1",services:[],calls:[]};}
export function diff(base:DGraph|null,draft:DGraph){
  function compare<T>(before:T[],after:T[],key:(v:T)=>string){const a=new Map(before.map(v=>[key(v),JSON.stringify(v)])),b=new Map(after.map(v=>[key(v),JSON.stringify(v)]));return {added:[...b.keys()].filter(k=>!a.has(k)).sort(),modified:[...b.keys()].filter(k=>a.has(k)&&a.get(k)!==b.get(k)).sort(),removed:[...a.keys()].filter(k=>!b.has(k)).sort()};}
  return {nodes:compare(base?.services??[],draft.services,s=>s.service_id),edges:compare(base?.calls??[],draft.calls,c=>c.call_id)};
}
export function removeService(g:DGraph,id:string):void {g.services=g.services.filter(s=>s.service_id!==id);g.calls=g.calls.filter(c=>c.source_service_id!==id&&c.target_service_id!==id);}
export function draftElements(g:DGraph):ElementDefinition[]{const id=(kind:string,key:string)=>JSON.stringify([g.application_id,g.dgraph_id,kind,key]);const known=new Set(g.services.map(s=>s.service_id));return [...g.services.map((s,i)=>({data:{id:id("service",s.service_id),label:s.service_id,visual_category:"software",canonical_id:s.service_id},position:{x:100+(i%8)*180,y:90+Math.floor(i/8)*110}})),...g.calls.filter(c=>known.has(c.source_service_id)&&known.has(c.target_service_id)).map(c=>({data:{id:id("call",c.call_id),source:id("service",c.source_service_id),target:id("service",c.target_service_id),label:c.call_id,visual_relation:"logical",canonical_id:c.call_id}}))];}
