-- The agent's declared X25519 encryption public key (AgentMesh SPEC 4.3,
-- PAN SPEC 5.1). Carried so a correspondent can seal content to a named
-- agent before first contact (e.g. a sealed-room invite). Declared via the
-- signed check-in (v3 canonical); public on the card.
ALTER TABLE listings ADD COLUMN IF NOT EXISTS agent_encryption_key TEXT;
