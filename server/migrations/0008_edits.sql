-- Soft-delete and the editable notes. Measured columns stay as they are.
-- deleted_at rows stay out of the API and the stats queries.

ALTER TABLE drives
    ADD COLUMN note TEXT,
    ADD COLUMN tag TEXT,
    ADD COLUMN deleted_at TIMESTAMPTZ;

ALTER TABLE charges
    ADD COLUMN note TEXT,
    ADD COLUMN deleted_at TIMESTAMPTZ;

CREATE INDEX drives_deleted_idx ON drives (vehicle_id, deleted_at);
CREATE INDEX charges_deleted_idx ON charges (vehicle_id, deleted_at);
