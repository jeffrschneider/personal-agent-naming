// PAN registrar app: one screen. Signed in, you see your agent names as a
// table (claim / connect / release); the only other surface is the public
// card, which share links (?h=) render standalone and rows open in a NEW TAB,
// so there is no in-app navigation to manage. Session persists in
// localStorage until the server 401s.

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
    el.textContent = "copied";
    setTimeout(() => { el.textContent = prev; }, 1000);
  }).catch((e) => console.log("[pan:ui] copy failed", e));
}
function msgAt(id, text, isErr) {
  const el = $(id);
  el.textContent = text || "";
  el.classList.toggle("err", !!isErr);
}

// ── session ─────────────────────────────────────────────────────────────────
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
function sessionExpired() { saveSession(null); render(); }

// ── the console ─────────────────────────────────────────────────────────────
let operatorName = null;
let openConnect = null;   // handle whose connect panel is expanded
let connectPoll = null;   // timer that watches for the pairing to land

function render() {
  const signedIn = !!session;
  $("gate").hidden = signedIn;
  $("console").hidden = !signedIn;
  $("who").hidden = !signedIn;
  $("sign-out").hidden = !signedIn;
  $("op-line").hidden = !signedIn || !operatorName;
  if (signedIn) {
    $("who").textContent = session.email;
    loadConsole();
  } else {
    $("gate-email").hidden = false;
    $("gate-code").hidden = true;
    stopConnectPoll();
  }
}

$("sign-out").addEventListener("click", () => { saveSession(null); operatorName = null; render(); });

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
$("claim-code").addEventListener("keydown", (e) => { if (e.key === "Enter") $("claim-verify").click(); });
$("claim-email").addEventListener("keydown", (e) => { if (e.key === "Enter") $("claim-send").click(); });

/** The agent-name part of a handle under this session's email. */
function namePart(handle) {
  const suffix = "." + session.email;
  return handle.toLowerCase().endsWith(suffix.toLowerCase())
    ? handle.slice(0, handle.length - suffix.length)
    : handle;
}

let lastRosterJson = "";
async function loadConsole() {
  let r;
  try { r = await fetch("/api/handles/mine", { headers: authHeaders() }); }
  catch { return; }
  if (r.status === 401) { sessionExpired(); return; }
  const d = await r.json();

  fetch("/api/operator", { headers: authHeaders() })
    .then((r2) => r2.json())
    .then((o) => {
      operatorName = o.ok ? o.name : null;
      $("op-line").hidden = !operatorName;
      if (operatorName) { $("op-current").textContent = operatorName; $("op-name").value = operatorName; }
      $("op-field").hidden = !!operatorName;
    })
    .catch(() => {});

  renderRoster(d.handles || []);
}

