-- Community: rules screening, applications, onboarding, channel opt-in, the
-- server directory, server templates and insights. See docs/features/community.md.

-- One row per server that configured admission or onboarding. Both are JSON
-- documents validated by the server on every write. `configured_by` is who
-- last saved onboarding: its roles are granted only below that person's top role.
CREATE TABLE community_settings(
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 admission TEXT,
 onboarding TEXT,
 configured_by INTEGER,
 updated_at INTEGER NOT NULL
);

-- A member's own community state. A member with `rules_pending` or
-- `questions_pending` keeps only View Channel and Read History, like a timeout.
CREATE TABLE community_members(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 rules_pending INTEGER NOT NULL DEFAULT 0 CHECK(rules_pending IN(0,1)),
 questions_pending INTEGER NOT NULL DEFAULT 0 CHECK(questions_pending IN(0,1)),
 rules_version INTEGER NOT NULL DEFAULT 0,
 responses TEXT NOT NULL DEFAULT '[]',
 opted_in TEXT NOT NULL DEFAULT '[]',
 show_all INTEGER NOT NULL DEFAULT 0 CHECK(show_all IN(0,1)),
 guide_done TEXT NOT NULL DEFAULT '[]',
 steps TEXT NOT NULL DEFAULT '[]',
 onboarded INTEGER NOT NULL DEFAULT 0 CHECK(onboarded IN(0,1)),
 PRIMARY KEY(guild_id,user_id)
) WITHOUT ROWID;

-- One application per person and server. Decided ones are kept for 30 days.
CREATE TABLE community_applications(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 status TEXT NOT NULL CHECK(status IN('pending','approved','rejected','withdrawn')),
 questions TEXT NOT NULL,
 answers TEXT NOT NULL,
 rules_accepted INTEGER NOT NULL DEFAULT 0,
 source TEXT NOT NULL,
 invite_code TEXT,
 reason TEXT,
 reviewer_id INTEGER,
 interview_channel_id INTEGER REFERENCES channels(id) ON DELETE SET NULL,
 created_at INTEGER NOT NULL,
 decided_at INTEGER,
 PRIMARY KEY(guild_id,user_id)
);
CREATE INDEX community_applications_queue ON community_applications(guild_id,status,created_at);
CREATE INDEX community_applications_user ON community_applications(user_id);

-- The directory of this instance. `featured`, `hidden` and `delisted` are set
-- by the instance administrator (`sticktalk-server admin directory ...`).
CREATE TABLE directory_listings(
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 category TEXT NOT NULL,
 description TEXT NOT NULL,
 tags TEXT NOT NULL,
 language TEXT NOT NULL,
 featured_emoji TEXT NOT NULL,
 listed INTEGER NOT NULL DEFAULT 1 CHECK(listed IN(0,1)),
 featured INTEGER NOT NULL DEFAULT 0 CHECK(featured IN(0,1)),
 hidden INTEGER NOT NULL DEFAULT 0 CHECK(hidden IN(0,1)),
 delisted INTEGER NOT NULL DEFAULT 0 CHECK(delisted IN(0,1)),
 listed_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE TABLE directory_settings(
 id INTEGER PRIMARY KEY CHECK(id=1),
 enabled INTEGER NOT NULL CHECK(enabled IN(0,1))
);

-- Server templates: structure only, as a validated JSON snapshot.
CREATE TABLE guild_templates(
 code TEXT PRIMARY KEY,
 source_guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 creator_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 name TEXT NOT NULL,
 description TEXT NOT NULL,
 snapshot TEXT NOT NULL,
 uses INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE INDEX guild_templates_source ON guild_templates(source_guild_id);

-- Insights: small daily rollups updated as events happen, kept 120 days.
CREATE TABLE insight_daily(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 day INTEGER NOT NULL,
 joins INTEGER NOT NULL DEFAULT 0,
 leaves INTEGER NOT NULL DEFAULT 0,
 messages INTEGER NOT NULL DEFAULT 0,
 voice_minutes INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(guild_id,day)
) WITHOUT ROWID;
CREATE TABLE insight_channels(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 day INTEGER NOT NULL,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 messages INTEGER NOT NULL DEFAULT 0,
 voice_minutes INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(guild_id,day,channel_id)
) WITHOUT ROWID;
CREATE INDEX insight_channels_channel ON insight_channels(channel_id);
CREATE TABLE insight_sources(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 day INTEGER NOT NULL,
 source TEXT NOT NULL,
 code TEXT NOT NULL DEFAULT '',
 joins INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(guild_id,day,source,code)
) WITHOUT ROWID;
CREATE TABLE insight_onboarding(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 day INTEGER NOT NULL,
 step TEXT NOT NULL,
 reached INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(guild_id,day,step)
) WITHOUT ROWID;
-- One row per join, for new-member retention (aggregated, never listed).
CREATE TABLE insight_joins(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL,
 joined_at INTEGER NOT NULL,
 first_message_at INTEGER,
 left_at INTEGER,
 PRIMARY KEY(guild_id,user_id,joined_at)
) WITHOUT ROWID;
CREATE INDEX insight_joins_time ON insight_joins(guild_id,joined_at);
