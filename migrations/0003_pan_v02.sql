-- PAN v0.2 schema batch: listing ownership, binding methods, pairing codes,
-- hash-chained transparency log, registrar meta (signing key), and the
-- columns the domain tier needs (anchor / verified_at / stale).

-- Who submitted a manual listing (verified anchor). NULL for harvested rows.
ALTER TABLE listings ADD COLUMN IF NOT EXISTS owner_email TEXT;

-- How a handle's current binding was proven (§4):
-- 'email-submitter' | 'agent-key' | 'domain-record' | NULL (unbound)
ALTER TABLE handles ADD COLUMN IF NOT EXISTS bind_method TEXT;
-- Anchor tier (§2): 'email' | 'domain'. For domain-tier rows, the email
-- column holds the anchor domain.
ALTER TABLE handles ADD COLUMN IF NOT EXISTS anchor TEXT NOT NULL DEFAULT 'email';
-- Domain tier only: last successful record re-verification; staleness flag.
ALTER TABLE handles ADD COLUMN IF NOT EXISTS verified_at TIMESTAMPTZ;
ALTER TABLE handles ADD COLUMN IF NOT EXISTS stale BOOLEAN NOT NULL DEFAULT false;

-- Single-use pairing codes (§4.2).
CREATE TABLE IF NOT EXISTS pairing_codes (
    code       TEXT PRIMARY KEY,
    handle     TEXT NOT NULL,
    email      TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ
);

-- Hash chain (§6). Entries before this migration keep NULL hashes; the
-- chain starts at the next append with prev_hash = '' (genesis).
ALTER TABLE handle_log ADD COLUMN IF NOT EXISTS prev_hash TEXT;
ALTER TABLE handle_log ADD COLUMN IF NOT EXISTS entry_hash TEXT;

-- Registrar-scoped durable values (log signing seed, etc.).
CREATE TABLE IF NOT EXISTS registrar_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
