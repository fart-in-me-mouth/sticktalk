-- Themes (docs/features/themes.md): custom themes saved on the account, a
-- theme per server, whether each person sees server themes, and the Send TTS
-- Messages permission.

CREATE TABLE user_themes (
 id INTEGER PRIMARY KEY,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 name TEXT NOT NULL,
 spec TEXT NOT NULL CHECK(length(spec) <= 1024),
 created_at INTEGER NOT NULL
);
CREATE INDEX user_themes_user ON user_themes(user_id);

CREATE TABLE guild_themes (
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 theme TEXT NOT NULL CHECK(length(theme) <= 1024),
 updated_by INTEGER,
 updated_at INTEGER NOT NULL
);

CREATE TABLE theme_settings (
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 settings TEXT NOT NULL CHECK(length(settings) <= 16384)
);

-- Send TTS Messages (bit 56) is on for @everyone by default, in existing
-- servers and new ones.
UPDATE roles SET permissions = permissions | 72057594037927936 WHERE id = guild_id;
CREATE TRIGGER themes_everyone_defaults AFTER INSERT ON roles
WHEN NEW.id = NEW.guild_id BEGIN
 UPDATE roles SET permissions = permissions | 72057594037927936
 WHERE guild_id = NEW.guild_id AND id = NEW.id;
END;
