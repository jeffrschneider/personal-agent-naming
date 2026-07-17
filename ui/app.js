// Agent Catalog UI. Plain JS against the catalog's own JSON API — the same
// API any other consumer gets. State lives in the URL where it matters
// (specialty filter is shareable); everything re-renders from fetch.

"use strict";

const $ = (id) => document.getElementById(id);
const grid = $("grid"), empty = $("empty"), drawer = $("drawer"), scrim = $("drawer-scrim");

const state = {
  q: "",
  liveOnly: false,
  source: "",
  protocol: "",
  specialty: new URLSearchParams(location.search).get("specialty") || "",
};

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function ago(iso) {
  if (!iso) return null;
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

// ── tally ─────────────────────────────────────────────────────────────────
async function refreshStats() {
  try {
    const r = await fetch("/api/stats");
    const d = await r.json();
    $("tally-total").textContent = d.total;
    $("tally-online").textContent = d.online;
  } catch (e) {
    console.log("[catalog:ui] stats fetch failed", e);
  }
}

// ── search ────────────────────────────────────────────────────────────────
function query() {
  const p = new URLSearchParams();
  if (state.q) p.set("q", state.q);
  if (state.liveOnly) p.set("presence", "online");
  if (state.source) p.set("source", state.source);
  if (state.protocol) p.set("protocol", state.protocol);
  if (state.specialty) p.set("specialty", state.specialty);
  return p;
}

async function refresh() {
  try {
    // A query containing '@' is a handle — exact-string lookup, no parsing.
    if (state.q.includes("@")) {
      const r = await fetch("/api/resolve?handle=" + encodeURIComponent(state.q));
      if (r.ok) {
        const d = await r.json();
        render(d.listing ? [d.listing] : []);
        if (!d.listing) {
          empty.hidden = false;
          empty.innerHTML = `<b class="mono">${esc(d.card.handle)}</b> is claimed but has no agent attached yet.`;
        }
      } else {
        render([]);
        empty.hidden = false;
        empty.textContent = "No agent by that name.";
      }
      return;
    }
    const r = await fetch("/api/listings?" + query());
    const d = await r.json();
    render(d.listings || []);
  } catch (e) {
    console.log("[catalog:ui] search fetch failed", e);
  }
}

function copyText(t, el) {
  navigator.clipboard.writeText(t).then(() => {
    const prev = el.textContent;
    el.textContent = "Copied";
    setTimeout(() => { el.textContent = prev; }, 1200);
  }).catch((e) => console.log("[catalog:ui] copy failed", e));
}

function render(listings) {
  renderFilterPills();
  grid.innerHTML = listings.map(card).join("");
  const filtered = state.q || state.liveOnly || state.source || state.protocol || state.specialty;
  empty.hidden = listings.length > 0;
  if (listings.length === 0) {
    empty.innerHTML = filtered
      ? "No agents match. Clear a filter or search for something broader."
      : "The shelf is empty. Connect a mesh (<code>MESH_NATS_URL</code>) or submit a listing to <code>POST /api/listings</code>.";
  }
  for (const el of grid.querySelectorAll(".agent-card")) {
    el.addEventListener("click", () => openDrawer(el.dataset.id));
  }
  for (const el of grid.querySelectorAll(".skill-chip")) {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      state.specialty = el.dataset.claim;
      refresh();
    });
  }
  for (const el of grid.querySelectorAll(".handle-chip")) {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      copyText(el.dataset.handle, el);
    });
  }
}

function presencePill(p) {
  const st = p || "unknown";
  const label = st === "unknown" ? "no signal" : st;
  return `<span class="pill presence ${esc(st)}"><span class="dot"></span>${esc(label)}</span>`;
}

