-- Account security and personal data (docs/features/authsec.md).

-- Passkeys: passwordless sign-in only. Public keys are not secrets; the stored
-- JSON is webauthn-rs's Passkey (credential ID, COSE public key, counter).
CREATE TABLE passkey_users(
  user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  -- Random WebAuthn user handle, never derived from the user ID.
  handle BLOB NOT NULL UNIQUE CHECK(length(handle)=16)
);
CREATE TABLE passkeys(
  id INTEGER PRIMARY KEY CHECK(id>0),
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  credential_id BLOB NOT NULL UNIQUE,
  name TEXT NOT NULL,
  passkey TEXT NOT NULL,
  backed_up INTEGER NOT NULL DEFAULT 0 CHECK(backed_up IN(0,1)),
  created_at INTEGER NOT NULL,
  last_used_at INTEGER
);
CREATE INDEX passkeys_user ON passkeys(user_id);

-- One email address per account. Not unique: a reset by address mails every
-- account that verified it, so adding one never reveals another account.
CREATE TABLE user_emails(
  user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  email TEXT,
  verified_at INTEGER,
  pending_email TEXT
);
CREATE INDEX user_emails_email ON user_emails(email) WHERE verified_at IS NOT NULL;
-- Verification and reset links: only SHA-256 of the secret is stored.
CREATE TABLE email_tokens(
  token_hash BLOB PRIMARY KEY CHECK(length(token_hash)=32),
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK(kind IN('verify','reset')),
  email TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  UNIQUE(user_id,kind)
);

-- Personal data export jobs. The archive lives in <data>/exports/<file_key>.zip.
CREATE TABLE data_exports(
  id INTEGER PRIMARY KEY CHECK(id>0),
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK(state IN('queued','running','ready','failed','expired')),
  requested_at INTEGER NOT NULL,
  ready_at INTEGER,
  expires_at INTEGER,
  byte_size INTEGER,
  file_key TEXT UNIQUE
);
CREATE INDEX data_exports_user ON data_exports(user_id,requested_at);
CREATE INDEX data_exports_due ON data_exports(expires_at) WHERE state='ready';
