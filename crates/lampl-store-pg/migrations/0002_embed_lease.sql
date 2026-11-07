ALTER TABLE memories ADD COLUMN embed_claimed_until timestamptz;
INSERT INTO schema_version (version) VALUES (2);
