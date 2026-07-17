# agentcatalog.com — Content & Positioning (working draft)

**Repositioning:** presence-first → personal-agent naming first (PAN as the standard, presence as proof). Drafted 2026-07-17; pending owner review.

## Positioning statement

The Agent Catalog is the reference registrar for PAN — Personal Agent Naming — a small draft standard that gives AI agents human-handleable names anchored to an email address you already own. It's for the largest and least-served agent population on earth: personal agents — Claude Code, OpenClaw, Hermes, Codex, gateway assistants — which outnumber every other kind of agent by orders of magnitude and which nobody names, lists, or resolves. Now is the moment because these agents are starting to talk to each other, and you can't delegate to, vouch for, or even mention an agent that has no name.

## Hero options (ranked)

**A (recommended) — the handle IS the headline.** Rendered as a live handle, monospace, brass accent: `Coder.you@gmail.com` (the `you` segment animates through real-looking addresses). Eyebrow: *A name for your agent.* Subhead: "Your agent works for you every day and has no name anyone else can use. PAN fixes that: one unique string, anchored to the email you already own. Claim it in 30 seconds. Put it in your email signature." CTAs: **Claim your agent's name** / Read the spec →. *Shows the entire product in one string before a sentence is read.*

**B — the deficit framing.** "Your agent doesn't have a name." Strong problem-naming; spends the headline on the problem instead of the artifact.

**C — the standard framing.** "Personal agents, meet your namespace." Leads with the strategic message but uses insider language; best if the primary audience were host developers.

The current hero "Live agents only." is demoted, not deleted — it becomes the Presence section's heading, where it's exactly right.

## Page architecture

| # | Section | Job |
|---|---------|-----|
| 0 | Nav | Claim / The spec / The shelf / Self-host / GitHub |
| 1 | Hero | The handle as the product; CTA = claim |
| 2 | The claim moment | Interactive email → code → name widget; prove "30 seconds" by doing it |
| 3 | Why now | Personal agents are the most numerous and the only unnamed kind |
| 4 | The standard: PAN | Two tiers, two rules; a protocol, not a walled feature |
| 5 | Pairing | "Anyone can claim a name. Pairing proves it's your agent's." |
| 6 | Trust model | Notary, transparency log, what a handle does NOT prove |
| 7 | Presence | "Live agents only." + the decaying-shelf demo, relocated here |
| 8 | For agent-host developers | The ~25-line pairing pitch (OpenClaw/Hermes/coding agents) |
| 9 | Self-host & registrar neutrality | One binary; "we intend to earn the default, not own it" |
| 10 | For standards people | Running code first; /spec; deliberately "draft" |
| 11 | Ecosystem strip + footer | ARD finds · A2A talks · AgentMesh connects · **PAN names** |

## Key copy blocks (final-form highlights)

**Claim widget, step helper lines:** anchor email — "This is the anchor. Whoever controls this inbox controls the handles under it — same rule as everything else in your life." Code — "Expires in 15 minutes. That one verification is all the registrar ever asks of you." Name — live preview `Coder.you@gmail.com — available`; "Any name up to 64 characters. Dots allowed. First come, first served, across the whole registry." Success: "…It resolves publicly, right now — and via WebFinger, so twenty years of `acct:` tooling already understands it. Next: pair it to your actual agent. [Pair your agent →]"

**Why-now pull line:** "Your agent gets a name you can put in an email signature, say in a meeting, or paste into another agent's config."

**PAN section:** the two-tier code block (email tier notarized once / domain tier re-verifiable by anyone) + two rule cards (nobody parses handles; one string, one owner, 90-day cooling-off).

**Pairing section:** three steps ending "The binding is the intersection of the two proofs." Callout: "Host neutrality is normative in the spec, not a marketing promise."

**Trust model (publish as written):** "For email-tier handles, here is exactly what happened: the registrar watched someone prove control of an inbox, once. … A registrar that rewrites history breaks its own chain, in public, permanently." Plain list of what a handle does NOT prove (competent / safe / endorsed / claims true). Closing: "A handle is an address, not a badge."

**Developer pitch:** "Give every agent you host a name. It's one signature." — three steps, `pan-pair-v1:<code>:<agent-id>`, "No API key, no OAuth, no account — the code plus the signature is the whole handshake. … You get out of the naming business entirely."

**Standards section:** "The spec is at /spec. It says 'draft' on purpose. … running code first, then rough consensus. Read it in ten minutes. File issues where it's wrong."

## Kept from the presence-centric prototype

- **Decaying-shelf simulation** — strongest demo on the page; moves to §7, mini-cards now show PAN handles (`Translator.maria@gmail.com`); keep the "simulated shelf" honesty caption.
- "Live agents only." → §7 heading. Phone-book line → body copy under the shelf.
- "It indexes and verifies; it never hosts" → hero small print.
- Dark + brass visual language, presence pills, decay opacity — the notary/ledger aesthetic fits the registrar story even better.
- Self-host strip, reframed to registrar neutrality. Ecosystem strip gains "PAN names."
- Cut from front page: the three-doors "List your agent" section (replaced by claim widget + developer pitch) and the pillar trio (each pillar absorbed into §5–§7, §9).
