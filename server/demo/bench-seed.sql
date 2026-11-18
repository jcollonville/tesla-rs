-- Synthetic archive sized like a few years of daily use, for timing the console.
--
-- Everything is invented: one car with a fake VIN, places in the open ocean
-- near 0°N 0°E, random-walk traces. Run it against a throwaway migrated
-- database, never against real data:
--
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -v drives=5000 -v pts_div=3 -f server/demo/bench-seed.sql
--
-- `drives` is the number of drives (default 5000) and `pts_div` the seconds
-- between stored points (default 3, which gives about 3.3 million positions
-- for 5000 drives). The car is created with id 1 on an empty database.

\if :{?drives}
\else
\set drives 5000
\endif
\if :{?pts_div}
\else
\set pts_div 3
\endif

BEGIN;
SELECT setseed(0.4242);

INSERT INTO vehicles (vin, name, model, trim_badging, marketing_name, exterior_color, wheel_type, efficiency_kwh_per_km, source)
VALUES ('BENCHVIN000000001', 'Bench', 'modely', '74D', 'Model Y', 'DeepBlue', 'Gemini19', 0.15, 'bench');

INSERT INTO geofences (name, latitude, longitude, radius_m, source)
VALUES ('Maison', 0.0412, 0.0287, 60, 'bench'), ('Bureau', 0.0921, 0.0835, 120, 'bench');

-- A few thousand cached addresses, scattered like a Nominatim cache.
INSERT INTO addresses (latitude, longitude, display_name, cache_key, source)
SELECT lat, lon, 'Adresse ' || g || ', 00100 Valbrume', 'bench-' || g, 'bench'
FROM (
    SELECT g, 0.0412 + (random() - 0.5) * 0.9 AS lat, 0.0287 + (random() - 0.5) * 0.9 AS lon
    FROM generate_series(1, 4000) g
) a;
INSERT INTO addresses (latitude, longitude, display_name, cache_key, source)
VALUES (0.0412, 0.0287, '8 allée des Tilleuls, 00100 Valbrume', 'bench-home', 'bench');

-- Drives: round trips from home to eight fixed destinations, newest first.
CREATE TEMP TABLE bench_dest ON COMMIT DROP AS
SELECT n, 0.0412 + 0.012 * n * cos(n * 0.9) AS lat, 0.0287 + 0.016 * n * sin(n * 0.9) AS lon
FROM generate_series(1, 8) n;

INSERT INTO drives (vehicle_id, start_at, end_at, start_latitude, start_longitude, end_latitude, end_longitude,
                    start_odometer_km, end_odometer_km, distance_km, duration_s,
                    start_battery_level, end_battery_level, start_rated_range_km, end_rated_range_km,
                    energy_used_kwh, max_speed_kmh, avg_speed_kmh, source)
SELECT 1,
       start_at,
       start_at + make_interval(secs => dur),
       0.0412, 0.0287, d.lat, d.lon,
       i * 11.0, i * 11.0 + dist, dist, dur,
       80, 70, 400, 350,
       dist * 0.15, 110, dist / (dur / 3600.0), 'bench'
FROM (
    SELECT i,
           now() - interval '20 minutes' - make_interval(secs => i * 20000 + (random() * 3000)::int) AS start_at,
           300 + (random() * 3300)::int AS dur,
           3 + random() * 40 AS dist,
           1 + (i % 8) AS dest
    FROM generate_series(1, :drives) i
) s
JOIN bench_dest d ON d.n = s.dest;

INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, duration_s, start_battery_level, end_battery_level, is_dc, source)
SELECT 1, now() - make_interval(hours => i * 60), now() - make_interval(hours => i * 60) + interval '2 hours',
       20 + random() * 30, 7200, 30, 80, false, 'bench'
FROM generate_series(1, :drives / 3) i;

-- Positions along each drive: a line to the destination plus a wobble.
INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km, battery_level,
                       heading_deg, drive_id, source)
SELECT 1,
       d.start_at + make_interval(secs => k * :pts_div),
       d.start_latitude + (d.end_latitude - d.start_latitude) * (k * :pts_div::float / d.duration_s)
           + 0.0004 * sin(k * 0.05 + d.id),
       d.start_longitude + (d.end_longitude - d.start_longitude) * (k * :pts_div::float / d.duration_s)
           + 0.0004 * cos(k * 0.05 + d.id),
       60 + 30 * sin(k * 0.02),
       10 + 20 * sin(k * 0.03),
       d.start_odometer_km + d.distance_km * (k * :pts_div::float / d.duration_s),
       80 - 10 * (k * :pts_div::float / d.duration_s),
       90,
       d.id,
       'bench'
FROM drives d
CROSS JOIN LATERAL generate_series(0, d.duration_s / :pts_div) k
WHERE d.vehicle_id = 1 AND d.source = 'bench';

INSERT INTO vehicle_state (vehicle_id, activity, battery_level, rated_range_km, odometer_km, latitude, longitude,
                           heading_deg, locked, charging_state, charge_limit_soc, software_version,
                           last_poll_at, last_data_at)
VALUES (1, 'online', 72, 360, 59000, 0.0412, 0.0287, 90, true, 'Disconnected', 80, '2099.1.1',
        now(), now());

COMMIT;
ANALYZE;
