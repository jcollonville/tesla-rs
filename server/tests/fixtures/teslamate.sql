CREATE TABLE cars (
    id integer PRIMARY KEY,
    eid integer NOT NULL,
    vid bigint NOT NULL,
    vin text NOT NULL UNIQUE,
    name text,
    model text,
    efficiency double precision,
    trim_badging text,
    marketing_name text,
    exterior_color text,
    wheel_type text,
    spoiler_type text,
    display_priority integer NOT NULL DEFAULT 1,
    inserted_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE addresses (
    id integer PRIMARY KEY,
    display_name text,
    name text,
    house_number text,
    road text,
    neighbourhood text,
    city text,
    county text,
    state text,
    state_district text,
    postcode text,
    country text,
    latitude numeric,
    longitude numeric,
    osm_id bigint,
    osm_type text,
    raw jsonb,
    inserted_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TYPE billing_type AS ENUM ('per_kwh', 'per_minute');

CREATE TABLE geofences (
    id integer PRIMARY KEY,
    name text NOT NULL,
    latitude numeric NOT NULL,
    longitude numeric NOT NULL,
    radius integer NOT NULL,
    billing_type billing_type NOT NULL DEFAULT 'per_kwh',
    cost_per_unit numeric,
    session_fee numeric,
    inserted_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE positions (
    id bigint PRIMARY KEY,
    date timestamptz NOT NULL,
    latitude numeric,
    longitude numeric,
    speed integer,
    power integer,
    odometer double precision,
    ideal_battery_range_km numeric,
    battery_level integer,
    outside_temp numeric,
    elevation integer,
    car_id integer NOT NULL,
    drive_id integer,
    inside_temp numeric,
    est_battery_range_km numeric,
    rated_battery_range_km numeric,
    usable_battery_level integer
);

CREATE TABLE drives (
    id integer PRIMARY KEY,
    start_date timestamptz NOT NULL,
    end_date timestamptz,
    outside_temp_avg numeric,
    speed_max integer,
    power_max integer,
    power_min integer,
    start_ideal_range_km numeric,
    end_ideal_range_km numeric,
    start_km double precision,
    end_km double precision,
    distance double precision,
    duration_min integer,
    car_id integer NOT NULL,
    inside_temp_avg numeric,
    start_address_id integer,
    end_address_id integer,
    start_rated_range_km numeric,
    end_rated_range_km numeric,
    start_position_id integer,
    end_position_id integer,
    start_geofence_id integer,
    end_geofence_id integer,
    ascent integer,
    descent integer
);

CREATE TABLE charging_processes (
    id integer PRIMARY KEY,
    start_date timestamptz NOT NULL,
    end_date timestamptz,
    charge_energy_added numeric,
    charge_energy_used numeric,
    duration_min integer,
    car_id integer NOT NULL,
    outside_temp_avg numeric,
    position_id integer,
    address_id integer,
    start_battery_level integer,
    end_battery_level integer,
    start_ideal_range_km numeric,
    end_ideal_range_km numeric,
    start_rated_range_km numeric,
    end_rated_range_km numeric,
    geofence_id integer,
    cost numeric
);

CREATE TABLE charges (
    id integer PRIMARY KEY,
    date timestamptz NOT NULL,
    battery_level integer,
    usable_battery_level integer,
    charge_energy_added numeric,
    charger_actual_current integer,
    charger_power integer,
    charger_voltage integer,
    ideal_battery_range_km numeric,
    rated_battery_range_km numeric,
    charging_process_id integer NOT NULL,
    outside_temp numeric,
    fast_charger_present boolean,
    fast_charger_type text
);

INSERT INTO cars (id, eid, vid, vin, name, model, efficiency, trim_badging, marketing_name, exterior_color)
VALUES
    (1, 10, 20, '5YJ3E1EA7KF000001', 'Red', 'model3', 0.15, '74D', 'Model 3', 'Red'),
    (2, 11, 21, '5YJYGDEE5MF000002', 'Blue', 'modely', NULL, NULL, 'Model Y', 'Blue');

INSERT INTO addresses (id, display_name, house_number, road, city, postcode, country, latitude, longitude, osm_id, osm_type, raw)
VALUES (1, 'Home', '1', 'Rue de Rivoli', 'Paris', '75001', 'France', 48.8566, 2.3522, 42, 'way', '{}'::jsonb);

INSERT INTO geofences (id, name, latitude, longitude, radius, billing_type, cost_per_unit, session_fee)
VALUES (1, 'Home', 48.8566, 2.3522, 100, 'per_kwh', 0.20, 1.00);

INSERT INTO positions (id, date, latitude, longitude, speed, power, odometer, battery_level, elevation, car_id, drive_id, ideal_battery_range_km, rated_battery_range_km, est_battery_range_km, inside_temp, outside_temp, usable_battery_level)
VALUES
    (1, '2024-06-01 10:00:00+00', 48.8566, 2.3522, 0, 0, 1000, 80, 40, 1, 1, 420, 400, 390, 21, 18, 79),
    (2, '2024-06-01 10:30:00+00', 48.8600, 2.3600, 80, 40, 1012.5, 74, 55, 1, 1, 410, 390, 380, 22, 19, 73),
    (3, '2024-06-02 08:00:00+00', 48.8700, 2.3000, 30, 10, 1020, 70, 35, 1, 2, 380, 360, 350, 20, 16, 69),
    (4, '2024-06-02 08:10:00+00', 48.8800, 2.3100, 0, 0, 1021, 69, 36, 1, 2, 379, 359, 349, 20, 16, 68);

INSERT INTO drives (
    id, start_date, end_date, speed_max, power_max, power_min, start_km, end_km, distance, duration_min,
    car_id, inside_temp_avg, outside_temp_avg, start_address_id, end_address_id,
    start_rated_range_km, end_rated_range_km, start_ideal_range_km, end_ideal_range_km,
    start_position_id, end_position_id, start_geofence_id, end_geofence_id, ascent, descent
) VALUES (
    1, '2024-06-01 10:00:00+00', '2024-06-01 10:30:00+00', 110, 80, -20, 1000, 1012.5, 12.5, 30,
    1, 21.5, 18.5, 1, 1, 400, 390, 420, 410, 1, 2, 1, 1, 15, 6
), (
    2, '2024-06-02 08:00:00+00', '2024-06-02 08:10:00+00', 40, 15, -5, 1020, 1021, 1, 10,
    1, 20, 16, NULL, NULL, 360, 359, 380, 379, 3, 4, NULL, NULL, 1, 0
);

INSERT INTO charging_processes (
    id, start_date, end_date, charge_energy_added, charge_energy_used, duration_min, car_id,
    outside_temp_avg, position_id, address_id, start_battery_level, end_battery_level,
    start_ideal_range_km, end_ideal_range_km, start_rated_range_km, end_rated_range_km,
    geofence_id, cost
) VALUES (
    1, '2024-06-01 12:00:00+00', '2024-06-01 13:00:00+00', 20, 22, 60, 1,
    19, 2, 1, 40, 80, 200, 360, 180, 340, 1, 4.50
);

INSERT INTO charges (
    id, date, battery_level, usable_battery_level, charge_energy_added, charger_actual_current,
    charger_power, charger_voltage, ideal_battery_range_km, rated_battery_range_km,
    charging_process_id, outside_temp, fast_charger_present, fast_charger_type
) VALUES
    (1, '2024-06-01 12:05:00+00', 45, 44, 2, 100, 50, 400, 220, 200, 1, 19, true, 'Combo'),
    (2, '2024-06-01 12:30:00+00', 70, 69, 15, 80, 40, 390, 300, 280, 1, 20, true, 'Combo');
