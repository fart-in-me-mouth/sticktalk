//! Visible-only thumbnail loading. Decode runs on Tokio's blocking pool.
use crate::{
    backend::{limited, Config, Http},
    storage,
};
use iced::widget::image::Handle;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};
use sticktalk_proto::Id;
const RAM: usize = 16 * 1024 * 1024;
const DISK: u64 = 128 * 1024 * 1024;
#[derive(Default)]
pub struct Images {
    entries: BTreeMap<Id, (Handle, usize)>,
    lru: VecDeque<Id>,
    pub pending: std::collections::BTreeSet<Id>,
    pub failed: std::collections::BTreeSet<Id>,
    bytes: usize,
}
#[derive(Debug, Clone)]
pub struct Decoded {
    pub id: Id,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Images {
    pub fn get(&self, id: Id) -> Option<&Handle> {
        self.entries.get(&id).map(|v| &v.0)
    }
    pub fn insert(&mut self, v: Decoded) {
        self.pending.remove(&v.id);
        self.lru.retain(|id| *id != v.id);
        if let Some((_, bytes)) = self.entries.remove(&v.id) {
            self.bytes -= bytes;
        }
        let bytes = v.rgba.len();
        self.bytes += bytes;
        self.entries
            .insert(v.id, (Handle::from_rgba(v.width, v.height, v.rgba), bytes));
        self.lru.push_back(v.id);
        while self.entries.len() > 64 || self.bytes > RAM {
            if let Some(id) = self.lru.pop_front() {
                if let Some((_, bytes)) = self.entries.remove(&id) {
                    self.bytes -= bytes;
                }
            } else {
                break;
            }
        }
    }
    pub fn touch(&mut self, id: Id) {
        if self.entries.contains_key(&id) {
            self.lru.retain(|v| *v != id);
            self.lru.push_back(id);
        }
    }
    pub fn contains(&self, id: Id) -> bool {
        self.entries.contains_key(&id) || self.pending.contains(&id) || self.failed.contains(&id)
    }
}
pub fn namespace(config: &Config, account: Id) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    config.server.hash(&mut h);
    storage::root().join("Cache").join(format!(
        "{}{:016x}-{}",
        if config.mock { "mock-" } else { "" },
        h.finish(),
        account.0
    ))
}
pub fn maintain(config: &Config, account: Id) {
    prune(&namespace(config, account));
}
pub fn purge(config: &Config, account: Id) {
    let path = namespace(config, account);
    let root = storage::root().join("Cache");
    if path.parent() == Some(root.as_path()) {
        let _ = std::fs::remove_dir_all(path);
    }
}
pub async fn load(config: Config, account: Id, id: Id) -> Result<Decoded, String> {
    static SLOTS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    let _slot = SLOTS
        .get_or_init(|| tokio::sync::Semaphore::new(2))
        .acquire()
        .await
        .map_err(|_| "Thumbnail worker closed".to_string())?;
    let dir = namespace(&config, account);
    let path = dir.join(format!("{}.thumb", id.0));
    let cache_path = path.clone();
    let cached = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&cache_path).ok().filter(|v| v.len() <= 65536);
        if bytes.is_some() {
            if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&cache_path) {
                let _ = file.set_times(
                    std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()),
                );
            }
        }
        bytes
    })
    .await
    .unwrap_or(None);
    let bytes = if let Some(v) = cached {
        v
    } else {
        let http = Http::new(&config.server, &config.token)?;
        let response = http
            .get(&format!("/files/{}/thumbnail", id.0))
            .send()
            .await
            .map_err(|_| "Thumbnail request failed".to_string())?;
        if !response.status().is_success() {
            return Err("Thumbnail unavailable".into());
        }
        let bytes = limited(response, 65536).await?;
        let save = bytes.clone();
        tokio::task::spawn_blocking(move || {
            let _ = std::fs::create_dir_all(&dir);
            prune(&dir);
            let _ = std::fs::write(&path, save);
            prune(&dir);
        })
        .await
        .map_err(|_| "Thumbnail cache failed".to_string())?;
        bytes
    };
    tokio::task::spawn_blocking(move || {
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "Invalid thumbnail".to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(256);
        limits.max_image_height = Some(256);
        limits.max_alloc = Some(1024 * 1024);
        let mut reader = reader;
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| "Invalid thumbnail image".to_string())?
            .to_rgba8();
        Ok(Decoded {
            id,
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        })
    })
    .await
    .map_err(|_| "Thumbnail decoder failed".to_string())?
}
fn prune(dir: &std::path::Path) {
    let Ok(files) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = files
        .filter_map(Result::ok)
        .filter_map(|f| {
            f.metadata()
                .ok()
                .filter(|m| m.is_file())
                .map(|m| (m.modified().ok(), m.len(), f.path()))
        })
        .collect();
    entries.sort_by_key(|v| v.0);
    let mut total: u64 = entries.iter().map(|v| v.1).sum();
    for (_, len, path) in entries {
        if total <= DISK {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_lru_is_bounded_and_touches_preserve_recent_images() {
        let mut images = Images::default();
        for n in 0..100 {
            images.insert(Decoded {
                id: Id(n),
                width: 256,
                height: 256,
                rgba: vec![255; 256 * 256 * 4],
            });
            if n == 63 {
                images.touch(Id(0));
            }
        }
        assert_eq!(images.entries.len(), 64);
        assert!(images.bytes <= RAM);
        assert!(images.get(Id(0)).is_some());
        assert!(images.get(Id(1)).is_none());
        assert!(images.get(Id(99)).is_some());
    }
}
