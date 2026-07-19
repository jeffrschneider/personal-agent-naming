// PAN registrar app. Console-first: the default surface is YOUR agent names
// (a roster with add / pair / release, like a registrar's record table), gated
// by the email-code sign-in. Public lookup is the secondary surface. Plain JS
// against the JSON API; session persists in localStorage until the server
// says 401 (sessions are short-lived by design).

"use strict";

const $ = (id) => document.getElementById(id);
const params = new URLSearchParams(location.search);

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
function copyText(t, el) {
  navigator.clipboard.writeText(t).then(() => {
    const prev = el.textContent;
    el.textContent = "Copied";
    setTimeout(() => { el.textContent = prev; }, 1200);
  }).catch((e) => console.log("[pan:ui] copy failed", e));
}
function msgAt(id, text, isErr) {
  const el = $(id);
  el.textContent = text || "";
  el.classList.toggle("err", !!isErr);
}

// ── session (localStorage until the server 401s) ────────────────────────────
let session = null;
try { session = JSON.parse(localStorage.getItem("pan_session")); } catch { /* fresh */ }
function saveSession(s) {
  session = s;
  if (s) localStorage.setItem("pan_session", JSON.stringify(s));
  else localStorage.removeItem("pan_session");
}
function authHeaders() {
  return { "Content-Type": "application/json", Authorization: "Bearer " + session.token };
}
function sessionExpired() {
  saveSession(null);
  render();
}

// ── views ───────────────────────────────────────────────────────────────────
function showView(which) {
  $("view-console").hidden = which !== "console";
  $("view-lookup").hidden = which !== "lookup";
}

// Routing contract: the URL is the state (?h=<handle> = a card, ?find=1 =
// blank lookup, bare = the console). In-app navigation PUSHES a history
// entry; popstate re-renders from the URL, so the browser's Back button
// always works.
function goLookup(handle, push = true) {
  if (push) {
    const url = new URL(location);
    url.searchParams.delete("claim");
    if (handle) { url.searchParams.set("h", handle); url.searchParams.delete("find"); }
    else { url.searchParams.delete("h"); url.searchParams.set("find", "1"); }
    history.pushState(null, "", url);
  }
  showView("lookup");
  if (handle) { $("lk-input").value = handle; lookup(handle); }
  else { $("lk-msg").textContent = ""; $("lk-card").hidden = true; $("lk-input").focus(); }
}
function goConsole(push = true) {
  if (push) {
    const url = new URL(location);
    for (const k of ["h", "find", "claim"]) url.searchParams.delete(k);
    history.pushState(null, "", url);
  }
  showView("console");
}
function applyRoute() {
  const p = new URLSearchParams(location.search);
  const h = p.get("h");
  if (h) goLookup(h, false);
  else if (p.get("find") === "1") goLookup(null, false);
  else goConsole(false);
}
window.addEventListener("popstate", applyRoute);

$("nav-lookup").addEventListener("click", (e) => { e.preventDefault(); goLookup(null); });
$("nav-console").addEventListener("click", (e) => { e.preventDefault(); goConsole(); });

// ── the console ─────────────────────────────────────────────────────────────
let operatorName = null;

function render() {
  const signedIn = !!session;
  $("gate").hidden = signedIn;
  $("console").hidden = !signedIn;
  $("who").hidden = !signedIn;
  $("sign-out").hidden = !signedIn;
  if (signedIn) {
    $("who").textContent = session.email;
    loadConsole();
  } else {
    $("gate-email").hidden = false;
    $("gate-code").hidden = true;
  }
}

$("sign-out").addEventListener("click", () => { saveSession(null); render(); });

$("claim-send").addEventListener("click", async () => {
  const email = $("claim-email").value.trim();
  msgAt("email-msg", "Sending…");
  const r = await fetch("/api/handles/start", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email }),
  });
  const d = await r.json();
  if (!d.ok) { msgAt("email-msg", d.error, true); return; }
  $("code-email").textContent = d.email;
  $("gate-email").hidden = true;
  $("gate-code").hidden = false;
  msgAt("code-msg", d.delivery === "console"
    ? "Dev mode: the code is in the registrar's console log."
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
  if (!d.ok) { msgAt("code-msg", d.error, true); return; }
  $("claim-code").value = "";
  saveSession({ token: d.token, email });
  render();
});

/** The agent-name part of a handle owned by this session's email. */
function namePart(handle) {
  const suffix = "." + session.email;
  return handle.toLowerCase().endsWith(suffix.toLowerCase())
    ? handle.slice(0, handle.length - suffix.length)
    : handle;
}

