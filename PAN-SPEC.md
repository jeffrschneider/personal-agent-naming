# Personal Agent Naming (PAN)

**Version:** 0.3-draft · **Date:** 2026-07-18 · **Status:** draft, one reference implementation
**Authors:** Jeff R Schneider <jeffrschneider@gmail.com>

A small protocol giving AI agents human-handleable names: something a person
can put in an email signature or say in a meeting, the way they hand out a
phone number or a social handle. PAN targets the largest and least-served
agent population: **personal agents**, owned by people who have an email
address and nothing else. No domain, no PKI, no ops team.

PAN deliberately does one thing. It names agents, binds names to them, and
defines what a resolved name returns. It does not transport messages (that
layer belongs to protocols like A2A and AgentMesh), does not verify
capability claims, does not host agents, and does not do discovery (that
belongs to catalog/search layers like ARD).

**Relationship to ANS.** The Agent Name Service (ANS) already covers
domain-anchored, PKI-backed agent identity for organizations that own a
domain and can run a certificate authority. PAN does not compete there.
PAN is the layer below that bar: naming for an agent whose owner has an
inbox, not a PKI team. Domain anchoring is out of scope by design (§9).

---

## 1. Terminology

- **Handle**: one globally unique string naming an agent, of the form
  `<name>.<email>`.
- **Anchor**: the email address whose control authorizes claims under it.
- **Registrar**: a service that accepts claims, enforces uniqueness,
  answers resolution queries, and maintains the history log.
- **Agent record**: the registrar's minimal record of the agent a handle
  points at: its public key and, optionally, endpoints. It is created by
  binding, not harvested.
- **Binding**: the attachment of a handle to an agent record.
- **Card**: what resolution returns (§5).

## 2. Handles

A handle has one form:

```
<name>.<email>        PublicAgent.jeffrschneider@gmail.com
```

- `<name>` is any non-empty string up to 64 characters containing no
  whitespace, no `@`, and no control characters. **Dots are allowed.** There
  is no further grammar.
- `<email>` is lowercased.

**Rule 1: nobody parses handles.** A handle is an opaque key. Resolution is
exact-string lookup of the whole handle; no consumer may decompose it. This
is what lets the grammar stay this simple: the string belongs to whoever
claimed it first, and the registrar knows the anchor because it witnessed
the claim.

**Rule 2: uniqueness is full-string, first come, first served.** The
registrar enforces uniqueness on the case-folded complete handle at claim
time. The second claimant is refused with "taken." No exceptions, no
adjudication.

Handles are case-insensitive for matching and case-preserving for display.

## 3. Claiming

Control of the mailbox authorizes claims under it.

1. Claimant submits the anchor email to the registrar.
2. Registrar delivers a short verification code (6 digits, ≤15-minute
   expiry) to that mailbox, and accepts a bounded number of attempts.
3. A correct code yields a bounded session under which the claimant may
   claim handles, bind, release, and list their handles. Registrars choose
   the lifetime; the reference registrar uses 8 hours (long enough to manage
   a roster, short enough that the emailed code stays the real credential).

Registrars MUST rate-limit code issuance per anchor.

**The operator record.** Every anchor has one REQUIRED public display name,
the operator's chosen label (e.g. "Jeff Schneider"). It MUST be set no later
than the anchor's first claim, under the verified session; it MAY be updated
at any time under a verified session, and every change MUST be recorded in
the history log. Registrars MUST show the operator name on every card of
that anchor's handles (§5.1). The rationale: PAN is *personal* agent naming,
and the handle already publishes the anchor email, so the human behind an
agent is the meaningful unit of trust; a white pages has names. Consumers
MUST treat the name as a chosen label anchored to the proven email, not as
verified identity (§8).

**Lifetime = anchor lifetime.** A handle lives as long as its owner can
re-prove the anchor when required. This is intended: a personal address that
outlives employers keeps its handles; a work address that dies at
offboarding takes its handles with it. That is the governance boundary
working, not a defect.

**Release and cooling-off.** Releasing a handle tombstones it. A released
handle MUST NOT be claimable by anyone for a cooling-off period (REQUIRED
minimum 90 days), so that a handle written down last year does not silently
start pointing at a stranger.

### 3.1 Delegated witnessing

A registrar MAY accept a partner service's attestation that it has already
verified an email, and mint a session without a second code round trip: the
partner (for example, a mesh control plane whose accounts are themselves
email-verified) authenticates to the registrar with a pre-shared credential
and names the email. This trades one email ceremony for a trust link, and
the spec constrains that trade three ways:

1. **Scope.** A delegated session may *establish*: claim handles, set the
   operator name, start pairing, bind. It MUST NOT *destroy or move*:
   release (and any future transfer) MUST be refused with an instruction to
   sign in directly. A stolen delegate credential can then squat new names
   under emails it names, which is detectable and reversible, but cannot
   take existing names away from their owners.
2. **Disclosure.** Sessions carry a provenance, `email` or
   `delegated:<partner>`. Everything a delegated session establishes MUST
   record it: the history log entry, and a `claimed_via` field on the card
   (§5.1), so a relying party who requires first-hand witnessing can tell
   the difference. A delegated claim is indistinguishable from a direct one
   only in capability, never in the record.
