-- Threads, forum channels and announcement channels (docs/features/threads.md).
-- New channel kinds: 'announcement', 'forum' and 'thread'. A thread (and a
-- forum post) is a channel row whose parent is the channel it belongs to.
-- SQLite cannot change a CHECK, so the table is rebuilt with the same columns
-- in the same order (foreign keys are off on the migration connection).
CREATE TABLE channels_threads(id INTEGER PRIMARY KEY CHECK(id>0),kind TEXT NOT NULL CHECK(kind IN ('category','text','voice','direct','group_dm','announcement','forum','thread')),guild_id INTEGER REFERENCES guilds(id) ON DELETE CASCADE,parent_id INTEGER REFERENCES channels(id) ON DELETE SET NULL,name TEXT,topic TEXT,position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 65535),owner_id INTEGER REFERENCES users(id),created_at INTEGER NOT NULL,CHECK((kind IN ('category','text','voice','announcement','forum','thread') AND guild_id IS NOT NULL AND name IS NOT NULL AND owner_id IS NULL) OR (kind IN ('direct','group_dm') AND guild_id IS NULL AND parent_id IS NULL AND topic IS NULL)),CHECK(kind<>'category' OR topic IS NULL));
INSERT INTO channels_threads(id,kind,guild_id,parent_id,name,topic,position,owner_id,created_at) SELECT id,kind,guild_id,parent_id,name,topic,position,owner_id,created_at FROM channels;
DROP TABLE channels;
ALTER TABLE channels_threads RENAME TO channels;
CREATE INDEX channel_guild ON channels(guild_id,parent_id,position,id);

-- Thread details. `parent_id` repeats the channel row's parent so a deleted
-- parent takes its threads with it (with the trigger below).
CREATE TABLE threads(
    channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    parent_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    owner_id INTEGER NOT NULL REFERENCES users(id),
    starter_message_id INTEGER UNIQUE REFERENCES messages(id) ON DELETE SET NULL,
    first_message_id INTEGER,
    forum_post INTEGER NOT NULL DEFAULT 0 CHECK(forum_post IN (0,1)),
    private INTEGER NOT NULL DEFAULT 0 CHECK(private IN (0,1)),
    invitable INTEGER NOT NULL DEFAULT 1 CHECK(invitable IN (0,1)),
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)),
    locked INTEGER NOT NULL DEFAULT 0 CHECK(locked IN (0,1)),
    pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1)),
    auto_archive_minutes INTEGER NOT NULL CHECK(auto_archive_minutes IN (60,1440,4320,10080)),
    created_at INTEGER NOT NULL,
    last_activity_at INTEGER NOT NULL,
    archived_at INTEGER,
    -- When the archive timer fires for it; NULL while archived.
    archive_at INTEGER,
    -- Kept by the triggers below.
    message_count INTEGER NOT NULL DEFAULT 0,
    member_count INTEGER NOT NULL DEFAULT 0,
    last_message_id INTEGER,
    CHECK((archived=1) = (archive_at IS NULL))
);
CREATE INDEX thread_parent ON threads(parent_id,archived,last_activity_at);
CREATE INDEX thread_guild_active ON threads(guild_id) WHERE archived=0;
CREATE INDEX thread_archive_due ON threads(archive_at) WHERE archive_at IS NOT NULL;

CREATE TABLE thread_members(
    thread_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    joined_at INTEGER NOT NULL,
    notify TEXT NOT NULL DEFAULT 'default' CHECK(notify IN ('default','all','mentions','nothing')),
    PRIMARY KEY(thread_id,user_id)
);
CREATE INDEX thread_member_user ON thread_members(user_id,thread_id);

