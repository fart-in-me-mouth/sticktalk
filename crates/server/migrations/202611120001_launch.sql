-- Numeric permission bits 0..13 and legacy member role strings are preserved.
-- @everyone always applies implicitly. Legacy admins get a real Administrator role.
CREATE TABLE roles (
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 id INTEGER NOT NULL CHECK(id>0), name TEXT NOT NULL, color INTEGER NOT NULL DEFAULT 0,
 position INTEGER NOT NULL, hoist INTEGER NOT NULL DEFAULT 0, mentionable INTEGER NOT NULL DEFAULT 0,
 permissions INTEGER NOT NULL, PRIMARY KEY(guild_id,id)
);
CREATE TABLE member_roles (
 guild_id INTEGER NOT NULL, user_id INTEGER NOT NULL, role_id INTEGER NOT NULL,
 PRIMARY KEY(guild_id,user_id,role_id),
 FOREIGN KEY(guild_id,user_id) REFERENCES guild_members(guild_id,user_id) ON DELETE CASCADE,
 FOREIGN KEY(guild_id,role_id) REFERENCES roles(guild_id,id) ON DELETE CASCADE
);
-- New default rights: nickname, embeds and ordinary voice. Existing text rights unchanged.
INSERT INTO roles(guild_id,id,name,position,permissions)
 SELECT id,id,'@everyone',0,75137311 FROM guilds;
INSERT INTO roles(guild_id,id,name,position,permissions)
 SELECT id,id+1,'Admin',1,4096 FROM guilds;
INSERT INTO member_roles SELECT guild_id,user_id,guild_id+1 FROM guild_members WHERE role='admin';
CREATE TRIGGER launch_guild AFTER INSERT ON guilds BEGIN
 INSERT INTO roles(guild_id,id,name,position,permissions) VALUES(NEW.id,NEW.id,'@everyone',0,75137311),(NEW.id,NEW.id+1,'Admin',1,4096);
END;
CREATE TRIGGER launch_admin_insert AFTER INSERT ON guild_members WHEN NEW.role='admin' BEGIN
 INSERT OR IGNORE INTO member_roles VALUES(NEW.guild_id,NEW.user_id,NEW.guild_id+1);
END;
CREATE TRIGGER launch_admin_update AFTER UPDATE OF role ON guild_members BEGIN
 DELETE FROM member_roles WHERE guild_id=NEW.guild_id AND user_id=NEW.user_id AND role_id=NEW.guild_id+1 AND OLD.role='admin';
 INSERT OR IGNORE INTO member_roles SELECT NEW.guild_id,NEW.user_id,NEW.guild_id+1 WHERE NEW.role='admin';
END;
CREATE TABLE permission_overwrites (
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('role','member')), target_id INTEGER NOT NULL,
 allow INTEGER NOT NULL, deny INTEGER NOT NULL, PRIMARY KEY(channel_id,kind,target_id)
);
CREATE TABLE guild_profiles (
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 description TEXT NOT NULL DEFAULT '', banner_color INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE user_profiles (
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 banner_color INTEGER NOT NULL DEFAULT 0, pronouns TEXT NOT NULL DEFAULT '', about_me TEXT NOT NULL DEFAULT ''
);
CREATE TABLE nicknames (
 guild_id INTEGER NOT NULL, user_id INTEGER NOT NULL, nickname TEXT NOT NULL,
 PRIMARY KEY(guild_id,user_id), FOREIGN KEY(guild_id,user_id) REFERENCES guild_members(guild_id,user_id) ON DELETE CASCADE
);
CREATE TABLE pins (
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 pinned_by INTEGER NOT NULL REFERENCES users(id), pinned_at INTEGER NOT NULL,
 PRIMARY KEY(channel_id,message_id)
);
CREATE TABLE notification_settings (
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 scope TEXT NOT NULL, target_id INTEGER NOT NULL, settings TEXT NOT NULL,
 PRIMARY KEY(user_id,scope,target_id)
);
CREATE TABLE session_devices (
 token_hash BLOB PRIMARY KEY REFERENCES sessions(token_hash) ON DELETE CASCADE ON UPDATE CASCADE,
 id TEXT NOT NULL UNIQUE, device_name TEXT NOT NULL DEFAULT 'This device', platform TEXT NOT NULL DEFAULT 'Unknown'
);
INSERT INTO session_devices(token_hash,id) SELECT token_hash,lower(hex(randomblob(16))) FROM sessions;
CREATE TRIGGER launch_session AFTER INSERT ON sessions BEGIN
 INSERT INTO session_devices(token_hash,id) VALUES(NEW.token_hash,lower(hex(randomblob(16))));
END;
