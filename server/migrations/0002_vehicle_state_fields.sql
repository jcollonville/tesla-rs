-- Extra live state for the read API and Home Assistant.
-- Volts, amperes, kilowatts, bar, Celsius, and metres. Distances stay kilometres.
-- Door and window columns use the Fleet API names. 0 is closed. Nonzero is open.
-- scheduled_charging_start_time is minutes after midnight, the Fleet API unit.

ALTER TABLE vehicle_state
    ADD COLUMN charger_voltage DOUBLE PRECISION,
    ADD COLUMN charger_current DOUBLE PRECISION,
    ADD COLUMN charge_current_request DOUBLE PRECISION,
    ADD COLUMN charge_port_door_open BOOLEAN,
    ADD COLUMN charge_port_latch TEXT,
    ADD COLUMN plugged_in BOOLEAN,
    ADD COLUMN time_to_full_charge_h DOUBLE PRECISION,
    ADD COLUMN scheduled_charging_start_time INTEGER,
    ADD COLUMN tpms_pressure_fl DOUBLE PRECISION,
    ADD COLUMN tpms_pressure_fr DOUBLE PRECISION,
    ADD COLUMN tpms_pressure_rl DOUBLE PRECISION,
    ADD COLUMN tpms_pressure_rr DOUBLE PRECISION,
    ADD COLUMN df INTEGER,
    ADD COLUMN dr INTEGER,
    ADD COLUMN pf INTEGER,
    ADD COLUMN pr INTEGER,
    ADD COLUMN fd_window INTEGER,
    ADD COLUMN fp_window INTEGER,
    ADD COLUMN rd_window INTEGER,
    ADD COLUMN rp_window INTEGER,
    ADD COLUMN ft INTEGER,
    ADD COLUMN rt INTEGER,
    ADD COLUMN is_user_present BOOLEAN,
    ADD COLUMN elevation_m DOUBLE PRECISION,
    ADD COLUMN battery_heater_on BOOLEAN,
    ADD COLUMN is_preconditioning BOOLEAN;
