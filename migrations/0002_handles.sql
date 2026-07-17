-- Handles: email-anchored unique names for agents. A handle is one string,
-- "<name>.<email>", claimed by proving control of the email. The catalog is
-- the registrar: uniqueness is enforced on the full string (first come,
-- first served — no grammar, dots welcome), resolution is exact-string
-- lookup, and every action lands in the append-only handle_log so the
-- catalog's notary power is auditable.

CREATE TABLE IF NOT EXISTS handles (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Display form, e.g. 'PublicAgent.jeffrschneider@gmail.com'
    handle      TEXT NOT NULL,
    -- Canonical (lowercased) form; uniqueness key
    handle_key  TEXT NOT NULL,
    -- The verified anchor (lowercased)
    email       TEXT NOT NULL,
    -- The agent this handle points at; NULL = reserved, no agent attached
    listing_id  UUID REFERENCES listings(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Tombstone. Released handles stay as rows (cooling-off + history).
    released_at TIMESTAMPTZ
);
-- One ACTIVE claim per handle string.
CREATE UNIQUE INDEX IF NOT EXISTS idx_handles_active
    ON handles (handle_key) WHERE released_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_handles_email   ON handles (email);
CREATE INDEX IF NOT EXISTS idx_handles_listing ON handles (listing_id);

-- Append-only registrar log (transparency). Never updated, never deleted.
CREATE TABLE IF NOT EXISTS handle_log (
    id     BIGSERIAL PRIMARY KEY,
    at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    action TEXT NOT NULL,           -- 'claimed' | 'bound' | 'released'
    handle TEXT NOT NULL,
    email  TEXT NOT NULL,
    detail JSONB NOT NULL DEFAULT '{}'
);

-- Email verification: short-lived codes, and the sessions they mint.
CREATE TABLE IF NOT EXISTS email_codes (
    email      TEXT NOT NULL,
    code       TEXT NOT NULL,
    attempts   INT NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_email_codes_email ON email_codes (email);

CREATE TABLE IF NOT EXISTS email_sessions (
    token      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email      TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
