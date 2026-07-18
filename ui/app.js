// PAN registrar UI. Two surfaces: lookup (a handle in, its card out) and
// my-handles (claim / pair / release). Plain JS against the JSON API.

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

// ── lookup ────────────────────────────────────────────────────────────────
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
  // Shareable: ?h=<handle>
  const url = new URL(location);
  url.searchParams.set("h", c.handle || handle);
  history.replaceState(null, "", url);

  if (c.reserved) {
    msg.textContent = "";
    card.innerHTML = `<div class="d-head"><h2>${esc(c.handle)}</h2></div>
      ${c.operator && c.operator.name ? `<div class="d-sub">operated by <b>${esc(c.operator.name)}</b> <span class="muted">(their chosen label)</span></div>` : ""}
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
    ${c.operator && c.operator.name ? `<div class="d-sub">operated by <b>${esc(c.operator.name)}</b> <span class="muted">(their chosen label, anchored to their verified email)</span></div>` : ""}
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
  if (h) lookup(h);
});
$("lk-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") { const h = $("lk-input").value.trim(); if (h) lookup(h); }
});
const deepHandle = params.get("h");
if (deepHandle) { $("lk-input").value = deepHandle; lookup(deepHandle); }

// ── my handles (claim / pair / release) ────────────────────────────────────
const claimModal = $("claim-modal"), claimScrim = $("claim-scrim");
let session = null;

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
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeClaim(); });

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
  if (!d.ok) { claimMsg("code-msg", d.error, true); return; }
  session = { token: d.token, email };
  showStep("step-shelf");
  loadShelf();
  loadOperator();
});

async function loadOperator() {
  const r = await fetch("/api/operator", { headers: { Authorization: "Bearer " + session.token } });
  const d = await r.json().catch(() => ({}));
  if (d.ok && d.name) $("op-name").value = d.name;
}

async function loadShelf() {
  const r = await fetch("/api/handles/mine", { headers: { Authorization: "Bearer " + session.token } });
  const d = await r.json();
  if (r.status === 401) { session = null; showStep("step-email"); return; }
  const rows = (d.handles || []).map((h) => `
    <div class="mh-row">
      <span class="mono">${esc(h.handle)}</span>
      ${h.listing_id
        ? `<span class="muted">${esc(h.bind_method || "bound")}</span>`
        : `<span class="muted">reserved</span>`}
      <button class="pair-btn" data-h="${esc(h.handle)}" title="Attach your agent by pairing">pair</button>
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
      info.innerHTML = `Pairing code <b class="mono">${esc(d2.code)}</b>, expires in 10 minutes.<br>
        Have your agent's host sign <span class="mono">pan-pair-v1:${esc(d2.code)}:&lt;agent-id&gt;</span>
        and send it to <span class="mono">POST /api/pair/complete</span>. The handle binds the moment it lands.`;
    });
  }
}

function updatePreview() {
  const name = $("claim-name").value.trim();
  $("handle-preview").textContent = name && session ? `${name}.${session.email}` : "";
}
$("claim-name").addEventListener("input", updatePreview);

$("claim-do").addEventListener("click", async () => {
  const name = $("claim-name").value.trim();
  const operatorName = $("op-name").value.trim();
  if (!operatorName) { claimMsg("claim-msg", "Your public name is required: it is shown on your handles' cards.", true); return; }
  claimMsg("claim-msg", "Claiming…");
  const r = await fetch("/api/handles/claim", {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: "Bearer " + session.token },
    body: JSON.stringify({ name, operator_name: operatorName }),
  });
  const d = await r.json();
  if (r.status === 401) { session = null; showStep("step-email"); return; }
  if (!d.ok) { claimMsg("claim-msg", d.error, true); return; }
  $("done-handle").textContent = d.handle;
  showStep("step-done");
});

$("done-copy").addEventListener("click", (e) => copyText($("done-handle").textContent, e.currentTarget));
$("done-another").addEventListener("click", () => {
  $("claim-name").value = "";
  showStep("step-shelf");
  loadShelf();
});

if (params.get("claim") === "1") openClaim();
