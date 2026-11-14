-- Cascading channel/guild deletion checks this self-referencing foreign key for
-- every message. Without the child-key index erasing a 10k-message account is
-- quadratic and can exceed the API deadline (rolling back the whole erasure).
CREATE INDEX message_reply ON messages(reply_to);
