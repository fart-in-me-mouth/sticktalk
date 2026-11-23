-- Moderation: audit log, timeouts, safety settings, AutoMod, age-restricted
-- channels, the wider slowmode list, and indexes for the raised limits.

-- Actors keep no foreign key: entries outlive the accounts that made them.
CREATE TABLE audit_log(
 id INTEGER PRIMARY KEY,
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 actor_id INTEGER NOT NULL CHECK(actor_id>0),
 action TEXT NOT NULL,
 target_kind TEXT NOT NULL,
 target_id INTEGER,
 target_name TEXT,
 changes TEXT NOT NULL DEFAULT '[]',
 reason TEXT,
 created_at INTEGER NOT NULL
);
CREATE INDEX audit_guild ON audit_log(guild_id,id);
CREATE INDEX audit_guild_actor ON audit_log(guild_id,actor_id,id);
CREATE INDEX audit_guild_action ON audit_log(guild_id,action,id);

-- A timeout survives leaving and rejoining, so it cannot be evaded.
CREATE TABLE member_timeouts(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 until INTEGER NOT NULL,
 reason TEXT,
 actor_id INTEGER NOT NULL,
 PRIMARY KEY(guild_id,user_id)
);
CREATE INDEX member_timeout_until ON member_timeouts(until);

CREATE TABLE guild_safety(
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 verification_level TEXT NOT NULL DEFAULT 'none' CHECK(verification_level IN('none','account_age','member_age')),
 invites_paused_until INTEGER,
 dms_paused_until INTEGER,
 raid_detected_at INTEGER,
 raid_joins INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE automod_rules(
 id INTEGER PRIMARY KEY CHECK(id>0),
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 rule TEXT NOT NULL
);
CREATE INDEX automod_guild ON automod_rules(guild_id,id);

CREATE TABLE channel_age_restricted(
 channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE
);
CREATE TABLE age_confirmations(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 confirmed_at INTEGER NOT NULL,
 PRIMARY KEY(user_id,channel_id)
);

-- The slowmode list gains 15s, 2m, 10m, 30m and 2h.
CREATE TABLE channel_slowmode_v2(
 channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
 seconds INTEGER NOT NULL CHECK(seconds IN(0,5,10,15,30,60,120,300,600,900,1800,3600,7200,21600))
);
INSERT INTO channel_slowmode_v2(channel_id,seconds) SELECT channel_id,seconds FROM channel_slowmode;
DROP TABLE channel_slowmode;
ALTER TABLE channel_slowmode_v2 RENAME TO channel_slowmode;

-- Raised limits: 250 pins per channel are listed newest first; role and
-- overwrite lookups per server stay on indexes.
CREATE INDEX pins_channel_recent ON pins(channel_id,pinned_at,message_id);
CREATE INDEX member_roles_role ON member_roles(guild_id,role_id);
CREATE INDEX overwrite_target ON permission_overwrites(kind,target_id);
