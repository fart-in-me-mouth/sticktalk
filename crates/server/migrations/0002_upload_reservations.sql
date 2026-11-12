CREATE TABLE upload_reservations(key TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),utc_day INTEGER NOT NULL,reserved_bytes INTEGER NOT NULL);
