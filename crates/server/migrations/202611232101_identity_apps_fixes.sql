-- Server tag changes reach every wearer: the wearers to tell, told in bounded
-- batches after the change commits (`identity::fanout`). The admin CLI queues
-- here too, so a revoked tag reaches open apps while the server runs.
CREATE TABLE tag_announce(
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE
);

-- A bot removed from a server by any path (kick, ban, prune) takes its app's
-- managed role with it: the trigger records it, `apps::settle` deletes it in
-- the same command (with its overwrites, the roles event and an audit entry).
CREATE TABLE app_role_drops(
 guild_id INTEGER NOT NULL,
 app_id INTEGER NOT NULL,
 role_id INTEGER NOT NULL,
 name TEXT NOT NULL
);
DROP TRIGGER apps_bot_left;
CREATE TRIGGER apps_bot_left AFTER DELETE ON guild_members
 WHEN EXISTS(SELECT 1 FROM apps WHERE bot_id=OLD.user_id) BEGIN
 INSERT INTO app_role_drops(guild_id,app_id,role_id,name)
  SELECT i.guild_id,i.app_id,i.role_id,a.name FROM app_installs i JOIN apps a ON a.id=i.app_id
  WHERE i.guild_id=OLD.guild_id AND a.bot_id=OLD.user_id AND i.role_id IS NOT NULL;
 DELETE FROM app_command_permissions WHERE guild_id=OLD.guild_id AND app_id=(SELECT id FROM apps WHERE bot_id=OLD.user_id);
 DELETE FROM app_commands WHERE guild_id=OLD.guild_id AND app_id=(SELECT id FROM apps WHERE bot_id=OLD.user_id);
 DELETE FROM app_installs WHERE guild_id=OLD.guild_id AND app_id=(SELECT id FROM apps WHERE bot_id=OLD.user_id);
END;
