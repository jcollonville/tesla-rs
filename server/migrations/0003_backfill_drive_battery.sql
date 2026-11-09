-- TeslaMate keeps drive state of charge on the start and end positions, not on
-- drives. Imports before that join stored NULL. Fill only missing values from
-- the earliest and latest position already copied for each closed or open drive.
-- Re-running this is a no-op once the columns are set.

UPDATE drives AS d
SET start_battery_level = src.level
FROM (
    SELECT DISTINCT ON (drive_id)
        drive_id,
        COALESCE(battery_level, usable_battery_level) AS level
    FROM positions
    WHERE drive_id IS NOT NULL
    ORDER BY drive_id, recorded_at ASC, id ASC
) AS src
WHERE d.id = src.drive_id
  AND d.start_battery_level IS NULL
  AND src.level IS NOT NULL;

UPDATE drives AS d
SET end_battery_level = src.level
FROM (
    SELECT DISTINCT ON (drive_id)
        drive_id,
        COALESCE(battery_level, usable_battery_level) AS level
    FROM positions
    WHERE drive_id IS NOT NULL
    ORDER BY drive_id, recorded_at DESC, id DESC
) AS src
WHERE d.id = src.drive_id
  AND d.end_at IS NOT NULL
  AND d.end_battery_level IS NULL
  AND src.level IS NOT NULL;