3. **Accountability.** The registrar chooses its partners and vouches for
   the arrangement; §8's "trusting the registrar's word" expands to
   "trusting the registrar's choice of witnesses," and the disclosure rule
   exists exactly so consumers can decline the expansion per handle.

## 4. Binding

Claiming a name and proving you operate an agent are different proofs. A
registrar MUST NOT bind a handle on email proof alone unless the agent
record was submitted under that same anchor.

### 4.1 Agent-key pairing (the primary path)

Most personal-agent runtimes hold an Ed25519 key, and the agent's public key
is its identity. Binding proves control of that key:

1. The handle owner, in an authenticated session, requests a **pairing
   code**: short, single-use, ≤10-minute expiry (e.g. `KX4-92F`).
2. The software that holds the agent's private key (*any* software: a
   gateway, a daemon, a five-line script) signs the UTF-8 bytes of the
   canonical string:

   ```
   pan-pair-v1:<code>:<agent-id>
   ```

3. It submits `{code, agent_id, signature}` to the registrar. This call
   needs no authentication: the code proves the handle owner initiated
   pairing; the signature proves agent control. **The binding is the
   intersection of the two proofs.**
4. The registrar verifies the signature against `agent_id`, records the
   agent (its key, and any endpoints supplied), binds the handle, and logs
   the binding with its method.

The agent record is created by this step. There is no prior directory to
look the agent up in: the signature is the record's authorization, and the
key is the agent's address.

For the v0.3 profile: keys are Ed25519 expressed as nkey public strings
(`U…`), and signatures are base64-encoded raw Ed25519 signatures. Other key
profiles may be added; the canonical string is versioned for this reason.

Replay is prevented by construction: codes are single-use and expiring, and
the signed string includes both the code and the agent ID.

**Host neutrality is normative.** A registrar MUST NOT require any
particular agent host, framework, or network for pairing. Whoever holds the
key can pair, from any runtime. This is what makes PAN implementable by any
personal-agent runtime, not just the reference stack.

### 4.2 Submitter-match

For an agent that does not hold a key (for example, an A2A card reachable
only by URL), the owner may submit a minimal agent record under a verified
anchor. A handle anchored to the same email may then bind to it directly.
The proof is that the same email both submitted the record and owns the
name.

## 5. Resolution

Resolution maps a handle to its **card**.

- Resolution MUST be exact-string: case-folded whole-handle lookup.
- Resolution MUST be publicly available without authentication for active
  handles.
- Released handles MUST NOT resolve; registrars SHOULD signal a tombstone
  distinctly from "never existed" in their history log, and MAY do so at
  resolution.
- A handle claimed but not yet bound resolves to its claim record without an
  address ("reserved").

### 5.1 The card

```json
{
  "handle":   "Coder.jeff@gmail.com",
  "operator": { "name": "Jeff Schneider" },   // REQUIRED: the anchor's chosen public label (§3)
  "binding":  "agent-key",                 // "agent-key" | "email-submitter" | null
  "claimed_via": "email",                  // "email" | "delegated:<partner>" (§3.1)
  "claimed_at":  "2026-07-18T…",
  "presence": { "state": "online", "last_seen_at": "…" },   // OPTIONAL, only if a source provides it
  "encryption_key": "<X25519 public key>",   // OPTIONAL, only if the agent declares one
  "endpoints": [
    { "protocol": "agentmesh", "agent_id": "UD653KLV…", "node": "UB2FF…" },
    { "protocol": "a2a", "url": "https://…/.well-known/agent-card.json" }
  ]
}
```

The **endpoints are the address**: the reachable coordinates a messaging
layer uses. For a key-bearing agent, the key itself is the address (on
AgentMesh the agent ID is its inbox); a node or an A2A card URL may
accompany it. Presence is optional and appears only where the registrar
actually observes liveness; PAN does not require or build a presence
subsystem. What a consumer *does* with an endpoint belongs to that
endpoint's protocol, not to PAN.

The OPTIONAL `encryption_key` is the agent's X25519 public key, carried so a
correspondent can *seal* content to the agent before first contact: to
invite a named agent into an end-to-end-private room, or to send it a
confidential message, you resolve the handle and seal to this key. It is a
capability the card advertises, not an address; PAN neither defines nor uses
it, and simply passes through what the agent declares (AgentMesh SPEC §4.3).
Absent means the agent participates only in cleartext.

### 5.2 WebFinger

A PAN handle is a valid `acct:` URI. Registrars SHOULD serve
**WebFinger (RFC 7033)**:

```
GET /.well-known/webfinger?resource=acct:Coder.jeff@gmail.com

{
  "subject": "acct:Coder.jeff@gmail.com",
  "properties": { "urn:pan:binding": "agent-key" },
  "links": [ { "rel": "urn:pan:card", "type": "application/json", "href": "…/api/resolve?handle=…" } ]
}
```

so that two decades of existing `acct:` tooling resolves handles with no
PAN-specific code.

## 6. The history log

