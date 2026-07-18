-- PAN v0.3: the operator record. One required display name per verified
-- email, set at first claim, shown on every card of that owner's handles.
-- The name is the operator's chosen public label, anchored to the proven
-- email; it is NOT verified identity (spec makes this explicit).
CREATE TABLE IF NOT EXISTS operators (
    email       TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
