// Optional headless DOM gate, NOT graphical/visual browser verification.
// Happy DOM is supplied from a disposable external install, never an app dependency.
// Set DATUM_DOM_GATE_MODULE=/absolute/path/to/happy-dom/lib/index.js,
// then run: node --experimental-strip-types --test tests/freeze-gate.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import { wallboardFixture } from "./wallboard-fixture.mjs";
import { projectFixture, sharedEvidence } from "./project-fixture.mjs";
import { graphFixture, unevenGraphFixture } from "./graph-fixture.mjs";
import { runtimeFixture } from "./runtime-graph-fixture.mjs";
import { catalogFixture, containerSnapshot, compositeFixture } from "./catalog-fixture.mjs";

const modulePath = process.env.DATUM_DOM_GATE_MODULE;
if (!modulePath)
  throw new Error(
    "Set DATUM_DOM_GATE_MODULE to the temporary Happy DOM module path.",
  );
const { Window } = await import(pathToFileURL(modulePath).href);
const assets = await readdir(new URL("../dist/assets/", import.meta.url));
const bundle = await readFile(
  new URL(
    `../dist/assets/${assets.find((name) => /^index-.*\.js$/u.test(name))}`,
    import.meta.url,
  ),
  "utf8",
);
const css = await readFile(
  new URL("../src/styles/console.css", import.meta.url),
  "utf8",
);
const live = await (
  await fetch("http://127.0.0.1:5173/api/v1/operations/overview")
).json();
const liveProject = await (
  await fetch(
    "http://127.0.0.1:5173/api/v1/operations/projects/foundation-demo",
    { headers: { "X-Datum-Project": "foundation-demo" } },
  )
).json();
const known = structuredClone(live);
known.known_projects.page.items = ["foundation-demo"];
const calls = [];

test("Phase133A catalog form saves modeling only, preserves tenancy and references", async () => {
  const f=catalogFixture(); let saved;
  const window=boot("/catalog",768,(url,init)=>{
    if(url.endsWith("artifacts"))return f.artifacts;
    if(init.method==="POST"){saved=JSON.parse(init.body);return {...f.record,definition:saved.definition};}
    return f.page;
  });
  try {
    await waitFor(window,()=>!!window.document.querySelector('[data-catalog-form]'));
    const doc=window.document;
    assert.match(text(window),/Global reusable definitions/);
    const form=doc.querySelector('form');
    form.elements.namedItem("id").value="p4-sniffer";form.elements.namedItem("name").value="P4 Sniffer";
    const tenancy=form.elements.namedItem("tenancy");tenancy.value="shared_tenant_aware";tenancy.dispatchEvent(new window.Event("change"));
    form.elements.namedItem("tenant-unit").value="domain";form.elements.namedItem("cadence").value="30";
    [...form.querySelectorAll('button')].find(b=>b.textContent==="Review definition").click();assert.match(text(window),/Shared tenant-aware/);
    form.dispatchEvent(new window.Event("submit",{bubbles:true,cancelable:true}));
    await waitFor(window,()=>text(window).includes("Saved p4-sniffer revision 1"));
    assert.equal(saved.expected_revision,0);assert.equal(saved.definition.tenant_unit,"domain");assert.equal(saved.definition.monitoring.cadence_seconds,30);
    assert.equal(doc.querySelector('input[type="password"]'),null);assert.doesNotMatch(text(window),/Deploy now|Accept D-Map|Create tenant binding/);
    assert.equal(window.getComputedStyle(doc.querySelector('.catalog-grid')).gridTemplateColumns,"minmax(0, 1fr)");
  }finally{await window.happyDOM.close();}
});
test("Phase133A historical revisions are read-only and validation errors are safely associated",async()=>{
 const f=catalogFixture();const window=boot("/catalog",1024,(url)=>url.endsWith("artifacts")?f.artifacts:url.includes("/revisions/")?f.record:f.page);
 try{await waitFor(window,()=>!!window.document.querySelector('form'));
 const doc=window.document;[...doc.querySelectorAll('button')].find(b=>b.textContent==="Inspect historical revision").click();
 await waitFor(window,()=>!!doc.querySelector('fieldset:disabled'));
 assert.match(text(window),/Historical revision 1.*read-only/s);assert.ok(doc.querySelector('label[for="catalog-cadence"]'));
 assert.equal(doc.querySelector('#catalog-secret-ref').getAttribute('aria-describedby'),'catalog-validation-error');
 }finally{await window.happyDOM.close();}
});
test("Phase133A selected native/container artifacts expose complete immutable pins before Save",async()=>{
 const f=catalogFixture(),container=containerSnapshot();f.artifacts.items.push(container);
 const window=boot("/catalog",1024,url=>url.endsWith("artifacts")?f.artifacts:f.page);
 try{await waitFor(window,()=>!!window.document.querySelector('[data-artifact-pins]'));
 const doc=window.document;assert.ok(text(window).includes("d".repeat(64)));
 const selector=doc.querySelector('#catalog-artifact');selector.value="1";selector.dispatchEvent(new window.Event("change"));
 const pins=doc.querySelector('[data-artifact-pins]').textContent;
 assert.match(pins,/linux\/amd64/);assert.ok(pins.includes(container.resolved_image_identity.platforms[0].manifest_digest));assert.ok(pins.includes(container.resolved_image_identity.platforms[0].config_digest));assert.ok(pins.includes(container.resolved_image_identity.platforms[0].execution_reference));assert.ok(pins.includes(container.source.execution_projection_digest));
 }finally{await window.happyDOM.close();}
});
test("Phase133A historical A/B pins remain exact after live artifact source removal",async()=>{
 const f=catalogFixture(),a=f.artifacts.items[0],b=containerSnapshot(),one=structuredClone(f.record),two=structuredClone(f.record);
 two.revision=2;two.artifact_snapshot_id=`sha256:${"9".repeat(64)}`;two.definition.artifact=b.source;two.definition.runtime_kind="container";
 f.page.items=[two];f.artifacts.items=[];const requests=[];
 const window=boot("/catalog",1024,url=>{requests.push(url);if(url.endsWith("artifacts"))return f.artifacts;if(url.includes("/artifacts/"))return url.includes(encodeURIComponent(one.artifact_snapshot_id))?a:b;if(url.endsWith("/revisions/1"))return one;return f.page;});
 try{await waitFor(window,()=>!!window.document.querySelector('form'));
 const doc=window.document;
 [...doc.querySelectorAll('button')].find(b=>b.textContent==="Inspect pinned artifact").click();
 await waitFor(window,()=>text(window).includes(b.resolved_image_identity.platforms[0].manifest_digest));
 assert.equal(doc.querySelector('#catalog-artifact'),null);assert.ok(doc.querySelector('fieldset:disabled'));
 doc.querySelector('input[aria-label="Historical revision for p4-sniffer"]').value="1";
 [...doc.querySelectorAll('button')].find(b=>b.textContent==="Inspect historical revision").click();
 await waitFor(window,()=>text(window).includes("d".repeat(64)));
 const pins=doc.querySelector('[data-artifact-pins]').textContent;assert.ok(pins.includes(one.artifact_snapshot_id));assert.ok(!pins.includes(b.resolved_image_identity.platforms[0].manifest_digest));assert.ok(requests.includes(`/api/v1/catalog/artifacts/${encodeURIComponent(one.artifact_snapshot_id)}`));assert.equal(doc.querySelector('#catalog-artifact'),null);
 }finally{await window.happyDOM.close();}
});
test("Phase133A composite editor declares discovery and external capabilities without execution",async()=>{
 const f=catalogFixture();let saved;
 const window=boot("/catalog",1024,(url,init)=>{if(url.endsWith("artifacts"))return f.artifacts;if(init.method==="POST"){saved=JSON.parse(init.body);return {...f.record,definition:saved.definition,artifact_snapshot_id:null};}return f.page;});
 try{await waitFor(window,()=>!!window.document.querySelector('form'));const doc=window.document,form=doc.querySelector('form'),value=(n,v)=>form.elements.namedItem(n).value=v;
 value("id","magistrala");value("name","Magistrala");value("shape","composite");form.elements.namedItem("shape").dispatchEvent(new window.Event("change"));value("composition-mode","discovered");form.elements.namedItem("composition-mode").dispatchEvent(new window.Event("change"));
 value("repository","https://github.com/absmach/magistrala.git");value("commit","a".repeat(40));value("requires","database.postgresql messaging.mqtt");value("provides","api.magistrala");value("tenancy","shared_tenant_aware");form.elements.namedItem("tenancy").dispatchEvent(new window.Event("change"));value("tenant-unit","domain");
 [...form.querySelectorAll('button')].find(b=>b.textContent==="Review definition").click();assert.match(text(window),/composite discovered/);assert.match(text(window),/database.postgresql/);assert.equal(form.elements.namedItem("artifact").required,false);assert.equal(form.elements.namedItem("dependencies"),null);
 form.dispatchEvent(new window.Event("submit",{bubbles:true,cancelable:true}));await waitFor(window,()=>!!saved);assert.equal(saved.definition.artifact,null);assert.deepEqual(saved.definition.provisioning.command,["make","run_latest"]);assert.equal(saved.definition.composition.discovery.kind,"docker_compose");assert.equal(saved.definition.requires.length,2);
 }finally{await window.happyDOM.close();}
});
test("Phase133A static component and dependency editor preserve exact identities on new revision",async()=>{
 const f=compositeFixture();f.record.definition.provisioning=null;f.record.definition.composition={mode:"static",components:[{key:"db",definition:{definition_id:"postgresql",revision:7}}]};f.record.definition.exact_dependencies=[{definition_id:"mqtt",revision:4}];
 const window=boot("/catalog",768,url=>url.endsWith("artifacts")?f.artifacts:f.page);
 try{await waitFor(window,()=>!!window.document.querySelector('form'));const doc=window.document;
 [...doc.querySelectorAll('button')].find(b=>b.textContent==="Create new revision").click();assert.equal(doc.querySelector('#catalog-shape').value,"composite");assert.equal(doc.querySelector('#catalog-component-id').value,"postgresql");assert.equal(doc.querySelector('#catalog-component-revision').value,"7");assert.equal(doc.querySelector('#catalog-dependency-revision').value,"4");
 [...doc.querySelectorAll('button')].find(b=>b.textContent==="Inspect composite definition").click();assert.ok(doc.querySelector('fieldset:disabled'));assert.equal(doc.querySelector('#catalog-component-revision').value,"7");assert.doesNotMatch(text(window),/Dependency definition IDs \(comma-separated\)/);
 }finally{await window.happyDOM.close();}
});

