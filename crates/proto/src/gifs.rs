//! GIF discovery and private account favorites. No media bytes travel via REST.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GifProviderName {
    Klipy,
}

/// `false` when disabled, otherwise a mode-tagged configuration object.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GifCapability {
    #[default]
    Disabled,
    Enabled(GifConfig),
}
impl Serialize for GifCapability {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Disabled => false.serialize(serializer),
            Self::Enabled(config) => config.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for GifCapability {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Disabled(bool),
            Enabled(GifConfig),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Disabled(false) => Ok(Self::Disabled),
            Wire::Enabled(config) => Ok(Self::Enabled(config)),
            Wire::Disabled(true) => Err(serde::de::Error::custom("expected false or GIF config")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum GifConfig {
    Proxy {
        provider: GifProviderName,
    },
    Direct {
        provider: GifProviderName,
        public_client_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifMedia {
    pub url: String,
    pub width: u16,
    pub height: u16,
    /// video/mp4, video/webm, image/webp, or (send only) image/gif.
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifItem {
    /// Provider-qualified opaque ID, e.g. `klipy:123456`.
    pub id: String,
    pub title: String,
    pub preview: GifMedia,
    pub send: GifMedia,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GifQuery {
    pub q: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifPage {
    pub items: Vec<GifItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifItemsQuery {
    /// One to 24 comma-separated provider-qualified IDs.
    pub ids: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifCategory {
    pub title: String,
    /// Submit this text as `q` to search, not a provider-supplied search URL.
    pub query: String,
    pub preview: GifMedia,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifCategoriesResponse {
    pub items: Vec<GifCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GifFavoritesResponse {
    /// Resolve through `/gifs/items` in proxy mode, or provider posts in direct mode.
    pub ids: Vec<String>,
}

pub const GIF_PAGE_SIZE: usize = 24;
pub const MAX_GIF_FAVORITES: usize = 200;
