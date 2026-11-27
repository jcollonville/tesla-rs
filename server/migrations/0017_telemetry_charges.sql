-- Live charges now record their real source: 'poll' or 'telemetry'.
-- Keep at most one open live charge per vehicle, whatever its source.
DROP INDEX IF EXISTS charges_one_open_idx;
CREATE UNIQUE INDEX charges_one_open_idx
    ON charges (vehicle_id)
    WHERE end_at IS NULL AND source IN ('poll', 'telemetry');
