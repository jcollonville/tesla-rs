-- The free vehicle list stops when it returns HTTP 403 EXCEEDED_LIMIT.
-- vehicle_data keeps its own pause in billing_paused_until. Manual resume clears both.

ALTER TABLE fleet_control ADD COLUMN list_paused_until TIMESTAMPTZ;
