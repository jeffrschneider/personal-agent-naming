-- Owner-facing provenance ("last connected from", Gmail-style recognition):
-- the public IP the agent's machine last used when it talked to the registrar
-- (pairing completion or a signed check-in). Shown ONLY to the owner in their
-- console; never on the public card.
ALTER TABLE listings ADD COLUMN IF NOT EXISTS last_seen_ip TEXT;
ALTER TABLE listings ADD COLUMN IF NOT EXISTS last_seen_ip_at TIMESTAMPTZ;
