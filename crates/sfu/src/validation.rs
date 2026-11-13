//! Bounded, conservative ingress policy before any str0m state mutation.
use crate::Error;
use std::collections::{BTreeMap, BTreeSet};
use str0m::{
    change::{SdpAnswer, SdpOffer},
    net::Protocol,
    Candidate,
};

pub(crate) const MAX_SDP_BYTES: usize = 65_536;
pub(crate) const MAX_MEDIA: usize = 33;
pub(crate) const MAX_SSRC: usize = 128;
pub(crate) const MAX_ICE: usize = 128;
pub(crate) const MAX_CONTROL_BYTES: usize = 512;
pub(crate) const MAX_RENEGOTIATIONS: usize = 30;

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 16
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub(crate) fn candidate(value: &str) -> Result<Candidate, Error> {
    if value.len() > 1024 {
        return Err(Error::Limit);
    }
    if !value.is_ascii() || value.bytes().any(|b| b.is_ascii_control()) {
        return Err(Error::Invalid);
    }
    let fields: Vec<_> = value.split_ascii_whitespace().collect();
    if fields.len() < 8
        || fields[1] != "1"
        || !fields[2].eq_ignore_ascii_case("udp")
        || fields[6] != "typ"
    {
        return Err(Error::Invalid);
    }
    let candidate = Candidate::from_sdp_string(value).map_err(|_| Error::Invalid)?;
    if candidate.proto() != Protocol::Udp
        || candidate.addr().port() == 0
        || candidate.addr().ip().is_unspecified()
        || candidate.addr().ip().is_multicast()
    {
        return Err(Error::Invalid);
    }
    Ok(candidate)
}

#[derive(Default)]
pub(crate) struct Description {
    pub mids: BTreeMap<String, String>,
    pub candidates: BTreeSet<String>,
    pub ssrcs: BTreeSet<u32>,
}

// Check raw identifiers/numbers BEFORE str0m can truncate identifiers or index PT arrays.
// Unknown codecs are left in the SDP for normal unsupported-codec rejection; their
// PT/RTX mappings must still be unambiguous within a bundled transport.
fn preflight(sdp: &str) -> Result<Description, Error> {
    if sdp.len() > MAX_SDP_BYTES {
        return Err(Error::Limit);
    }
    if !sdp.is_ascii()
        || sdp
            .bytes()
            .any(|b| b.is_ascii_control() && !matches!(b, b'\r' | b'\n' | b'\t'))
    {
        return Err(Error::Invalid);
    }
    let mut description = Description::default();
    let mut media = 0;
    let mut kind = "";
    let mut has_mid = false;
    let mut pts = BTreeSet::new();
    let mut rtp = BTreeMap::new();
    let mut formats = BTreeMap::new();
    let mut lines = 0;
    for line in sdp.lines() {
        lines += 1;
        if lines > 2048 || line.len() > 2048 {
            return Err(Error::Limit);
        }
        if line.len() < 2 || line.as_bytes()[1] != b'=' {
            return Err(Error::Invalid);
        }
        if let Some(m) = line.strip_prefix("m=") {
            if media > 0 && !has_mid {
                return Err(Error::Invalid);
            }
            media += 1;
            if media > MAX_MEDIA {
                return Err(Error::Limit);
            }
            let fields: Vec<_> = m.split_ascii_whitespace().collect();
            if fields.len() < 4 || !matches!(fields[0], "audio" | "video" | "application") {
                return Err(Error::Invalid);
            }
            kind = fields[0];
            has_mid = false;
            pts.clear();
            if kind != "application" {
                for p in &fields[3..] {
                    let pt = payload_type(p)?;
                    if !pts.insert(pt) {
                        return Err(Error::Invalid);
                    }
                }
            }
        } else if let Some(mid) = line.strip_prefix("a=mid:") {
            if media == 0
                || has_mid
                || !valid_id(mid)
                || description.mids.insert(mid.into(), kind.into()).is_some()
            {
                return Err(Error::Invalid);
            }
            has_mid = true;
        } else if let Some(rid) = line.strip_prefix("a=rid:") {
            if !rid.split_ascii_whitespace().next().is_some_and(valid_id) {
                return Err(Error::Invalid);
            }
        } else if let Some(value) = line.strip_prefix("a=ssrc-group:") {
            let mut fields = value.split_ascii_whitespace();
            fields.next().ok_or(Error::Invalid)?;
            for value in fields {
                let id = value.parse::<u32>().map_err(|_| Error::Invalid)?;
                if id == 0 {
                    return Err(Error::Invalid);
                }
                description.ssrcs.insert(id);
                if description.ssrcs.len() > MAX_SSRC {
                    return Err(Error::Limit);
                }
            }
        } else if let Some(value) = line.strip_prefix("a=ssrc:") {
            let id = value
                .split_ascii_whitespace()
                .next()
                .ok_or(Error::Invalid)?
                .parse::<u32>()
                .map_err(|_| Error::Invalid)?;
            if id == 0 {
                return Err(Error::Invalid);
            }
            description.ssrcs.insert(id);
            if description.ssrcs.len() > MAX_SSRC {
                return Err(Error::Limit);
            }
        } else if let Some(value) = line.strip_prefix("a=candidate:") {
            let value = format!("candidate:{value}");
            candidate(&value)?;
            description.candidates.insert(value);
            if description.candidates.len() > MAX_ICE {
                return Err(Error::Limit);
            }
        } else if let Some(value) = line.strip_prefix("a=rtpmap:") {
            let (pt, spec) = value.split_once(' ').ok_or(Error::Invalid)?;
            let pt = payload_type(pt)?;
            if !pts.contains(&pt) {
                return Err(Error::Invalid);
            }
            let spec = spec.to_ascii_lowercase();
            if rtp.insert((media, pt), spec.clone()).is_some() {
                return Err(Error::Invalid);
            }
            // str0m uses a single codec config for all bundled m-lines.
            if let Some(previous) = formats.insert(pt, spec.clone()) {
                if previous != spec {
                    return Err(Error::Invalid);
                }
            }
        } else if let Some(value) = line.strip_prefix("a=fmtp:") {
            let (pt, params) = value.split_once(' ').ok_or(Error::Invalid)?;
            let pt = payload_type(pt)?;
            if !pts.contains(&pt) {
                return Err(Error::Invalid);
            }
            for param in params.split(';') {
                if let Some(apt) = param.trim().strip_prefix("apt=") {
                    let apt = payload_type(apt)?;
                    if apt == pt || !pts.contains(&apt) {
                        return Err(Error::Invalid);
                    }
                }
            }
        }
    }
    if media == 0 || !has_mid {
        return Err(Error::Invalid);
    }
    Ok(description)
}
fn payload_type(value: &str) -> Result<u8, Error> {
    let pt = value.parse::<u8>().map_err(|_| Error::Invalid)?;
    if pt > 127 || (64..=95).contains(&pt) {
        return Err(Error::Invalid);
    }
    Ok(pt)
}

