-- Files attached to a scheduled message, held until it is sent or cancelled
-- (`content/scheduled.rs`). A held upload keeps counting against its owner's
-- quota; cancelling (or deleting the scheduled message) releases it to the
-- ordinary collection of unattached uploads.
CREATE TABLE scheduled_attachments(
    scheduled_id INTEGER NOT NULL REFERENCES scheduled_messages(id) ON DELETE CASCADE,
    attachment_id INTEGER NOT NULL UNIQUE REFERENCES uploads(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY(scheduled_id, attachment_id)
);
