-- Invented demo archive for screenshots and local UI work.
--
-- Everything here is fictional: the VINs, the car names, the towns and
-- addresses, and the coordinates, which sit in the open ocean near 0°N 0°E.
-- Run it against a database the server has already migrated:
--
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f server/demo/seed.sql
--
-- It replaces the two demo cars on every run and leaves other rows alone.
-- Times are relative to now() so the overview always looks recent.

BEGIN;

SELECT setseed(0.4242);

DELETE FROM vehicles WHERE vin IN ('DEMOVIN0000000001', 'DEMOVIN0000000002');
DELETE FROM geofences WHERE source = 'demo';
DELETE FROM addresses WHERE source = 'demo';

CREATE TEMP TABLE demo_places (
    key TEXT PRIMARY KEY,
    latitude DOUBLE PRECISION NOT NULL,
    longitude DOUBLE PRECISION NOT NULL,
    elevation DOUBLE PRECISION NOT NULL,
    display_name TEXT NOT NULL,
    fence TEXT,
    radius DOUBLE PRECISION,
    address_id BIGINT,
    geofence_id BIGINT
) ON COMMIT DROP;

INSERT INTO demo_places (key, latitude, longitude, elevation, display_name, fence, radius) VALUES
    ('home',   0.04120, 0.02870, 118, '8 allée des Tilleuls, 00100 Valbrume', 'Maison', 60),
    ('office', 0.09210, 0.08350, 164, '24 rue de l''Industrie, 00100 Valbrume', 'Bureau', 120),
    ('shop',   0.01750, 0.07020, 96,  'Centre commercial Les Arcades, 00120 Brionne', NULL, NULL),
    ('gym',    0.05300, 0.05600, 141, '3 chemin du Stade, 00100 Valbrume', NULL, NULL),
    ('train',  0.06330, 0.01080, 132, 'Place de la Gare, 00110 Montlieu', NULL, NULL),
    ('sc',     0.58040, 0.66110, 212, 'Aire de la Croisée, 00200 Saint-Aubin-des-Landes', 'Superchargeur de la Croisée', 150),
    ('lake',   0.61500, 0.70800, 186, 'Plage du Lac, 00210 Brionne-sur-Lac', NULL, NULL),
    ('home2',  0.03310, 0.04460, 109, '15 rue des Glycines, 00100 Valbrume', 'Maison', 60),
    ('school', 0.02500, 0.05150, 104, 'Groupe scolaire Jules-Verne, 00100 Valbrume', NULL, NULL);

INSERT INTO addresses (latitude, longitude, display_name, name, city, country, source, external_id)
SELECT latitude, longitude, display_name, split_part(display_name, ',', 1),
       trim(split_part(display_name, ',', 2)), 'Démo', 'demo', 'demo-' || key
FROM demo_places;

INSERT INTO geofences (name, latitude, longitude, radius_m, source, external_id)
SELECT fence, latitude, longitude, radius, 'demo', 'demo-' || key
FROM demo_places WHERE fence IS NOT NULL;

UPDATE demo_places p SET
    address_id = (SELECT id FROM addresses a WHERE a.source = 'demo' AND a.external_id = 'demo-' || p.key),
    geofence_id = (SELECT id FROM geofences g WHERE g.source = 'demo' AND g.external_id = 'demo-' || p.key);

INSERT INTO vehicles (vin, name, model, trim_badging, marketing_name, exterior_color, wheel_type,
                      efficiency_kwh_per_km, distance_unit, source, external_id)
VALUES
    ('DEMOVIN0000000001', 'Nova', 'model3', '74D', 'Long Range', 'DeepBlue', 'Photon18',
     0.150, 'km', 'demo', 'demo-1'),
    ('DEMOVIN0000000002', 'Atlas', 'modely', '74D', 'Grande Autonomie', 'PearlWhite', 'Gemini19',
     0.162, 'km', 'demo', 'demo-2');

