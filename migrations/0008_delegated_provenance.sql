-- Delegated witnessing: sessions carry provenance ('email' for a directly
-- verified inbox, 'delegated:<partner>' when a trusted partner attested the
-- verification), and handles record how their claim was witnessed. The card
-- and the history log disclose it; delegated sessions cannot release.
ALTER TABLE email_sessions ADD COLUMN provenance text NOT NULL DEFAULT 'email';
ALTER TABLE handles ADD COLUMN claimed_via text NOT NULL DEFAULT 'email';