async function graphCheck(data, assertion, width = 1440) {
  const start = calls.length;
  const window = boot(
    `/projects/${encodeURIComponent(data.project_id)}/graph`,
    width,
    () => data,
  );
  try {
    await ready(window);
    await assertion(window, calls.slice(start));
  } finally {
    await window.happyDOM.close();
  }
}

test("Phase132 F1 focus follows qualified service through repeated expansion and collapse", async () => {
  await graphCheck(runtimeFixture(), (window) => {
    const doc = window.document;
    const control = (id) => [...doc.querySelectorAll(".graph-expand")]
      .find((b) => b.dataset.serviceId === id);
    for (const id of ["app-a-two", "app-b-one", "app-a-one", "app-b-one"]) {
      const collapsed = control(id);
      collapsed.focus();
      assert.equal(doc.activeElement, collapsed);
      collapsed.click(); // Native button activation shared by keyboard/pointer.
      const expanded = control(id);
      assert.equal(doc.activeElement, expanded);
      assert.equal(expanded.getAttribute("aria-expanded"), "true");
      const entries = [...expanded.parentElement.querySelectorAll(".graph-entity-button")]
        .filter((b) => b.textContent.startsWith("Realization evidence"));
      assert.equal(entries.length, 2);
      entries[1].focus();
      entries[1].click();
      assert.ok(doc.querySelector(".graph-evidence-details").textContent);
      expanded.focus();
      expanded.click();
      assert.equal(doc.activeElement, control(id));
      assert.equal(control(id).getAttribute("aria-expanded"), "false");
      assert.equal(doc.querySelector(".graph-evidence-details").textContent, "");
    }
  });
});
test("Phase132 F2 duplicate observation response renders safe error and no partial graph", async () => {
  const data = runtimeFixture();
  data.runtime.value.evidence[1].observation_id = data.runtime.value.evidence[0].observation_id;
  data.temporal_quality[1].observation_id = data.runtime.value.evidence[0].observation_id;
  await graphCheck(data, (window) => {
    assert.match(text(window), /invalid runtime evidence projection/);
    assert.equal(window.document.querySelector(".project-graph"), null);
    assert.equal(window.document.querySelector(".graph-expand"), null);
    assert.equal(window.document.querySelector(".graph-canvas"), null);
    assert.doesNotMatch(text(window), /observation-app|epoch-|sha256/);
  });
});
test("Phase132 F2 selected evidence retains its exact timestamps and temporal dimensions", async () => {
  const data = runtimeFixture();
  const second = data.runtime.value.evidence[1];
  second.observed_at_utc = "2026-09-30T15:01:00Z";
  second.received_at_utc = "2026-09-30T15:00:30Z";
  Object.assign(data.temporal_quality[1], {
    source_observed_at_utc: second.observed_at_utc,
    server_received_at_utc: second.received_at_utc,
    communication_freshness: "stale", observation_freshness: "fresh",
    source_time_quality: "source_ahead",
  });
  await graphCheck(data, (window) => {
    const doc = window.document;
    doc.querySelector('.graph-expand[data-service-id="app-a-one"]').click();
    const entries = [...doc.querySelectorAll(".graph-entity-button")]
      .filter((b) => b.textContent.startsWith("Realization evidence"));
    for (const i of [0, 1, 0]) {
      entries[i].click();
      const detail = doc.querySelector(".graph-evidence-details").textContent;
      const e = data.runtime.value.evidence[i], t = data.temporal_quality[i];
      for (const value of [e.observation_id, e.observed_at_utc, e.received_at_utc,
        `Communication · ${t.communication_freshness}`,
        `Observation · ${t.observation_freshness}`,
        `Source time · ${t.source_time_quality.replaceAll("_", " ")}`])
        assert.ok(detail.includes(value), value);
      assert.ok(!detail.includes(data.runtime.value.evidence[1 - i].observation_id));
    }
  });
});
test("Phase132 runtime expansion, details, filter and Reset remain read-only and local", async () => {
  const data = runtimeFixture();
  const start = calls.length;
  const window = boot("/projects/alpha/graph", 1024, () => data);
  try {
    await ready(window);
    const doc = window.document;
    assert.ok(
      [...doc.querySelectorAll(".graph-expand")].every(
        (b) => b.getAttribute("aria-expanded") === "false",
      ),
    );
    const toggle = doc.querySelector(".graph-expand");
    toggle.click();
    assert.equal(
      doc.querySelector(".graph-expand").getAttribute("aria-expanded"),
      "true",
    );
    const entry = [...doc.querySelectorAll(".graph-entity-button")].find((b) =>
      b.textContent.startsWith("Realization evidence"),
    );
    entry.click();
    assert.match(text(window), /Monitor epoch \(not execution-instance ID\)/);
    assert.match(text(window), /Source observation time.*Server receipt time/s);
    assert.match(
      text(window),
      /Communication · fresh.*Observation · stale.*Source time · behind or delayed/s,
    );
    assert.match(text(window), /node reference \(not ownership\)/);
    assert.doesNotMatch(text(window), /fog-0 belongs|owns fog-0|occupied by/);
    assert.match(
      doc.querySelector(".graph-visible-legend").textContent,
      /Runtime realization/,
    );
    const filter = doc.querySelector(".graph-filter");
    filter.value = "app-b";
    filter.dispatchEvent(new window.Event("change"));
    assert.ok(
      [...doc.querySelectorAll(".graph-expand")].every(
        (b) => b.getAttribute("aria-expanded") === "false",
      ),
    );
    assert.equal(doc.querySelector(".graph-evidence-details").textContent, "");
    filter.value = "";
    filter.dispatchEvent(new window.Event("change"));
    doc.querySelector(".graph-expand").click();
    const reset = [...doc.querySelectorAll("button")].find(
      (b) => b.textContent === "Reset selection / view",
    );
    reset.click();
    assert.ok(
      [...doc.querySelectorAll(".graph-expand")].every(
        (b) => b.getAttribute("aria-expanded") === "false",
      ),
    );
    assert.doesNotMatch(
      doc.querySelector(".graph-visible-legend").textContent,
      /Runtime realization/,
    );
    assert.equal(calls.length - start, 1);
  } finally {
    await window.happyDOM.close();
  }
});
test("Phase132 runtime bounded previews expose included/omitted evidence, including bounded-empty", async () => {
  for (const empty of [false, true]) {
    const f = runtimeFixture(undefined, !empty);
    f.runtime.value.completeness = "partial";
    f.runtime.value.total_evidence_count = 1001;
    const window = boot("/projects/alpha/graph", 1440, () => f);
    try {
      await ready(window);
      assert.match(text(window), /Partial evidence preview/);
      assert.match(
        text(window),
        /of 1001 latest subject\/monitor-epoch records included/,
      );
      assert.match(text(window), /whole-service omission/);
      if (empty)
        assert.match(text(window), /No whole service evidence set fit/);
      assert.match(
        window.document.querySelector("#connection").textContent,
        /Degraded/,
      );
    } finally {
      await window.happyDOM.close();
    }
  }
});
test("Phase132 unmatched evidence remains inspectable; degraded source does not remove logical structure", async () => {
  const data = runtimeFixture();
  data.runtime.value.evidence[0].service_id = "not-in-logical";
  data.runtime.value.evidence[0].logical_node_id = null;
  const unmatched = data.runtime.value.evidence[0];
  unmatched.id = JSON.stringify([
    "runtime_evidence",
    unmatched.project_id,
    unmatched.application_id,
    unmatched.service_id,
    unmatched.node_id,
    unmatched.reporting_node_id,
    unmatched.monitor_instance_id,
  ]);
  const window = boot("/projects/alpha/graph", 768, () => data);
  try {
    await ready(window);
    assert.match(text(window), /Evidence not matched to loaded accepted/);
    assert.ok(window.document.querySelector(".graph-entity-list"));
  } finally {
    await window.happyDOM.close();
  }
  for (const availability of [
    "not_configured",
    "indeterminate",
    "unavailable",
    "corrupt",
  ]) {
    const f = runtimeFixture();
    f.runtime = {
      availability,
      value:
        availability === "indeterminate"
          ? "uncertain"
          : {
              source: "dmonitor",
              code: "source_failed",
              safe_message: "Safe failure",
            },
    };
    f.temporal_quality = [];
    const w = boot("/projects/alpha/graph", 1440, () => f);
    try {
      await ready(w);
      assert.ok(w.document.querySelector(".graph-entity-list"));
      assert.match(text(w), /No zero or health claim|cannot currently provide/);
      assert.doesNotMatch(
        w.document.querySelector(".graph-runtime-source").textContent,
        /0 of/,
      );
    } finally {
      await w.happyDOM.close();
    }
  }
});
test("Phase131 graph navigation aborts obsolete reads and does not overwrite project Overview", async () => {
  let oldSignal;
  const window = boot("/projects/alpha/graph", 1440, (url, init) => {
    if (url.endsWith("/runtime-graph")) {
      oldSignal = init.signal;
      return new Promise((_resolve, reject) =>
        init.signal.addEventListener(
          "abort",
          () => reject(init.signal.reason),
          { once: true },
        ),
      );
    }
    return projectFixture();
  });
  try {
    window.history.pushState(null, "", "/projects/alpha");
    window.dispatchEvent(new window.PopStateEvent("popstate"));
    await ready(window);
    assert.equal(oldSignal.aborted, true);
    assert.ok(window.document.querySelector(".project-workspace"));
    assert.equal(window.document.querySelector(".project-graph"), null);
    const graphLink = window.document.querySelector(
      '.project-subnav a[href="/projects/alpha/graph"]',
    );
    assert.ok(graphLink);
  } finally {
    await window.happyDOM.close();
  }
});
test("Phase131 graph route identifies accepted logical source and enables only Overview / Graph", async () => {
  await graphCheck(graphFixture(), (window, requests) => {
    assert.match(
      text(window),
      /Logical graph overview.*Accepted application D-Graph snapshots.*logical structure/s,
    );
    assert.equal(requests.length, 1);
    assert.equal(
      requests[0].url,
      "/api/v1/operations/projects/alpha/runtime-graph",
    );
    assert.equal(requests[0].init.headers["X-Datum-Project"], "alpha");
    const nav = window.document.querySelector(".project-subnav");
    assert.equal(nav.querySelectorAll("a").length, 2);
    assert.equal(nav.querySelectorAll('[aria-disabled="true"]').length, 5);
    assert.equal(
      nav.querySelector('[aria-current="page"]').textContent,
      "Graph",
    );
    assert.match(
      window.document.querySelector(".graph-legend").textContent,
      /cycles.*self-calls.*No deployment-order/s,
    );
    assert.equal(
      window.document.querySelectorAll(".graph-entity-button").length,
      12,
    );
  });
});
test("Phase131 keyboard alternative selects full typed node identity and bounded declared metadata", async () => {
  await graphCheck(graphFixture(), (window) => {
    const button = window.document.querySelector(
      '[data-entity-id="app-a-one"]',
    );
    button.focus();
    assert.equal(window.document.activeElement, button);
    button.click();
    const detail = window.document.querySelector(".graph-detail-body");
    assert.match(
      detail.textContent,
      /D-Serv.*Canonical ID · one.*Application · app-a.*D-Graph · logical-app-a.*revision · 4/s,
    );
    assert.match(
      detail.textContent,
      /Incoming D-Calls · 2 \/ Outgoing D-Calls · 2/,
    );
    assert.ok(detail.textContent.includes("18446744073709551615"));
    assert.doesNotMatch(
      detail.textContent,
      /Healthy|Running correctly|Placed on/,
    );
  });
});
test("Phase131 D-Call selection preserves source/target/canonical call identity without editing", async () => {
  await graphCheck(graphFixture(), (window) => {
    window.document.querySelector('[data-entity-id="app-a-self"]').click();
    const detail =
      window.document.querySelector(".graph-detail-body").textContent;
    assert.match(
      detail,
      /D-Call.*Canonical ID · self.*Source D-Serv · one.*Target D-Serv · one/s,
    );
    assert.equal(
      window.document.querySelectorAll(".graph-details button").length,
      0,
    );
    assert.doesNotMatch(
      text(window),
      /Save graph|Add node|Remove node|Deploy now|Assign service/,
    );
  });
});
test("Phase131 local application filter changes visible structure only and Reset clears selection", async () => {
  const data = graphFixture();
  const original = JSON.stringify(data);
  await graphCheck(data, (window, requests) => {
    const filter = window.document.querySelector(".graph-filter");
    filter.value = "app-b";
    filter.dispatchEvent(new window.Event("change", { bubbles: true }));
    assert.equal(
      window.document.querySelectorAll(".graph-entity-button").length,
      6,
    );
    assert.match(
      window.document.querySelector(".graph-entity-list").textContent,
      /app-b/,
    );
    assert.doesNotMatch(
      window.document.querySelector(".graph-entity-list").textContent,
      /app-a/,
    );
    window.document.querySelector('[data-entity-id="app-b-one"]').click();
    [...window.document.querySelectorAll(".graph-toolbar button")]
      .find((b) => b.textContent.startsWith("Reset"))
      .click();
    assert.match(
      window.document.querySelector(".graph-detail-body").textContent,
      /Select an entity/,
    );
    assert.equal(requests.length, 1);
    assert.equal(JSON.stringify(data), original);
  });
});
test("Phase131 partial graph coverage remains explicit and source revision is the DGraph revision", async () => {
  const data = graphFixture();
  data.graph.value.coverage.completeness = "partial";
  data.graph.value.coverage.active_application_count = 5;
  await graphCheck(data, (window) => {
    assert.match(
      text(window),
      /Partial accepted-snapshot coverage.*Bounded graph preview: 2 of 5 accepted/s,
    );
    assert.match(
      window.document.querySelector("#connection").textContent,
      /Degraded/,
    );
    assert.match(
      window.document.querySelector(".graph-application-list").textContent,
      /D-Graph · logical-app-a · revision 4/,
    );
    assert.doesNotMatch(text(window), /DMap revision/);
    assert.match(
      text(window),
      /3 accepted snapshots were omitted by preview limits/,
    );
    assert.match(text(window), /later snapshots are not considered/);
  });
});

