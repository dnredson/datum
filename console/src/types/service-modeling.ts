import type { ArtifactReference, DefinitionReference, DefinitionRevision, ServiceDefinition } from "./catalog.ts";
export interface ModelingRequest { intent:string;mode:"new_definition"|"new_revision";base:DefinitionReference|null;artifact:ArtifactReference|null;answers:{field:string;value:string}[];references:DefinitionReference[] }
export interface ModelingCandidate {
 partial_definition:Partial<ServiceDefinition>;
 assumptions:string[];unresolved_questions:{question_id:string;field:string;question:string}[];warnings:string[];
 provenance:{field:string;source:"operator_input"|"existing_catalog"|"catalog_context"|"selected_artifact"|"system_default"|"model_inference";reference:string}[];
 unsupported_requests:string[];rationale:string[];
}
export interface ModelingResponse {schema_version:"datum.service-modeling-candidate/1";prompt_version:"datum.service-modeling-assistant/1";candidate:ModelingCandidate;validation:{state:"valid"|"incomplete";unresolved_fields:string[];findings:string[]};provider:string;model_ref:string;generated_at:string;base:DefinitionRevision|null;side_effects_performed:false}