function renderRoster(handles) {
  lastRosterJson = JSON.stringify(handles.map((h) => [h.handle, !!h.listing_id]));
  const body = $("rows");
  if (!handles.length) {
    body.innerHTML = `<tr><td colspan="4" class="empty-state">
      <p><b>No agent names yet.</b></p>
      <p class="muted">Pick a name below; its handle becomes
        <span class="mono">Name.${esc(session.email)}</span>, then connect it to the
        agent that should answer for it.</p>
    </td></tr>`;
    return;
  }
  body.innerHTML = handles.map((h) => {
    const connected = !!h.listing_id;
    let rows = `
    <tr>
      <td class="nm">${esc(namePart(h.handle))}</td>
      <td><span class="handle-chip js-copy" data-h="${esc(h.handle)}" title="Click to copy">${esc(h.handle)}</span><a
        class="ext" href="/app?h=${encodeURIComponent(h.handle)}" target="_blank" rel="noopener"
        title="The public card: what anyone resolving this handle sees">↗</a></td>
      <td>${connected
        ? `<span class="st-ok" title="${esc(h.bind_method || "")}">✓ connected</span>`
        : `<span class="st-no">not connected yet</span>`}</td>
      <td class="ta-r row-actions">
        ${connected ? "" : `<button class="row-act primary conn" data-h="${esc(h.handle)}">connect</button>`}
        <button class="row-act rel" data-h="${esc(h.handle)}">release</button>
      </td>
    </tr>`;
    if (openConnect === h.handle) rows += `
    <tr class="connect-row"><td colspan="4"><div class="pair-info" id="connect-panel">requesting a code…</div></td></tr>`;
    return rows;
  }).join("");

  for (const el of body.querySelectorAll(".js-copy"))
    el.addEventListener("click", () => copyText(el.dataset.h, el));

  for (const b of body.querySelectorAll(".conn"))
    b.addEventListener("click", () => {
      if (openConnect === b.dataset.h) { openConnect = null; stopConnectPoll(); loadConsole(); }
      else { openConnect = b.dataset.h; startConnect(b.dataset.h); }
    });

  for (const b of body.querySelectorAll(".rel"))
    b.addEventListener("click", async () => {
      if (!confirm(`Release ${b.dataset.h}?\n\nIt stops resolving, and nobody (including you) can claim it again for 90 days.`)) return;
      const r2 = await fetch("/api/handles/release", {
        method: "POST", headers: authHeaders(),
        body: JSON.stringify({ handle: b.dataset.h }),
      });
      if (r2.status === 401) { sessionExpired(); return; }
      if (openConnect === b.dataset.h) { openConnect = null; stopConnectPoll(); }
      loadConsole();
    });

  if (openConnect && !handles.some((h) => h.handle === openConnect)) {
    openConnect = null;
    stopConnectPoll();
  }
}

async function startConnect(handle) {
  await loadConsoleKeepPanel();
  const r = await fetch("/api/pair/start", {
    method: "POST", headers: authHeaders(),
    body: JSON.stringify({ handle }),
  });
  if (r.status === 401) { sessionExpired(); return; }
  const d = await r.json();
  const panel = $("connect-panel");
  if (!panel) return;
  if (!d.ok) { panel.textContent = d.error; panel.classList.add("err"); return; }
  panel.classList.remove("err");
  panel.innerHTML = `Connect <b class="mono">${esc(namePart(handle))}</b> to the agent that should answer for it.
    One-time code: <span class="code">${esc(d.code)}</span> <span class="muted">(expires in 10 minutes)</span><br>
    If the agent runs behind the mesh-adapter, run this on its machine:
    <span class="cmd js-cmd" title="Click to copy">mesh-adapter pair ${esc(handle)} ${esc(d.code)}</span>
    Any other host signs <span class="mono">pan-pair-v1:${esc(d.code)}:&lt;agent-id&gt;</span> and posts it
    to <span class="mono">/api/pair/complete</span>. The moment it lands, this row flips to ✓ connected.`;
  const cmd = panel.querySelector(".js-cmd");
  cmd.addEventListener("click", () => copyText(cmd.textContent, cmd));
  startConnectPoll();
}

/** Re-render the roster (so the panel row exists) without collapsing state. */
async function loadConsoleKeepPanel() {
  const r = await fetch("/api/handles/mine", { headers: authHeaders() });
  if (r.status === 401) { sessionExpired(); return; }
  const d = await r.json();
  renderRoster(d.handles || []);
}

// While a connect panel is open, watch for the pairing to land and flip the row.
function startConnectPoll() {
  stopConnectPoll();
  connectPoll = setInterval(async () => {
    if (!session || !openConnect) { stopConnectPoll(); return; }
    try {
      const r = await fetch("/api/handles/mine", { headers: authHeaders() });
      if (r.status === 401) { sessionExpired(); return; }
      const d = await r.json();
      const mine = (d.handles || []).find((h) => h.handle === openConnect);
      if (mine && mine.listing_id) {          // it landed
        openConnect = null;
        stopConnectPoll();
        renderRoster(d.handles || []);
      }
    } catch { /* transient */ }
  }, 4000);
}
function stopConnectPoll() {
  if (connectPoll) { clearInterval(connectPoll); connectPoll = null; }
}