test("graph visual semantics legend shows only supported visible types without fabricated future categories", async () => {
  const data = graphFixture();
  data.graph.value.nodes = data.graph.value.nodes.filter(
    (node) => node.application_id !== "app-b" || node.kind === "application",
  );
  data.graph.value.edges = data.graph.value.edges.filter(
    (edge) => edge.application_id !== "app-b",
  );
  await graphCheck(data, (window) => {
    const legend = window.document.querySelector(".graph-visible-legend");
    assert.match(legend.textContent, /Logical software.*D-Serv/);
    assert.match(legend.textContent, /D-Call.*Directed logical invocation/);
    assert.doesNotMatch(
      legend.textContent,
      /Infrastructure|Runtime realization|IoT \/ device|Data \/ storage|Placement/,
    );
    assert.ok(
      [
        ...legend.querySelectorAll(
          ".graph-category-swatch,.graph-relation-swatch",
        ),
      ].every((swatch) => swatch.getAttribute("aria-hidden") === "true"),
    );
    const filter = window.document.querySelector(".graph-filter");
    filter.value = "app-b";
    filter.dispatchEvent(new window.Event("change", { bubbles: true }));
    assert.match(legend.textContent, /Application grouping.*Presentation-only/);
    assert.doesNotMatch(legend.textContent, /Logical software|D-Call/);
    filter.value = "";
    filter.dispatchEvent(new window.Event("change", { bubbles: true }));
    assert.match(legend.textContent, /Logical software.*D-Serv/);
  });
});

test("Phase131 bounded-empty partial graph explicitly describes omitted snapshots, never empty project", async () => {
  const data = graphFixture();
  Object.assign(data.graph.value, {
    applications: [],
    nodes: [],
    edges: [],
    coverage: {
      scope: "active_accepted_applications_only",
      completeness: "partial",
      active_application_count: 2,
      included_application_count: 0,
    },
  });
  await graphCheck(data, (window) => {
    assert.match(text(window), /Bounded graph preview: 0 of 2 accepted/);
    assert.match(
      text(window),
      /2 accepted snapshots were omitted by preview limits/,
    );
    assert.match(
      text(window),
      /No accepted application snapshot fit inside the bounded graph preview/,
    );
    assert.match(text(window), /not an empty project/);
    assert.equal(window.document.querySelector(".graph-canvas"), null);
  });
});

test("Phase131 manual-review uneven fixture preserves accessible membership and view controls at 1440/1024", async () => {
  for (const width of [1440, 1024])
    await graphCheck(
      unevenGraphFixture(),
      (window, requests) => {
        const groups = [
          ...window.document.querySelectorAll(".graph-application-list"),
        ];
        assert.deepEqual(
          groups.map(
            (group) =>
              [...group.querySelectorAll("button")].filter((b) =>
                b.textContent.startsWith("D-Serv"),
              ).length,
          ),
          [2, 3, 2, 8],
        );
        assert.match(text(window), /Accepted application D-Graph snapshots/);
        const buttons = [
          ...window.document.querySelectorAll(".graph-toolbar button"),
        ];
        assert.ok(buttons.find((b) => b.textContent === "Fit graph"));
        buttons.find((b) => b.textContent.startsWith("Reset")).click();
        assert.match(
          window.document.querySelector(".graph-detail-body").textContent,
          /Select an entity/,
        );
        assert.equal(requests.length, 1);
      },
      width,
    );
});
for (const state of [
  "empty",
  "not_configured",
  "unavailable",
  "corrupt",
  "indeterminate",
])
  test(`Phase131 graph source / empty state: ${state}`, async () => {
    const data = graphFixture();
    if (state === "empty")
      Object.assign(data.graph.value, {
        applications: [],
        nodes: [],
        edges: [],
        coverage: {
          scope: "active_accepted_applications_only",
          completeness: "complete",
          active_application_count: 0,
          included_application_count: 0,
        },
      });
    else
      data.graph =
        state === "not_configured"
          ? { availability: state }
          : {
              availability: state,
              value:
                state === "indeterminate"
                  ? "internal secret path diagnostic"
                  : {
                      source: "ddeploy",
                      kind: "storage",
                      code: "source_unavailable",
                      safe_message: "Graph source cannot be read.",
                      scope: null,
                    },
            };
    await graphCheck(data, (window) => {
      const expected = {
        empty: /No active accepted logical application graph/,
        not_configured: /Not configured/,
        unavailable: /Unavailable/,
        corrupt: /Integrity error/,
        indeterminate: /Indeterminate/,
      };
      assert.match(text(window), expected[state]);
      assert.equal(window.document.querySelector(".graph-canvas"), null);
      assert.doesNotMatch(
        text(window),
        /internal secret path diagnostic|project does not exist/i,
      );
      if (state !== "empty")
        assert.doesNotMatch(text(window), /0 of 0 active accepted/);
    });
  });
