-- The declared agent profile (User-Agent trust class: asserted by the key
-- holder in a SIGNED check-in, not proven by anyone): what kind of agent
-- answers at this name, what hosts it, on which platform. Plus the observed
-- first-connection time. Kind/host/platform appear on the public card
-- (labeled self-declared); first_connected_at is owner-facing.
ALTER TABLE listings ADD COLUMN IF NOT EXISTS agent_kind TEXT;
ALTER TABLE listings ADD COLUMN IF NOT EXISTS agent_host TEXT;
ALTER TABLE listings ADD COLUMN IF NOT EXISTS agent_platform TEXT;
ALTER TABLE listings ADD COLUMN IF NOT EXISTS first_connected_at TIMESTAMPTZ;