-- One drive with dense positions. Returns the battery level on arrival.
CREATE FUNCTION pg_temp.demo_drive(
    vid BIGINT, started TIMESTAMPTZ, origin TEXT, destination TEXT, battery DOUBLE PRECISION,
    odometer DOUBLE PRECISION, capacity DOUBLE PRECISION
) RETURNS DOUBLE PRECISION LANGUAGE plpgsql AS $$
DECLARE
    a demo_places;
    b demo_places;
    straight_km DOUBLE PRECISION;
    road_km DOUBLE PRECISION;
    cruise DOUBLE PRECISION;
    duration_s INTEGER;
    n INTEGER;
    dt CONSTANT DOUBLE PRECISION := 5;
    raw DOUBLE PRECISION[] := '{}';
    total DOUBLE PRECISION := 0;
    scale DOUBLE PRECISION;
    i INTEGER;
    u DOUBLE PRECISION;
    v DOUBLE PRECISION;
    v_prev DOUBLE PRECISION := 0;
    s DOUBLE PRECISION := 0;
    lat DOUBLE PRECISION;
    lon DOUBLE PRECISION;
    lat_prev DOUBLE PRECISION;
    lon_prev DOUBLE PRECISION;
    nx DOUBLE PRECISION;
    ny DOUBLE PRECISION;
    bend DOUBLE PRECISION;
    phase DOUBLE PRECISION := random() * 6.28;
    elev DOUBLE PRECISION;
    elev_prev DOUBLE PRECISION;
    gain DOUBLE PRECISION := 0;
    loss DOUBLE PRECISION := 0;
    power DOUBLE PRECISION;
    energy DOUBLE PRECISION := 0;
    max_speed DOUBLE PRECISION := 0;
    max_power DOUBLE PRECISION := 0;
    min_power DOUBLE PRECISION := 0;
    soc DOUBLE PRECISION := battery;
    heading DOUBLE PRECISION;
    drive_id BIGINT;
    outside DOUBLE PRECISION := 6 + 8 * random();
    at TIMESTAMPTZ;
    mps DOUBLE PRECISION;
    target DOUBLE PRECISION;
    smooth DOUBLE PRECISION := 0;
