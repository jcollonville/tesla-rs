-- One open drive per vehicle, whatever recorded it.
-- The previous index ignored stream rows, so a lagged gear-D sample could
-- insert a second drive after a poll had already closed the real one.

WITH ranked AS (
    SELECT id,
           ROW_NUMBER() OVER (PARTITION BY vehicle_id ORDER BY start_at, id) AS n
    FROM drives
    WHERE end_at IS NULL
)
UPDATE drives AS d
SET end_at = GREATEST(
    d.start_at,
    COALESCE(
        (SELECT MAX(p.recorded_at) FROM positions p WHERE p.drive_id = d.id),
        d.start_at
    )
)
FROM ranked AS r
WHERE d.id = r.id
  AND r.n > 1;

DROP INDEX IF EXISTS drives_one_open_idx;

CREATE UNIQUE INDEX drives_one_open_idx
    ON drives (vehicle_id)
    WHERE end_at IS NULL;