pub(crate) fn offer(sdp: &str) -> Result<(SdpOffer, Description), Error> {
    let description = preflight(sdp)?;
    let offer = SdpOffer::from_sdp_string(sdp).map_err(|_| Error::Invalid)?;
    for media in &offer.media_lines {
        if media.check_consistent().is_some() {
            return Err(Error::Invalid);
        }
    }
    // Reject codec/PT aliasing across the bundle, including RTX apt and profile
    // differences. Otherwise str0m may lock the same PT to distinct formats.
    let mut formats = BTreeMap::new();
    for media in &offer.media_lines {
        for p in media.rtp_params() {
            for pt in [Some(p.pt()), p.resend()].into_iter().flatten() {
                let signature = (p.spec(), p.pt(), p.resend());
                if let Some(old) = formats.insert(*pt, signature) {
                    if old != signature {
                        return Err(Error::Invalid);
                    }
                }
            }
        }
    }
    Ok((offer, description))
}

// SFU is always answerer: answers arriving as a client signal are a typed
// schema error. Still validate generated answers through the same bounded parser.
pub(crate) fn answer(sdp: &str) -> Result<SdpAnswer, Error> {
    preflight(sdp)?;
    let answer = SdpAnswer::from_sdp_string(sdp).map_err(|_| Error::Invalid)?;
    if answer
        .media_lines
        .iter()
        .any(|media| media.check_consistent().is_some())
    {
        return Err(Error::Invalid);
    }
    Ok(answer)
}

pub(crate) fn control(data: &[u8]) -> Result<sticktalk_proto::ControlData, Error> {
    if data.len() > MAX_CONTROL_BYTES {
        return Err(Error::Limit);
    }
    let message = decode(data)?;
    if !matches!(message, sticktalk_proto::ControlData::Input { .. }) {
        return Err(Error::Invalid);
    }
    Ok(message)
}

pub(crate) fn layer(
    source: &str,
    receiver: &str,
    layers: &[String],
    rid: Option<&str>,
) -> Result<(), Error> {
    if !valid_id(source)
        || !valid_id(receiver)
        || rid.is_some_and(|r| !valid_id(r))
        || (layers.is_empty() && rid.is_some())
        || (!layers.is_empty() && !rid.is_some_and(|r| layers.iter().any(|layer| layer == r)))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T, Error> {
    let mut decoder = rmp_serde::Deserializer::new(std::io::Cursor::new(data));
    decoder.set_max_depth(32);
    let message = serde::Deserialize::deserialize(&mut decoder).map_err(|_| Error::Invalid)?;
    if decoder.get_ref().position() != data.len() as u64 {
        return Err(Error::Invalid);
    }
    Ok(message)
}
