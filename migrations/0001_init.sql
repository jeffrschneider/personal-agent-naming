-- Agent Catalog v0 schema.
--
-- Listings are JSON documents (the full manifest) with the filterable fields
-- lifted into real columns. Search is a stored tsvector over name +
-- description. Presence is current-state-only (history comes later, sampled).
-- Probes are the append-only "prove it" record backing verification badges.

CREATE TABLE IF NOT EXISTS listings (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Which connector produced this row: 'manual' | 'agentmesh' | 'a2a' | …
    source      TEXT NOT NULL,
    -- The listing's identity within its source (mesh agent id, A2A card URL,
    -- submission slug). (source, source_id) is the upsert key — federation
    -- and re-harvesting stay idempotent.
    source_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- The full document, verbatim from the source.
    manifest    JSONB NOT NULL DEFAULT '{}',
    -- Namespaced specialty claims: 'build:macos', 'access:hubspot-api',
    -- 'runtime:coding-assistant'. Thin convention, not an ontology.
    specialties TEXT[] NOT NULL DEFAULT '{}',
    -- 'mesh' | 'a2a' | 'mcp' | 'http' | 'unknown'
    protocol    TEXT NOT NULL DEFAULT 'unknown',
    -- Source-asserted trust tier, when the source has one (the mesh does).
    trust       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    search      TSVECTOR GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(description, '')), 'B')
    ) STORED,
    UNIQUE (source, source_id)
);

CREATE INDEX IF NOT EXISTS idx_listings_search      ON listings USING GIN (search);
CREATE INDEX IF NOT EXISTS idx_listings_manifest    ON listings USING GIN (manifest);
CREATE INDEX IF NOT EXISTS idx_listings_specialties ON listings USING GIN (specialties);
CREATE INDEX IF NOT EXISTS idx_listings_protocol    ON listings (protocol);

-- Current liveness per listing. A cache fed by connectors, not a record:
-- rebuilt from the source network after a restart.
CREATE TABLE IF NOT EXISTS presence (
    listing_id  UUID PRIMARY KEY REFERENCES listings(id) ON DELETE CASCADE,
    -- 'online' | 'offline' | 'unknown'
    state       TEXT NOT NULL DEFAULT 'unknown',
    last_seen_at TIMESTAMPTZ,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Append-only probe outcomes ("verified responding, 240ms, 3h ago").
CREATE TABLE IF NOT EXISTS probes (
    id          BIGSERIAL PRIMARY KEY,
    listing_id  UUID NOT NULL REFERENCES listings(id) ON DELETE CASCADE,
    at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    ok          BOOLEAN NOT NULL,
    latency_ms  INTEGER,
    detail      TEXT
);

CREATE INDEX IF NOT EXISTS idx_probes_listing ON probes (listing_id, at DESC);
