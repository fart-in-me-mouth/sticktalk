-- Rich message content: attachment details, polls, forwarding, link previews
-- and scheduled messages. See docs/features/content.md.
ALTER TABLE uploads ADD COLUMN description TEXT CHECK(description IS NULL OR length(description) <= 4096);
ALTER TABLE uploads ADD COLUMN spoiler INTEGER NOT NULL DEFAULT 0 CHECK(spoiler IN (0,1));
ALTER TABLE uploads ADD COLUMN duration_ms INTEGER CHECK(duration_ms IS NULL OR duration_ms BETWEEN 1 AND 1200000);
ALTER TABLE uploads ADD COLUMN waveform BLOB CHECK(waveform IS NULL OR length(waveform) <= 256);

CREATE TABLE polls(
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    multiple INTEGER NOT NULL CHECK(multiple IN (0,1)),
    expires_at INTEGER NOT NULL,
    closed_at INTEGER
);
CREATE INDEX poll_open_deadline ON polls(expires_at) WHERE closed_at IS NULL;
CREATE TABLE poll_answers(
    message_id INTEGER NOT NULL REFERENCES polls(message_id) ON DELETE CASCADE,
    idx INTEGER NOT NULL CHECK(idx BETWEEN 0 AND 9),
    text TEXT NOT NULL,
    emoji TEXT,
    PRIMARY KEY(message_id, idx)
);
CREATE TABLE poll_votes(
    message_id INTEGER NOT NULL REFERENCES polls(message_id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL CHECK(idx BETWEEN 0 AND 9),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(message_id, user_id, idx)
);
CREATE INDEX poll_votes_answer ON poll_votes(message_id, idx, user_id);

CREATE TABLE forwarded_messages(
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    source_channel_id INTEGER NOT NULL,
    source_message_id INTEGER NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE forwarded_attachments(
    message_id INTEGER NOT NULL REFERENCES forwarded_messages(message_id) ON DELETE CASCADE,
    attachment_id INTEGER NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY(message_id, attachment_id)
);
CREATE INDEX forwarded_attachment_lookup ON forwarded_attachments(attachment_id);

CREATE TABLE link_embeds(
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL CHECK(idx BETWEEN 0 AND 4),
    url TEXT NOT NULL,
    site_name TEXT,
    title TEXT,
    description TEXT,
    video INTEGER NOT NULL DEFAULT 0,
    image BLOB CHECK(image IS NULL OR length(image) <= 262144),
    image_type TEXT,
    image_width INTEGER,
    image_height INTEGER,
    PRIMARY KEY(message_id, idx)
);
CREATE TABLE link_embeds_removed(
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
);

CREATE TABLE scheduled_messages(
    id INTEGER PRIMARY KEY CHECK(id > 0),
    author_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    mention_ids TEXT NOT NULL DEFAULT '[]',
    send_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    failure TEXT
);
CREATE INDEX scheduled_due ON scheduled_messages(send_at) WHERE failure IS NULL;
CREATE INDEX scheduled_author ON scheduled_messages(author_id, channel_id, send_at);