test("Phase131 graph identities remain inspectable at desktop/tablet widths with accessible fallback", async () => {
  for (const width of [1440, 1024, 768]) {
    const data = graphFixture();
    const id = "canonical-service-".repeat(40);
    data.graph.value.nodes[1].canonical_id = id;
    await graphCheck(
      data,
      (window) => {
        const button = window.document.querySelector(
          '[data-entity-id="app-a-one"]',
        );
        assert.ok(button.textContent.includes(id));
        button.click();
        assert.ok(
          window.document
            .querySelector(".graph-detail-body")
            .textContent.includes(id),
        );
        const layout = window.getComputedStyle(
          window.document.querySelector(".graph-workspace-grid"),
        );
        assert.equal(
          layout.gridTemplateColumns,
          width === 768
            ? "minmax(0, 1fr)"
            : "minmax(0, 1fr) minmax(16rem, 22rem)",
        );
        assert.match(
          window.document.querySelector(".graph-renderer-notice").textContent,
          /accessible structure and details remain available/,
        );
        assert.equal(
          window.getComputedStyle(
            window.document.querySelector(".graph-entity-button"),
          ).overflowWrap,
          "anywhere",
        );
      },
      width,
    );
  }
});
test("Phase131 malformed cross-project graph is rejected with safe error, not silently rewritten", async () => {
  const data = graphFixture();
  data.graph.value.nodes[0].project_id = "beta";
  await graphCheck(data, (window) => {
    assert.match(text(window), /invalid runtime evidence projection/);
    assert.equal(window.document.querySelector(".graph-canvas"), null);
    assert.doesNotMatch(text(window), /beta/);
  });
});

for (const [state, label] of [
  ["available-zero", "Available"],
  ["available-active", "Available"],
  ["not_configured", "Not configured"],
  ["unavailable", "Unavailable"],
  ["corrupt", "Integrity error"],
  ["indeterminate", "Indeterminate"],
]) {
  test(`Phase130 F1 application alert source state: ${state}`, async () => {
    const data = projectFixture();
    if (state === "available-zero")
      data.alerts.value.attention_ordered.items = [];
    else if (!state.startsWith("available-"))
      data.alerts =
        state === "not_configured"
          ? { availability: state }
          : state === "indeterminate"
            ? {
                availability: state,
                value: "Internal source diagnostic must not render",
              }
            : {
                availability: state,
                value: {
                  source: "datum_alerts",
                  kind: "storage",
                  code: "storage_unavailable",
                  safe_message: "Alert source cannot be projected.",
                  scope: null,
                },
              };
    await projectCheck(data, (window) => {
      const row = window.document.querySelector(".application-item");
      assert.equal(
        row.querySelector(".application-alert-state .status-badge").textContent,
        label,
      );
      const count = row.querySelector(".application-alert-count");
      if (state.startsWith("available-"))
        assert.match(
          count.textContent,
          state === "available-zero"
            ? /^0 active alert records/
            : /^3 active alert records/,
        );
      else assert.equal(count, null);
      assert.doesNotMatch(
        row.textContent,
        /Internal source diagnostic|Alert source unavailable for/,
      );
    });
  });
}
for (const project of ["alpha", "beta", null]) {
  test(`Phase130 F2 latest correlated evidence scope: ${project ?? "absent"}`, async () => {
    const data = projectFixture();
    const alert = data.alerts.value.attention_ordered.items[0];
    alert.subject = { kind: "dnode", node_id: "fog-01" };
    alert.latest_condition_event = sharedEvidence(
      project,
      `event-${project ?? "unscoped"}`,
    );
    await projectCheck(data, (window) => {
      assert.equal(
        window.document.querySelectorAll(".attention-item").length,
        3,
      );
      const evidence = window.document.querySelector(
        ".attention-item .correlated-evidence",
      );
      assert.equal(
        evidence.dataset.scope,
        project === null
          ? "unscoped"
          : project === "alpha"
            ? "project_local"
            : "cross_project",
      );
      assert.match(evidence.textContent, /Subject ·.*fog-01/);
      if (project !== null)
        assert.ok(
          evidence.textContent.includes(`Evidence project · ${project}`),
        );
      else assert.doesNotMatch(evidence.textContent, /Evidence project/);
      if (project === "beta") {
        assert.match(
          evidence.textContent,
          /another project.*Evidence project · beta/s,
        );
        assert.doesNotMatch(
          evidence.textContent,
          /Evidence project · alpha|corrupt|wrong project/i,
        );
      }
      assert.equal(
        window.document.querySelector(".project-header h1").textContent,
        "alpha",
      );
    });
  });
}
test("Phase130 F2 cross-project recovery remains inspectable, not active attention", async () => {
  const data = projectFixture();
  const alert = data.alerts.value.attention_ordered.items.find(
    (a) => a.lifecycle_status === "resolved",
  );
  alert.recovery_event = sharedEvidence("beta", "recovery-from-beta");
  alert.recovery_event.kind = "observation_stale_cleared";
  alert.recovery_event.classification = "recovery";
  alert.recovery_event.application_id = "beta-application";
  await projectCheck(data, (window) => {
    assert.equal(window.document.querySelectorAll(".attention-item").length, 3);
    const recovery = window.document.querySelector(".recovery-references");
    assert.match(
      recovery.textContent,
      /not active attention.*Resolved · pending closure/s,
    );
    assert.match(
      recovery.textContent,
      /another project.*Evidence project · beta.*Evidence application · beta-application.*recovery-from-beta/s,
    );
  });
});
test("Phase130 F2 shared latest evidence preserves application identity without inventing a local application", async () => {
  const data = projectFixture();
  const alert = data.alerts.value.attention_ordered.items[0];
  alert.latest_condition_event = sharedEvidence("beta", "beta-scoped-event");
  alert.latest_condition_event.application_id = "beta-app";
  alert.latest_condition_event.subject = {
    kind: "dserv",
    project_id: "beta",
    application_id: "beta-app",
    service_id: "beta-service",
  };
  await projectCheck(data, (window) => {
    const evidence = window.document.querySelector(
      ".attention-item .correlated-evidence",
    );
    assert.match(
      evidence.textContent,
      /Evidence project · beta.*Evidence application · beta-app.*beta-scoped-event.*beta-service/s,
    );
    assert.doesNotMatch(
      evidence.textContent,
      /Evidence application · mqtt|Evidence project · alpha/,
    );
    assert.equal(evidence.querySelectorAll("a").length, 0);
  });
});
test("Phase130 F2 multiple global DNode references retain their own evidence projects without corruption", async () => {
  const data = projectFixture();
  for (const [index, alert] of data.alerts.value.attention_ordered.items
    .slice(0, 2)
    .entries()) {
    alert.subject = { kind: "dnode", node_id: "fog-01" };
    alert.latest_condition_event = sharedEvidence(
      index ? "gamma" : "beta",
      `global-node-evidence-${index}`,
    );
    if (index) {
      alert.kind = "observation_clock_skew";
      alert.latest_condition_event.kind = "observation_clock_skew";
    }
  }
  await projectCheck(data, (window) => {
    const evidence = [
      ...window.document.querySelectorAll(
        '.attention-item .correlated-evidence[data-scope="cross_project"]',
      ),
    ];
    assert.equal(evidence.length, 2);
    assert.match(evidence[0].textContent, /beta.*global-node-evidence-0/s);
    assert.match(evidence[1].textContent, /gamma.*global-node-evidence-1/s);
    assert.doesNotMatch(
      window.document.querySelector(".project-attention").textContent,
      /Integrity error|invalid projection/,
    );
  });
});
test("Phase130 F2 contradictory nested event scope surfaces safe validation error", async () => {
  const data = projectFixture();
  data.alerts.value.attention_ordered.items[0].latest_condition_event.subject.project_id =
    "beta";
  await projectCheck(data, (window) => {
    assert.match(text(window), /invalid projection values/);
    assert.doesNotMatch(text(window), /Evidence project · beta/);
  });
});

function waitFor(window, predicate) {
  if (predicate()) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      observer.disconnect();
      reject(new Error("DOM condition not reached"));
    }, 5000);
    const observer = new window.MutationObserver(() => {
      if (predicate()) {
        clearTimeout(timer);
        observer.disconnect();
        resolve();
      }
    });
    observer.observe(window.document, {
      subtree: true,
      childList: true,
      attributes: true,
    });
  });
}
function boot(path = "/", width = 1440, response = () => known) {
  const window = new Window({
    url: `http://127.0.0.1:5173${path}`,
    width,
    height: 900,
  });
  window.AbortController = globalThis.AbortController;
  window.TextEncoder = globalThis.TextEncoder;
  window.fetch = async (url, init) => {
    calls.push({ url, init });
    let body = await response(url, init);
    if (
      url.endsWith("/runtime-graph") &&
      body?.source_kind === "accepted_application_dgraph_snapshots"
    )
      body = runtimeFixture(body, false);
    return body instanceof Response
      ? body
      : new Response(JSON.stringify(body), { status: 200 });
  };
  window.document.body.innerHTML = '<div id="app"></div>';
  const style = window.document.createElement("style");
  style.textContent = css;
  window.document.head.append(style);
  window.eval(bundle);
  return window;
}
const text = (window) => window.document.querySelector("#page").textContent;
const ready = (window) =>
  waitFor(
    window,
    () => !window.document.querySelector("#page").hasAttribute("aria-busy"),
  );
