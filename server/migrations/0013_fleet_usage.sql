-- Billable Fleet API calls. `vehicle_data` is the metered endpoint: every response
-- counts, including HTTP 408 and other 4xx. The day is UTC. Caps and the billing
-- pause read this table; the vehicle list and the Owner stream do not write it.

CREATE TABLE fleet_usage (
    day DATE NOT NULL,
    endpoint TEXT NOT NULL,
    status INTEGER NOT NULL,
    count BIGINT NOT NULL DEFAULT 0 CHECK (count >= 0),
    PRIMARY KEY (day, endpoint, status)
);

CREATE TABLE fleet_control (
    id SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    billing_paused_until TIMESTAMPTZ
);

INSERT INTO fleet_control (id) VALUES (1);
