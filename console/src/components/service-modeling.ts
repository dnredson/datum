import {el} from "./presentation.ts";
import {CatalogError} from "../api/catalog.ts";
import {ModelingClient,ModelingValidationError} from "../api/service-modeling.ts";
import type {ModelingRequest,ModelingResponse} from "../types/service-modeling.ts";
export function modelingView(client:ModelingClient, requestBase:()=>Omit<ModelingRequest,"intent"|"answers">, apply:(r:ModelingResponse)=>void, candidateChanged?:()=>void):HTMLElement {
 const panel=el("section","console-card modeling-panel");panel.dataset.modelingAssistant="true";
 const heading=el("h2","section-title","Build with assistant · modeling only");
 const form=el("form"),label=el("label","form-label","Describe the software (Português or English; never paste credentials)");
 const intent=el("textarea","form-control");intent.id="modeling-intent";intent.required=true;intent.maxLength=8192;intent.rows=4;label.htmlFor=intent.id;
 const status=el("p","small");status.setAttribute("role","status");status.setAttribute("aria-live","polite");
 const result=el("section","modeling-candidate");result.tabIndex=-1;result.setAttribute("aria-label","Candidate review, assumptions and unresolved questions");
 const generate=el("button","btn btn-outline-primary","Generate modeling draft");generate.type="submit";
 const cancel=el("button","btn btn-outline-secondary","Cancel generation");cancel.type="button";cancel.hidden=true;
 form.append(label,intent,el("p","small text-secondary","No automatic save, repository fetch, command execution, placement or deployment. Candidate output is never authority."),generate,cancel);
 panel.append(heading,status,form,result);let sequence=0,active:AbortController|undefined,answers:{field:string;value:string}[]=[];
 void client.status().then(state=>{status.textContent=state==="not_configured"?"Assistant not configured. Manual editing remains fully available.":client.providerId==="test_only"?"TEST PROVIDER · deterministic fixture · no real AI request was made. Use preset scenario keys only.":`AI assistant available · ${client.providerId??"provider"} · ${client.modelRef??"configured model"}. Operator review and explicit Save required.`;generate.disabled=state!=="available";if(state==="available"&&client.providerId==="test_only"){const presets=el("div","d-flex flex-wrap gap-2");presets.setAttribute("aria-label","Deterministic test scenarios");for(const key of ["magistrala","smart-coffee","p4-sniffer","incomplete","invalid","unsupported-lifecycle","not-configured"]){const button=el("button","btn btn-sm btn-outline-secondary",key);button.type="button";button.addEventListener("click",()=>{intent.value=key;intent.focus();});presets.append(button);}form.append(presets);}}).catch(()=>{status.textContent="Assistant unavailable. Manual editing remains available.";generate.disabled=true;});
 cancel.addEventListener("click",()=>{++sequence;active?.abort();active=undefined;generate.disabled=false;cancel.hidden=true;status.textContent="Generation cancelled. No candidate was applied.";generate.focus();});
 form.addEventListener("submit",async event=>{
  event.preventDefault();if(active||!form.reportValidity())return;
  const token=++sequence;active=new AbortController();const signal=active.signal;generate.disabled=true;cancel.hidden=false;status.textContent="Generating modeling draft…";
  try{const r=await client.generate({...requestBase(),intent:intent.value,answers:answers.filter(a=>a.value.length).slice(0,8)},signal);if(token!==sequence||signal.aborted)return;
   candidateChanged?.();
   if(r.provider==="test_only")result.dataset.testProvider="true";else delete result.dataset.testProvider;
   result.replaceChildren(el("h3","section-title","AI-generated draft — not saved"),el("p","small",`Deterministic validator: ${r.validation.state}. Provider ${r.provider} · ${r.model_ref}.`));
   if(r.provider==="test_only")result.prepend(el("p","fw-semibold","TEST PROVIDER · Deterministic fixture · No real AI request was made"));
   for(const [title,items] of [["Rationale",r.candidate.rationale],["Assumptions / suggestions",r.candidate.assumptions],["AI warnings",r.candidate.warnings],["Unsupported requests",r.candidate.unsupported_requests],["Validator unresolved fields",r.validation.unresolved_fields],["Validator findings",r.validation.findings]] as const){if(items.length){result.append(el("h4","small fw-semibold",title));const list=el("ul");items.forEach(s=>list.append(el("li","small",s)));result.append(list);}}
   answers=[];for(const q of r.candidate.unresolved_questions){const label=el("label","form-label",`${q.question} (${q.field})`),input=el("input","form-control");input.id=`modeling-answer-${encodeURIComponent(q.question_id)}`;input.dataset.questionField=q.field;input.maxLength=2048;label.htmlFor=input.id;const answer={field:q.field,value:""};answers.push(answer);input.addEventListener("input",()=>{answer.value=input.value;});result.append(label,input);}
   const preview=el("pre","modeling-preview",JSON.stringify(r.candidate.partial_definition,null,2));preview.setAttribute("aria-label","Structured candidate fields · not catalog persistence");result.append(preview);
   if(r.candidate.provenance.length){result.append(el("h4","small fw-semibold","Field provenance"));r.candidate.provenance.forEach(p=>result.append(el("p","small",`${p.field} · ${p.source} · ${p.reference}`)));}
   const button=el("button","btn btn-outline-primary","Apply draft to manual editor (does not save)");button.type="button";button.addEventListener("click",()=>apply(r));result.append(button);status.textContent="Candidate ready for review. Apply and Save are separate actions.";result.focus({preventScroll:true});
  }catch(e){if(token!==sequence)return;status.textContent=e instanceof CatalogError?e.message:"Modeling assistant could not produce a draft. Nothing was saved.";
   if(e instanceof ModelingValidationError){candidateChanged?.();result.replaceChildren(el("h3","section-title","AI draft rejected by deterministic validator"),el("p","small","No draft was applied."));const list=el("ul");for(const f of e.findings)list.append(el("li","small",`${f.path} — ${f.message} (${f.code})`));result.append(list);}
   result.focus({preventScroll:true});}
  finally{if(token===sequence){active=undefined;generate.disabled=false;cancel.hidden=true;}}
 });return panel;
}