-- A deleted channel takes its threads with it (a guild's cascade does already).
CREATE TRIGGER thread_parent_deleted BEFORE DELETE ON channels WHEN OLD.kind<>'thread' BEGIN
    DELETE FROM channels WHERE kind='thread' AND parent_id=OLD.id;
END;
-- Message and member counts, kept by the database itself.
CREATE TRIGGER thread_message_added AFTER INSERT ON messages BEGIN
    UPDATE threads SET message_count=message_count+1,last_message_id=NEW.id WHERE channel_id=NEW.channel_id;
END;
CREATE TRIGGER thread_message_removed AFTER UPDATE OF deleted_at ON messages WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL BEGIN
    UPDATE threads SET message_count=MAX(message_count-1,0) WHERE channel_id=NEW.channel_id;
END;
CREATE TRIGGER thread_member_added AFTER INSERT ON thread_members BEGIN
    UPDATE threads SET member_count=member_count+1 WHERE channel_id=NEW.thread_id;
END;
CREATE TRIGGER thread_member_removed AFTER DELETE ON thread_members BEGIN
    UPDATE threads SET member_count=MAX(member_count-1,0) WHERE channel_id=OLD.thread_id;
END;
-- Leaving or being removed from a server ends its thread memberships.
CREATE TRIGGER thread_member_left AFTER DELETE ON guild_members BEGIN
    DELETE FROM thread_members WHERE user_id=OLD.user_id AND thread_id IN (SELECT channel_id FROM threads WHERE guild_id=OLD.guild_id);
END;

CREATE TABLE forum_settings(
    channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    guidelines TEXT NOT NULL DEFAULT '',
    require_tags INTEGER NOT NULL DEFAULT 0 CHECK(require_tags IN (0,1)),
    default_reaction TEXT,
    sort TEXT NOT NULL DEFAULT 'activity' CHECK(sort IN ('activity','created')),
    layout TEXT NOT NULL DEFAULT 'list' CHECK(layout IN ('list','gallery')),
    post_slowmode INTEGER NOT NULL DEFAULT 0 CHECK(post_slowmode BETWEEN 0 AND 21600),
    reply_slowmode INTEGER NOT NULL DEFAULT 0 CHECK(reply_slowmode BETWEEN 0 AND 21600),
    default_auto_archive_minutes INTEGER NOT NULL DEFAULT 4320 CHECK(default_auto_archive_minutes IN (60,1440,4320,10080)),
    next_tag_id INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE forum_tags(
    forum_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    id INTEGER NOT NULL CHECK(id>0),
    name TEXT NOT NULL,
    emoji TEXT,
    moderated INTEGER NOT NULL DEFAULT 0 CHECK(moderated IN (0,1)),
    position INTEGER NOT NULL,
    PRIMARY KEY(forum_id,id)
);
CREATE TABLE thread_tags(
    thread_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY(thread_id,tag_id)
);

-- Announcements: what was published, who follows, and the copies delivered.
CREATE TABLE published_messages(
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    published_by INTEGER NOT NULL REFERENCES users(id),
    published_at INTEGER NOT NULL
);
CREATE INDEX published_channel_time ON published_messages(channel_id,published_at);
CREATE TABLE channel_follows(
    id INTEGER PRIMARY KEY CHECK(id>0),
    source_channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    target_channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    created_by INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    UNIQUE(source_channel_id,target_channel_id)
);
CREATE INDEX follow_target ON channel_follows(target_channel_id);
-- The source is not a foreign key: a copy outlives its source server.
CREATE TABLE crossposts(
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    source_message_id INTEGER NOT NULL,
    source_channel_id INTEGER NOT NULL,
    source_guild_id INTEGER NOT NULL,
    source_guild_name TEXT NOT NULL,
    source_channel_name TEXT NOT NULL,
    follow_id INTEGER
);
CREATE INDEX crosspost_source ON crossposts(source_message_id);
-- Deliveries waiting for the worker. One row per (message, following): later
-- edits fold into a waiting row, so the queue cannot grow with edits.
CREATE TABLE crosspost_queue(
    source_message_id INTEGER NOT NULL,
    follow_id INTEGER NOT NULL REFERENCES channel_follows(id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK(action IN ('create','update','delete')),
    queued_at INTEGER NOT NULL,
    PRIMARY KEY(source_message_id,follow_id)
);

-- New default rights, as in Discord: everyone may start threads and post in
-- them (Create Public Threads, Create Private Threads, Send Messages in
-- Threads: bits 32, 33 and 34 = 30064771072). A channel that denied Send
-- Messages also denies them, so read-only channels stay read-only. New
-- servers get them through their own trigger, leaving `launch_guild` as is.
UPDATE roles SET permissions=permissions|30064771072 WHERE id=guild_id;
UPDATE permission_overwrites SET deny=deny|30064771072 WHERE (deny & 2)<>0;
CREATE TRIGGER thread_rights_default AFTER INSERT ON roles WHEN NEW.id=NEW.guild_id BEGIN
 UPDATE roles SET permissions=permissions|30064771072 WHERE guild_id=NEW.guild_id AND id=NEW.id;
END;