function card(l) {
  const st = l.presence || "unknown";
  const chips = (l.specialties || []).slice(0, 5).map((s) =>
    `<button class="skill-chip" data-claim="${esc(s)}" title="Filter by ${esc(s)}">${esc(s)}</button>`).join("");
  const more = (l.specialties || []).length > 5 ? `<span class="muted">+${l.specialties.length - 5}</span>` : "";
  const seen = st !== "online" && l.last_seen_at ? `<span>last seen ${ago(l.last_seen_at)}</span>` : "";
  const trust = l.trust ? `<span class="seal ${l.trust === "verified" ? "good" : ""}">${esc(l.trust)}</span>` : "";
  const handle = l.handle
    ? `<div><span class="handle-chip" data-handle="${esc(l.handle)}" title="Copy handle">${esc(l.handle)}</span></div>`
    : "";
  return `
  <article class="agent-card ${esc(st)}" data-id="${esc(l.id)}" tabindex="0" role="button">
    <div class="ac-head">
      <span class="ac-name">${esc(l.name)}</span>
      ${presencePill(l.presence)}
    </div>
    <div class="ac-desc">${esc(l.description || "")}</div>
    <div class="ac-chips">${chips}${more}</div>
    ${handle}
    <div class="ac-foot">
      <span class="node-badge">${esc(l.source)}</span>
      ${trust}
      <span>${esc(l.protocol)}</span>
      ${seen}
    </div>
  </article>`;
}

function renderFilterPills() {
  const wrap = $("active-filters");
  const pills = [];
  if (state.specialty) pills.push(`<button class="filter-pill" data-clear="specialty"><b>claim</b> ${esc(state.specialty)} ✕</button>`);
  wrap.innerHTML = pills.join("");
  wrap.hidden = pills.length === 0;
  for (const el of wrap.querySelectorAll("[data-clear]")) {
    el.addEventListener("click", () => { state[el.dataset.clear] = ""; refresh(); });
  }
}

// ── detail drawer ─────────────────────────────────────────────────────────
async function openDrawer(id) {
  let d;
  try {
    d = await (await fetch(`/api/listings/${id}`)).json();
  } catch (e) {
    console.log("[catalog:ui] detail fetch failed", e);
    return;
  }
  const l = d.listing, m = l.manifest || {};
  const skills = (m.skills || []).map((s) => `
    <div class="d-skill">
      <div class="sk-name">${esc(s.name || s.id)}</div>
      <div class="sk-desc">${esc(s.description || "")}</div>
    </div>`).join("") || `<div class="muted">No skills declared.</div>`;

  const probes = (d.probes || []).map((p) => `
    <div class="probe-row">
      <span class="${p.ok ? "probe-ok" : "probe-bad"}">${p.ok ? "✓ responded" : "✗ failed"}</span>
      ${p.latency_ms != null ? `<span class="mono">${p.latency_ms}ms</span>` : ""}
      <span class="muted">${ago(p.at) || ""}</span>
      ${p.detail ? `<span class="muted">${esc(p.detail)}</span>` : ""}
    </div>`).join("");

  const node = m.node ? `
    <dl class="kv">
      <dt>node</dt><dd class="mono">${esc(m.node.id || "")}</dd>
      ${m.node.profile ? `<dt>platform</dt><dd>${esc(m.node.profile.platform || "—")}${m.node.profile.client ? " · " + esc(m.node.profile.client) : ""}</dd>` : ""}
      <dt>vouch</dt><dd>${m.node.attestation ? "node-signed attestation" : "—"}</dd>
    </dl>` : `<div class="muted">Not node-hosted (${esc(l.source)} listing).</div>`;

  drawer.innerHTML = `
    <div class="d-head">
      <h2>${esc(l.name)}</h2>
      <button class="d-close" id="d-close" aria-label="Close">✕</button>
    </div>
    <div class="d-sub">${presencePill(l.presence)}
      ${l.presence !== "online" && l.last_seen_at ? `<span class="muted"> last seen ${ago(l.last_seen_at)}</span>` : ""}
    </div>
    <div>${esc(l.description || "")}</div>
    ${l.handle ? `
    <div class="d-section"><h3>Handle</h3>
      <span class="handle-chip" id="d-handle" data-handle="${esc(l.handle)}" title="Copy handle">${esc(l.handle)}</span>
    </div>` : ""}

    <div class="d-section"><h3>Verification</h3>
      ${probes || `<div class="muted">No probe checks yet — claims below are as asserted by the source.</div>`}
    </div>

    <div class="d-section"><h3>Skills</h3>${skills}</div>

    <div class="d-section"><h3>Hosting</h3>${node}</div>

    <div class="d-section"><h3>Identity</h3>
      <dl class="kv">
        <dt>source</dt><dd>${esc(l.source)}</dd>
        <dt>id</dt><dd class="mono">${esc(l.source_id)}</dd>
        <dt>protocol</dt><dd>${esc(l.protocol)}</dd>
        ${l.trust ? `<dt>trust</dt><dd>${esc(l.trust)}</dd>` : ""}
        <dt>first indexed</dt><dd>${new Date(l.created_at).toLocaleString()}</dd>
      </dl>
    </div>

    <div class="d-section"><h3>Manifest</h3>
      <pre class="manifest-json">${esc(JSON.stringify(m, null, 2))}</pre>
    </div>`;
  drawer.hidden = false;
  scrim.hidden = false;
  $("d-close").addEventListener("click", closeDrawer);
  const dh = $("d-handle");
  if (dh) dh.addEventListener("click", () => copyText(dh.dataset.handle, dh));
  // Listing pages are shareable: ?open=<id> deep-links straight to this drawer.
  const url = new URL(location);
  url.searchParams.set("open", id);
  history.replaceState(null, "", url);
}

