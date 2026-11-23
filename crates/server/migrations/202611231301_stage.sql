-- Stage channels and scheduled events (docs/features/stage.md).
--
-- New channel kind 'stage'. SQLite cannot change a CHECK, so the table is
-- rebuilt exactly as 202611230701_threads.sql did: the same columns in the
-- same order (foreign keys are off on the migration connection), then its
-- index and the trigger that migration put on it.
CREATE TABLE channels_stage(id INTEGER PRIMARY KEY CHECK(id>0),kind TEXT NOT NULL CHECK(kind IN ('category','text','voice','direct','group_dm','announcement','forum','thread','stage')),guild_id INTEGER REFERENCES guilds(id) ON DELETE CASCADE,parent_id INTEGER REFERENCES channels(id) ON DELETE SET NULL,name TEXT,topic TEXT,position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 65535),owner_id INTEGER REFERENCES users(id),created_at INTEGER NOT NULL,CHECK((kind IN ('category','text','voice','announcement','forum','thread','stage') AND guild_id IS NOT NULL AND name IS NOT NULL AND owner_id IS NULL) OR (kind IN ('direct','group_dm') AND guild_id IS NULL AND parent_id IS NULL AND topic IS NULL)),CHECK(kind<>'category' OR topic IS NULL));
INSERT INTO channels_stage(id,kind,guild_id,parent_id,name,topic,position,owner_id,created_at) SELECT id,kind,guild_id,parent_id,name,topic,position,owner_id,created_at FROM channels;
DROP TABLE channels;
ALTER TABLE channels_stage RENAME TO channels;
CREATE INDEX channel_guild ON channels(guild_id,parent_id,position,id);
CREATE TRIGGER thread_parent_deleted BEFORE DELETE ON channels WHEN OLD.kind<>'thread' BEGIN
    DELETE FROM channels WHERE kind='thread' AND parent_id=OLD.id;
END;

-- What a Stage or an event adds to a message: a finished Stage's record, or
-- a shared event's card. `payload` is the MessagePack StageSummary/EventCard.
CREATE TABLE stage_messages(
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('record','event')),
    payload BLOB NOT NULL
);

-- One row per event series. Occurrences come from the rule: `start_local`
-- (a civil date and time) in `time_zone`, stepped by `recurrence`. Each
-- occurrence is identified by its UTC start as the rule computes it.
CREATE TABLE scheduled_events(
    id INTEGER PRIMARY KEY CHECK(id>0),
    guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    creator_id INTEGER NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    location_kind TEXT NOT NULL CHECK(location_kind IN ('stage','voice','external')),
    channel_id INTEGER REFERENCES channels(id) ON DELETE CASCADE,
    place TEXT,
    start_local TEXT NOT NULL,
    time_zone TEXT NOT NULL,
    first_at INTEGER NOT NULL,
    duration_ms INTEGER CHECK(duration_ms IS NULL OR duration_ms>0),
    recurrence TEXT NOT NULL CHECK(recurrence IN ('none','daily','weekly','every_two_weeks','monthly','yearly','weekdays')),
    -- Occurrences at or after this belong to a later series ("this and following").
    until_at INTEGER,
    status TEXT NOT NULL CHECK(status IN ('scheduled','live','ended','cancelled')),
    live_occurrence INTEGER,
    live_since INTEGER,
    -- The latest occurrence whose start was announced to interested members.
    notified_through INTEGER NOT NULL DEFAULT 0,
    -- The latest occurrence that went live and ended.
    done_through INTEGER NOT NULL DEFAULT 0,
    -- The next time the event timer has something to do for it; NULL: nothing.
    next_at INTEGER,
    ended_at INTEGER,
    cover_version INTEGER NOT NULL DEFAULT 0,
    interested_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    CHECK((location_kind='external') = (channel_id IS NULL)),
    CHECK((location_kind='external') = (place IS NOT NULL)),
    CHECK((status='live') = (live_occurrence IS NOT NULL))
);
CREATE INDEX scheduled_event_guild ON scheduled_events(guild_id,status);
CREATE INDEX scheduled_event_due ON scheduled_events(next_at) WHERE next_at IS NOT NULL;
CREATE INDEX scheduled_event_channel ON scheduled_events(channel_id) WHERE channel_id IS NOT NULL;

-- "Edit this event": one occurrence's changes, or its cancellation.
CREATE TABLE event_exceptions(
    event_id INTEGER NOT NULL REFERENCES scheduled_events(id) ON DELETE CASCADE,
    occurrence INTEGER NOT NULL,
    cancelled INTEGER NOT NULL DEFAULT 0 CHECK(cancelled IN (0,1)),
    name TEXT,
    description TEXT,
    start_at INTEGER,
    end_at INTEGER,
    end_cleared INTEGER NOT NULL DEFAULT 0 CHECK(end_cleared IN (0,1)),
    location_kind TEXT CHECK(location_kind IS NULL OR location_kind IN ('stage','voice','external')),
    channel_id INTEGER REFERENCES channels(id) ON DELETE CASCADE,
    place TEXT,
    PRIMARY KEY(event_id,occurrence)
);

CREATE TABLE event_interest(
    event_id INTEGER NOT NULL REFERENCES scheduled_events(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(event_id,user_id)
);
CREATE INDEX event_interest_user ON event_interest(user_id,event_id);
CREATE TRIGGER event_interest_added AFTER INSERT ON event_interest BEGIN
    UPDATE scheduled_events SET interested_count=interested_count+1 WHERE id=NEW.event_id;
END;
CREATE TRIGGER event_interest_removed AFTER DELETE ON event_interest BEGIN
    UPDATE scheduled_events SET interested_count=MAX(interested_count-1,0) WHERE id=OLD.event_id;
END;
-- Leaving a server ends interest in its events.
CREATE TRIGGER event_interest_left AFTER DELETE ON guild_members BEGIN
    DELETE FROM event_interest WHERE user_id=OLD.user_id AND event_id IN (SELECT id FROM scheduled_events WHERE guild_id=OLD.guild_id);
END;

CREATE TABLE event_covers(
    event_id INTEGER PRIMARY KEY REFERENCES scheduled_events(id) ON DELETE CASCADE,
    content_type TEXT NOT NULL,
    bytes BLOB NOT NULL
);

-- An invite that leads to an event: its preview shows the event, and joining
-- through it marks the new member Interested.
CREATE TABLE invite_events(
    invite_id INTEGER PRIMARY KEY REFERENCES invites(id) ON DELETE CASCADE,
    event_id INTEGER NOT NULL REFERENCES scheduled_events(id) ON DELETE CASCADE
);
CREATE INDEX invite_event ON invite_events(event_id);

-- Request to Speak (bit 54 = 18014398509481984) is on for @everyone, as in
-- Discord. New servers get it through their own trigger.
UPDATE roles SET permissions=permissions|18014398509481984 WHERE id=guild_id;
CREATE TRIGGER stage_rights_default AFTER INSERT ON roles WHEN NEW.id=NEW.guild_id BEGIN
 UPDATE roles SET permissions=permissions|18014398509481984 WHERE guild_id=NEW.guild_id AND id=NEW.id;
END;