BEGIN
    SELECT * INTO a FROM demo_places WHERE key = origin;
    SELECT * INTO b FROM demo_places WHERE key = destination;
    straight_km := sqrt(power(b.latitude - a.latitude, 2) + power(b.longitude - a.longitude, 2)) * 111.2;
    road_km := straight_km * (1.22 + random() * 0.08);
    cruise := CASE WHEN road_km > 20 THEN 112 WHEN road_km > 8 THEN 74 ELSE 52 END;
    duration_s := (road_km / (CASE WHEN road_km > 20 THEN 92 ELSE cruise * 0.72 END) * 3600)::INTEGER;
    n := greatest(duration_s / dt, 12);
    nx := -(b.longitude - a.longitude);
    ny := b.latitude - a.latitude;

    FOR i IN 0..n LOOP
        u := i::DOUBLE PRECISION / n;
        v := cruise * (1 - exp(-u * n * dt / 22)) * (1 - exp(-(1 - u) * n * dt / 18));
        IF road_km <= 20 THEN
            v := v * (0.42 + 0.58 * power(abs(sin(u * pi() * (2 + road_km / 1.6) + phase)), 0.35));
        ELSE
            -- Town, then a main road, then the motorway, and back down.
            target := CASE
                WHEN u < 0.06 OR u > 0.95 THEN 46 * (0.45 + 0.55 * power(abs(sin(u * 90 + phase)), 0.35))
                WHEN u < 0.2 OR u > 0.84 THEN 84 + 6 * sin(u * 40 + phase)
                ELSE 127 + 3 * sin(u * 31 + phase)
            END;
            smooth := smooth + (target - smooth) * (1 - exp(-dt::DOUBLE PRECISION / 14));
            v := smooth * (1 - exp(-(1 - u) * n * dt / 18));
        END IF;
        v := greatest(v + 3 * sin(u * 57 + phase) + 2 * sin(u * 141), 0);
        IF i = 0 OR i = n THEN v := 0; END IF;
        raw := raw || v;
        total := total + v;
    END LOOP;
    scale := road_km / greatest(total * dt / 3600, 0.001);

    INSERT INTO drives (vehicle_id, start_at, start_latitude, start_longitude, start_odometer_km,
                        start_battery_level, start_rated_range_km, start_ideal_range_km,
                        start_address_id, start_geofence_id, source)
    VALUES (vid, started, a.latitude, a.longitude, odometer, round(battery), battery * capacity,
            battery * capacity * 1.04, a.address_id, a.geofence_id, 'poll')
    RETURNING id INTO drive_id;

    lat_prev := a.latitude;
    lon_prev := a.longitude;
    elev_prev := a.elevation;
    FOR i IN 0..n LOOP
        u := i::DOUBLE PRECISION / n;
        v := least(raw[i + 1] * scale, 131);
        s := s + v * dt / 3600 / road_km;
        u := least(s, 1);
        bend := (0.06 + random() * 0.0005) * sin(pi() * u) * sin(3 * pi() * u + phase);
        lat := a.latitude + (b.latitude - a.latitude) * u + ny * bend;
        lon := a.longitude + (b.longitude - a.longitude) * u + nx * bend;
        elev := a.elevation + (b.elevation - a.elevation) * u
              + 22 * sin(u * 7.1 + phase) * sin(pi() * u) + 9 * sin(u * 23) * sin(pi() * u);
        mps := v / 3.6;
        power := (1900 * (mps - v_prev / 3.6) / dt * mps
                  + 0.5 * 1.2 * 0.53 * power(mps, 3)
                  + 0.011 * 1900 * 9.81 * mps
                  + 1900 * 9.81 * (elev - elev_prev) / dt) / 820 + 1.7;
        IF power < 0 THEN power := greatest(power * 0.65, -62); END IF;
        power := least(power, 240);
        IF i = 0 OR i = n THEN power := 0.4; END IF;
        energy := energy + power * dt / 3600;
        soc := battery - greatest(energy, 0) / 75 * 100;
        IF lat <> lat_prev OR lon <> lon_prev THEN
            heading := mod((degrees(atan2(lon - lon_prev, lat - lat_prev)) + 360)::NUMERIC, 360)::DOUBLE PRECISION;
        END IF;
        IF elev > elev_prev THEN gain := gain + (elev - elev_prev); ELSE loss := loss + (elev_prev - elev); END IF;
        max_speed := greatest(max_speed, v);
        max_power := greatest(max_power, power);
        min_power := least(min_power, power);
        at := started + make_interval(secs => i * dt);
        INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh, power_kw,
                               odometer_km, elevation_m, battery_level, rated_range_km,
                               outside_temp_c, inside_temp_c, heading_deg, drive_id, source)
        VALUES (vid, at, lat, lon, round(v::NUMERIC, 1), round(power::NUMERIC, 1),
                odometer + road_km * u, round(elev::NUMERIC, 1), round(soc), soc * capacity,
                outside, 20.5, heading, drive_id, 'stream');
        lat_prev := lat;
        lon_prev := lon;
        elev_prev := elev;
        v_prev := v;
    END LOOP;

    UPDATE drives SET
        end_at = started + make_interval(secs => n * dt),
        end_latitude = b.latitude,
        end_longitude = b.longitude,
        end_odometer_km = odometer + road_km,
        distance_km = road_km,
        duration_s = (n * dt)::INTEGER,
        end_battery_level = round(soc),
        end_rated_range_km = soc * capacity,
        end_ideal_range_km = soc * capacity * 1.04,
        energy_used_kwh = greatest(energy, 0.1),
        max_speed_kmh = max_speed,
        avg_speed_kmh = road_km / (n * dt / 3600),
        max_power_kw = max_power,
        min_power_kw = min_power,
        elevation_gain_m = gain,
        elevation_loss_m = loss,
        inside_temp_avg_c = 20.5,
        outside_temp_avg_c = outside,
        end_address_id = b.address_id,
        end_geofence_id = b.geofence_id
    WHERE id = drive_id;
    RETURN soc;
