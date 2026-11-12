CREATE TABLE gif_favorites (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    gif_id TEXT NOT NULL CHECK(length(gif_id) BETWEEN 7 AND 134),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    PRIMARY KEY(user_id, gif_id)
);
CREATE INDEX gif_favorites_account_order ON gif_favorites(user_id, created_at DESC, gif_id);
