# Personal Agent Naming (PAN)

**Version:** 0.1-draft · **Date:** 2026-07-17 · **Status:** draft, one reference implementation

A small protocol giving AI agents human-handleable names — something a person
can put in an email signature or say in a meeting, the way they hand out a
phone number or a social handle. PAN targets the largest and least-served
agent population: **personal agents**, owned by people who have an email
address and nothing else — no domain, no PKI, no ops team.

PAN deliberately does one thing. It names agents and binds names to them. It
does not transport messages (that layer belongs to protocols like A2A and
AgentMesh), does not verify capability claims, and does not host agents.

---

## 1. Terminology

- **Handle** — one globally unique string naming an agent, e.g.
  `PublicAgent.jeffrschneider@gmail.com`.
- **Anchor** — the email address whose control authorizes claims. Everything
  after the first structural role of the handle is anchored to it.
- **Registrar** — a service that accepts claims, enforces uniqueness,
  answers resolution queries, and maintains the transparency log.
- **Listing** — the registrar's record of an agent (however sourced:
  harvested from an agent network, or submitted directly).
- **Binding** — the attachment of a handle to a listing.

## 2. Handles

A handle is formed as:

```
<name>.<email>
```

- `<name>` is any non-empty string up to 64 characters containing no
  whitespace, no `@`, and no control characters. **Dots are allowed.** There
  is no further grammar.
- `<email>` is the anchor, lowercased.

**Rule 1 — nobody parses handles.** A handle is an opaque key. Resolution is
exact-string lookup of the whole handle; no consumer may decompose it. This
is what makes the absence of grammar safe: `my.cool.agent.jeff.schneider@corp.com`
never needs to be split by anyone but the registrar that minted it, which
knows the split because it witnessed the claim.

**Rule 2 — uniqueness is full-string, first come, first served.** The
registrar enforces uniqueness on the case-folded complete handle at claim
time. If two (name, email) pairs would render the same string, the second
claimant is refused with "taken." No exceptions, no adjudication.

Handles are case-insensitive for matching and case-preserving for display.

## 3. Claiming

Control of the anchor authorizes claims under it.

1. Claimant submits the anchor email to the registrar.
2. Registrar delivers a short verification code (6 digits, ≤15-minute
   expiry) to that mailbox, and accepts a bounded number of attempts.
3. A correct code yields a short-lived session (≤30 minutes) under which the
   claimant may claim handles, bind, release, and list their handles.

Registrars MUST rate-limit code issuance per anchor and MUST NOT disclose
whether an anchor has existing handles to unauthenticated parties beyond
what resolution already reveals.

**Lifetime = anchor lifetime.** A handle lives as long as its owner can
re-prove the anchor when required. This is intended: a personal address that
outlives employers keeps its handles; a work address that dies at
offboarding takes its handles with it — that is the governance boundary
working, not a defect.

**Release and cooling-off.** Releasing a handle tombstones it. A released
handle MUST NOT be claimable by anyone for a cooling-off period
(REQUIRED minimum 90 days), so that a handle written down last year does not
silently start pointing at a stranger.

## 4. Binding

Claiming a name and proving you operate an agent are different proofs. A
registrar MUST NOT bind a handle to a listing on email proof alone unless
the listing itself was submitted under that same anchor.

**4.1 Submitter-match (email-proven listings).** A listing submitted
directly to the registrar under a verified anchor records that anchor as its
submitter. A handle anchored to the same email may bind to it freely.

**4.2 Agent-key pairing (key-bearing agents).** For listings that carry a
public key (e.g. agents harvested from a network where the agent ID *is* an
Ed25519 public key), binding requires a signature from that key:

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

For the v0.1 profile: keys are Ed25519 expressed as nkey public strings
(`U…`), and signatures are base64-encoded raw Ed25519 signatures. Other key
profiles may be added; the canonical string is versioned for this reason.

Replay is prevented by construction: codes are single-use and expiring, and
the signed string includes both the code and the agent ID.

**Host neutrality is normative.** A registrar MUST NOT require any
particular agent host, framework, or network for pairing. If a listing
declares a public key, whoever holds that key can pair — this is what makes
PAN implementable by any personal-agent runtime, not just the reference
stack.

## 5. Resolution

Resolution maps a handle to its **card**: the bound listing (agent metadata,
endpoints, and — where the registrar has it — liveness).

- Resolution MUST be exact-string: case-folded whole-handle lookup.
- Resolution MUST be publicly available without authentication for bound,
  active handles.
- Released handles MUST NOT resolve; registrars SHOULD signal a tombstone
  distinctly from "never existed" in their transparency log, and MAY do so
  at resolution.
- A handle claimed but not yet bound resolves to its claim record without a
  card ("reserved").

Registrars SHOULD additionally expose resolution as
**WebFinger (RFC 7033)** — a PAN handle is already a valid `acct:` URI
(`acct:PublicAgent.jeffrschneider@gmail.com`) — so that existing tooling
resolves handles without PAN-specific code.

## 6. Registrar obligations

A conforming registrar:

1. Enforces full-string uniqueness and the cooling-off window.
2. Never parses handles on behalf of consumers, and never exposes an API
   that requires consumers to parse them.
3. Maintains an **append-only transparency log** of every claim, binding
   (with its proof method), and release, publicly readable. Entries are
   never updated or deleted.
4. Serves resolution without authentication.
5. Distinguishes binding methods (`email-submitter` vs `agent-key`) in both
   the log and, where surfaced, the card — so consumers can weigh a
   key-proven binding above a notarized one.

## 7. Trust model — read this before trusting a handle

**The registrar is a notary, not an oracle.** An email verification is
witnessed once, by one party; unlike a DNS record, no third party can ever
re-run the proof. Everyone who trusts a PAN handle is trusting the
registrar's word that the proof happened. This is a deliberate trade —
it is what makes claiming effortless — and it is mitigated, not eliminated,
by the transparency log: a registrar that rewrites history produces a
visible discontinuity.

What a handle proves, precisely:

- *Claimed*: at claim time, the claimant controlled the anchor mailbox —
  as witnessed by this registrar.
- *Bound (agent-key)*: additionally, someone holding the agent's private key
  cooperated with the handle owner. This proof is cryptographic and does not
  depend on the registrar's honesty at binding time.
- *Bound (email-submitter)*: the same anchor both submitted the listing and
  claimed the handle. Notarized, not cryptographic.

What a handle does **not** prove: that the agent is competent, safe,
endorsed by anyone, or that its capability claims are true. A handle is an
address, not a badge.

## 8. Security considerations

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
- **Anchor compromise** — whoever controls the mailbox controls its
  handles; anchor hygiene (2FA on the email account) is inherited, which is
  also PAN's strength: it rides the most hardened credential most people
  have.

## 9. Future work (explicitly out of scope for 0.1)

- **Domain-anchored tier** — `agent@domain` handles proven by DNS/well-known
  records: publicly re-verifiable, no notary. The upgrade path for orgs.
- **Federation** — multiple registrars, referral resolution, and what
  uniqueness means across them.
- **Stronger anchors** — OIDC sign-in as an alternative mailbox proof;
  SCITT/CT-shaped transparency logs with inclusion proofs.
- **Reachability** — what you *do* with a resolved card belongs to the
  messaging layer (A2A, AgentMesh), not to PAN.

## 10. Reference implementation

The Agent Catalog (this repository) is the reference registrar: claim,
resolution, transparency log, and binding per §4 (pairing per §4.2 in
progress at time of writing). The AgentMesh Rust SDK carries a standalone
pairing signer example demonstrating §4.2 without any particular agent
host.