END $$;

-- One charge with samples. Returns the battery level when it stops.
CREATE FUNCTION pg_temp.demo_charge(
    vid BIGINT, started TIMESTAMPTZ, place TEXT, battery DOUBLE PRECISION, target DOUBLE PRECISION,
    dc BOOLEAN, capacity DOUBLE PRECISION, price DOUBLE PRECISION, finish BOOLEAN DEFAULT true
) RETURNS DOUBLE PRECISION LANGUAGE plpgsql AS $$
DECLARE
    p demo_places;
    charge_id BIGINT;
    soc DOUBLE PRECISION := battery;
    added DOUBLE PRECISION := 0;
    kw DOUBLE PRECISION;
    peak DOUBLE PRECISION := 0;
    step DOUBLE PRECISION := CASE WHEN dc THEN 30 ELSE 120 END;
    t DOUBLE PRECISION := 0;
    voltage DOUBLE PRECISION;
    at TIMESTAMPTZ;
    stop_at TIMESTAMPTZ := now() - interval '2 minutes';
BEGIN
    SELECT * INTO p FROM demo_places WHERE key = place;
    INSERT INTO charges (vehicle_id, start_at, start_battery_level, start_rated_range_km,
                         start_ideal_range_km, latitude, longitude, address_id, geofence_id,
                         is_dc, charger_type, source, outside_temp_avg_c)
    VALUES (vid, started, round(battery), battery * capacity, battery * capacity * 1.04,
            p.latitude, p.longitude, p.address_id, p.geofence_id, dc,
            CASE WHEN dc THEN 'Supercharger' ELSE 'Wall Connector' END, 'poll', 9)
    RETURNING id INTO charge_id;

    WHILE soc < target LOOP
        at := started + make_interval(secs => t);
        EXIT WHEN NOT finish AND at > stop_at;
        IF dc THEN
            kw := CASE
                WHEN t < 60 THEN 40 + t * 2.1
                WHEN soc < 32 THEN 168 + 6 * sin(t / 50)
                ELSE greatest(168 - (soc - 32) * 2.75, 38)
            END + random() * 3;
            voltage := 360 + soc * 0.9;
        ELSE
            kw := CASE
                WHEN t < 240 THEN 2 + t / 240 * 9
                WHEN soc > target - 3 THEN 11 * (target - soc) / 3 + 1.2
                ELSE 11 - random() * 0.25
            END;
            voltage := 228 + random() * 4;
        END IF;
        peak := greatest(peak, kw);
        INSERT INTO charge_samples (charge_id, recorded_at, battery_level, usable_battery_level,
                                    charger_power_kw, charger_voltage, charger_current,
                                    charge_energy_added_kwh, rated_range_km, ideal_range_km,
                                    outside_temp_c, source)
        VALUES (charge_id, at, round(soc::NUMERIC, 1), round(soc::NUMERIC, 1), round(kw::NUMERIC, 1),
                round(voltage::NUMERIC, 0), round((kw * 1000 / voltage)::NUMERIC, 0),
                round(added::NUMERIC, 2), soc * capacity, soc * capacity * 1.04, 9, 'poll');
        added := added + kw * step / 3600;
        soc := battery + added / 75 * 100;
        t := t + step;
    END LOOP;

    IF finish THEN
        UPDATE charges SET
            end_at = started + make_interval(secs => t),
            end_battery_level = round(soc),
            end_rated_range_km = soc * capacity,
            end_ideal_range_km = soc * capacity * 1.04,
            energy_added_kwh = added,
            energy_used_kwh = added / CASE WHEN dc THEN 0.96 ELSE 0.89 END,
            max_power_kw = peak,
            duration_s = t::INTEGER,
            cost = round((CASE WHEN dc THEN added ELSE added / 0.89 END * price)::NUMERIC, 2)
        WHERE id = charge_id;
    END IF;
    RETURN soc;
