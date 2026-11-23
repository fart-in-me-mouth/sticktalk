-- Shared activities (docs/features/activities.md). Sessions live in memory;
-- only the registry, each server's choices, invite cards and short results
-- are stored.

-- Activities the instance administrator registered by URL.
CREATE TABLE custom_activities (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    icon TEXT NOT NULL,
    url TEXT NOT NULL,
    origin TEXT NOT NULL,
    min_players INTEGER NOT NULL,
    max_players INTEGER NOT NULL,
    in_dms INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- A server's choice per activity (built-in ones are on, registered ones off,
-- until a row says otherwise).
CREATE TABLE guild_activities (
    guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    PRIMARY KEY (guild_id, activity_id)
) WITHOUT ROWID;

-- Invite cards: the message's `extra.activity`.
CREATE TABLE activity_cards (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL,
    payload BLOB NOT NULL
);

-- Short results of finished sessions, the newest 50 per conversation.
CREATE TABLE activity_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    text TEXT NOT NULL,
    ended_at INTEGER NOT NULL
);
CREATE INDEX activity_results_channel ON activity_results(channel_id, id);

-- Use Activities (1 << 48) is on for @everyone, in existing and new servers.
UPDATE roles SET permissions = permissions | 281474976710656 WHERE id = guild_id;
CREATE TRIGGER activities_everyone_default AFTER INSERT ON roles
WHEN NEW.id = NEW.guild_id
BEGIN
    UPDATE roles SET permissions = permissions | 281474976710656
    WHERE guild_id = NEW.guild_id AND id = NEW.id;
END;
