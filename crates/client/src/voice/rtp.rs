//! Bounded RFC 6184 H.264 packetization/reassembly. No raw frame readbacks.
const MAX_ACCESS_UNIT: usize = 8 * 1024 * 1024;

/// Parse dimensions from an in-band SPS, including High-profile chroma/crop.
/// No dimensions are trusted from the remote UI/publication declaration.
pub fn dimensions(annex_b: &[u8]) -> Option<sticktalk_media::Size> {
    let starts: Vec<_> = annex_b
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == [0, 0, 0, 1])
        .map(|(i, _)| i + 4)
        .collect();
    for (index, &start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(annex_b.len(), |i| i - 4);
        if start >= end || annex_b[start] & 31 != 7 {
            continue;
        }
        let mut rbsp = Vec::new();
        let mut zeros = 0;
        for &b in &annex_b[start + 1..end] {
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            rbsp.push(b);
            zeros = if b == 0 { zeros + 1 } else { 0 };
        }
        let mut bits = Bits {
            bytes: &rbsp,
            at: 0,
        };
        let profile = bits.read(8)?;
        bits.read(16)?;
        bits.ue()?;
        let mut chroma = 1;
        let mut separate = 0;
        if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
            chroma = bits.ue()?;
            if chroma > 3 {
                return None;
            }
            if chroma == 3 {
                separate = bits.read(1)?;
            }
            bits.ue()?;
            bits.ue()?;
            bits.read(1)?;
            if bits.read(1)? != 0 {
                for i in 0..if chroma != 3 { 8 } else { 12 } {
                    if bits.read(1)? != 0 {
                        let mut last = 8i32;
                        let mut next = 8i32;
                        for _ in 0..if i < 6 { 16 } else { 64 } {
                            if next != 0 {
                                let e = bits.ue()?;
                                let delta = if e % 2 == 0 {
                                    -((e / 2) as i32)
                                } else {
                                    e.div_ceil(2) as i32
                                };
                                next = (last + delta + 256) % 256;
                            }
                            if next != 0 {
                                last = next;
                            }
                        }
                    }
                }
            }
        }
        bits.ue()?;
        let order = bits.ue()?;
        if order == 0 {
            bits.ue()?;
        } else if order == 1 {
            bits.read(1)?;
            bits.ue()?;
            bits.ue()?;
            let count = bits.ue()?;
            if count > 256 {
                return None;
            }
            for _ in 0..count {
                bits.ue()?;
            }
        } else if order > 2 {
            return None;
        }
        bits.ue()?;
        bits.read(1)?;
        let width = bits.ue()?.checked_add(1)?.checked_mul(16)?;
        let units = bits.ue()?.checked_add(1)?;
        let frame = bits.read(1)?;
        let height = units.checked_mul(16)?.checked_mul(2 - frame)?;
        if frame == 0 {
            bits.read(1)?;
        }
        bits.read(1)?;
        let (mut left, mut right, mut top, mut bottom) = (0, 0, 0, 0);
        if bits.read(1)? != 0 {
            left = bits.ue()?;
            right = bits.ue()?;
            top = bits.ue()?;
            bottom = bits.ue()?;
        }
        let array = if separate != 0 { 0 } else { chroma };
        let crop_x = if array == 1 || array == 2 { 2 } else { 1 };
        let crop_y = (if array == 1 { 2 } else { 1 }) * (2 - frame);
        let size = sticktalk_media::Size {
            width: width.checked_sub(left.checked_add(right)?.checked_mul(crop_x)?)?,
            height: height.checked_sub(top.checked_add(bottom)?.checked_mul(crop_y)?)?,
        };
        return size.validate().ok();
    }
    None
}
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Bits<'_> {
    fn read(&mut self, n: usize) -> Option<u32> {
        if n > 32 || self.at + n > self.bytes.len() * 8 {
            return None;
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from((self.bytes[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 30 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.read(zeros)?)
    }
}

pub fn packetize(annex_b: &[u8], mtu: usize) -> Result<Vec<Vec<u8>>, String> {
    if !(256..=1180).contains(&mtu) || annex_b.len() > MAX_ACCESS_UNIT {
        return Err("Invalid H.264 size/MTU".into());
    }
    let mut starts = vec![];
    let mut i = 0;
    while i + 3 <= annex_b.len() {
        let n = if annex_b[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if annex_b[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        starts.push((i, i + n));
        i += n;
    }
    if starts.is_empty() {
        return Err("H.264 access unit needs Annex-B start codes".into());
    }
    let mut packets = vec![];
    for (index, &(_, start)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(annex_b.len(), |s| s.0);
        let nal = &annex_b[start..end];
        if nal.is_empty() {
            continue;
        }
        if nal.len() <= mtu {
            packets.push(nal.to_vec());
            continue;
        }
        let chunks = nal[1..].chunks(mtu - 2);
        let count = chunks.len();
        for (j, chunk) in chunks.enumerate() {
            let mut out = Vec::with_capacity(chunk.len() + 2);
            out.push((nal[0] & 0xe0) | 28);
            out.push(
                (nal[0] & 0x1f)
                    | if j == 0 { 0x80 } else { 0 }
                    | if j + 1 == count { 0x40 } else { 0 },
            );
            out.extend_from_slice(chunk);
            packets.push(out);
        }
    }
    Ok(packets)
}

#[derive(Default)]
pub struct Reassembler {
    timestamp: Option<u32>,
    sequence: Option<u64>,
    bytes: Vec<u8>,
    fragment: bool,
    damaged: bool,
    keyframe: bool,
}
impl Reassembler {
    pub fn push(
        &mut self,
        sequence: u64,
        timestamp: u32,
        marker: bool,
        payload: &[u8],
    ) -> Option<(Vec<u8>, bool)> {
        if self.timestamp != Some(timestamp) {
            self.timestamp = Some(timestamp);
            self.bytes.clear();
            self.fragment = false;
            self.damaged = false;
            self.keyframe = false;
        }
        if self.sequence.is_some_and(|s| sequence != s + 1) {
            self.damaged = true;
        }
        self.sequence = Some(sequence);
        if payload.is_empty() || self.bytes.len() + payload.len() + 4 > MAX_ACCESS_UNIT {
            self.damaged = true;
            return None;
        }
        match payload[0] & 31 {
            kind @ 1..=23 => {
                if self.fragment {
                    self.damaged = true;
                }
                self.keyframe |= kind == 5;
                self.bytes.extend_from_slice(&[0, 0, 0, 1]);
                self.bytes.extend_from_slice(payload);
            }
            24 => {
                let mut p = &payload[1..];
                while p.len() >= 2 {
                    let n = u16::from_be_bytes([p[0], p[1]]) as usize;
                    p = &p[2..];
                    if n == 0 || n > p.len() {
                        self.damaged = true;
                        break;
                    }
                    self.keyframe |= p[0] & 31 == 5;
                    self.bytes.extend_from_slice(&[0, 0, 0, 1]);
                    self.bytes.extend_from_slice(&p[..n]);
                    p = &p[n..];
                }
                self.damaged |= !p.is_empty();
            }
            28 if payload.len() >= 3 => {
                let start = payload[1] & 0x80 != 0;
                let end = payload[1] & 0x40 != 0;
                if start {
                    if self.fragment {
                        self.damaged = true;
                    }
                    self.bytes.extend_from_slice(&[0, 0, 0, 1]);
                    self.bytes.push((payload[0] & 0xe0) | (payload[1] & 31));
                    self.keyframe |= payload[1] & 31 == 5;
                    self.fragment = true;
                } else if !self.fragment {
                    self.damaged = true;
                }
                self.bytes.extend_from_slice(&payload[2..]);
                if end {
                    self.fragment = false;
                }
            }
            _ => self.damaged = true,
        }
        if marker && !self.damaged && !self.fragment && !self.bytes.is_empty() {
            Some((std::mem::take(&mut self.bytes), self.keyframe))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_idr_round_trip_and_loss_is_dropped() {
        let mut data = vec![0, 0, 0, 1, 0x65];
        data.extend(vec![42; 6000]);
        let p = packetize(&data, 1180).unwrap();
        let count = p.len();
        assert!(p.iter().all(|p| p.len() <= 1180));
        let mut r = Reassembler::default();
        let mut result = None;
        for (i, p) in p.iter().enumerate() {
            result = r.push(i as u64, 10, i + 1 == count, p);
        }
        assert_eq!(result, Some((data, true)));
        let mut r = Reassembler::default();
        for (i, p) in p.iter().enumerate().filter(|(i, _)| *i != 2) {
            assert!(r.push(i as u64, 10, i + 1 == count, p).is_none());
        }
    }
}
