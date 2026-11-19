-- Shift on each position lets a restart derive park-versus-gap grace from the
-- last samples. Nullable: older rows stay NULL and use the longer signal-loss grace.

ALTER TABLE positions ADD COLUMN shift_state TEXT;
