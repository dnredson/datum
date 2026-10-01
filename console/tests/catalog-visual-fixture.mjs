// TEST ONLY: ephemeral UI fixture, not catalog persistence or deployment authority.
// Run from console: node --experimental-strip-types tests/catalog-visual-fixture.mjs
import { createServer } from "node:http";
import { catalogFixture, containerSnapshot, compositeFixture, softwareCapabilityFixtures } from "./catalog-fixture.mjs";
const f = catalogFixture(), a = f.artifacts.items[0], b = structuredClone(a), c = containerSnapshot();
b.source.execution_generation = 2; b.source.execution_projection_digest = `sha256:${"2".repeat(64)}`; b.executable_sha256 = "3".repeat(64);
const one = structuredClone(f.record), two = structuredClone(one), magistrala = compositeFixture().record;
two.revision = 2; two.artifact_snapshot_id = `sha256:${"4".repeat(64)}`; two.definition.artifact = b.source;
const snapshots = new Map([[one.artifact_snapshot_id,a],[two.artifact_snapshot_id,b]]);
const {coffee,postgres}=softwareCapabilityFixtures();
snapshots.set(coffee.record.artifact_snapshot_id,coffee.artifacts.items[0]);snapshots.set(postgres.record.artifact_snapshot_id,postgres.artifacts.items[0]);
const revisions = new Map([["p4-sniffer",[one,two]],["magistrala",[magistrala]],["smart-coffee-api",[coffee.record]],["postgresql",[postgres.record]]]);
const live = [b,c,coffee.artifacts.items[0],postgres.artifacts.items[0]]; // Historical A is deliberately absent from the live source.
let sequence = 10;
createServer(async (req,res) => {
  res.setHeader("Content-Type","application/json"); res.setHeader("X-Datum-Test-Fixture","true");
  const send = (value,status=200) => {res.statusCode=status;res.end(JSON.stringify(value));};
  const error = (code,status) => send({error:{code,safe_message:"Test fixture request failed."}},status);
  const path = new URL(req.url,"http://127.0.0.1:8133").pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (path.slice(0,3).join("/") !== "api/v1/catalog") return error("not_found",404);
  if (req.method === "GET") {
    if (path[3]==="artifacts") return path[4] ? snapshots.has(path[4]) ? send(snapshots.get(path[4])) : error("not_found",404) : send({items:live,limit:100,has_more:false,source:"pinned_operational_artifact_references"});
    if (path[3]!=="services") return error("not_found",404);
    if (!path[4]) return send({items:[...revisions.values()].map(r=>r.at(-1)),limit:100,has_more:false});
    const history=revisions.get(path[4]),record=path[5]==="revisions" ? history?.find(r=>r.revision===Number(path[6])) : history?.at(-1);
    return record ? send(record) : error("not_found",404);
  }
  if (req.method!=="POST" || path[3]!=="services") return error("not_found",405);
  try {
    let text="";for await(const part of req){text+=part;if(text.length>65536)throw new Error();}
    const {definition,expected_revision}=JSON.parse(text),history=revisions.get(definition.service_definition_id)??[];
    if (expected_revision!==(history.at(-1)?.revision??0)) return error("revision_conflict",409);
    const snapshot=live.find(a=>JSON.stringify(a.source)===JSON.stringify(definition.artifact));if(definition.service_shape==="atomic"&&!snapshot)return error("invalid_definition",400);
    const id=`sha256:${String(sequence++).padStart(64,"0")}`;
    const record={definition,revision:expected_revision+1,digest:id,artifact_snapshot_id:snapshot?id:null,created_at_utc:new Date().toISOString(),scope:"global_catalog"};
    if(snapshot)snapshots.set(id,structuredClone(snapshot));history.push(record);revisions.set(definition.service_definition_id,history);send(record);
  } catch {error("invalid_definition",400);}
}).listen(8133,"127.0.0.1",()=>console.log("TEST-ONLY catalog fixture on http://127.0.0.1:8133; ephemeral saves, no production data or runtime actions."));
