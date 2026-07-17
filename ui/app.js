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
    const r = await fetch("/api/listings?" + query());
    const d = await r.json();
    render(d.listings || []);
  } catch (e) {
    console.log("[catalog:ui] search fetch failed", e);
  }
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
  return `
  <article class="agent-card ${esc(st)}" data-id="${esc(l.id)}" tabindex="0" role="button">
    <div class="ac-head">
      <span class="ac-name">${esc(l.name)}</span>
      ${presencePill(l.presence)}
    </div>
    <div class="ac-desc">${esc(l.description || "")}</div>
    <div class="ac-chips">${chips}${more}</div>
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
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeDrawer(); });

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

refreshStats();
refresh();
const deepLink = new URLSearchParams(location.search).get("open");
if (deepLink) openDrawer(deepLink);
// Presence moves on its own; keep the shelf current without user action.
setInterval(refreshStats, 10_000);
setInterval(refresh, 20_000);
