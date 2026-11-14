CREATE TABLE server_folders (
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 id INTEGER NOT NULL CHECK(id>0 AND id<=4294967295),
 name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 32),
 color INTEGER NOT NULL CHECK(color BETWEEN 0 AND 16777215),
 collapsed INTEGER NOT NULL CHECK(collapsed IN(0,1)),
 position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 9),
 PRIMARY KEY(user_id,id), UNIQUE(user_id,position)
);
CREATE TABLE server_folder_guilds (
 user_id INTEGER NOT NULL, folder_id INTEGER NOT NULL, guild_id INTEGER NOT NULL,
 position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 9),
 PRIMARY KEY(user_id,guild_id), UNIQUE(user_id,folder_id,position),
 FOREIGN KEY(user_id,folder_id) REFERENCES server_folders(user_id,id) ON DELETE CASCADE,
 FOREIGN KEY(guild_id,user_id) REFERENCES guild_members(guild_id,user_id) ON DELETE CASCADE
);
