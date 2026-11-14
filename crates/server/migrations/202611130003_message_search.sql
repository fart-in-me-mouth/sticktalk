-- External content avoids a duplicate stored copy of every message body.
-- Soft-deleted messages must never be indexed, including during backfill/rebuild.
CREATE VIEW message_search_content AS SELECT id,content FROM messages WHERE deleted_at IS NULL;
CREATE VIRTUAL TABLE message_fts USING fts5(content,content='message_search_content',content_rowid='id',tokenize='unicode61 remove_diacritics 2',prefix='2 3');
CREATE TRIGGER message_search_insert AFTER INSERT ON messages WHEN new.deleted_at IS NULL BEGIN
 INSERT INTO message_fts(rowid,content) VALUES(new.id,new.content);
END;
CREATE TRIGGER message_search_delete AFTER DELETE ON messages WHEN old.deleted_at IS NULL BEGIN
 INSERT INTO message_fts(message_fts,rowid,content) VALUES('delete',old.id,old.content);
END;
CREATE TRIGGER message_search_update AFTER UPDATE OF content,deleted_at ON messages BEGIN
 INSERT INTO message_fts(message_fts,rowid,content) SELECT 'delete',old.id,old.content WHERE old.deleted_at IS NULL;
 INSERT INTO message_fts(rowid,content) SELECT new.id,new.content WHERE new.deleted_at IS NULL;
END;
INSERT INTO message_fts(message_fts) VALUES('rebuild');
