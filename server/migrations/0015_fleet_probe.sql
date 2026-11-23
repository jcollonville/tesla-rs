-- Billing block is no longer a date on the 1st. A probe checks whether Tesla
-- has lifted it. blocked_since is when the block started. next_probe_at is the
-- next single check. probe_list is true when the free vehicle list also returned
-- the block, so the probe uses that list instead of vehicle_data.

ALTER TABLE fleet_control
    ADD COLUMN blocked_since TIMESTAMPTZ,
    ADD COLUMN next_probe_at TIMESTAMPTZ,
    ADD COLUMN probe_list BOOLEAN NOT NULL DEFAULT FALSE;