async function wallboardCheck(data, assertion, width = 1440) {
  const window = boot("/", width, () => data);
  try {
    await ready(window);
    await assertion(window);
  } finally {
    await window.happyDOM.close();
  }
}

async function projectCheck(data, assertion, width = 1440) {
  const firstCall = calls.length;
  const window = boot(
    `/projects/${encodeURIComponent(data.project_id)}`,
    width,
    () => data,
  );
  try {
    await ready(window);
    await assertion(window, calls.slice(firstCall));
  } finally {
    await window.happyDOM.close();
  }
}

test("project workspace canonical identity, breadcrumb and global navigation remain explicit", async () => {
  await projectCheck(projectFixture(), (window) => {
    assert.equal(window.document.querySelector("h1").textContent, "alpha");
    assert.match(
      window.document.querySelector('[aria-label="Breadcrumb"]').textContent,
      /Projects.*alpha.*Overview/,
    );
    assert.ok(window.document.querySelector('[data-nav="overview"]'));
    assert.match(text(window), /PROJECT WORKSPACE/);
    assert.match(
      window.document.querySelector(".header-attention").textContent,
      /3 active alerts/,
    );
  });
});
test("project known applications are discovered references with multi-source provenance", async () => {
  await projectCheck(projectFixture(), (window) => {
    const section = window.document.querySelector(".project-applications");
    assert.match(
      section.textContent,
      /Known applications.*Discovered references/s,
    );
    assert.match(section.textContent, /Event history · Alert lifecycle/);
    assert.equal(section.querySelectorAll(".application-item").length, 1);
    assert.doesNotMatch(section.textContent, /All applications/);
  });
});

test("one discovered application keeps multiple provenance markers without duplicate cards", async () => {
  const data = projectFixture();
  data.known_applications.page.items[0].provenance = [
    "accepted_dmap",
    "dmonitor",
    "event",
    "alert",
  ];
  await projectCheck(data, (window) => {
    const cards = window.document.querySelectorAll(".application-item");
    assert.equal(cards.length, 1);
    assert.match(
      cards[0].textContent,
      /Accepted D-Map · D-Monitor evidence · Event history · Alert lifecycle/,
    );
  });
});

test("project source availability stays explicit across not configured, unavailable and indeterminate", async () => {
  const data = projectFixture();
  data.accepted_deployments = {
    availability: "unavailable",
    value: {
      source: "ddeploy",
      kind: "source_unavailable",
      code: "storage_unavailable",
      safe_message: "Deployment source unavailable.",
      scope: { project_id: "alpha", application_id: null },
    },
  };
  data.operational_graph = {
    availability: "indeterminate",
    value: "internal source details not for UI",
  };
  await projectCheck(data, (window) => {
    const sources = window.document.querySelector(".project-sources");
    assert.match(sources.textContent, /Unavailable/);
    assert.match(sources.textContent, /Indeterminate/);
    assert.match(sources.textContent, /Not configured/);
    assert.doesNotMatch(text(window), /internal source details/);
    assert.equal(
      window.document
        .querySelectorAll(".project-summary .metric-card")[5]
        .querySelector(".metric-value"),
      null,
    );
  });
});
test("proposal provenance never establishes pending or accepted authority", async () => {
  const data = projectFixture();
  data.accepted_deployments.value.items = [];
  data.known_applications.page.items[0].provenance = ["ddeploy_proposal"];
  await projectCheck(data, (window) => {
    const row = window.document.querySelector(".application-item");
    assert.match(row.textContent, /proposal reference/);
    assert.match(row.textContent, /Not in loaded accepted page/);
    assert.match(row.textContent, /Planned \/ pending proposal.*Not included/s);
    assert.doesNotMatch(row.textContent, /accepted-map-only/);
  });
});
test("project desired, observed, convergence and accepted sections never collapse", async () => {
  await projectCheck(projectFixture(), (window) => {
    const row = window.document.querySelector(".application-item");
    for (const label of [
      "Accepted placement",
      "Desired runtime",
      "Observed runtime",
      "Convergence",
      "Planned / pending proposal",
    ])
      assert.ok(row.textContent.includes(label));
    const dimensions = window.document.querySelector(".project-dimensions");
    assert.match(
      dimensions.textContent,
      /desired-graph.*6 desired service assignments/,
    );
    assert.match(
      dimensions.textContent,
      /Desired is not observed or converged/,
    );
  });
});
test("no alerts is never a Healthy assertion", async () => {
  const data = projectFixture();
  data.alerts.value.attention_ordered.items = [];
  data.alerts.value.counts.open =
    data.alerts.value.counts.acknowledged =
    data.alerts.value.counts.in_progress =
      0;
  await projectCheck(data, (window) => {
    assert.match(
      window.document.querySelector(".project-attention").textContent,
      /not a positive runtime-health assertion/,
    );
    assert.doesNotMatch(
      text(window),
      /\bHealthy\b|Application Health|Project Health =/,
    );
  });
});
test("project active attention excludes Resolved and Closed; counts preserve pending closure", async () => {
  await projectCheck(projectFixture(), (window) => {
    assert.equal(
      window.document.querySelectorAll(".project-attention .attention-item")
        .length,
      3,
    );
    assert.doesNotMatch(
      window.document.querySelector(".project-attention").textContent,
      /Resolved ·|Closed ·/,
    );
    assert.match(
      window.document.querySelector(".project-summary").textContent,
      /Pending closure.*1.*Resolved, not active/s,
    );
  });
});
test("project temporal subjects retain three independent dimensions and canonical stream identity", async () => {
  await projectCheck(projectFixture(), (window) => {
    const rows = window.document.querySelectorAll(
      ".project-temporal .temporal-entry",
    );
    assert.match(rows[0].textContent, /Fresh.*Stale.*Behind \/ delayed/);
    assert.match(rows[1].textContent, /Stale.*Stale.*Aligned/);
    assert.match(rows[2].textContent, /Fresh.*Fresh.*Ahead \/ skew/);
    assert.ok(rows[0].textContent.includes("alpha/fog-01/monitor"));
    assert.equal(rows[0].querySelectorAll(".dimension-label").length, 3);
  });
});
test("unavailable project counts are not zero and source issues are not runtime failures", async () => {
  const data = projectFixture();
  data.temporal_quality = {
    availability: "corrupt",
    value: {
      source: "dmonitor",
      kind: "source_corrupt",
      code: "invalid_snapshot",
      safe_message: "Cannot project observation evidence.",
      scope: { project_id: "alpha", application_id: null },
    },
  };
  await projectCheck(data, (window) => {
    const metrics = [
      ...window.document.querySelectorAll(".project-summary .metric-card"),
    ];
    assert.equal(metrics[4].querySelector(".metric-value"), null);
    assert.match(metrics[4].textContent, /Integrity error/);
    const sources = window.document.querySelector(".project-sources");
    assert.match(sources.textContent, /SOURCE ISSUE.*not runtime failure/s);
    assert.match(sources.textContent, /invalid_snapshot/);
    assert.match(
      window.document.querySelector("#connection").textContent,
      /^Degraded/,
    );
    assert.ok(window.document.querySelector(".project-applications"));
  });
});
test("project recent events retain application/project/subject identity and occurrence chronology", async () => {
  await projectCheck(projectFixture(), (window) => {
    const rows = window.document.querySelectorAll(".project-events .event-row");
    assert.equal(rows.length, 10);
    assert.match(rows[0].textContent, /Project · alpha.*Application · mqtt/s);
    assert.equal(
      rows[0].querySelector("time").dateTime,
      "2026-09-30T14:59:00Z",
    );
    assert.match(rows[0].textContent, /recovery/);
  });
});
test("accepted map identity is read only from accepted projection, never from event subject", async () => {
  const data = projectFixture();
  data.recent_events.value.items[0].subject = {
    kind: "dmap",
    dmap_id: "historical-map",
    project_id: "alpha",
    application_id: "mqtt",
  };
  await projectCheck(data, (window) => {
    assert.match(
      window.document.querySelector(".project-dimensions").textContent,
      /accepted-map-only/,
    );
    assert.doesNotMatch(
      window.document.querySelector(".project-dimensions").textContent,
      /historical-map/,
    );
    assert.match(
      window.document.querySelector(".project-events").textContent,
      /historical-map/,
    );
  });
});
test("non-discovered project is a valid scoped snapshot, not canonical Not Found", async () => {
  const data = projectFixture();
  data.project_id = "unknown-scope";
  data.known_applications.page.items = [];
  data.alerts = { availability: "not_configured" };
  data.recent_events = { availability: "not_configured" };
  data.temporal_quality = { availability: "not_configured" };
  data.accepted_deployments = { availability: "not_configured" };
  data.operational_graph = { availability: "indeterminate", value: "no graph" };
  await projectCheck(data, (window) => {
    assert.match(text(window), /No applications are currently discoverable/);
    assert.match(text(window), /not proof of registration/);
    assert.doesNotMatch(
      text(window),
      /Project does not exist|Project not found/,
    );
    assert.equal(
      window.document.querySelector("h1").textContent,
      "unknown-scope",
    );
  });
});
test("one scoped project request, no application or overview fan-out", async () => {
  await projectCheck(projectFixture(), (_window, requests) => {
    assert.equal(requests.length, 1);
    assert.equal(requests[0].url, "/api/v1/operations/projects/alpha");
    assert.equal(requests[0].init.headers["X-Datum-Project"], "alpha");
    assert.equal(requests[0].init.method, "GET");
  });
});
test("long project/application identities wrap safely with no HTML authority", async () => {
  const data = projectFixture();
  data.project_id = "project-".repeat(30);
  data.known_applications.page.items = [
    {
      project_id: data.project_id,
      application_id: "<app>".repeat(50),
      provenance: ["event", "alert"],
    },
  ];
  for (const key of [
    "alerts",
    "recent_events",
    "temporal_quality",
    "accepted_deployments",
  ])
    data[key] = { availability: "not_configured" };
  await projectCheck(
    data,
    (window) => {
      assert.equal(
        window.document.querySelector("h1").textContent,
        data.project_id,
      );
      assert.ok(text(window).includes("<app>".repeat(50)));
      assert.equal(window.document.querySelector("app"), null);
      assert.equal(
        window.getComputedStyle(
          window.document.querySelector(".entity-identity"),
        ).overflowWrap,
        "anywhere",
      );
    },
    768,
  );
});
test("project future navigation is disabled, contains no fake routes or write controls", async () => {
  for (const width of [1440, 1024, 768])
    await projectCheck(
      projectFixture(),
      (window) => {
        const nav = window.document.querySelector(".project-subnav");
        assert.equal(nav.querySelectorAll("a").length, 2);
        assert.equal(nav.querySelectorAll('[aria-disabled="true"]').length, 5);
        assert.match(nav.textContent, /Services · Phase 133A \/ 133B.*Graph/s);
        assert.equal(nav.querySelectorAll("button").length, 0);
        assert.equal(
          window.document.querySelectorAll("#page button").length,
          0,
        );
        assert.equal(window.getComputedStyle(nav).flexWrap, "wrap");
        assert.equal(
          window.getComputedStyle(
            window.document.querySelector(".project-summary"),
          ).gridTemplateColumns,
          `repeat(${width === 1440 ? 4 : width === 1024 ? 3 : 2}, minmax(0, 1fr))`,
        );
      },
      width,
    );
});