// add row
$("add-name").addEventListener("input", () => {
  const v = $("add-name").value.trim();
  $("add-prev").textContent = v && session ? `handle: ${v}.${session.email}` : "";
});
$("add-name").addEventListener("keydown", (e) => { if (e.key === "Enter") $("add-btn").click(); });
$("add-btn").addEventListener("click", async () => {
  const name = $("add-name").value.trim();
  if (!name) return;
  const opName = $("op-name").value.trim();
  if (!operatorName && !opName) {
    msgAt("add-msg", "Your public name (the operator) is required once: it is shown on your handles' cards.", true);
    $("op-field").hidden = false;
    $("op-name").focus();
    return;
  }
  msgAt("add-msg", "Claiming…");
  const body = { name };
  if (opName && opName !== operatorName) body.operator_name = opName;
  const r = await fetch("/api/handles/claim", {
    method: "POST", headers: authHeaders(), body: JSON.stringify(body),
  });
  if (r.status === 401) { sessionExpired(); return; }
  const d = await r.json();
  if (!d.ok) { msgAt("add-msg", d.error, true); return; }
  msgAt("add-msg", "");
  $("add-name").value = "";
  $("add-prev").textContent = "";
  openConnect = d.handle;          // claiming flows straight into connecting
  startConnect(d.handle);
});

$("op-edit").addEventListener("click", async (e) => {
  e.preventDefault();
  const next = prompt("Your public name (shown on all your cards):", operatorName || "");
  if (next === null || !next.trim() || next.trim() === operatorName) return;
  const r = await fetch("/api/operator", {
    method: "POST", headers: authHeaders(), body: JSON.stringify({ name: next.trim() }),
  });
  if (r.status === 401) { sessionExpired(); return; }
  loadConsole();
});

// ── the public card (share links: ?h=) ──────────────────────────────────────
function endpointLine(e) {
  if (e.protocol === "agentmesh") {
    return `agentmesh · agent <span class="mono">${esc(e.agent_id || "")}</span>` +
      (e.node ? ` @ node <span class="mono">${esc(e.node)}</span>` : "");
  }
  if (e.url) return `${esc(e.protocol)} · <span class="mono">${esc(e.url)}</span>`;
  return esc(e.protocol || "endpoint");
}

async function renderCard(handle) {
  const msg = $("lk-msg"), card = $("lk-card");
  msg.classList.remove("err");
  msg.textContent = "Resolving…";
  let r;
  try { r = await fetch("/api/resolve?handle=" + encodeURIComponent(handle)); }
  catch { msg.textContent = "The registrar didn't answer. Try again."; msg.classList.add("err"); return; }
  if (!r.ok) { msg.textContent = "No agent by that name."; msg.classList.add("err"); return; }
  const d = await r.json();
  const c = d.card || {};
  msg.textContent = "";
  const operator = c.operator && c.operator.name
    ? `<div class="d-sub">operated by <b>${esc(c.operator.name)}</b> <span class="muted">(their chosen label, anchored to their verified email)</span></div>`
    : "";
  if (c.reserved) {
    card.innerHTML = `<div class="d-head"><h2>${esc(c.handle)}</h2></div>
      ${operator}
      <div class="d-sub muted">Claimed, but not connected to an agent yet.</div>`;
    card.hidden = false;
    return;
  }
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
    <div class="d-section"><h3>Connected</h3>
      <div>${c.binding ? `<span class="seal good">✓ ${esc(c.binding)}</span>` : `<span class="muted">not connected</span>`}
        <span class="muted" style="margin-left:.5rem">claimed ${ago(c.claimed_at) || ""}</span></div>
    </div>
    <div class="d-section"><h3>Address</h3>
      <div class="muted" style="font-size:.82rem;margin-bottom:.5rem">the endpoints a messaging layer reaches. PAN hands these off; it does not carry the message.</div>
      ${endpoints}
    </div>`;
  card.hidden = false;
  for (const el of card.querySelectorAll(".js-copy-handle"))
    el.addEventListener("click", () => copyText(el.dataset.handle, el));
}

// ── boot ────────────────────────────────────────────────────────────────────
const deepHandle = params.get("h");
if (deepHandle) {
  $("view-console").hidden = true;
  $("view-card").hidden = false;
  $("who").hidden = true; $("sign-out").hidden = true;
  renderCard(deepHandle);
} else {
  render();
}
