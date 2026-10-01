import {catalogFixture,compositeFixture,softwareCapabilityFixtures} from "./catalog-fixture.mjs";
export function modelingFixture(kind="magistrala"){
 let definition;
 if(kind==="magistrala"){definition=compositeFixture().record.definition;definition.provisioning.commit=null;}
 else if(kind==="coffee"){definition=softwareCapabilityFixtures().coffee.record.definition;definition.artifact=null;definition.runtime_kind="container";definition.resources={cpu_millis:null,memory_mb:null,storage_mb:null,capabilities:[]};definition.monitoring=null;definition.provides=[];}
 else definition=catalogFixture().record.definition;
 if(kind==="incomplete"){definition.service_definition_id=null;definition.name=null;definition.artifact=null;}
 const unresolved=kind==="magistrala"?["provisioning.commit"]:kind==="coffee"?["artifact"]:kind==="incomplete"?["service_definition_id","name","artifact"]:[];
 const prompts={"service_definition_id":"What stable service definition ID should be used?",name:"What display name should be used?",artifact:"Which existing pinned artifact should this Atomic service use?","provisioning.commit":"Which immutable 40-hex repository commit should be used?"};
 return {schema_version:"datum.service-modeling-candidate/1",prompt_version:"datum.service-modeling-assistant/1",candidate:{partial_definition:definition,assumptions:kind==="coffee"?["Dedicated tenancy is a suggestion; confirm it."]:[],unresolved_questions:unresolved.map(field=>({question_id:`missing-${field}`,field,question:prompts[field]})),warnings:["TEST PROVIDER. Deterministic fixture. No real AI request was made.","Modeling only. No execution, provider binding or deployment."],provenance:[{field:"service_shape",source:"operator_input",reference:"explicit fixture scenario"}],unsupported_requests:[],rationale:[kind==="magistrala"?"Composite product, not one container.":"Atomic service; external providers remain requirements."]},validation:{state:unresolved.length?"incomplete":"valid",unresolved_fields:unresolved,findings:[]},provider:"test_only",model_ref:"deterministic_fixture",generated_at:"2026-10-01T00:00:00Z",base:null,side_effects_performed:false};
}
export function fixtureScenario(intent){return ({magistrala:"magistrala","smart-coffee":"coffee","p4-sniffer":"p4",incomplete:"incomplete"})[intent]??null;}
