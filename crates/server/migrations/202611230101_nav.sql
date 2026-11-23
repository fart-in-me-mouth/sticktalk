-- Per-account navigation: pinned conversations, favorites, server rail order,
-- saved messages and reminders. Private to the account; bounded in nav.rs.
CREATE TABLE nav_pins (
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 49),
 PRIMARY KEY(user_id,channel_id)
);
CREATE TABLE nav_favorite_categories (
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 id INTEGER NOT NULL CHECK(id>0 AND id<=4294967295),
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 128),
 collapsed INTEGER NOT NULL CHECK(collapsed IN(0,1)),
 position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 19),
 PRIMARY KEY(user_id,id)
);
CREATE TABLE nav_favorites (
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 category_id INTEGER,
 position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 199),
 PRIMARY KEY(user_id,channel_id)
);
CREATE TABLE nav_rail (
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 19),
 guild_id INTEGER,
 folder_id INTEGER,
 CHECK((guild_id IS NULL) <> (folder_id IS NULL)),
 PRIMARY KEY(user_id,position)
);
-- No foreign key to messages: a saved message that is deleted, or whose
-- conversation is left, stays listed as unavailable and never shows content.
CREATE TABLE saved_messages (
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 message_id INTEGER NOT NULL,
 channel_id INTEGER NOT NULL,
 saved_at INTEGER NOT NULL,
 remind_at INTEGER,
 fired_at INTEGER,
 UNIQUE(user_id,message_id)
);
CREATE INDEX saved_by_user ON saved_messages(user_id,seq);
CREATE INDEX saved_reminders ON saved_messages(remind_at) WHERE remind_at IS NOT NULL AND fired_at IS NULL;
