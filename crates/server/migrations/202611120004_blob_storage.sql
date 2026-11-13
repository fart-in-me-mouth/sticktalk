-- Durable remote replicas; only acknowledged replicas may be evicted locally.
CREATE TABLE blob_replicas (
    key TEXT PRIMARY KEY,
    byte_size INTEGER NOT NULL,
    accessed_at INTEGER NOT NULL,
    synced INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX blob_replicas_lru ON blob_replicas(accessed_at);