function closeDrawer() {
  drawer.hidden = true;
  scrim.hidden = true;
  const url = new URL(location);
  url.searchParams.delete("open");
  history.replaceState(null, "", url);
}
scrim.addEventListener("click", closeDrawer);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") { closeDrawer(); closeClaim(); }
});

// ── wiring ────────────────────────────────────────────────────────────────
let debounce;
$("q").addEventListener("input", (e) => {
  clearTimeout(debounce);
  debounce = setTimeout(() => { state.q = e.target.value.trim(); refresh(); }, 250);
});
$("live-toggle").addEventListener("click", (e) => {
  state.liveOnly = !state.liveOnly;
  e.currentTarget.setAttribute("aria-pressed", String(state.liveOnly));
  refresh();
});
$("source").addEventListener("change", (e) => { state.source = e.target.value; refresh(); });
$("protocol").addEventListener("change", (e) => { state.protocol = e.target.value; refresh(); });

// ── claim a handle ────────────────────────────────────────────────────────
const claimModal = $("claim-modal"), claimScrim = $("claim-scrim");
let session = null; // { token, email }

function showStep(id) {
  for (const s of claimModal.querySelectorAll(".claim-step")) s.hidden = s.id !== id;
}
function claimMsg(id, text, isErr) {
  const el = $(id);
  el.textContent = text || "";
  el.classList.toggle("err", !!isErr);
}
function openClaim() {
  claimModal.hidden = false;
  claimScrim.hidden = false;
  showStep(session ? "step-shelf" : "step-email");
  if (session) loadShelf();
}
function closeClaim() { claimModal.hidden = true; claimScrim.hidden = true; }
$("claim-open").addEventListener("click", openClaim);
$("claim-close").addEventListener("click", closeClaim);
claimScrim.addEventListener("click", closeClaim);

$("claim-send").addEventListener("click", async () => {
  const email = $("claim-email").value.trim();
  claimMsg("email-msg", "Sending…");
  const r = await fetch("/api/handles/start", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email }),
  });
  const d = await r.json();
  if (!d.ok) { claimMsg("email-msg", d.error, true); return; }
  $("code-email").textContent = d.email;
  showStep("step-code");
  claimMsg("code-msg", d.delivery === "console"
    ? "Dev mode: the code is in the catalog server's console log."
    : "Check your inbox.");
  $("claim-code").focus();
});

$("claim-verify").addEventListener("click", async () => {
  const email = $("code-email").textContent;
  const code = $("claim-code").value.trim();
  const r = await fetch("/api/handles/verify", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, code }),
  });
  const d = await r.json();
  if (!d.ok) { claimMsg("code-msg", d.error, true); return; }
  session = { token: d.token, email };
  showStep("step-shelf");
  loadShelf();
});