async function loadConsole() {
  let r;
  try {
    r = await fetch("/api/handles/mine", { headers: authHeaders() });
  } catch { return; }
  if (r.status === 401) { sessionExpired(); return; }
  const d = await r.json();

  // operator line (parallel-ish; cheap)
  fetch("/api/operator", { headers: authHeaders() })
    .then((r2) => r2.json())
    .then((o) => {
      operatorName = o.ok ? o.name : null;
      $("op-line").hidden = !operatorName;
      if (operatorName) $("op-current").textContent = operatorName;
      $("op-field").hidden = !!operatorName;
      if (operatorName) $("op-name").value = operatorName;
    })
    .catch(() => {});

  const handles = d.handles || [];
  $("names-table").hidden = handles.length === 0;
  $("names-empty").hidden = handles.length !== 0;
  $("empty-preview").textContent = `Name.${session.email}`;

  $("names-body").innerHTML = handles.map((h) => `
    <tr>
      <td class="nm">${esc(namePart(h.handle))}</td>
      <td><span class="handle-chip js-copy" data-h="${esc(h.handle)}" title="Copy handle">${esc(h.handle)}</span></td>
      <td>${h.listing_id
        ? `<span class="seal good">bound · ${esc(h.bind_method || "")}</span>`
        : `<span class="seal">reserved</span>`}</td>
      <td class="ta-r row-actions">
        <button class="row-act pair-btn" data-h="${esc(h.handle)}" title="Point this name at your agent">pair</button>
        <button class="row-act card-btn" data-h="${esc(h.handle)}" title="View the public card">card</button>
        <button class="row-act rel" data-h="${esc(h.handle)}" title="Release (90-day cooling-off)">release</button>
      </td>
    </tr>`).join("");

  for (const el of $("names-body").querySelectorAll(".js-copy"))
    el.addEventListener("click", () => copyText(el.dataset.h, el));

  for (const b of $("names-body").querySelectorAll(".card-btn"))
    b.addEventListener("click", () => goLookup(b.dataset.h));

  for (const b of $("names-body").querySelectorAll(".rel"))
    b.addEventListener("click", async () => {
      if (!confirm(`Release ${b.dataset.h}?\n\nIt stops resolving, and nobody (including you) can claim it again for 90 days.`)) return;
      const r2 = await fetch("/api/handles/release", {
        method: "POST", headers: authHeaders(),
        body: JSON.stringify({ handle: b.dataset.h }),
      });
      if (r2.status === 401) { sessionExpired(); return; }
      loadConsole();
    });

  for (const b of $("names-body").querySelectorAll(".pair-btn"))
    b.addEventListener("click", async () => {
      const r2 = await fetch("/api/pair/start", {
        method: "POST", headers: authHeaders(),
        body: JSON.stringify({ handle: b.dataset.h }),
      });
      if (r2.status === 401) { sessionExpired(); return; }
      const d2 = await r2.json();
      const info = $("pair-info");
      info.hidden = false;
      if (!d2.ok) { info.textContent = d2.error; info.classList.add("err"); return; }
      info.classList.remove("err");
      info.innerHTML = `Pairing code for <span class="mono">${esc(b.dataset.h)}</span>:
        <b class="mono big">${esc(d2.code)}</b> <span class="muted">(single-use, expires in 10 minutes)</span><br>
        If the agent runs behind the mesh-adapter, run:<br>
        <span class="mono block">mesh-adapter pair ${esc(b.dataset.h)} ${esc(d2.code)}</span>
        Any other host signs <span class="mono">pan-pair-v1:${esc(d2.code)}:&lt;agent-id&gt;</span>
        and posts it to <span class="mono">/api/pair/complete</span>. The name binds the moment it lands.`;
    });
}

// ── add an agent name ───────────────────────────────────────────────────────
function openAdd() {
  $("add-form").hidden = false;
  $("op-field").hidden = !!operatorName;
  $("claim-name").focus();
  updatePreview();
}
function closeAdd() {
  $("add-form").hidden = true;
  $("claim-name").value = "";
  msgAt("claim-msg", "");
  updatePreview();
}
$("add-open").addEventListener("click", openAdd);
$("add-close").addEventListener("click", closeAdd);

function updatePreview() {
  const name = $("claim-name").value.trim();
  $("handle-preview").textContent = name && session ? `${name}.${session.email}` : "";
}
$("claim-name").addEventListener("input", updatePreview);

$("op-edit").addEventListener("click", (e) => {
  e.preventDefault();
  openAdd();
  $("op-field").hidden = false;
  $("op-name").focus();
});

