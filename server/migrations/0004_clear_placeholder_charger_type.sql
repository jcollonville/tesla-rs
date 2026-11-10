-- Tesla reports fast_charger_type as "<invalid>" when the car is not on a
-- fast charger. Those placeholders were stored on charges.charger_type.
-- Re-running this is a no-op once they are cleared.

UPDATE charges
SET charger_type = NULL
WHERE charger_type IN ('<invalid>', '', 'none', 'null')
   OR charger_type ~ '^<.*>$';
