-- Privacy: timed status, custom status emoji and expiry, friend nicknames,
-- notes, ignore, who can DM you and message requests, notification controls,
-- and in-app reports. See docs/features/privacy.md.

-- Status: the chosen presence may end at a time; the custom status may clear itself.
ALTER TABLE users ADD COLUMN status_emoji TEXT;
ALTER TABLE users ADD COLUMN status_clear_at INTEGER;
ALTER TABLE users ADD COLUMN presence_until INTEGER;
CREATE INDEX users_status_clear ON users(status_clear_at) WHERE status_clear_at IS NOT NULL;
CREATE INDEX users_presence_until ON users(presence_until) WHERE presence_until IS NOT NULL;

-- Private to user_id: a nickname for a friend, and a note about anyone.
CREATE TABLE friend_nicknames(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 other_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 nickname TEXT NOT NULL,
 PRIMARY KEY(user_id,other_id)
);
CREATE TABLE user_notes(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 other_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 note TEXT NOT NULL,
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(user_id,other_id)
);

-- Ignore: separate from blocking, and never visible to the ignored person.
CREATE TABLE user_ignores(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 ignored_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 created_at INTEGER NOT NULL,
 PRIMARY KEY(user_id,ignored_id)
);
CREATE INDEX user_ignores_ignored ON user_ignores(ignored_id);

-- Who may start a DM with you. No row means friends only.
CREATE TABLE dm_privacy(
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 allow TEXT NOT NULL CHECK(allow IN('friends','server_members'))
);
CREATE TABLE dm_privacy_servers(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 allow INTEGER NOT NULL CHECK(allow IN(0,1)),
 PRIMARY KEY(user_id,guild_id)
);

-- A first-contact DM. Until it is accepted the recipient's dm_members row
-- stays hidden (visible=0). Accepted rows stay, to count first contacts.
CREATE TABLE message_requests(
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 recipient_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 sender_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 state TEXT NOT NULL CHECK(state IN('pending','spam','ignored','accepted')),
 created_at INTEGER NOT NULL,
 PRIMARY KEY(channel_id,recipient_id)
);
CREATE INDEX message_requests_recipient ON message_requests(recipient_id,state,created_at);
CREATE INDEX message_requests_sender ON message_requests(sender_id,created_at);

-- Notification controls beyond `notification_settings` (same scope keys),
-- and the mute deadline the expiry timer arms for.
CREATE TABLE notification_extras(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 scope TEXT NOT NULL CHECK(scope IN('guild','channel')),
 target_id INTEGER NOT NULL,
 suppress_roles INTEGER NOT NULL DEFAULT 0,
 mobile_push INTEGER NOT NULL DEFAULT 1,
 mute_until INTEGER,
 PRIMARY KEY(user_id,scope,target_id)
);
CREATE INDEX notification_extras_mute ON notification_extras(mute_until) WHERE mute_until IS NOT NULL;
CREATE TABLE global_notifications(
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 settings TEXT NOT NULL
);

-- Mentions that must not count for that person (their settings suppress
-- them, or they ignore the author). Decided when the mention is stored.
CREATE TABLE suppressed_mentions(
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL,
 PRIMARY KEY(message_id,user_id)
);

-- Reports. The reporter has no foreign key: a report outlives the account,
-- and the reporter is never shown to its handlers.
CREATE TABLE reports(
 id INTEGER PRIMARY KEY CHECK(id>0),
 reporter_id INTEGER NOT NULL,
 queue TEXT NOT NULL CHECK(queue IN('guild','admin')),
 guild_id INTEGER REFERENCES guilds(id) ON DELETE SET NULL,
 target TEXT NOT NULL,
 target_key TEXT NOT NULL,
 target_user_id INTEGER,
 category TEXT NOT NULL,
 note TEXT,
 evidence TEXT NOT NULL,
 status TEXT NOT NULL DEFAULT 'open' CHECK(status IN('open','dismissed','actioned')),
 action TEXT,
 created_at INTEGER NOT NULL,
 resolved_at INTEGER,
 resolved_by INTEGER
);
CREATE INDEX reports_queue ON reports(queue,guild_id,status,id);
CREATE INDEX reports_reporter ON reports(reporter_id,id);
CREATE UNIQUE INDEX reports_once ON reports(reporter_id,target_key) WHERE status='open';
