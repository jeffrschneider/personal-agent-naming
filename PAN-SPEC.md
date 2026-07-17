# Personal Agent Naming (PAN)

**Version:** 0.2-draft · **Date:** 2026-07-17 · **Status:** draft, one reference implementation

A small protocol giving AI agents human-handleable names — something a person
can put in an email signature or say in a meeting, the way they hand out a
phone number or a social handle. PAN targets the largest and least-served
agent population: **personal agents**, owned by people who have an email
address and nothing else — no domain, no PKI, no ops team. For those who do
control a domain, PAN adds a second anchor tier whose proofs anyone can
re-verify.

PAN deliberately does one thing. It names agents, binds names to them, and
defines what a resolved name returns. It does not transport messages (that
layer belongs to protocols like A2A and AgentMesh), does not verify
capability claims, and does not host agents.

---

## 1. Terminology

- **Handle** — one globally unique string naming an agent.
- **Anchor** — the thing whose control authorizes claims: an **email
  address** (proven to the registrar once) or a **domain** (proven by
  records anyone can re-fetch).
- **Registrar** — a service that accepts claims, enforces uniqueness,
  answers resolution queries, and maintains the transparency log.
- **Listing** — the registrar's record of an agent (however sourced:
  harvested from an agent network, or submitted directly).
- **Binding** — the attachment of a handle to a listing.
- **Card** — what resolution returns: the PAN envelope plus the agent's
  manifest (§5).
- **Domain record** — the self-published file or DNS record by which a
  domain declares its handles (§3.2).

## 2. Handles

Two anchor tiers, two forms:

```
email tier:    <name>.<email>        PublicAgent.jeffrschneider@gmail.com
domain tier:   <name>@<domain>       PublicAgent@jeffschneider.com
```

- `<name>` is any non-empty string up to 64 characters containing no
  whitespace, no `@`, and no control characters. **Dots are allowed.** There
  is no further grammar.
- `<email>` and `<domain>` are lowercased.

**Rule 1 — nobody parses handles.** A handle is an opaque key. Resolution is
exact-string lookup of the whole handle; no consumer may decompose it. The
two tiers can even render colliding strings
(`translate.PublicAgent@jeffschneider.com` could arise from either tier) —
this is harmless *because* nobody parses: the string belongs to whoever
claimed it first, and the registrar knows its tier because it witnessed the
claim.

**Rule 2 — uniqueness is full-string, first come, first served, across both
tiers.** The registrar enforces uniqueness on the case-folded complete
handle at claim time. Second claimant is refused with "taken," whatever
their tier. No exceptions, no adjudication.

Handles are case-insensitive for matching and case-preserving for display.

## 3. Claiming

### 3.1 Email anchors (notarized)

Control of the mailbox authorizes claims under it.

1. Claimant submits the anchor email to the registrar.
2. Registrar delivers a short verification code (6 digits, ≤15-minute
   expiry) to that mailbox, and accepts a bounded number of attempts.
3. A correct code yields a short-lived session (≤30 minutes) under which the
   claimant may claim handles, bind, release, and list their handles.

Registrars MUST rate-limit code issuance per anchor.

**Lifetime = anchor lifetime.** A handle lives as long as its owner can
re-prove the anchor when required. This is intended: a personal address that
outlives employers keeps its handles; a work address that dies at
offboarding takes its handles with it — that is the governance boundary
working, not a defect.

**Release and cooling-off.** Releasing a handle tombstones it. A released
handle MUST NOT be claimable by anyone for a cooling-off period (REQUIRED
minimum 90 days), so that a handle written down last year does not silently
start pointing at a stranger.

### 3.2 Domain anchors (self-published, publicly re-verifiable)

The domain tier is **record-driven**: the domain publishes the truth and the
registrar mirrors it. There are no codes and no sessions — the ability to
publish at the domain *is* the proof, and unlike an email proof, anyone can
re-fetch it at any time.

A domain declares its handles in either or both of:

- **Well-known file** (HTTPS required):

  ```
  https://<domain>/.well-known/pan.json

  {
    "version": "pan-0.2",
    "handles": [
      { "name": "PublicAgent", "key": "UD653KLV…" },
      { "name": "concierge" }
    ]
  }
  ```

- **DNS TXT records** at `_pan.<domain>`, one record per handle
  (the record names the handle explicitly, so dotted names never meet DNS
  label rules):

  ```
  _pan.jeffschneider.com.  TXT  "v=pan1; name=PublicAgent; key=UD653KLV…"
  ```

`key` is optional; when present it both anchors the name *and* authorizes
binding (§4.3).

