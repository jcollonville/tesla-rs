-- Charge schedule already present on vehicle_data and Fleet Telemetry.
-- Times are minutes after midnight on the car's local clock, the same unit as
-- scheduled_charging_start_time. Mode is Off, StartAt, or DepartBy.

ALTER TABLE vehicle_state
    ADD COLUMN scheduled_charging_mode TEXT,
    ADD COLUMN scheduled_departure_time INTEGER,
    ADD COLUMN off_peak_charging_enabled BOOLEAN,
    ADD COLUMN off_peak_hours_end_time INTEGER;
