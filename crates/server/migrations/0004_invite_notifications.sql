ALTER TABLE invites ADD COLUMN inactive_notified INTEGER NOT NULL DEFAULT 0 CHECK(inactive_notified IN (0,1));