async function loadShelf() {
  // Existing handles under this email.
  const r = await fetch("/api/handles/mine", { headers: { Authorization: "Bearer " + session.token } });
  const d = await r.json();
  if (r.status === 401) { session = null; showStep("step-email"); return; }
  const rows = (d.handles || []).map((h) => `
    <div class="mh-row">
      <span class="mono">${esc(h.handle)}</span>
      ${h.listing_id
        ? `<span class="muted">${esc(h.bind_method || "bound")}</span>`
        : `<span class="muted">reserved</span>`}
      <button class="pair-btn" data-h="${esc(h.handle)}" title="Attach a key-bearing agent by pairing">pair</button>
      <button class="rel" data-h="${esc(h.handle)}">release</button>
    </div>`).join("");
  $("my-handles").innerHTML = rows
    ? `<div class="mh-head">Your handles</div>${rows}<div id="pair-info" class="claim-msg"></div>`
    : "";
  for (const b of $("my-handles").querySelectorAll(".rel")) {
    b.addEventListener("click", async () => {
      await fetch("/api/handles/release", {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: "Bearer " + session.token },
        body: JSON.stringify({ handle: b.dataset.h }),
      });
      loadShelf();
      refresh();
    });
  }
  for (const b of $("my-handles").querySelectorAll(".pair-btn")) {
    b.addEventListener("click", async () => {
      const r2 = await fetch("/api/pair/start", {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: "Bearer " + session.token },
        body: JSON.stringify({ handle: b.dataset.h }),
      });
      const d2 = await r2.json();
      const info = $("pair-info");
      if (!d2.ok) { info.textContent = d2.error; info.classList.add("err"); return; }
      info.classList.remove("err");
      info.innerHTML = `Pairing code <b class="mono">${esc(d2.code)}</b> — expires in 10 minutes.<br>
        Have your agent's host sign <span class="mono">pan-pair-v1:${esc(d2.code)}:&lt;agent-id&gt;</span>
        and send it to <span class="mono">POST /api/pair/complete</span>. The handle attaches the moment it lands.`;
    });
  }
  // Attachable agents: only listings this email submitted (PAN §4.1) —
  // key-bearing agents attach via pairing instead.
  const lr = await fetch("/api/listings/mine", { headers: { Authorization: "Bearer " + session.token } });
  const ld = await lr.json();
  $("claim-attach").innerHTML = `<option value="">reserve name only</option>` +
    (ld.listings || []).map((l) =>
      `<option value="${esc(l.id)}">${esc(l.name)} (${esc(l.source)})</option>`).join("");
  updatePreview();
}

function updatePreview() {
  const name = $("claim-name").value.trim();
  $("handle-preview").textContent = name && session ? `${name}.${session.email}` : "";
}
$("claim-name").addEventListener("input", updatePreview);

$("claim-do").addEventListener("click", async () => {
  const name = $("claim-name").value.trim();
  const listing_id = $("claim-attach").value || null;
  claimMsg("claim-msg", "Claiming…");
  const r = await fetch("/api/handles/claim", {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: "Bearer " + session.token },
    body: JSON.stringify({ name, listing_id }),
  });
  const d = await r.json();
  if (r.status === 401) { session = null; showStep("step-email"); return; }
  if (!d.ok) { claimMsg("claim-msg", d.error, true); return; }
  $("done-handle").textContent = d.handle;
  showStep("step-done");
  refresh();
});

$("done-copy").addEventListener("click", (e) =>
  copyText($("done-handle").textContent, e.currentTarget));
$("done-another").addEventListener("click", () => {
  $("claim-name").value = "";
  showStep("step-shelf");
  loadShelf();
});

refreshStats();
refresh();
const deepLink = new URLSearchParams(location.search).get("open");
if (deepLink) openDrawer(deepLink);
// Presence moves on its own; keep the shelf current without user action.
setInterval(refreshStats, 10_000);
setInterval(refresh, 20_000);
