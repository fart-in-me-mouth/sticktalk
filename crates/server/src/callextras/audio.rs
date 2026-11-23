//! Soundboard clip validation. A clip is accepted only when its whole file is a
//! well-formed MP3 (MPEG audio layer III frames), Ogg (Opus or Vorbis, one
//! stream, every page checksum correct) or WAV (PCM or float) file, and its
//! duration, measured from the stream itself rather than from a header claim,
//! is at most `MAX_SOUND_MS`. Nothing is decoded, so a hostile file costs one
//! linear scan of at most 512 KiB.
use sticktalk_proto::{SoundFormat, MAX_SOUND_BYTES, MAX_SOUND_MS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clip {
    pub format: SoundFormat,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    TooLarge,
    TooLong,
    Unsupported,
}

pub fn inspect(bytes: &[u8]) -> Result<Clip, Rejected> {
    if bytes.len() > MAX_SOUND_BYTES {
        return Err(Rejected::TooLarge);
    }
    let (format, micros) = if bytes.starts_with(b"RIFF") {
        (SoundFormat::Wav, wav(bytes)?)
    } else if bytes.starts_with(b"OggS") {
        (SoundFormat::Ogg, ogg(bytes)?)
    } else {
        (SoundFormat::Mp3, mp3(bytes)?)
    };
    let duration_ms = micros.div_ceil(1000);
    if duration_ms == 0 {
        return Err(Rejected::Unsupported);
    }
    if duration_ms > u64::from(MAX_SOUND_MS) {
        return Err(Rejected::TooLong);
    }
    Ok(Clip {
        format,
        duration_ms: duration_ms as u32,
    })
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// RIFF/WAVE with one `fmt ` and one `data` chunk: duration is data bytes over
/// the byte rate, which must match the declared sample format exactly.
fn wav(b: &[u8]) -> Result<u64, Rejected> {
    let bad = Rejected::Unsupported;
    if b.get(8..12) != Some(b"WAVE") {
        return Err(bad);
    }
    let riff = u32le(b, 4).ok_or(bad)? as usize;
    if riff + 8 != b.len() && riff + 9 != b.len() {
        return Err(bad);
    }
    let (mut at, mut format, mut data) = (12usize, None, None);
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = u32le(b, at + 4).ok_or(bad)? as usize;
        let body = b.get(at + 8..at + 8 + size).ok_or(bad)?;
        match id {
            b"fmt " if format.is_none() => {
                if size < 16 {
                    return Err(bad);
                }
                let tag = u16le(body, 0).ok_or(bad)?;
                let channels = u16le(body, 2).ok_or(bad)?;
                let rate = u32le(body, 4).ok_or(bad)?;
                let byte_rate = u32le(body, 8).ok_or(bad)?;
                let align = u16le(body, 12).ok_or(bad)?;
                let bits = u16le(body, 14).ok_or(bad)?;
                let float = tag == 3;
                let ok = matches!(tag, 1 | 3 | 0xfffe)
                    && (1..=2).contains(&channels)
                    && (8_000..=96_000).contains(&rate)
                    && if float {
                        bits == 32
                    } else {
                        matches!(bits, 8 | 16 | 24 | 32)
                    }
                    && u32::from(align) == u32::from(channels) * u32::from(bits) / 8
                    && byte_rate == rate * u32::from(align);
                if !ok {
                    return Err(bad);
                }
                format = Some((byte_rate, align));
            }
            b"data" if data.is_none() => data = Some(size),
            b"fmt " | b"data" => return Err(bad),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    if at < b.len().saturating_sub(1) {
        return Err(bad);
    }
    let ((byte_rate, align), data) = (format.ok_or(bad)?, data.ok_or(bad)?);
    if data == 0 || data % usize::from(align) != 0 {
        return Err(bad);
    }
    Ok(data as u64 * 1_000_000 / u64::from(byte_rate))
}

/// The Ogg CRC: polynomial 0x04c11db7, not reflected, initial value zero.
fn ogg_crc(page: &[u8]) -> u32 {
    let mut crc = 0u32;
    for (i, &byte) in page.iter().enumerate() {
        // The checksum field itself counts as zero.
        let byte = if (22..26).contains(&i) { 0 } else { byte };
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// One logical Ogg stream of Opus or Vorbis, pages back to back to the end of
/// the file. Duration is the last page's granule position (less Opus pre-skip)
/// over the stream's sample rate.
fn ogg(b: &[u8]) -> Result<u64, Rejected> {
    let bad = Rejected::Unsupported;
    let (mut at, mut serial, mut sequence) = (0usize, None, 0u32);
    let (mut rate, mut pre_skip, mut last) = (0u64, 0u64, 0i64);
    while at < b.len() {
        let header = b.get(at..at + 27).ok_or(bad)?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Err(bad);
        }
        let flags = header[5];
        let granule = i64::from_le_bytes(header[6..14].try_into().unwrap());
        let stream = u32::from_le_bytes(header[14..18].try_into().unwrap());
        let page_no = u32::from_le_bytes(header[18..22].try_into().unwrap());
        let crc = u32::from_le_bytes(header[22..26].try_into().unwrap());
        let segments = usize::from(header[26]);
        let lacing = b.get(at + 27..at + 27 + segments).ok_or(bad)?;
        let body_len: usize = lacing.iter().map(|&v| usize::from(v)).sum();
        let end = at + 27 + segments + body_len;
        let page = b.get(at..end).ok_or(bad)?;
        if ogg_crc(page) != crc {
            return Err(bad);
        }
        let first = serial.is_none();
        if first != (flags & 2 != 0) || serial.is_some_and(|s| s != stream) || page_no != sequence {
            return Err(bad);
        }
        serial = Some(stream);
        sequence = sequence.checked_add(1).ok_or(bad)?;
        if first {
            let body = &page[27 + segments..];
            if body.starts_with(b"OpusHead") && body.len() >= 19 {
                let channels = body[9];
                if body[8] & 0xf0 != 0 || channels == 0 || channels > 2 {
                    return Err(bad);
                }
                rate = 48_000;
                pre_skip = u64::from(u16le(body, 10).ok_or(bad)?);
            } else if body.len() >= 30 && body.starts_with(b"\x01vorbis") {
                let channels = body[11];
                rate = u64::from(u32le(body, 12).ok_or(bad)?);
                if channels == 0 || channels > 2 || !(8_000..=96_000).contains(&rate) {
                    return Err(bad);
                }
            } else {
                return Err(bad);
            }
        }
        if granule >= 0 {
            last = last.max(granule);
        }
        at = end;
        if flags & 4 != 0 && at != b.len() {
            return Err(bad);
        }
    }
    if rate == 0 || sequence < 2 {
        return Err(bad);
    }
    let samples = (last as u64).checked_sub(pre_skip).ok_or(bad)?;
    Ok(samples * 1_000_000 / rate)
}

/// MPEG-1/2/2.5 layer III frames from the first sync to the end of the file,
/// after an optional ID3v2 tag and before an optional ID3v1 tag. A Xing/Info
/// frame is metadata, not audio; its LAME encoder delay and padding are not
/// part of the clip either.
fn mp3(b: &[u8]) -> Result<u64, Rejected> {
    let bad = Rejected::Unsupported;
    let mut at = 0usize;
    if b.starts_with(b"ID3") {
        let h = b.get(..10).ok_or(bad)?;
        if h[6..10].iter().any(|v| v & 0x80 != 0) {
            return Err(bad);
        }
        let size = h[6..10]
            .iter()
            .fold(0usize, |n, &v| (n << 7) | usize::from(v));
        at = 10 + size + if h[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let end = if b.len() >= 128 && &b[b.len() - 128..b.len() - 125] == b"TAG" {
        b.len() - 128
    } else {
        b.len()
    };
    let (mut frames, mut rate, mut per_frame, mut trimmed) = (0u64, 0u32, 0u64, 0u64);
    let mut first = true;
    while at < end {
        let h = b.get(at..at + 4).ok_or(bad)?;
        if h[0] != 0xff || h[1] & 0xe0 != 0xe0 {
            return Err(bad);
        }
        let version = (h[1] >> 3) & 3; // 0: 2.5, 2: 2, 3: 1
        let layer = (h[1] >> 1) & 3; // 1: layer III
        let bitrate_index = usize::from(h[2] >> 4);
        let rate_index = usize::from((h[2] >> 2) & 3);
        let padding = u32::from((h[2] >> 1) & 1);
        if version == 1
            || layer != 1
            || bitrate_index == 0
            || bitrate_index == 15
            || rate_index == 3
        {
            return Err(bad);
        }
        const V1: [u32; 15] = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ];
        const V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
        let mpeg1 = version == 3;
        let kbps = if mpeg1 { V1 } else { V2 }[bitrate_index];
        let sample_rate = [44_100, 48_000, 32_000][rate_index]
            >> match version {
                3 => 0,
                2 => 1,
                _ => 2,
            };
        let samples = if mpeg1 { 1152 } else { 576 };
        let length = (samples / 8 * kbps * 1000 / sample_rate + padding) as usize;
        if length < 4 || at + length > end {
            return Err(bad);
        }
        if rate != 0 && rate != sample_rate {
            return Err(bad);
        }
        rate = sample_rate;
        per_frame = u64::from(samples);
        let mono = h[3] >> 6 == 3;
        let side = match (mpeg1, mono) {
            (true, false) => 32,
            (true, true) => 17,
            (false, false) => 17,
            (false, true) => 9,
        };
        let tag = b.get(at + 4 + side..at + 8 + side);
        if first && (tag == Some(b"Xing") || tag == Some(b"Info")) {
            // LAME's extension follows the 120-byte Xing block.
            let lame = at + 4 + side + 120;
            if b.get(lame..lame + 4) == Some(b"LAME") {
                if let Some(d) = b.get(lame + 21..lame + 24) {
                    let delay = (u64::from(d[0]) << 4) | u64::from(d[1] >> 4);
                    let pad = (u64::from(d[1] & 0x0f) << 8) | u64::from(d[2]);
                    trimmed = delay + pad;
                }
            }
        } else {
            frames += 1;
        }
        first = false;
        at += length;
    }
    if frames == 0 || at != end {
        return Err(bad);
    }
    let samples = (frames * per_frame).saturating_sub(trimmed);
    Ok(samples * 1_000_000 / u64::from(rate))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PCM WAV file of `ms` milliseconds of silence.
    pub(crate) fn wav_clip(ms: u32) -> Vec<u8> {
        let rate = 16_000u32;
        let data = rate / 1000 * ms * 2;
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(36 + data).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2).to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data.to_le_bytes());
        // A quiet tone rather than silence, so a browser test can hear it play.
        for i in 0..data / 2 {
            let s = ((i as f32 * 0.07).sin() * 3000.0) as i16;
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    fn ogg_page(flags: u8, granule: i64, page: u32, body: &[u8]) -> Vec<u8> {
        let mut v = b"OggS\0".to_vec();
        v.push(flags);
        v.extend_from_slice(&granule.to_le_bytes());
        v.extend_from_slice(&7u32.to_le_bytes());
        v.extend_from_slice(&page.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        let mut lacing = vec![255u8; body.len() / 255];
        lacing.push((body.len() % 255) as u8);
        v.push(lacing.len() as u8);
        v.extend_from_slice(&lacing);
        v.extend_from_slice(body);
        let crc = ogg_crc(&v);
        v[22..26].copy_from_slice(&crc.to_le_bytes());
        v
    }

    pub(crate) fn opus_clip(ms: u64) -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 1]);
        head.extend_from_slice(&312u16.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&[0, 0, 0]);
        let mut v = ogg_page(2, 0, 0, &head);
        v.extend(ogg_page(0, 0, 1, b"OpusTags\0\0\0\0\0\0\0\0"));
        v.extend(ogg_page(4, (ms * 48 + 312) as i64, 2, &[0xf8; 40]));
        v
    }

    fn mp3_clip(frames: usize) -> Vec<u8> {
        // MPEG-1 layer III, 128 kbps, 44.1 kHz, no padding: 417-byte frames.
        let mut v = vec![];
        for _ in 0..frames {
            let mut f = vec![0u8; 417];
            f[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
            v.extend(f);
        }
        v
    }

    #[test]
    fn wav_duration_comes_from_data_and_format() {
        assert_eq!(
            inspect(&wav_clip(1500)),
            Ok(Clip {
                format: SoundFormat::Wav,
                duration_ms: 1500
            })
        );
        assert_eq!(inspect(&wav_clip(5200)).unwrap().duration_ms, 5200);
        assert_eq!(inspect(&wav_clip(5300)), Err(Rejected::TooLong));
        // A lying RIFF size or a truncated data chunk is not a WAV file.
        let mut lying = wav_clip(100);
        lying[4] ^= 0x40;
        assert_eq!(inspect(&lying), Err(Rejected::Unsupported));
        let mut cut = wav_clip(100);
        cut.truncate(cut.len() - 10);
        assert_eq!(inspect(&cut), Err(Rejected::Unsupported));
        // A byte rate that disagrees with the sample format is refused.
        let mut rate = wav_clip(100);
        rate[28] ^= 1;
        assert_eq!(inspect(&rate), Err(Rejected::Unsupported));
    }

    #[test]
    fn ogg_opus_duration_is_last_granule_less_pre_skip() {
        assert_eq!(
            inspect(&opus_clip(2000)),
            Ok(Clip {
                format: SoundFormat::Ogg,
                duration_ms: 2000
            })
        );
        assert_eq!(inspect(&opus_clip(6000)), Err(Rejected::TooLong));
        let mut corrupt = opus_clip(2000);
        let n = corrupt.len();
        corrupt[n - 1] ^= 1;
        assert_eq!(inspect(&corrupt), Err(Rejected::Unsupported));
        let mut trailing = opus_clip(2000);
        trailing.extend_from_slice(b"junk");
        assert_eq!(inspect(&trailing), Err(Rejected::Unsupported));
    }

    #[test]
    fn mp3_duration_counts_frames() {
        // 100 frames of 1152 samples at 44.1 kHz: 2.612 s.
        assert_eq!(
            inspect(&mp3_clip(100)),
            Ok(Clip {
                format: SoundFormat::Mp3,
                duration_ms: 2613
            })
        );
        assert_eq!(inspect(&mp3_clip(200)), Err(Rejected::TooLong));
        let mut tagged = b"ID3\x04\0\0\0\0\0\x05hello".to_vec();
        tagged.extend(mp3_clip(10));
        tagged.extend_from_slice(b"TAG");
        tagged.extend_from_slice(&[0; 125]);
        assert_eq!(inspect(&tagged).unwrap().duration_ms, 262);
        let mut garbage = mp3_clip(10);
        garbage.extend_from_slice(b"<script>");
        assert_eq!(inspect(&garbage), Err(Rejected::Unsupported));
    }

    #[test]
    fn anything_else_or_oversized_is_refused() {
        assert_eq!(inspect(b""), Err(Rejected::Unsupported));
        assert_eq!(inspect(b"<html></html>"), Err(Rejected::Unsupported));
        assert_eq!(
            inspect(&vec![0; MAX_SOUND_BYTES + 1]),
            Err(Rejected::TooLarge)
        );
        // Arbitrary bytes never panic.
        for seed in 0..2000u32 {
            let bytes: Vec<u8> = (0..(seed % 300))
                .map(|i| (seed.wrapping_mul(2654435761).wrapping_add(i * 40503) >> 13) as u8)
                .collect();
            for prefix in [&b""[..], b"RIFF", b"OggS", b"ID3", b"\xff\xfb"] {
                let mut v = prefix.to_vec();
                v.extend_from_slice(&bytes);
                let _ = inspect(&v);
            }
        }
    }
}
