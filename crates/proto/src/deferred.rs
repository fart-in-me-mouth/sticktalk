//! A value the web app decodes only when it draws it (docs/features/perf.md).
//!
//! Every message the web app reads goes through `Message`'s decoder, in the
//! app's first-load module. Two of its extras are large types only one
//! feature draws: an app's embeds and components (`AppMessage`) and an
//! activity's invite card (`ActivityCard`). In the web build (wasm32) these
//! fields are [`Deferred`]: the value is kept as its JSON, and the feature's
//! lazily loaded views decode it the first time they draw it. The wire format
//! is the same; other builds (the server, native tests) keep the plain types.
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
use std::{cell::OnceCell, fmt, marker::PhantomData};

/// The JSON of a `T`, decoded on first use (then kept).
pub struct Deferred<T> {
    raw: serde_json::Value,
    decoded: OnceCell<Option<Box<T>>>,
    _t: PhantomData<fn() -> T>,
}

impl<T> Deferred<T> {
    pub fn from_raw(raw: serde_json::Value) -> Self {
        Self {
            raw,
            decoded: OnceCell::new(),
            _t: PhantomData,
        }
    }
    /// The value as received, for cheap reads of one field without its
    /// decoder (`raw()["username"]`).
    pub fn raw(&self) -> &serde_json::Value {
        &self.raw
    }
}
impl<T: Serialize> Deferred<T> {
    pub fn new(value: &T) -> Self {
        Self::from_raw(serde_json::to_value(value).unwrap_or_default())
    }
}
impl<T: DeserializeOwned> Deferred<T> {
    /// The decoded value; None when it does not decode as a `T`.
    pub fn get(&self) -> Option<&T> {
        self.decoded
            .get_or_init(|| serde_json::from_value(self.raw.clone()).ok().map(Box::new))
            .as_deref()
    }
}
impl<T> Clone for Deferred<T> {
    fn clone(&self) -> Self {
        Self::from_raw(self.raw.clone())
    }
}
impl<T> PartialEq for Deferred<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Deferred<T> {}
impl<T> fmt::Debug for Deferred<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.raw.fmt(f)
    }
}
impl<T> Serialize for Deferred<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(s)
    }
}
impl<'de, T> Deserialize<'de> for Deferred<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        serde_json::Value::deserialize(d).map(Self::from_raw)
    }
}