Every claim, binding (with its proof method), and release appends an entry.
Entries are never updated or deleted.

**The log is not public.** Handles embed the owner's address in the name, so
a world-readable log would let anyone enumerate an owner's entire roster by
filtering on their email. The log is therefore owner-scoped: an owner may
retrieve the full history of their own handles after proving control of the
anchor, and it is not otherwise readable.

**Hash chaining is REQUIRED.** Each entry carries the SHA-256 hash of the
previous entry, computed over a canonical serialization that includes that
previous hash, so the log is a chain and any rewrite of history is
detectable:

```json
{ "seq": 1041, "at": "…", "action": "bound", "handle": "…",
  "detail": { "method": "agent-key" },
  "prev_hash": "b64…", "entry_hash": "b64…" }
```

Public, privacy-preserving verifiability (letting a third party confirm the
registrar has not rewritten history without learning who owns what) is
future work; see §10.

## 7. Registrar obligations

A conforming registrar:

1. Enforces full-string uniqueness and the cooling-off window.
2. Never parses handles on behalf of consumers, and never exposes an API
   that requires consumers to parse them.
3. Maintains the append-only, hash-chained history log of §6, and serves
   each owner only their own entries.
4. Serves resolution without authentication, and labels every card with its
   binding method.
5. Requires binding proofs per §4: email proof alone never binds to an agent
   record the anchor did not submit.

## 8. Trust model: read this before trusting a handle

**The registrar is a notary, not an oracle.** An email verification is
witnessed once, by one party; no third party can re-run it. Everyone who
trusts a handle is trusting the registrar's word — and, for a handle whose
card says `claimed_via: delegated:<partner>`, the registrar's choice of
witness (§3.1). The history log records
every action under a hash chain, so tampering is detectable, and an owner
can audit the full history of their own handles. But the log is private
(§6), so this version offers no public cross-check on the registrar: a
deliberate trade of external auditability for owner privacy.

Binding sharpens what a handle claims. An `agent-key` binding is
cryptographic: someone holding the agent's key cooperated with the handle
owner, and that proof does not depend on the registrar's honesty. An
`email-submitter` binding is notarized only.

**The operator name is a label, not an identity.** It is required, stable
across the anchor's handles, set only under a verified session, and
change-logged — which makes it a consistent, auditable claim rather than a
per-message assertion. It is still whatever the mailbox owner chose to
type. The verified fact remains the anchor email (which the handle itself
displays); the name rides on it. Renderers SHOULD source the name from
resolution, never from message contents, so a message sender cannot assert
an operator name at all.

What a handle does **not** prove: that the agent is competent, safe,
endorsed by anyone, or that its capability claims are true. A handle is an
address, not a badge. If you need domain-anchored, publicly re-verifiable,
CA-backed identity, that is what ANS is for; PAN does not reach that bar and
does not try to.

## 9. Security considerations and non-goals

- **Code guessing**: registrars MUST bound verification attempts and rate-
  limit issuance (reference: 5 attempts/code, 5 codes/hour/anchor).
- **Squatting**: full-string uniqueness plus visible anchors makes
  impersonation self-labeling: `support.paypal.attacker@gmail.com` carries
  its own anchor in plain sight. Registrars MAY additionally police names
  but the protocol does not require taste.
- **Email-costume confusion**: handles look like email addresses; mail sent
  to one goes wherever the mail system says, which is unrelated to the
  agent. Registrars SHOULD present handles in contexts that discourage
  mailto interpretation.
- **Anchor compromise**: whoever controls the mailbox controls its handles.
  Anchor hygiene (2FA on the account) is inherited, which is also PAN's
  strength: it rides the most hardened credential most people already have.
- **Non-goal, domain anchoring**: PAN does not anchor names to domains or
  issue certificates. That is ANS's domain, and PAN defers to it rather than
  duplicating a lighter, weaker version.

## 10. Out of scope

- **Domain-anchored identity**: names proven by DNS or well-known records,
  CA-backed certificates, DANE. Covered by ANS.
- **Discovery**: search and browse across agents. Covered by catalog/search
  layers like ARD. PAN resolves a name you already have.
- **Reachability and messaging**: contacting a resolved agent, including any
  registrar-hosted chat or relay surface, belongs to the messaging protocols
  named in the card's endpoints (A2A, AgentMesh).
- **Federation**: multiple registrars, referral resolution, cross-registrar
  uniqueness.
- **Additional anchor proofs**: e.g. OIDC sign-in as a mailbox proof.
- **Public, privacy-preserving verifiability**: letting outside parties
  confirm the registrar has not rewritten history without exposing who owns
  what (e.g. Merkle commitments / SCITT-style inclusion proofs over the §6
  chain).

## 11. Reference implementation

The Agent Catalog (this repository) is the reference registrar. Live and
verified end-to-end: email-tier claiming, §4.1 agent-key pairing (with the
agent record created from the signed pairing), §5 resolution (card +
WebFinger), and the §6 hash-chained, owner-scoped history log. The AgentMesh
Rust SDK carries `examples/pan_pair.rs`, a standalone pairing signer
demonstrating §4.1 without any particular agent host.
