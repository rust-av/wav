//! WAV Muxer and Demuxer
//!
//! To better understand the WAV format, read the
//! <a href="http://www-mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/WAVE.html" target="_blank">WAV Specification</a>.

pub mod demuxer;
pub mod muxer;
pub mod parser;

// A special case for floating-point audio
pub(crate) const PCM_FLOAT_FORMAT_ID: u16 = 0x0003;

static WAV_CODEC_REGISTER: &[(u16, &str)] = &[
    (0x0000, "unknown"),
    (0x0001, "pcm"),
    (0x0002, "ms-adpcm"),
    (PCM_FLOAT_FORMAT_ID, "pcm"),
    (0x0011, "ima-adpcm-ms"),
    (0x0061, "adpcm-dk4"),
    (0x0062, "adpcm-dk3"),
    (0x0401, "imc"),
    (0x0402, "iac"),
    (0x0500, "on2avc-500"),
    (0x0501, "on2avc-501"),
];

pub(crate) fn find_codec_from_wav_twocc(tcc: u16) -> Option<&'static str> {
    WAV_CODEC_REGISTER
        .iter()
        .find(|(twocc, _)| *twocc == tcc)
        .map(|(_, name)| *name)
}
