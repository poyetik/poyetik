//! WAV files: 32-bit floating-point samples, channels interleaved.

use crate::Rendered;

/// Size of the header written by [`wav_header`], in bytes.
pub const HEADER_LEN: usize = 58;

/// What the RIFF size field counts besides the samples: the header after its first
/// 8 bytes (`RIFF` and the size field itself), that is [`HEADER_LEN`] − 8.
const RIFF_SIZE_OVERHEAD: u32 = 50;

/// Builds the header of a 32-bit float WAV file (`RIFF`, `fmt `, `fact` and `data`
/// chunk headers) for `samples` interleaved samples of `channels` channels.
///
/// # Errors
///
/// Returns a message when the sizes do not fit the WAV format (65 535 channels,
/// 4 GiB of data).
pub fn wav_header(rate: u32, channels: usize, samples: usize) -> Result<[u8; HEADER_LEN], String> {
    let channel_count =
        u16::try_from(channels).map_err(|_| format!("too many channels for WAV: {channels}"))?;
    let too_long = || format!("too long for WAV: {samples} samples");
    let sample_count = u32::try_from(samples).map_err(|_| too_long())?;
    let data_len = sample_count.checked_mul(4).ok_or_else(too_long)?;
    let riff_len = data_len
        .checked_add(RIFF_SIZE_OVERHEAD)
        .ok_or_else(too_long)?;
    let frames = sample_count
        .checked_div(u32::from(channel_count))
        .unwrap_or(0);
    let block_align = channel_count
        .checked_mul(4)
        .ok_or_else(|| format!("too many channels for WAV: {channels}"))?;
    let byte_rate = rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| format!("sample rate too high for WAV: {rate}"))?;
    let mut header = [0_u8; HEADER_LEN];
    let fields: [&[u8]; 17] = [
        b"RIFF",
        &riff_len.to_le_bytes(),
        b"WAVE",
        b"fmt ",
        &18_u32.to_le_bytes(),
        &3_u16.to_le_bytes(), // WAVE_FORMAT_IEEE_FLOAT
        &channel_count.to_le_bytes(),
        &rate.to_le_bytes(),
        &byte_rate.to_le_bytes(),
        &block_align.to_le_bytes(),
        &32_u16.to_le_bytes(),
        &0_u16.to_le_bytes(), // no extension
        b"fact",
        &4_u32.to_le_bytes(),
        &frames.to_le_bytes(),
        b"data",
        &data_len.to_le_bytes(),
    ];
    for (byte, value) in header
        .iter_mut()
        .zip(fields.iter().flat_map(|field| field.iter()))
    {
        *byte = *value;
    }
    Ok(header)
}

/// Appends samples, little-endian, after a header from [`wav_header`].
#[must_use]
pub fn wav_file(header: &[u8; HEADER_LEN], samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN.saturating_add(samples.len().saturating_mul(4)));
    bytes.extend_from_slice(header);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Encodes a render as a complete WAV file.
///
/// # Errors
///
/// Returns a message when the render does not fit the WAV format.
pub fn wav_bytes(rendered: &Rendered) -> Result<Vec<u8>, String> {
    let header = wav_header(
        rendered.rate.hz(),
        rendered.channels,
        rendered.samples.len(),
    )?;
    Ok(wav_file(&header, &rendered.samples))
}
