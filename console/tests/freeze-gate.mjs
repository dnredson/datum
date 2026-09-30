// Optional headless DOM gate, NOT graphical/visual browser verification.
// Happy DOM is supplied from a disposable external install, never an app dependency.
// Set DATUM_DOM_GATE_MODULE=/absolute/path/to/happy-dom/lib/index.js,
// then run: node --experimental-strip-types --test tests/freeze-gate.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { createHash } from "node:crypto";

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
    const body = await response(url, init);
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
    assert.equal(window.document.querySelectorAll(".metric-card").length, 4);
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
        width === 1440 ? /repeat\(4/u : /repeat\(2/u,
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
  const { lifecycleBadge, availability } =
    await import("../src/components/presentation.ts");
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
    assert.equal(window.document.querySelectorAll(".metric-value").length, 0);
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
