-- Both shipped migration files retain their original version/checksum. This
-- bridge runs after either history, including launch databases missing v9.
INSERT INTO permission_overwrites(channel_id,kind,target_id,allow,deny)
 SELECT channel_id,'member',user_id,0,524288 FROM voice_connect_denials WHERE true
 ON CONFLICT(channel_id,kind,target_id) DO UPDATE
 SET allow=allow & ~524288, deny=deny | 524288;
DELETE FROM voice_connect_denials;

-- Expanded broadcast recipients remain ordinary mention metadata. Keep origin
-- separately to implement suppress_everyone without suppressing explicit pings.
CREATE TABLE message_broadcast_mentions (
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('everyone','here','role')),
 PRIMARY KEY(message_id,user_id,kind)
);

-- Server mute/deafen belong to the membership, not a replaceable WS/call.
CREATE TABLE guild_voice_moderation (
 guild_id INTEGER NOT NULL, user_id INTEGER NOT NULL,
 muted INTEGER NOT NULL DEFAULT 0 CHECK(muted IN(0,1)),
 deafened INTEGER NOT NULL DEFAULT 0 CHECK(deafened IN(0,1)),
 PRIMARY KEY(guild_id,user_id),
 FOREIGN KEY(guild_id,user_id) REFERENCES guild_members(guild_id,user_id) ON DELETE CASCADE
);