test("F1 canonical stream/application/map/proposal identities remain distinct and inspectable", async () => {
  const data = wallboardFixture();
  const original = data.temporal_quality.value.items[0];
  const longId = "stream-A-" + "long-canonical-identity-".repeat(30);
  data.temporal_quality.value.items = [
    {
      ...structuredClone(original),
      application_id: "app-A",
      subject: {
        kind: "other",
        entity_kind: "dmonitor_stream",
        entity_id: longId,
      },
    },
    {
      ...structuredClone(original),
      application_id: "app-B",
      subject: {
        kind: "other",
        entity_kind: "dmonitor_stream",
        entity_id: "stream-B",
      },
    },
    {
      ...structuredClone(original),
      application_id: null,
      subject: {
        kind: "other",
        entity_kind: "dmonitor_stream",
        entity_id: "stream-no-app",
      },
    },
  ];
  const base = data.recent_events.value.items[0];
  data.recent_events.value.items = [
    ...["map-A", "map-B"].map((dmap_id) => ({
      ...structuredClone(base),
      application_id: "event-app",
      subject: { kind: "dmap", dmap_id, application_id: "event-app" },
    })),
    ...["proposal-A", "proposal-B"].map((proposal_id) => ({
      ...structuredClone(base),
      application_id: null,
      subject: { kind: "deployment_proposal", proposal_id },
    })),
  ];
  await wallboardCheck(data, (window) => {
    const entries = [...window.document.querySelectorAll(".temporal-entry")];
    assert.equal(entries.length, 3);
    assert.ok(entries[0].textContent.includes(longId));
    assert.ok(entries[1].textContent.includes("stream-B"));
    assert.notEqual(entries[0].textContent, entries[1].textContent);
    assert.match(entries[0].textContent, /Application · app-A/);
    assert.match(entries[1].textContent, /Application · app-B/);
    assert.doesNotMatch(entries[2].textContent, /Application ·/);
    const rows = [...window.document.querySelectorAll(".event-row")];
    for (const [index, id] of [
      "map-A",
      "map-B",
      "proposal-A",
      "proposal-B",
    ].entries())
      assert.ok(rows[index].textContent.includes(id));
    assert.match(rows[0].textContent, /Application · event-app/);
    assert.doesNotMatch(rows[2].textContent, /Application ·/);
    for (const link of window.document.querySelectorAll(
      ".temporal-entry a, .event-row a",
    ))
      assert.equal(link.getAttribute("href"), "/projects/alpha");
    assert.match(css, /\.entity-identity\s*\{[^}]*overflow-wrap:\s*anywhere/s);
  });
});

for (const [loaded, hasMore, partial] of [
  [6, false, false],
  [15, false, false],
  [10, true, false],
  [15, true, false],
  [6, false, true],
  [10, true, true],
]) {
  test(`F2 DOM continuation and coverage: ${loaded}/${hasMore}/${partial}`, async () => {
    const data = wallboardFixture();
    data.known_projects.page.items = Array.from(
      { length: loaded },
      (_, i) => `project-${i}`,
    );
    data.known_projects.page.has_more = hasMore;
    data.known_projects.coverage.completeness = partial
      ? "partial"
      : "complete";
    await wallboardCheck(data, (window) => {
      const section = window.document.querySelector(".wallboard-projects");
      assert.equal(
        section.querySelectorAll(".wallboard-project").length,
        Math.min(10, loaded),
      );
      assert.equal(
        section.textContent.includes(
          "View Projects for the remaining loaded references",
        ),
        loaded > 10,
      );
      assert.equal(
        section.textContent.includes("Console pagination is not yet exposed"),
        hasMore,
      );
      assert.ok(
        section.textContent
          .toLowerCase()
          .includes(partial ? "partial" : "complete"),
      );
    });
    const window = boot("/projects", 1440, () => data);
    try {
      await ready(window);
      assert.equal(
        window.document.querySelectorAll("#page a[data-route]").length,
        loaded,
      );
    } finally {
      await window.happyDOM.close();
    }
  });
}

