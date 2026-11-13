//! Online SQLite snapshots sent to the private storage node, never a live DB/WAL copy.
use crate::{
    blob::{BlobStore, LocalDisk, RemoteStore},
    error::{require, Error, Result},
    state::{now, Config},
};
use sqlx::{Connection, Row};
use sticktalk_proto::ErrorCode;

async fn replicate(remote: &RemoteStore, local: &LocalDisk, key: &str, size: u64) -> Result<()> {
    let stat = match remote.stat(key).await {
        Ok(s) => Some(s),
        Err(e) if e.0.code == ErrorCode::NotFound => None,
        Err(e) => return Err(e),
    };
    if stat.as_ref().is_some_and(|s| s.complete && s.size == size) {
        return Ok(());
    }
    let mut offset = stat.map_or(0, |s| s.size).min(size);
    loop {
        let length = (size - offset).min(crate::resumable::CHUNK);
        let sha = crate::blob::hash_region(&local.path(key)?, offset, length).await?;
        remote
            .put_chunk(
                key,
                offset,
                length,
                &sha,
                local.get(key, offset, length).await?,
            )
            .await?;
        offset += length;
        if offset == size {
            break;
        }
    }
    remote
        .finalize(
            key,
            size,
            &crate::blob::hash_region(&local.path(key)?, 0, size).await?,
        )
        .await
}
pub async fn run(config: &Config) -> Result<()> {
    let recipient = config
        .operations
        .backup_recipient
        .as_deref()
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest))?;
    recipient
        .parse::<age::x25519::Recipient>()
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let tier = crate::tier::Tier::new(config)?;
    let remote = tier
        .remote
        .as_ref()
        .ok_or_else(|| Error::new(ErrorCode::FeatureDisabled))?;
    // Also drives bounded tombstone cleanup on the Pi.
    remote.free_space().await?;
    let dir = config.data_dir.join("backups");
    tokio::fs::create_dir_all(&dir).await?;
    let key = format!("backup-{}.sqlite3", now());
    let snapshot = dir.join(&key);
    let encrypted_key = format!("{key}.age");
    let encrypted = dir.join(&encrypted_key);
    let mut db = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(config.data_dir.join("sticktalk.sqlite3"))
            .busy_timeout(std::time::Duration::from_secs(30)),
    )
    .await?;
    sqlx::query("VACUUM INTO ?")
        .bind(snapshot.to_string_lossy().as_ref())
        .execute(&mut db)
        .await?;
    drop(db);
    let result = async {
        let mut backup = sqlx::SqliteConnection::connect_with(&sqlx::sqlite::SqliteConnectOptions::new().filename(&snapshot).read_only(true)).await?;
        let check: String = sqlx::query_scalar("PRAGMA quick_check").fetch_one(&mut backup).await?;
        require(check == "ok",ErrorCode::Unavailable)?;
        let mut after = String::new();
        loop {
            let rows = sqlx::query("SELECT key,size FROM (SELECT storage_key AS key,byte_size AS size FROM uploads UNION ALL SELECT thumbnail_key,thumbnail_size FROM uploads WHERE thumbnail_key IS NOT NULL) WHERE key>? ORDER BY key LIMIT 64").bind(&after).fetch_all(&mut backup).await?;
            if rows.is_empty() { break; }
            for row in rows {
                let key: String = row.get("key");
                replicate(remote,&tier.local,&key,row.get::<i64,_>("size") as u64).await?;
                after = key;
            }
        }
        drop(backup);
        let plain=snapshot.clone();let cipher=encrypted.clone();let recipient=recipient.to_owned();
        tokio::task::spawn_blocking(move||encrypt_snapshot(&plain,&cipher,&recipient)).await.map_err(|_|Error::new(ErrorCode::Internal))??;
        // Never send an unencrypted database snapshot to the storage node.
        replicate(remote,&LocalDisk::new(dir.clone()),&encrypted_key,tokio::fs::metadata(&encrypted).await?.len()).await?;
        println!("Verified age-encrypted backup stored on Pi: {encrypted_key}");
        Ok(())
    }.await;
    let _ = tokio::fs::remove_file(&snapshot).await;
    let _ = tokio::fs::remove_file(&encrypted).await;
    result
}

fn encrypt_snapshot(
    input: &std::path::Path,
    output: &std::path::Path,
    recipient: &str,
) -> Result<()> {
    let recipient: age::x25519::Recipient = recipient
        .parse()
        .map_err(|_| Error::new(ErrorCode::InvalidRequest))?;
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .map_err(|_| Error::new(ErrorCode::Internal))?;
    let mut input = std::fs::File::open(input)?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let mut writer = encryptor.wrap_output(output)?;
    std::io::copy(&mut input, &mut writer)?;
    writer.finish()?.sync_all()?;
    Ok(())
}
#[cfg(test)]
mod encryption_tests {
    use super::*;
    #[test]
    fn s7_snapshot_is_ciphertext_and_only_owner_identity_decrypts() {
        assert!(crate::blob::valid_key("backup-123456789.sqlite3.age"));
        assert!(!crate::blob::valid_key("backup-../secret.sqlite3.age"));
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("plain");
        let output = dir.path().join("cipher");
        std::fs::write(&source, b"SQLite snapshot containing private messages").unwrap();
        let owner = age::x25519::Identity::generate();
        encrypt_snapshot(&source, &output, &owner.to_public().to_string()).unwrap();
        let ciphertext = std::fs::read(&output).unwrap();
        assert!(ciphertext.starts_with(b"age-encryption.org/v1"));
        assert!(!ciphertext.windows(7).any(|w| w == b"private"));
        assert_eq!(
            age::decrypt(&owner, &ciphertext).unwrap(),
            std::fs::read(&source).unwrap()
        );
        assert!(age::decrypt(&age::x25519::Identity::generate(), &ciphertext).is_err());
        assert!(encrypt_snapshot(&source, &dir.path().join("invalid"), "not-a-recipient").is_err());
    }
}
