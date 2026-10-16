//! PCM WAV decoding for event sounds.
//!
//! Accepts 8-bit and 16-bit PCM, mono or stereo. Stereo is mixed to mono.

pub fn decode(bytes: &[u8]) -> Option<(u32, Vec<i16>)> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12;
    let mut format = None;
    let mut data = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let start = offset + 8;
        let end = start.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        if id == b"fmt " && size >= 16 {
            format = Some(Fmt {
                audio: u16::from_le_bytes(bytes[start..start + 2].try_into().ok()?),
                channels: u16::from_le_bytes(bytes[start + 2..start + 4].try_into().ok()?),
                rate: u32::from_le_bytes(bytes[start + 4..start + 8].try_into().ok()?),
                bits: u16::from_le_bytes(bytes[start + 14..start + 16].try_into().ok()?),
            });
        } else if id == b"data" {
            data = Some(&bytes[start..end]);
        }
        offset = end + (size % 2);
    }
    let format = format?;
    let data = data?;
    if format.audio != 1 || format.rate == 0 || !matches!(format.channels, 1 | 2) {
        return None;
    }
    let samples = match format.bits {
        8 => decode_u8(data, format.channels),
        16 => decode_i16(data, format.channels),
        _ => return None,
    }?;
    Some((format.rate, samples))
}

struct Fmt {
    audio: u16,
    channels: u16,
    rate: u32,
    bits: u16,
}

fn decode_i16(data: &[u8], channels: u16) -> Option<Vec<i16>> {
    let width = channels as usize * 2;
    if width == 0 || !data.len().is_multiple_of(width) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / width);
    for frame in data.chunks_exact(width) {
        let left = i16::from_le_bytes([frame[0], frame[1]]);
        if channels == 1 {
            out.push(left);
        } else {
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            out.push(((i32::from(left) + i32::from(right)) / 2) as i16);
        }
    }
    Some(out)
}

fn decode_u8(data: &[u8], channels: u16) -> Option<Vec<i16>> {
    let width = channels as usize;
    if width == 0 || !data.len().is_multiple_of(width) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / width);
    for frame in data.chunks_exact(width) {
        let left = (i16::from(frame[0]) - 128) << 8;
        if channels == 1 {
            out.push(left);
        } else {
            let right = (i16::from(frame[1]) - 128) << 8;
            out.push(((i32::from(left) + i32::from(right)) / 2) as i16);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm16(rate: u32, channels: u16, frames: &[i16]) -> Vec<u8> {
        let data_len = (frames.len() * 2) as u32;
        let mut out = Vec::new();
        out.extend(b"RIFF");
        out.extend((36 + data_len).to_le_bytes());
        out.extend(b"WAVE");
        out.extend(b"fmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(channels) * 2).to_le_bytes());
        out.extend((channels * 2).to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(data_len.to_le_bytes());
        for sample in frames {
            out.extend(sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn decodes_mono_and_mixes_stereo() {
        let (rate, samples) = decode(&pcm16(8_000, 1, &[1000, -1000])).unwrap();
        assert_eq!(rate, 8_000);
        assert_eq!(samples, vec![1000, -1000]);
        let (_, mixed) = decode(&pcm16(8_000, 2, &[1000, 3000])).unwrap();
        assert_eq!(mixed, vec![2000]);
    }

    #[test]
    fn rejects_a_non_wav() {
        assert!(decode(b"not a wave").is_none());
    }
}
