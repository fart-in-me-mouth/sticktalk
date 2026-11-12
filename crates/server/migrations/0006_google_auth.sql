-- Empty password_hash is the explicit no-password sentinel (never a PHC hash).
-- Keeping users' NOT NULL column avoids rebuilding all foreign-key parents.
CREATE TABLE oauth_identities (
    provider TEXT NOT NULL CHECK(provider='google'),
    subject TEXT NOT NULL CHECK(length(subject) BETWEEN 1 AND 255),
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY(provider,subject),
    UNIQUE(provider,user_id)
);
CREATE TABLE oauth_signup_tickets (
    token_hash BLOB PRIMARY KEY CHECK(length(token_hash)=32),
    subject TEXT NOT NULL UNIQUE CHECK(length(subject) BETWEEN 1 AND 255),
    expires_at INTEGER NOT NULL
);
CREATE INDEX oauth_ticket_expiry ON oauth_signup_tickets(expires_at);