test("wallboard summary renders normal available counts with transparent provenance", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const summary = window.document.querySelector(".summary-strip");
    assert.equal(summary.querySelectorAll(".metric-card").length, 7);
    assert.match(summary.textContent, /Managed projects2/);
    assert.match(summary.textContent, /Known applications loaded1/);
    assert.match(summary.textContent, /Active alerts3/);
    assert.match(summary.textContent, /Pending closure1/);
    assert.match(summary.textContent, /not canonical inventory/);
  });
});
test("wallboard unavailable count is not zero and unwired output loss is not zero", async () => {
  const data = wallboardFixture();
  data.managed_operational_project_count = { availability: "not_configured" };
  await wallboardCheck(data, (window) => {
    const cards = Array.from(window.document.querySelectorAll(".metric-card"));
    for (const label of ["Managed projects", "Monitoring output loss"]) {
      const card = cards.find(
        (c) => c.querySelector("h2").textContent === label,
      );
      assert.equal(card.querySelector(".metric-value"), null);
      assert.match(card.textContent, /Not configured/);
    }
  });
});
test("Attention Now displays active canonical alerts and exact subject/evidence", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const attention = window.document.querySelector(".wallboard-attention");
    assert.equal(attention.querySelectorAll(".attention-item").length, 3);
    assert.match(attention.textContent, /Critical/);
    assert.match(attention.textContent, /mqtt-service/);
    assert.match(attention.textContent, /120 confirmations/);
    assert.match(attention.textContent, /condition-alert-open/);
  });
});
test("Resolved and Closed never become active wallboard attention", async () => {
  const data = wallboardFixture();
  data.alerts.value.attention_ordered.items =
    data.alerts.value.attention_ordered.items.slice(3);
  await wallboardCheck(data, (window) => {
    assert.equal(window.document.querySelectorAll(".attention-item").length, 0);
    assert.match(
      window.document.querySelector(".wallboard-attention").textContent,
      /No active alert items/,
    );
  });
});
test("known-project coverage stays Partial and does not imply all inventory", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const projects = window.document.querySelector(".wallboard-projects");
    assert.match(projects.textContent, /Known projects/);
    assert.match(projects.textContent, /Discovery is partial/);
    assert.match(projects.textContent, /Placement \/ runtime: not included/);
    assert.doesNotMatch(projects.textContent, /All projects|System healthy/);
  });
});
test("recent receipt with old observation retains Fresh/Stale/Behind separately", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const row = window.document.querySelector(
      ".wallboard-temporal .temporal-row",
    );
    assert.match(row.textContent, /CommunicationFresh/);
    assert.match(row.textContent, /ObservationStale/);
    assert.match(row.textContent, /Source timeBehind \/ delayed/);
    assert.doesNotMatch(row.textContent, /Down|Offline|Unreachable/);
  });
});
test("aligned stale observation/receipt never implies clock discrepancy", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const row = window.document.querySelectorAll(
      ".wallboard-temporal .temporal-row",
    )[1];
    assert.match(row.textContent, /CommunicationStale/);
    assert.match(row.textContent, /ObservationStale/);
    assert.match(row.textContent, /Source timeAligned \/ no discrepancy/);
  });
});
test("source ahead remains an independent time-quality dimension", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const row = window.document.querySelectorAll(
      ".wallboard-temporal .temporal-row",
    )[2];
    assert.match(row.textContent, /CommunicationFresh/);
    assert.match(row.textContent, /ObservationFresh/);
    assert.match(row.textContent, /Source timeAhead \/ skew/);
    assert.doesNotMatch(row.textContent, /Down|Offline/);
  });
});
test("scheduler output-loss counters and owner/driver uncertainty stay separate", async () => {
  const data = wallboardFixture();
  data.scheduler.value = {
    configured: true,
    lifecycle: "running",
    driver_activity: "indeterminate",
    dropped_output_full: 3,
    dropped_output_closed: 2,
    completed_count: 9,
  };
  await wallboardCheck(data, (window) => {
    const sources = window.document.querySelector(".wallboard-sources");
    assert.match(sources.textContent, /Activity unknown/);
    assert.match(sources.textContent, /owner lifecycle: running/);
    assert.match(sources.textContent, /Output full: 3 · output closed: 2/);
    assert.match(sources.textContent, /Monitoring output loss reported/);
    assert.equal(window.document.querySelectorAll(".attention-item").length, 3);
  });
});
test("source issues, events and alerts are distinct wallboard semantics", async () => {
  const data = wallboardFixture();
  data.source_issues = [
    {
      source: "dmonitor",
      kind: "source_unavailable",
      code: "storage_unavailable",
      safe_message: "Evidence source unavailable.",
      scope: null,
    },
  ];
  await wallboardCheck(data, (window) => {
    assert.match(
      window.document.querySelector(".wallboard-attention").textContent,
      /ALERT · persistent/,
    );
    assert.match(
      window.document.querySelector(".wallboard-events").textContent,
      /EVENT · immutable/,
    );
    assert.match(
      window.document.querySelector(".wallboard-sources").textContent,
      /SOURCE ISSUE/,
    );
    assert.match(
      window.document.querySelector("#connection").textContent,
      /Degraded/,
    );
    assert.equal(window.document.querySelectorAll(".attention-item").length, 3);
  });
});
test("recent events remain bounded, preserve occurrence order and recovery class", async () => {
  await wallboardCheck(wallboardFixture(), (window) => {
    const events = window.document.querySelector(".wallboard-events");
    assert.equal(events.querySelectorAll(".event-row").length, 15);
    assert.match(events.querySelector(".event-row").textContent, /recovery/);
    assert.match(events.textContent, /D-Map accepted/);
    assert.equal(
      events.querySelectorAll("time")[0].dateTime,
      "2026-09-30T14:59:00Z",
    );
    assert.match(events.textContent, /not the Historical Explorer/);
  });
});
test("empty partial discovery does not claim that no projects exist", async () => {
  const data = wallboardFixture();
  data.known_projects.page.items = [];
  await wallboardCheck(data, (window) =>
    assert.match(
      window.document.querySelector(".wallboard-projects").textContent,
      /No projects are currently discoverable from the available sources/,
    ),
  );
});
test("degraded wallboard retains independent available sections", async () => {
  const data = wallboardFixture();
  data.temporal_quality = {
    availability: "corrupt",
    value: {
      source: "dmonitor",
      code: "invalid_snapshot",
      kind: "source_corrupt",
      safe_message: "Evidence integrity check failed.",
      scope: null,
    },
  };
  await wallboardCheck(data, (window) => {
    assert.match(
      window.document.querySelector(".wallboard-temporal").textContent,
      /Integrity error/,
    );
    assert.equal(window.document.querySelectorAll(".attention-item").length, 3);
    assert.equal(window.document.querySelectorAll(".event-row").length, 15);
    assert.match(
      window.document.querySelector("#connection").textContent,
      /Degraded/,
    );
  });
});
test("wallboard project click-through is URL-driven with no N+1 initial requests", async () => {
  let requests = 0;
  const window = boot("/", 1440, (url) => {
    requests++;
    return url.includes("/projects/")
      ? { ...liveProject, project_id: "alpha" }
      : wallboardFixture();
  });
  try {
    await ready(window);
    assert.equal(requests, 1);
    click(window, ".wallboard-projects .project-link");
    await ready(window);
    assert.equal(window.location.pathname, "/projects/alpha");
    assert.equal(requests, 2);
    assert.equal(calls.at(-1).init.headers["X-Datum-Project"], "alpha");
  } finally {
    await window.happyDOM.close();
  }
});
test("manual refresh cannot overlap and navigation aborts obsolete request", async () => {
  let requests = 0;
  let release;
  const window = boot("/", 1440, () => {
    requests++;
    return requests === 2
      ? new Promise((resolve) => {
          release = resolve;
        })
      : wallboardFixture();
  });
  try {
    await ready(window);
    click(window, "#refresh");
    assert.equal(requests, 2);
    assert.equal(window.document.querySelector("#refresh").disabled, true);
    click(window, "#refresh");
    assert.equal(requests, 2);
    const obsoleteSignal = calls.at(-1).init.signal;
    click(window, '[data-nav="projects"]');
    await ready(window);
    assert.equal(requests, 3);
    assert.equal(obsoleteSignal.aborted, true);
    release(wallboardFixture());
    assert.equal(window.location.pathname, "/projects");
    assert.match(text(window), /Known projects/);
    assert.equal(window.document.querySelector(".wallboard"), null);
  } finally {
    await window.happyDOM.close();
  }
});
function click(window, selector) {
  window.document
    .querySelector(selector)
    .dispatchEvent(
      new window.MouseEvent("click", { bubbles: true, button: 0 }),
    );
}

test("canonical DATUM assets remain byte-identical and compact navigation stays accessible", async () => {
  for (const name of [
    "datum-monochrome.png",
    "datum-logo-horizontal.png",
    "datum-icon.png",
  ]) {
    const original = await readFile(
      new URL(`../../documentation/assets/brand/${name}`, import.meta.url),
    );
    const digest = (value) => createHash("sha256").update(value).digest("hex");
    for (const directory of ["public", "dist"]) {
      assert.equal(
        digest(
          await readFile(
            new URL(`../${directory}/brand/${name}`, import.meta.url),
          ),
        ),
        digest(original),
      );
    }
    const response = await fetch(`http://127.0.0.1:5173/brand/${name}`);
    assert.equal(response.status, 200);
    assert.equal(
      digest(Buffer.from(await response.arrayBuffer())),
      digest(original),
    );
  }
  const index = await readFile(
    new URL("../index.html", import.meta.url),
    "utf8",
  );
  assert.match(index, /<title>DATUM Console<\/title>/);
  assert.match(index, /\/brand\/datum-icon\.png/);
  const window = boot();
  try {
    await ready(window);
    assert.match(
      window.document.querySelector(".datum-mark").getAttribute("src"),
      /datum-monochrome/,
    );
    click(window, "#menu-toggle");
    assert.equal(
      window.getComputedStyle(window.document.querySelector(".console-layout"))
        .marginLeft,
      "76px",
    );
    assert.equal(window.document.querySelector(".sidebar").inert, false);
    assert.equal(
      window.document
        .querySelector('[data-nav="projects"]')
        .getAttribute("aria-label"),
      "Projects",
    );
    assert.equal(
      window.document
        .querySelector("#menu-toggle")
        .getAttribute("aria-expanded"),
      "false",
    );
    click(window, "#menu-toggle");
    assert.equal(
      window.getComputedStyle(window.document.querySelector(".console-layout"))
        .marginLeft,
      "238px",
    );
  } finally {
    await window.happyDOM.close();
  }
});

test("actual bundled app boots with real runtime projection and scoped project response", async () => {
  const window = boot("/", 1440, async (url, init) =>
    (await fetch(`http://127.0.0.1:5173${url}`, init)).json(),
  );
  try {
    await ready(window);
    assert.match(text(window), /Operational overview/);
    assert.match(window.document.querySelector(".brand").textContent, /DATUM/);
    assert.match(
      window.document.querySelector("#connection").textContent,
      /Degraded/,
    );
    assert.equal(window.document.querySelectorAll(".metric-card").length, 7);
    assert.match(
      window.document.querySelector(".sidebar-footer").textContent,
      /Read-only/,
    );
    assert.match(text(window), /Projection notices/);
    assert.equal(window.document.querySelectorAll("button").length, 2);
  } finally {
    await window.happyDOM.close();
  }
});

test("URL navigation, deep link, project scope, history notifications and unknown route", async () => {
  const window = boot("/", 1440, (url) =>
    url.includes("/projects/") ? liveProject : known,
  );
  try {
    await ready(window);
    click(window, '[data-nav="projects"]');
    await ready(window);
    assert.equal(window.location.pathname, "/projects");
    assert.match(text(window), /Known projects/);
    assert.doesNotMatch(text(window), /All projects/);
    click(window, ".project-link");
    await ready(window);
    assert.equal(window.location.pathname, "/projects/foundation-demo");
    assert.match(text(window), /foundation-demo/);
    assert.equal(
      calls.at(-1).init.headers["X-Datum-Project"],
      "foundation-demo",
    );
    assert.equal(window.document.activeElement.id, "workspace");
    // Happy DOM does not implement native history traversal. Exercise the exact
    // popstate seam with backward/forward URL boundaries, not pretend browser Back.
    for (const path of ["/projects", "/projects/foundation-demo"]) {
      window.history.replaceState(null, "", path);
      window.dispatchEvent(new window.PopStateEvent("popstate"));
      await ready(window);
      assert.equal(window.location.pathname, path);
    }
    window.history.pushState(null, "", "/unknown");
    window.dispatchEvent(new window.PopStateEvent("popstate"));
    assert.match(text(window), /Page not found/);
    assert.equal(
      window.document.querySelector("#page").hasAttribute("aria-busy"),
      false,
    );
  } finally {
    await window.happyDOM.close();
  }
  const deep = boot("/projects/foundation-demo", 1440, () => liveProject);
  try {
    await ready(deep);
    assert.match(text(deep), /foundation-demo/);
  } finally {
    await deep.happyDOM.close();
  }
});

