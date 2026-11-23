-- Apps, bot users, application commands and incoming webhooks
-- (docs/features/apps.md). Bot and webhook accounts are ordinary `users`
-- rows with an empty password and a usernames no person can register; their
-- credentials are `sessions` rows listed in `app_sessions`, which the normal
-- session check refuses.

CREATE TABLE apps(
 id INTEGER PRIMARY KEY CHECK(id>0),
 owner_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 bot_id INTEGER NOT NULL UNIQUE REFERENCES users(id),
 name TEXT NOT NULL,
 description TEXT NOT NULL DEFAULT '',
 public INTEGER NOT NULL DEFAULT 0,
 intents INTEGER NOT NULL DEFAULT 0,
 permissions INTEGER NOT NULL DEFAULT 0,
 disabled INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL
);
CREATE INDEX apps_owner ON apps(owner_id,id);

-- A bot token or a webhook's posting credential. Never accepted as a person's session.
CREATE TABLE app_sessions(
 session_hash BLOB PRIMARY KEY REFERENCES sessions(token_hash) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN('bot','webhook')),
 owner_id INTEGER NOT NULL CHECK(owner_id>0)
);

CREATE TABLE app_installs(
 app_id INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 role_id INTEGER,
 permissions INTEGER NOT NULL,
 message_content INTEGER NOT NULL DEFAULT 0,
 installed_by INTEGER NOT NULL,
 installed_at INTEGER NOT NULL,
 PRIMARY KEY(app_id,guild_id)
);
CREATE INDEX app_installs_guild ON app_installs(guild_id,app_id);
CREATE UNIQUE INDEX app_installs_role ON app_installs(guild_id,role_id) WHERE role_id IS NOT NULL;

CREATE TABLE app_commands(
 id INTEGER PRIMARY KEY CHECK(id>0),
 app_id INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
 guild_id INTEGER REFERENCES guilds(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN('chat_input','user','message')),
 name TEXT NOT NULL,
 data TEXT NOT NULL,
 position INTEGER NOT NULL
);
CREATE UNIQUE INDEX app_commands_name ON app_commands(app_id,COALESCE(guild_id,0),kind,name);
CREATE INDEX app_commands_guild ON app_commands(guild_id,app_id);

-- Per-server overrides for one command (target_id = command) or all of an
-- app's commands (target_id = app).
CREATE TABLE app_command_permissions(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 app_id INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
 target_id INTEGER NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN('role','channel')),
 id INTEGER NOT NULL,
 allow INTEGER NOT NULL,
 PRIMARY KEY(guild_id,target_id,kind,id)
);
CREATE INDEX app_command_permissions_app ON app_command_permissions(guild_id,app_id);

CREATE TABLE webhooks(
 id INTEGER PRIMARY KEY CHECK(id>0),
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL UNIQUE REFERENCES users(id),
 name TEXT NOT NULL,
 token_hash BLOB NOT NULL UNIQUE,
 session_hash BLOB NOT NULL UNIQUE,
 creator_id INTEGER NOT NULL,
 created_at INTEGER NOT NULL,
 last_used_at INTEGER
);
CREATE INDEX webhooks_channel ON webhooks(channel_id,id);
CREATE INDEX webhooks_guild ON webhooks(guild_id,id);
-- A webhook removed with its channel or server takes its credential with it.
CREATE TRIGGER webhooks_removed AFTER DELETE ON webhooks BEGIN
 DELETE FROM sessions WHERE token_hash=OLD.session_hash;
END;

-- What an app or webhook added to a message, and the images the server fetched for it.
CREATE TABLE app_messages(
 message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
 app_id INTEGER,
 webhook_id INTEGER,
 data TEXT NOT NULL
);
CREATE TABLE app_images(
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 slot INTEGER NOT NULL,
 content_type TEXT NOT NULL,
 width INTEGER NOT NULL,
 height INTEGER NOT NULL,
 bytes BLOB NOT NULL,
 PRIMARY KEY(message_id,slot)
);
CREATE TRIGGER app_message_deleted AFTER UPDATE OF deleted_at ON messages
 WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL BEGIN
 DELETE FROM app_images WHERE message_id=NEW.id;
 DELETE FROM app_messages WHERE message_id=NEW.id;
END;

-- The instance administrator's policy (`sticktalk-server admin apps ...`).
CREATE TABLE app_settings(
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 creation TEXT NOT NULL DEFAULT 'everyone' CHECK(creation IN('everyone','allowlist','disabled'))
);
INSERT INTO app_settings(singleton) VALUES(1);
CREATE TABLE app_creators(user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE);

-- Use Application Commands (bit 47 = 140737488355328) is on for @everyone,
-- as in Discord; new servers get it through their own trigger.
UPDATE roles SET permissions=permissions|140737488355328 WHERE id=guild_id;
CREATE TRIGGER apps_rights_default AFTER INSERT ON roles WHEN NEW.id=NEW.guild_id BEGIN
 UPDATE roles SET permissions=permissions|140737488355328 WHERE guild_id=NEW.guild_id AND id=NEW.id;
END;

-- A bot removed from a server by any path (kick, ban, prune) takes its app's
-- installation, commands and overrides there with it; its role stays as an
-- ordinary role.
CREATE TRIGGER apps_bot_left AFTER DELETE ON guild_members
 WHEN EXISTS(SELECT 1 FROM apps WHERE bot_id=OLD.user_id) BEGIN
 DELETE FROM app_command_permissions WHERE guild_id=OLD.guild_id AND app_id=(SELECT id FROM apps WHERE bot_id=OLD.user_id);
 DELETE FROM app_commands WHERE guild_id=OLD.guild_id AND app_id=(SELECT id FROM apps WHERE bot_id=OLD.user_id);
 DELETE FROM app_installs WHERE guild_id=OLD.guild_id AND app_id=(SELECT id FROM apps WHERE bot_id=OLD.user_id);
END;