**Sync.** Anyone may ask the registrar to sync a domain
(`POST /api/domains/sync {domain}` in the reference implementation — no
authentication needed, since the record is the authorization). The registrar
fetches the record (well-known first, DNS fallback), verifies, and upserts:
new entries become claims (subject to Rule 2 — a string already taken at
either tier is refused and the conflict logged), removed entries become
releases (after the staleness grace below), key changes become re-bindings.

**Re-verification.** Registrars MUST re-verify domain records periodically
(reference: daily) and record the last-verified time on the card. A record
that stops resolving marks its claims **stale** after a grace window
(reference: 7 days) — resolution signals staleness — and releases them
(normal cooling-off applies) after a longer window (reference: 30 days).
Consumers who don't trust the registrar's schedule can always fetch the
domain record themselves; that is the point of this tier.

## 4. Binding

Claiming a name and proving you operate an agent are different proofs. A
registrar MUST NOT bind a handle to a listing on email proof alone unless
the listing itself was submitted under that same anchor.

### 4.1 Submitter-match (email-proven listings)

A listing submitted directly to the registrar under a verified anchor
records that anchor as its submitter. A handle anchored to the same email
may bind to it freely.

### 4.2 Agent-key pairing (key-bearing agents)

For listings that carry a public key (e.g. agents harvested from a network
where the agent ID *is* an Ed25519 public key), binding requires a signature
from that key:

1. The handle owner, in an authenticated session, requests a **pairing
   code**: short, single-use, ≤10-minute expiry (e.g. `KX4-92F`).
2. The software that holds the agent's private key — *any* software: a
   gateway, a daemon, a five-line script — signs the UTF-8 bytes of the
   canonical string:

   ```
   pan-pair-v1:<code>:<agent-id>
   ```

3. It submits `{code, agent_id, signature}` to the registrar. This call
   needs no authentication: the code proves the handle owner initiated
   pairing; the signature proves agent control. **The binding is the
   intersection of the two proofs.**
4. The registrar verifies the signature against the listing's public key,
   binds, and logs the binding with its method.

For the v0.2 profile: keys are Ed25519 expressed as nkey public strings
(`U…`), and signatures are base64-encoded raw Ed25519 signatures. Other key
profiles may be added; the canonical string is versioned for this reason.

Replay is prevented by construction: codes are single-use and expiring, and
the signed string includes both the code and the agent ID.

**Host neutrality is normative.** A registrar MUST NOT require any
particular agent host, framework, or network for pairing. If a listing
declares a public key, whoever holds that key can pair — this is what makes
PAN implementable by any personal-agent runtime, not just the reference
stack.

### 4.3 Domain-record binding

A domain record entry that declares a `key` binds the handle to the listing
bearing that key, with no pairing ceremony: the domain vouches for the
agent, re-verifiably. If no listing with that key exists yet, the binding
activates when one appears.

## 5. Resolution

Resolution maps a handle to its **card**.

- Resolution MUST be exact-string: case-folded whole-handle lookup.
- Resolution MUST be publicly available without authentication for active
  handles.
- Released handles MUST NOT resolve; registrars SHOULD signal a tombstone
  distinctly from "never existed" in their transparency log, and MAY do so
  at resolution.
- A handle claimed but not yet bound resolves to its claim record without a
  card ("reserved").

### 5.1 The card

```json
{
  "handle":   "PublicAgent@jeffschneider.com",
  "anchor":   "domain",                  // "email" | "domain"
  "binding":  "domain-record",           // "email-submitter" | "agent-key" | "domain-record" | null
  "claimed_at":  "2026-07-17T…",
  "verified_at": "2026-07-17T…",         // domain tier: last successful re-verification
  "stale":    false,                     // domain tier: record currently unfetchable
  "presence": { "state": "online", "last_seen_at": "…" },   // MAY, where the registrar observes liveness
  "endpoints": [
    { "protocol": "agentmesh", "agent_id": "UD653KLV…", "node": "UB2FF…" },
    { "protocol": "a2a", "url": "https://…/.well-known/agent-card.json" }
  ],
  "manifest": { /* the source document, verbatim */ }
}
```

The manifest is carried verbatim from its source (an AgentMesh manifest, an
A2A agent card, a manual submission) — PAN wraps existing card formats, it
does not replace them. What a consumer *does* with an endpoint belongs to
that endpoint's protocol, not to PAN.

### 5.2 WebFinger

A PAN handle is a valid `acct:` URI. Registrars SHOULD serve
**WebFinger (RFC 7033)**:

```
GET /.well-known/webfinger?resource=acct:PublicAgent@jeffschneider.com

{
  "subject": "acct:PublicAgent@jeffschneider.com",
  "properties": { "urn:pan:anchor": "domain", "urn:pan:binding": "domain-record" },
  "links": [ { "rel": "urn:pan:card", "type": "application/json", "href": "…/api/resolve?handle=…" } ]
}
```

