-- Pending authenticator secret for the signed-in session. It expires and is
-- not kept after the code is confirmed.

ALTER TABLE sessions
    ADD COLUMN totp_pending BYTEA,
    ADD COLUMN totp_pending_until TIMESTAMPTZ;