$("claim-do").addEventListener("click", async () => {
  const name = $("claim-name").value.trim();
  const opName = $("op-name").value.trim();
  if (!operatorName && !opName) {
    msgAt("claim-msg", "Your public name is required: it is shown on your handles' cards.", true);
    return;
  }
  // Change-operator-only path: name field empty but operator edited.
  if (!name && opName && opName !== operatorName) {
    const r0 = await fetch("/api/operator", {
      method: "POST", headers: authHeaders(), body: JSON.stringify({ name: opName }),
    });
    if (r0.status === 401) { sessionExpired(); return; }
    const d0 = await r0.json();
    if (!d0.ok) { msgAt("claim-msg", d0.error, true); return; }
    closeAdd();
    loadConsole();
    return;
  }
  msgAt("claim-msg", "Claiming…");
  const body = { name };
  if (opName && opName !== operatorName) body.operator_name = opName;
  const r = await fetch("/api/handles/claim", {
    method: "POST", headers: authHeaders(), body: JSON.stringify(body),
  });
  if (r.status === 401) { sessionExpired(); return; }
  const d = await r.json();
  if (!d.ok) { msgAt("claim-msg", d.error, true); return; }
  closeAdd();
  loadConsole();
});

// ── lookup (the public card) ────────────────────────────────────────────────
function endpointLine(e) {
  if (e.protocol === "agentmesh") {
    return `agentmesh · agent <span class="mono">${esc(e.agent_id || "")}</span>` +
      (e.node ? ` @ node <span class="mono">${esc(e.node)}</span>` : "");
  }
  if (e.url) return `${esc(e.protocol)} · <span class="mono">${esc(e.url)}</span>`;
  return esc(e.protocol || "endpoint");
}

async function lookup(handle) {
  const msg = $("lk-msg"), card = $("lk-card");
  msg.classList.remove("err");
  msg.textContent = "Resolving…";
  card.hidden = true;
  let r;
  try {
    r = await fetch("/api/resolve?handle=" + encodeURIComponent(handle));
  } catch (e) {
    msg.textContent = "The registrar didn't answer. Try again.";
    msg.classList.add("err");
    return;
  }
  if (!r.ok) {
    msg.textContent = "No agent by that name.";
    msg.classList.add("err");
    return;
  }
  const d = await r.json();
  const c = d.card || {};

  const operator = c.operator && c.operator.name
    ? `<div class="d-sub">operated by <b>${esc(c.operator.name)}</b> <span class="muted">(their chosen label, anchored to their verified email)</span></div>`
    : "";
  if (c.reserved) {
    msg.textContent = "";
    card.innerHTML = `<div class="d-head"><h2>${esc(c.handle)}</h2></div>
      ${operator}
      <div class="d-sub muted">Claimed, but no agent bound yet. It resolves to a reservation.</div>`;
    card.hidden = false;
    return;
  }
  msg.textContent = "";
  const endpoints = (c.endpoints || []).map((e) =>
    `<div class="ep">${endpointLine(e)}</div>`).join("") ||
    `<div class="muted">No endpoints published.</div>`;
  const presence = c.presence && c.presence.state
    ? `<span class="pill presence ${esc(c.presence.state)}"><span class="dot"></span>${esc(c.presence.state)}</span>`
    : "";
  card.innerHTML = `
    <div class="d-head">
      <span class="handle-chip js-copy-handle" data-handle="${esc(c.handle)}" title="Copy handle">${esc(c.handle)}</span>
      ${presence}
    </div>
    ${operator}
    <div class="d-section"><h3>Binding</h3>
      <div>${c.binding ? `<span class="seal good">${esc(c.binding)}</span>` : `<span class="muted">unbound</span>`}
        <span class="muted" style="margin-left:.5rem">claimed ${ago(c.claimed_at) || ""}</span></div>
    </div>
    <div class="d-section"><h3>Address</h3>
      <div class="muted" style="font-size:.82rem;margin-bottom:.5rem">the endpoints a messaging layer reaches. PAN hands these off; it does not carry the message.</div>
      ${endpoints}
    </div>`;
  card.hidden = false;
  for (const el of card.querySelectorAll(".js-copy-handle")) {
    el.addEventListener("click", () => copyText(el.dataset.handle, el));
  }
}

$("lk-btn").addEventListener("click", () => {
  const h = $("lk-input").value.trim();
  if (h) goLookup(h);
});
$("lk-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") { const h = $("lk-input").value.trim(); if (h) goLookup(h); }
});

// ── boot ────────────────────────────────────────────────────────────────────
applyRoute();
render();
