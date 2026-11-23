-- The record a finished direct or group call leaves on its system message.
CREATE TABLE call_records(message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,started_at INTEGER NOT NULL,ended_at INTEGER NOT NULL CHECK(ended_at>=started_at),missed INTEGER NOT NULL CHECK(missed IN(0,1)),participant_count INTEGER NOT NULL CHECK(participant_count BETWEEN 0 AND 65535));
-- Voice channels now keep their own text chat: give existing members a read
-- position there, the same way new members get one for text channels.
INSERT OR IGNORE INTO read_states(user_id,channel_id,last_read_id) SELECT gm.user_id,ch.id,NULL FROM channels ch JOIN guild_members gm ON gm.guild_id=ch.guild_id WHERE ch.kind='voice';
