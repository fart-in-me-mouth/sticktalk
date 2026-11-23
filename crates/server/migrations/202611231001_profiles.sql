-- Profiles and cosmetics (docs/features/profiles.md): name styles,
-- decorations, nameplates, banners, profile colors, effects, frames,
-- per-server profiles and badges.

-- The global look of a name and avatar (UserStyle JSON). Read with every
-- user summary, so it is a column of users rather than a join.
ALTER TABLE users ADD COLUMN style TEXT;

-- The global profile card (ProfileLook JSON) and the badges chosen for it.
CREATE TABLE profile_looks(
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 look TEXT NOT NULL DEFAULT '{}',
 shown_badges TEXT
);

-- One ServerProfile JSON per member who set one. Leaving the server drops it.
CREATE TABLE member_profiles(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 profile TEXT NOT NULL,
 PRIMARY KEY(guild_id,user_id)
);

-- Uploads in use as a banner or a server avatar. They decide who may download
-- the file and keep it from upload garbage collection. guild_id 0 is global.
CREATE TABLE profile_uploads(
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 guild_id INTEGER NOT NULL DEFAULT 0,
 slot TEXT NOT NULL CHECK(slot IN('avatar','banner')),
 upload_id INTEGER NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
 PRIMARY KEY(user_id,guild_id,slot)
);
CREATE INDEX profile_uploads_upload ON profile_uploads(upload_id);

-- Custom badges made by the instance administrator (admin CLI). The icon is a
-- small PNG made from the uploaded image.
CREATE TABLE badges(
 id INTEGER PRIMARY KEY,
 name TEXT NOT NULL,
 description TEXT NOT NULL,
 icon BLOB NOT NULL,
 created_at INTEGER NOT NULL
);
-- Awarded badges: a built-in ID (instance_admin, bug_reporter, contributor)
-- or 'c' followed by a custom badge ID.
CREATE TABLE badge_awards(
 badge TEXT NOT NULL,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 awarded_at INTEGER NOT NULL,
 PRIMARY KEY(badge,user_id)
);
CREATE INDEX badge_awards_user ON badge_awards(user_id);
CREATE TABLE profile_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TRIGGER member_profiles_leave AFTER DELETE ON guild_members BEGIN
 DELETE FROM member_profiles WHERE guild_id=OLD.guild_id AND user_id=OLD.user_id;
 DELETE FROM profile_uploads WHERE guild_id=OLD.guild_id AND user_id=OLD.user_id;
END;
CREATE TRIGGER badge_awards_badge_deleted AFTER DELETE ON badges BEGIN
 DELETE FROM badge_awards WHERE badge='c'||OLD.id;
END;
