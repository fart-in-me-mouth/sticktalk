use crate::{Error, Result};

/// MF normally returns Annex-B. Some hardware returns 4-byte length-prefixed
/// NALs. Validate every length before converting; never publish mixed framing.
pub(crate) fn normalize(data: &[u8]) -> Result<Vec<u8>> {
    if data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]) {
        return Ok(data.to_vec());
    }
    let mut out = Vec::with_capacity(data.len() + 32);
    let mut pos = 0;
    while pos < data.len() {
        let prefix = data
            .get(pos..pos + 4)
            .ok_or_else(|| Error::Platform("truncated NAL prefix".into()))?;
        let len = u32::from_be_bytes(prefix.try_into().unwrap()) as usize;
        pos += 4;
        if len == 0 || len > data.len() - pos {
            return Err(Error::Platform("invalid NAL length".into()));
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[pos..pos + len]);
        pos += len;
    }
    Ok(out)
}

pub(crate) fn contains(data: &[u8], kind: u8) -> bool {
    data.windows(4)
        .any(|w| w[..3] == [0, 0, 1] && w[3] & 31 == kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_and_reject_malformed() {
        let avcc = [0, 0, 0, 2, 0x65, 0xab, 0, 0, 0, 1, 0x67];
        let annexb = normalize(&avcc).unwrap();
        assert!(contains(&annexb, 5));
        assert!(contains(&annexb, 7));
        assert_eq!(normalize(&annexb).unwrap(), annexb);
        assert!(normalize(&[0, 0, 0, 9, 3]).is_err());
        assert!(normalize(&[0]).is_err());
    }
}