so that two decades of existing `acct:` tooling resolves handles with no
PAN-specific code.

## 6. The transparency log

Every claim, binding (with its proof method), release, staleness
transition, and refused conflict appends an entry. Entries are never
updated or deleted, and the log is publicly readable.

**Hash chaining is REQUIRED.** Each entry carries the SHA-256 hash of the
previous entry, computed over a canonical serialization that includes that
previous hash — so the log is a chain, and any rewrite of history breaks
every subsequent link:

```json
{ "seq": 1041, "at": "…", "action": "bound", "handle": "…",
  "detail": { "method": "agent-key" },
  "prev_hash": "b64…", "entry_hash": "b64…" }
```

**Signed checkpoints are REQUIRED.** The registrar holds an Ed25519 signing
key and periodically publishes a checkpoint — a signature over
`(seq, entry_hash, timestamp)` of the latest entry. A mirror that replays
the log, recomputes the chain, and checks the checkpoint detects tampering
without trusting the registrar. Merkle inclusion proofs (RFC 9162 / SCITT)
are a future refinement; the chain format is designed so they can be added
without breaking existing entries.

## 7. Registrar obligations

A conforming registrar:

1. Enforces full-string uniqueness across tiers and the cooling-off window.
2. Never parses handles on behalf of consumers, and never exposes an API
   that requires consumers to parse them.
3. Maintains the hash-chained, checkpoint-signed transparency log of §6.
4. Serves resolution without authentication, and labels every card with its
   anchor tier and binding method.
5. Re-verifies domain records on schedule and surfaces staleness honestly.
6. Requires binding proofs per §4 — email proof alone never binds to a
   listing the anchor didn't submit.

## 8. Trust model — read this before trusting a handle

Trust in PAN has two independent axes:

| Axis | Weakest → strongest |
|---|---|
| **Anchor** (who owns the name) | `email` — witnessed once by the registrar (notarized) → `domain` — self-published, anyone can re-fetch |
| **Binding** (is it really that agent) | `email-submitter` (notarized) → `agent-key` (cryptographic, witnessed) → `domain-record` (cryptographic *and* re-verifiable) |

**For email anchors the registrar is a notary, not an oracle.** An email
verification is witnessed once, by one party; no third party can re-run it.
Everyone who trusts an email-tier handle is trusting the registrar's word,
disciplined by the transparency log: a registrar that rewrites history
breaks its own hash chain in public. **Domain anchors remove the notary**:
the proof lives at the domain and anyone can check it.

What a handle does **not** prove, at any tier: that the agent is competent,
safe, endorsed by anyone, or that its capability claims are true. A handle
is an address, not a badge.

## 9. Security considerations

- **Code guessing** — registrars MUST bound verification attempts and rate-
  limit issuance (reference: 5 attempts/code, 5 codes/hour/anchor).
- **Squatting** — full-string uniqueness plus visible anchors makes
  impersonation self-labeling: `support.paypal.attacker@gmail.com` carries
  its own anchor in plain sight. Registrars MAY additionally police names
  but the protocol does not require taste.
- **Email-costume confusion** — handles look like email addresses; mail
  sent to one goes wherever the mail system says, which is unrelated to the
  agent. Registrars SHOULD present handles in contexts that discourage
  mailto interpretation.
- **Anchor compromise** — whoever controls the mailbox or domain controls
  its handles; anchor hygiene (2FA, registrar-lock on domains) is
  inherited — which is also PAN's strength: it rides the most hardened
  credentials people already have.
- **Domain expiry and transfer** — a lapsed domain's new owner can publish
  new records. Staleness marking, the release grace window, and cooling-off
  bound the damage; consumers doing high-stakes delegation SHOULD check
  `verified_at` and the log's history for recent re-anchoring.
- **DNS integrity** — registrars SHOULD resolve `_pan` records through a
  validating resolver (DNSSEC where present) or DNS-over-HTTPS; well-known
  fetches MUST use HTTPS.

## 10. Out of scope

- **Federation** — multiple registrars, referral resolution, cross-registrar
  uniqueness.
- **Additional anchor proofs** — e.g. OIDC sign-in as a mailbox proof.
- **Merkle/SCITT log upgrades** — inclusion proofs atop the §6 chain.
- **Reachability and messaging** — contacting a resolved agent, including
  any registrar-hosted chat or relay surface, belongs to the messaging
  protocols named in the card's endpoints.

## 11. Reference implementation

The Agent Catalog (this repository) is the reference registrar. At time of
writing: email-tier claiming, resolution, and the transparency log are
live; §4 binding rules, §6 hash chaining, the §5.1 card shape, WebFinger,
and the §3.2 domain tier are in progress in the order listed. The AgentMesh
Rust SDK carries a standalone pairing signer example demonstrating §4.2
without any particular agent host.