test("1440/1024/768 CSS cascade and tablet navigation focus contract", async () => {
  for (const width of [1440, 1024, 768]) {
    const window = boot("/", width);
    try {
      await ready(window);
      const sidebar = window.document.querySelector("#sidebar");
      assert.equal(sidebar.inert, width <= 900);
      assert.equal(sidebar.getAttribute("aria-hidden"), String(width <= 900));
      assert.equal(
        window.getComputedStyle(
          window.document.querySelector(".console-layout"),
        ).marginLeft,
        width <= 900 ? "0px" : "238px",
      );
      assert.match(
        window.getComputedStyle(window.document.querySelector(".metric-grid"))
          .gridTemplateColumns,
        width === 1440
          ? /repeat\(7/u
          : width === 1024
            ? /repeat\(3/u
            : /repeat\(2/u,
      );
      if (width === 768) {
        click(window, "#menu-toggle");
        assert.equal(sidebar.inert, false);
        const link = sidebar.querySelector('[data-nav="projects"]');
        link.focus();
        window.document.dispatchEvent(
          new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
        );
        assert.equal(sidebar.inert, true);
        assert.equal(window.document.activeElement.id, "menu-toggle");
        assert.equal(
          window.document
            .querySelector("#menu-toggle")
            .getAttribute("aria-expanded"),
          "false",
        );
      }
      assert.equal(
        window.document.querySelector(".skip-link").getAttribute("href"),
        "#workspace",
      );
      assert.equal(
        window.document.querySelector("#workspace").getAttribute("tabindex"),
        "-1",
      );
    } finally {
      await window.happyDOM.close();
    }
  }
});

test("lifecycle primitives and long API error retain explicit semantics", async () => {
  const { lifecycleBadge, availability } = await import(
    "../src/components/presentation.ts"
  );
  const window = boot("/unknown");
  const previous = globalThis.document;
  globalThis.document = window.document;
  try {
    for (const state of [
      "open",
      "acknowledged",
      "in_progress",
      "resolved",
      "closed",
    ]) {
      const item = lifecycleBadge(state);
      assert.ok(item.textContent.length);
      assert.equal(item.classList.contains("tone-neutral"), state === "closed");
      if (state === "resolved")
        assert.match(item.textContent, /pending closure/);
      if (state === "closed") assert.match(item.textContent, /history/);
    }
    for (const state of ["not_configured", "indeterminate"]) {
      assert.equal(
        availability({
          availability: state,
          value: "Uncertain",
        }).classList.contains("tone-danger"),
        false,
      );
    }
  } finally {
    globalThis.document = previous;
    await window.happyDOM.close();
  }
  const message = "safe-notice-".repeat(42);
  const error = boot(
    "/",
    768,
    () =>
      new Response(
        JSON.stringify({ error: { code: "storage_failed", message } }),
        { status: 503 },
      ),
  );
  try {
    await ready(error);
    assert.ok(text(error).includes(message));
    assert.equal(error.document.querySelector("#refresh").disabled, false);
    assert.equal(
      error.getComputedStyle(error.document.querySelector(".console-card"))
        .overflowWrap,
      "anywhere",
    );
  } finally {
    await error.happyDOM.close();
  }
});

test("muted operational text satisfies WCAG AA contrast numerically", () => {
  const luminance = (hex) => {
    const channels = hex
      .match(/[0-9a-f]{2}/gu)
      .map((v) => parseInt(v, 16) / 255)
      .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  };
  for (const background of ["ffffff", "f2f5f7"]) {
    assert.ok(
      (luminance(background) + 0.05) / (luminance("617283") + 0.05) >= 4.5,
    );
  }
});

test("three temporal dimensions, long identities/provenance and safe notices remain inspectable", async () => {
  const project = structuredClone(liveProject);
  project.project_id = "p".repeat(240);
  project.known_applications.page.items = [
    {
      project_id: project.project_id,
      application_id: "app-".repeat(120),
      provenance: ["DMap", "DMonitor", "Event", "Alert", "Operational graph"],
    },
  ];
  project.temporal_quality = {
    availability: "available",
    value: {
      limit: 100,
      has_more: false,
      items: [
        {
          node_id: "recent-receipt",
          communication_freshness: "fresh",
          observation_freshness: "stale",
          source_time_quality: "behind_or_delayed",
        },
        {
          node_id: "aligned-stale",
          communication_freshness: "stale",
          observation_freshness: "stale",
          source_time_quality: "aligned_or_no_discrepancy",
        },
        {
          node_id: "ahead",
          communication_freshness: "fresh",
          observation_freshness: "fresh",
          source_time_quality: "source_ahead",
        },
      ],
    },
  };
  project.temporal_quality.value.items =
    project.temporal_quality.value.items.map((record, i) => ({
      ...structuredClone(wallboardFixture().temporal_quality.value.items[i]),
      ...record,
      project_id: project.project_id,
      subject: {
        ...wallboardFixture().temporal_quality.value.items[i].subject,
        project_id: project.project_id,
      },
    }));
  const window = boot(`/projects/${project.project_id}`, 768, () => project);
  try {
    await ready(window);
    const rows = window.document.querySelectorAll(".temporal-row");
    assert.equal(rows.length, 3);
    assert.deepEqual(
      [...rows[0].querySelectorAll(".dimension-label")].map(
        (el) => el.textContent,
      ),
      ["Communication", "Observation", "Source time"],
    );
    assert.match(rows[0].textContent, /Fresh.*Stale.*Behind \/ delayed/);
    assert.match(rows[1].textContent, /Stale.*Stale.*Aligned/);
    assert.match(rows[2].textContent, /Fresh.*Fresh.*Ahead \/ skew/);
    assert.equal(
      window.document.querySelector("h1").textContent,
      project.project_id,
    );
    assert.ok(text(window).includes("app-".repeat(120)));
    assert.match(text(window), /DMap · DMonitor · Event · Alert/);
    assert.equal(
      window.getComputedStyle(window.document.querySelector(".console-card"))
        .overflowWrap,
      "anywhere",
    );
  } finally {
    await window.happyDOM.close();
  }
});

test("partial/complete empty discovery and all availability states never fabricate count zero", async () => {
  for (const completeness of ["partial", "complete"]) {
    const data = structuredClone(live);
    data.known_projects.page.items = [];
    data.known_projects.page.has_more = false;
    data.known_projects.coverage.completeness = completeness;
    const window = boot("/projects", 1024, () => data);
    try {
      await ready(window);
      assert.match(
        text(window),
        completeness === "partial"
          ? /No projects are currently discoverable/
          : /No project references were discovered/,
      );
      assert.match(text(window), /View source coverage/);
      assert.doesNotMatch(text(window), /All projects/);
    } finally {
      await window.happyDOM.close();
    }
  }
  const data = structuredClone(known);
  data.managed_operational_project_count = { availability: "not_configured" };
  data.alerts = {
    availability: "indeterminate",
    value: "Cannot interpret source",
  };
  data.recent_event_count = {
    availability: "corrupt",
    value: {
      code: "source_corrupt",
      kind: "source_corrupt",
      source: "datum_events",
      safe_message: "Safe message ".repeat(35),
      scope: null,
    },
  };
  data.structural_node_count = {
    availability: "unavailable",
    value: data.recent_event_count.value,
  };
  const window = boot("/", 1440, () => data);
  try {
    await ready(window);
    for (const metric of window.document.querySelectorAll(".metric-card")) {
      if (
        [
          "Managed projects",
          "Active alerts",
          "Pending closure",
          "Recent events",
        ].includes(metric.querySelector("h2").textContent)
      )
        assert.equal(metric.querySelector(".metric-value"), null);
    }
    assert.match(text(window), /Not configured/);
    assert.match(text(window), /Indeterminate/);
    assert.match(text(window), /Integrity error/);
    assert.match(text(window), /Unavailable/);
    assert.equal(
      window.document
        .querySelector(".source-state .status-badge")
        .classList.contains("tone-danger"),
      false,
    );
  } finally {
    await window.happyDOM.close();
  }
});

test("network error, manual retry, Connecting and obsolete unknown-route request", async () => {
  let fail = true;
  const window = boot("/", 1440, () => {
    if (fail) throw new Error("raw-stack secret/path");
    return known;
  });
  try {
    await ready(window);
    assert.match(
      window.document.querySelector("#connection").textContent,
      /Disconnected/,
    );
    assert.match(text(window), /Projection unavailable/);
    assert.doesNotMatch(text(window), /raw-stack|secret\/path/);
    assert.equal(window.document.querySelector("#refresh").disabled, false);
    fail = false;
    click(window, "#refresh");
    await ready(window);
    assert.match(text(window), /Operational overview/);
    assert.match(
      window.document.querySelector("#connection").textContent,
      /Degraded/,
    );
  } finally {
    await window.happyDOM.close();
  }
  const pending = boot(
    "/",
    768,
    (_url, init) =>
      new Promise((_resolve, reject) =>
        init.signal.addEventListener(
          "abort",
          () => reject(init.signal.reason),
          { once: true },
        ),
      ),
  );
  try {
    assert.match(
      pending.document.querySelector("#connection").textContent,
      /Connecting/,
    );
    pending.history.pushState(null, "", "/unknown");
    pending.dispatchEvent(new pending.PopStateEvent("popstate"));
    assert.match(text(pending), /Page not found/);
    assert.equal(
      pending.document.querySelector("#page").hasAttribute("aria-busy"),
      false,
    );
    assert.doesNotMatch(
      pending.document.querySelector("#connection").textContent,
      /Connecting/,
    );
  } finally {
    await pending.happyDOM.close();
  }
});