END $$;

DO $$
DECLARE
    nova BIGINT;
    atlas BIGINT;
    day DATE;
    dow INTEGER;
    soc DOUBLE PRECISION := 71;
    odo DOUBLE PRECISION := 18420;
    cap CONSTANT DOUBLE PRECISION := 5.02;
    tz CONSTANT TEXT := 'Europe/Paris';
    at TIMESTAMPTZ;
    before DOUBLE PRECISION;
    week INTEGER;
BEGIN
    SELECT id INTO nova FROM vehicles WHERE vin = 'DEMOVIN0000000001';
    SELECT id INTO atlas FROM vehicles WHERE vin = 'DEMOVIN0000000002';

    FOR day IN SELECT generate_series((now() AT TIME ZONE tz)::DATE - 44, (now() AT TIME ZONE tz)::DATE, '1 day')::DATE LOOP
        dow := extract(isodow FROM day);
        week := extract(week FROM day);
        IF dow <= 5 THEN
            at := (day + time '07:52' + make_interval(mins => (random() * 28)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                before := soc; soc := pg_temp.demo_drive(nova, at, 'home', 'office', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
            END IF;
            IF dow IN (2, 4) THEN
                at := (day + time '12:20' + make_interval(mins => (random() * 15)::INTEGER)) AT TIME ZONE tz;
                IF at < now() THEN
                    before := soc; soc := pg_temp.demo_drive(nova, at, 'office', 'gym', soc, odo, cap);
                    odo := odo + (before - soc) * 6.4;
                    at := at + interval '75 minutes';
                    IF at < now() THEN
                        before := soc; soc := pg_temp.demo_drive(nova, at, 'gym', 'office', soc, odo, cap);
                        odo := odo + (before - soc) * 6.4;
                    END IF;
                END IF;
            END IF;
            at := (day + time '17:40' + make_interval(mins => (random() * 40)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                IF dow = 5 THEN
                    before := soc; soc := pg_temp.demo_drive(nova, at, 'office', 'shop', soc, odo, cap);
                    odo := odo + (before - soc) * 6.4;
                    at := at + interval '55 minutes';
                    IF at < now() THEN
                        before := soc; soc := pg_temp.demo_drive(nova, at, 'shop', 'home', soc, odo, cap);
                        odo := odo + (before - soc) * 6.4;
                    END IF;
                ELSE
                    before := soc; soc := pg_temp.demo_drive(nova, at, 'office', 'home', soc, odo, cap);
                    odo := odo + (before - soc) * 6.4;
                END IF;
            END IF;
        ELSIF dow = 6 AND week % 2 = 0 THEN
            at := (day + time '10:05' + make_interval(mins => (random() * 30)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                before := soc; soc := pg_temp.demo_drive(nova, at, 'home', 'sc', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
                at := at + make_interval(mins => 38);
                soc := pg_temp.demo_charge(nova, at, 'sc', soc, 84 + random() * 6, true, cap, 0.52);
                at := at + interval '33 minutes';
                before := soc; soc := pg_temp.demo_drive(nova, at, 'sc', 'lake', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
                at := (day + time '17:10') AT TIME ZONE tz;
                IF at < now() THEN
                    before := soc; soc := pg_temp.demo_drive(nova, at, 'lake', 'home', soc, odo, cap);
                    odo := odo + (before - soc) * 6.4;
                END IF;
            END IF;
        ELSIF dow = 6 THEN
            at := (day + time '10:30' + make_interval(mins => (random() * 30)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                before := soc; soc := pg_temp.demo_drive(nova, at, 'home', 'shop', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
                at := at + interval '70 minutes';
                before := soc; soc := pg_temp.demo_drive(nova, at, 'shop', 'home', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
            END IF;
        ELSE
            at := (day + time '09:20' + make_interval(mins => (random() * 20)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                before := soc; soc := pg_temp.demo_drive(nova, at, 'home', 'train', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
                at := at + interval '25 minutes';
                before := soc; soc := pg_temp.demo_drive(nova, at, 'train', 'home', soc, odo, cap);
                odo := odo + (before - soc) * 6.4;
            END IF;
        END IF;
        at := (day + time '23:05') AT TIME ZONE tz;
        IF soc < 56 AND at < now() THEN
            soc := pg_temp.demo_charge(nova, at, 'home', soc, CASE WHEN dow = 5 THEN 90 ELSE 80 END, false, cap, 0.2062);
        END IF;
    END LOOP;

    INSERT INTO vehicle_state (vehicle_id, activity, battery_level, usable_battery_level,
                               rated_range_km, ideal_range_km, est_range_km, odometer_km,
                               latitude, longitude, heading_deg, speed_kmh, power_kw,
                               inside_temp_c, outside_temp_c, climate_on, locked, sentry_mode,
                               charging_state, charge_limit_soc, shift_state, software_version,
                               plugged_in, last_poll_at, last_data_at)
    SELECT nova, 'online', round(soc), round(soc), soc * cap, soc * cap * 1.04, soc * cap * 0.9, odo,
           p.latitude, p.longitude, 214, 0, 0, 19.5, 8.5, false, true, false,
           'Disconnected', 80, 'P', '2026.38.4', false,
           now() - interval '3 minutes', now() - interval '3 minutes'
    FROM demo_places p WHERE p.key = 'home';

    soc := 64;
    odo := 7310;
    FOR day IN SELECT generate_series((now() AT TIME ZONE tz)::DATE - 13, (now() AT TIME ZONE tz)::DATE, '1 day')::DATE LOOP
        dow := extract(isodow FROM day);
        IF dow <= 5 THEN
            at := (day + time '08:20' + make_interval(mins => (random() * 10)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                before := soc; soc := pg_temp.demo_drive(atlas, at, 'home2', 'school', soc, odo, 4.9);
                odo := odo + (before - soc) * 6;
                at := at + interval '14 minutes';
                before := soc; soc := pg_temp.demo_drive(atlas, at, 'school', 'train', soc, odo, 4.9);
                odo := odo + (before - soc) * 6;
            END IF;
            at := (day + time '18:15' + make_interval(mins => (random() * 20)::INTEGER)) AT TIME ZONE tz;
            IF at < now() THEN
                before := soc; soc := pg_temp.demo_drive(atlas, at, 'train', 'home2', soc, odo, 4.9);
                odo := odo + (before - soc) * 6;
            END IF;
        END IF;
        at := (day + time '19:30') AT TIME ZONE tz;
        IF soc < 50 AND at < now() THEN
            soc := pg_temp.demo_charge(atlas, at, 'home2', soc, 80, false, 4.9, 0.2062);
        END IF;
    END LOOP;

    at := now() - interval '52 minutes';
    soc := pg_temp.demo_charge(atlas, at, 'home2', soc, 80, false, 4.9, 0.2062, false);

    INSERT INTO vehicle_state (vehicle_id, activity, battery_level, usable_battery_level,
                               rated_range_km, ideal_range_km, est_range_km, odometer_km,
                               latitude, longitude, heading_deg, speed_kmh, power_kw,
                               inside_temp_c, outside_temp_c, climate_on, locked, sentry_mode,
                               charging_state, charger_power_kw, charge_energy_added_kwh,
                               charge_limit_soc, time_to_full_charge_h, plugged_in, is_dc,
                               shift_state, software_version, last_poll_at, last_data_at)
    SELECT atlas, 'charging', round(soc), round(soc), soc * 4.9, soc * 4.9 * 1.04, soc * 4.9 * 0.9, odo,
           p.latitude, p.longitude, 92, 0, 0, 17, 8, false, true, false,
           'Charging', 10.9, (soc - 40) * 0.75, 80, greatest((80 - soc) * 0.75 / 11, 0.2), true, false,
           'P', '2026.38.4', now() - interval '40 seconds', now() - interval '40 seconds'
    FROM demo_places p WHERE p.key = 'home2';
END $$;

COMMIT;
