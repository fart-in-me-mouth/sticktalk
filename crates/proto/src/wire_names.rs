//! Unit-only enums written as their snake_case names (task `headroom`,
//! docs/features/perf.md).
//!
//! serde's derived decoder for such an enum is about 1 to 2.7 KB of WASM each
//! (a field visitor for strings, bytes and integers, an enum visitor and their
//! error messages), and the web app's first-load module carries one for every
//! enum on a message, member, channel or call. In the web build (wasm32) the
//! enums listed with [`wire_names!`] decode instead by reading a string and
//! matching it against their names: the same wire format, a fraction of the
//! code. Other builds keep serde's derive; the names are checked against it
//! (`tests::wire_names_match_serde`).
//!
//! Every enum keeps `#[derive(Serialize)]` and `#[serde(rename_all =
//! "snake_case")]`, and derives `Deserialize` only off wasm32:
//!
//! ```ignore
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
//! #[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
//! #[serde(rename_all = "snake_case")]
//! pub enum TrackKind { Audio, Camera }
//! wire_names!(TrackKind { Audio = "audio", Camera = "camera" });
//! ```
//!
//! The list is matched exhaustively, so a variant added to the enum and not to
//! its list does not compile.

/// See the module documentation.
macro_rules! wire_names {
    ($t:ident { $($v:ident = $name:literal),+ $(,)? }) => {
        impl $t {
            /// The name this value is written as.
            pub fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$v => $name,)+
                }
            }
            /// The value written as `name`.
            pub fn from_wire(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$v),)+
                    _ => None,
                }
            }
            /// Every value, in declaration order.
            pub const WIRE: &'static [Self] = &[$(Self::$v),+];
        }
        #[cfg(target_arch = "wasm32")]
        impl<'de> serde::Deserialize<'de> for $t {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let name = <String as serde::Deserialize>::deserialize(d)?;
                Self::from_wire(&name).ok_or_else(|| {
                    <D::Error as serde::de::Error>::unknown_variant(&name, &[$($name),+])
                })
            }
        }
    };
}
pub(crate) use wire_names;

#[cfg(test)]
mod tests {
    use crate::*;

    fn check<T>(all: &[T], name: fn(T) -> &'static str, from: fn(&str) -> Option<T>)
    where
        T: Copy + PartialEq + std::fmt::Debug + serde::Serialize + serde::de::DeserializeOwned,
    {
        for &v in all {
            let written = serde_json::to_value(v).unwrap();
            assert_eq!(written, serde_json::Value::from(name(v)), "{v:?}");
            assert_eq!(serde_json::from_value::<T>(written).unwrap(), v);
            assert_eq!(from(name(v)), Some(v));
            let packed = rmp_serde::to_vec_named(&v).unwrap();
            assert_eq!(rmp_serde::from_slice::<String>(&packed).unwrap(), name(v));
        }
        assert_eq!(from("not_a_variant"), None);
    }

    #[test]
    fn wire_names_match_serde() {
        check(
            VideoCodec::WIRE,
            VideoCodec::wire_name,
            VideoCodec::from_wire,
        );
        check(
            ShareSurface::WIRE,
            ShareSurface::wire_name,
            ShareSurface::from_wire,
        );
        check(
            ResyncReason::WIRE,
            ResyncReason::wire_name,
            ResyncReason::from_wire,
        );
        check(TrackKind::WIRE, TrackKind::wire_name, TrackKind::from_wire);
        check(
            ChannelKind::WIRE,
            ChannelKind::wire_name,
            ChannelKind::from_wire,
        );
        check(
            PresenceStatus::WIRE,
            PresenceStatus::wire_name,
            PresenceStatus::from_wire,
        );
        check(
            RelationshipKind::WIRE,
            RelationshipKind::wire_name,
            RelationshipKind::from_wire,
        );
        check(
            RemovalReason::WIRE,
            RemovalReason::wire_name,
            RemovalReason::from_wire,
        );
        check(
            MemberRole::WIRE,
            MemberRole::wire_name,
            MemberRole::from_wire,
        );
        check(
            NotificationLevel::WIRE,
            NotificationLevel::wire_name,
            NotificationLevel::from_wire,
        );
        check(NameFont::WIRE, NameFont::wire_name, NameFont::from_wire);
        check(
            NameEffect::WIRE,
            NameEffect::wire_name,
            NameEffect::from_wire,
        );
        check(
            StickerFormat::WIRE,
            StickerFormat::wire_name,
            StickerFormat::from_wire,
        );
        check(ErrorCode::WIRE, ErrorCode::wire_name, ErrorCode::from_wire);
    }
}
