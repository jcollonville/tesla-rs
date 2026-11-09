-- TeslaMate 4.1.1 column types for the tables the importer reads.
-- Source: teslamate-org/teslamate tag v4.1.1, priv/repo/migrations.
-- Smallints come from 20200410112005_database_efficiency_improvements and
-- 20210812173700_car_priorities / 20250613133700_add_and_calculate_elevation_changes.
-- numeric precisions come from that same efficiency migration and
-- 20260718160000_increase_geofence_cost_precision.
-- Dates are timestamp(6) without time zone (Ecto :utc_datetime_usec), not timestamptz.
-- The BRIN opclass in 20260411070212_improve_brin_indexes_on_positions is
-- timestamp_minmax_multi_ops, which only exists for timestamp without time zone.
-- :string columns are varchar; :text columns stay text; billing_type is an enum.

CREATE TABLE cars (
    id smallint PRIMARY KEY,
    eid bigint NOT NULL,
    vid bigint NOT NULL,
    vin text NOT NULL,
    name text,
    model varchar(255),
    efficiency double precision,
    trim_badging text,
    marketing_name varchar(255),
    exterior_color text,
    wheel_type text,
    spoiler_type text,
    display_priority smallint NOT NULL DEFAULT 1,
    inserted_at timestamp(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at timestamp(0) NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE addresses (
    id integer PRIMARY KEY,
    display_name varchar(512),
    name varchar(255),
    house_number varchar(255),
    road varchar(255),
    neighbourhood varchar(255),
    city varchar(255),
    county varchar(255),
    state varchar(255),
    state_district varchar(255),
    postcode varchar(255),
    country varchar(255),
    latitude numeric(8, 6),
    longitude numeric(9, 6),
    osm_id bigint,
    osm_type text,
    raw jsonb,
    inserted_at timestamp(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at timestamp(0) NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TYPE billing_type AS ENUM ('per_kwh', 'per_minute');

CREATE TABLE geofences (
    id integer PRIMARY KEY,
    name varchar(255) NOT NULL,
    latitude numeric(8, 6) NOT NULL,
    longitude numeric(9, 6) NOT NULL,
    radius smallint NOT NULL,
    billing_type billing_type NOT NULL DEFAULT 'per_kwh',
    cost_per_unit numeric(9, 4),
    session_fee numeric(14, 2),
    inserted_at timestamp(0) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at timestamp(0) NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE positions (
    id integer PRIMARY KEY,
    date timestamp(6) NOT NULL,
    latitude numeric(8, 6) NOT NULL,
    longitude numeric(9, 6) NOT NULL,
    speed smallint,
    power smallint,
    odometer double precision,
    ideal_battery_range_km numeric(6, 2),
    battery_level smallint,
    outside_temp numeric(4, 1),
    elevation smallint,
    car_id smallint NOT NULL,
    drive_id integer,
    inside_temp numeric(4, 1),
    est_battery_range_km numeric(6, 2),
    rated_battery_range_km numeric(6, 2),
    usable_battery_level smallint
);

CREATE TABLE drives (
    id integer PRIMARY KEY,
    start_date timestamp(6) NOT NULL,
    end_date timestamp(6),
    outside_temp_avg numeric(4, 1),
    speed_max smallint,
    power_max smallint,
    power_min smallint,
    start_ideal_range_km numeric(6, 2),
    end_ideal_range_km numeric(6, 2),
    start_km double precision,
    end_km double precision,
    distance double precision,
    duration_min smallint,
    car_id smallint NOT NULL,
    inside_temp_avg numeric(4, 1),
    start_address_id integer,
    end_address_id integer,
    start_rated_range_km numeric(6, 2),
    end_rated_range_km numeric(6, 2),
    start_position_id integer,
    end_position_id integer,
    start_geofence_id integer,
    end_geofence_id integer,
    ascent smallint,
    descent smallint
);

CREATE TABLE charging_processes (
    id integer PRIMARY KEY,
    start_date timestamp(6) NOT NULL,
    end_date timestamp(6),
    charge_energy_added numeric(8, 2),
    charge_energy_used numeric(8, 2),
    duration_min smallint,
    car_id smallint NOT NULL,
    outside_temp_avg numeric(4, 1),
    position_id integer NOT NULL,
    address_id integer,
    start_battery_level smallint,
    end_battery_level smallint,
    start_ideal_range_km numeric(6, 2),
    end_ideal_range_km numeric(6, 2),
    start_rated_range_km numeric(6, 2),
    end_rated_range_km numeric(6, 2),
    geofence_id integer,
    cost numeric(14, 2)
);

CREATE TABLE charges (
    id integer PRIMARY KEY,
    date timestamp(6) NOT NULL,
    battery_level smallint,
    usable_battery_level smallint,
    charge_energy_added numeric(8, 2) NOT NULL,
    charger_actual_current smallint,
    charger_power smallint NOT NULL,
    charger_voltage smallint,
    ideal_battery_range_km numeric(6, 2) NOT NULL,
    rated_battery_range_km numeric(6, 2),
    charging_process_id integer NOT NULL,
    outside_temp numeric(4, 1),
    fast_charger_present boolean,
    fast_charger_type varchar(255)
);

INSERT INTO cars (
    id, eid, vid, vin, name, model, efficiency, trim_badging, marketing_name, exterior_color,
    display_priority
) VALUES
    (1, 10, 20, '5YJ3E1EA7KF000001', 'Red', 'model3', 0.15, '74D', 'Model 3', 'Red', 3),
    (2, 11, 21, '5YJYGDEE5MF000002', 'Blue', 'modely', NULL, NULL, 'Model Y', 'Blue', 1);

INSERT INTO addresses (
    id, display_name, house_number, road, city, postcode, country, latitude, longitude, osm_id,
    osm_type, raw
) VALUES (
    1, 'Home', '1', 'Rue de Rivoli', 'Paris', '75001', 'France', 48.856600, 2.352200, 42, 'way',
    '{}'::jsonb
);

INSERT INTO geofences (id, name, latitude, longitude, radius, billing_type, cost_per_unit, session_fee)
VALUES (1, 'Home', 48.856600, 2.352200, 100, 'per_kwh', 0.2000, 1.00);

INSERT INTO positions (
    id, date, latitude, longitude, speed, power, odometer, battery_level, elevation, car_id,
    drive_id, ideal_battery_range_km, rated_battery_range_km, est_battery_range_km, inside_temp,
    outside_temp, usable_battery_level
) VALUES
    (1, timestamp '2024-06-01 10:00:00', 48.856600, 2.352200, 0, 0, 1000, 80, 40, 1, 1, 420, 400, 390, 21, 18, 79),
    (2, timestamp '2024-06-01 10:30:00', 48.860000, 2.360000, 80, 40, 1012.5, 74, 55, 1, 1, 410, 390, 380, 22, 19, 73),
    (3, timestamp '2024-06-02 08:00:00', 48.870000, 2.300000, 30, 10, 1020, 70, 35, 1, 2, 380, 360, 350, 20, 16, 69),
    (4, timestamp '2024-06-02 08:10:00', 48.880000, 2.310000, 0, NULL, 1021, 69, 36, 1, 2, 379, 359, 349, 20, 16, 68);

INSERT INTO drives (
    id, start_date, end_date, speed_max, power_max, power_min, start_km, end_km, distance,
    duration_min, car_id, inside_temp_avg, outside_temp_avg, start_address_id, end_address_id,
    start_rated_range_km, end_rated_range_km, start_ideal_range_km, end_ideal_range_km,
    start_position_id, end_position_id, start_geofence_id, end_geofence_id, ascent, descent
) VALUES (
    1, timestamp '2024-06-01 10:00:00', timestamp '2024-06-01 10:30:00', 110, 80, -20, 1000, 1012.5,
    12.5, 30, 1, 21.5, 18.5, 1, 1, 400, 390, 420, 410, 1, 2, 1, 1, 15, 6
), (
    2, timestamp '2024-06-02 08:00:00', timestamp '2024-06-02 08:10:00', 40, 15, -5, 1020, 1021, 1,
    10, 1, 20, 16, NULL, NULL, 360, 359, 380, 379, 3, 4, NULL, NULL, 1, 0
);

INSERT INTO charging_processes (
    id, start_date, end_date, charge_energy_added, charge_energy_used, duration_min, car_id,
    outside_temp_avg, position_id, address_id, start_battery_level, end_battery_level,
    start_ideal_range_km, end_ideal_range_km, start_rated_range_km, end_rated_range_km,
    geofence_id, cost
) VALUES (
    1, timestamp '2024-06-01 12:00:00', timestamp '2024-06-01 13:00:00', 20, 22, 60, 1,
    19, 2, 1, 40, 80, 200, 360, 180, 340, 1, 4.50
);

INSERT INTO charges (
    id, date, battery_level, usable_battery_level, charge_energy_added, charger_actual_current,
    charger_power, charger_voltage, ideal_battery_range_km, rated_battery_range_km,
    charging_process_id, outside_temp, fast_charger_present, fast_charger_type
) VALUES
    (1, timestamp '2024-06-01 12:05:00', 45, 44, 2, 100, 50, 400, 220, 200, 1, 19, true, 'Combo'),
    (2, timestamp '2024-06-01 12:30:00', 70, 69, 15, 80, 40, 390, 300, 280, 1, 20, true, 'Combo');
