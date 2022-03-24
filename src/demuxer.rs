use std::io::SeekFrom;
use std::sync::Arc;

use nom::{Err, IResult, Needed, Offset};

use av_data::{
    audiosample::{ChannelMap, Soniton},
    packet::Packet,
    params::*,
    rational::Rational64,
    timeinfo::TimeInfo,
};
use av_format::{
    buffer::Buffered,
    common::GlobalInfo,
    demuxer::{Demuxer, Descr, Descriptor, Event},
    error::*,
    stream::Stream,
};

use crate::parser::{
    custom_error, get_data, header, parse_header_fmt, read_chunks_type, read_duration, skip_chunk,
    ErrorKind, Format,
};
use crate::{find_codec_from_wav_twocc, PCM_FLOAT_FORMAT_ID};

#[derive(Debug, Clone, Default)]
pub struct WavDemuxer {
    pub format: Format,
    data_pos: usize,
    data_end: usize,
    cname: &'static str,
    is_pcm: bool,
    duration: u64,
}

impl WavDemuxer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse_headers<'a>(
        &mut self,
        input: &'a [u8],
    ) -> IResult<&'a [u8], (), crate::parser::Error<'a>> {
        // Parse header and fmt chunk
        let (mut i, format) = parse_header_fmt(input)?;

        // Analyze fmt chunk
        self.analyze_fmt(format);

        while let Ok((inp, (ctype, csize))) = read_chunks_type(i) {
            i = match ctype {
                b"fact" => {
                    if csize != 4 {
                        return Err(Err::Error(custom_error(i, 1)));
                    }
                    let (i, duration) = read_duration(inp)?;
                    self.duration = duration as u64;
                    i
                }
                b"data" => {
                    self.data_pos = input.offset(inp);
                    self.data_end = self.data_pos + csize as usize;

                    self.duration = if self.duration != 0 {
                        (self.duration as u64) * 1000 / u64::from(self.format.samples_per_sec)
                    } else if self.format.avg_bytes_per_sec > 0 {
                        (self.data_end - self.data_pos) as u64 * 1000
                            / u64::from(self.format.avg_bytes_per_sec)
                    } else {
                        0
                    };

                    return Ok((inp, ()));
                }
                _ => skip_chunk(inp, csize as usize)?.0,
            };
        }
        Ok((i, ()))
    }

    fn analyze_fmt(&mut self, format: Format) {
        self.cname = find_codec_from_wav_twocc(format.format_tag).unwrap_or("unknown");
        self.is_pcm = self.cname == "pcm";
        self.format = format;
        self.format.avg_bytes_per_sec = if self.is_pcm && self.format.avg_bytes_per_sec == 0 {
            self.format.block_align as u32 * self.format.samples_per_sec
        } else {
            self.format.avg_bytes_per_sec
        };
    }
}

impl Demuxer for WavDemuxer {
    fn read_headers(&mut self, buf: &mut dyn Buffered, info: &mut GlobalInfo) -> Result<SeekFrom> {
        match self.parse_headers(buf.data()) {
            Ok((i, _)) => {
                let soniton = if self.cname == "pcm" {
                    if self.format.format_tag != PCM_FLOAT_FORMAT_ID {
                        if self.format.bits_per_sample == 8 {
                            Soniton::new(8, false, false, false, false, false)
                        } else {
                            Soniton::new(
                                self.format.bits_per_sample as u8,
                                false,
                                false,
                                false,
                                false,
                                true,
                            )
                        }
                    } else {
                        Soniton::new(
                            self.format.bits_per_sample as u8,
                            false,
                            false,
                            false,
                            true,
                            false,
                        )
                    }
                } else {
                    Soniton::new(
                        self.format.bits_per_sample as u8,
                        false,
                        false,
                        false,
                        false,
                        true,
                    )
                };
                let audio_info = AudioInfo {
                    rate: self.format.samples_per_sec as usize,
                    map: Some(ChannelMap::default_map(self.format.channels as usize)),
                    format: Some(Arc::new(soniton)),
                };
                let stream = Stream {
                    id: 0,
                    index: 0,
                    start: None,
                    duration: Some(self.duration),
                    timebase: Rational64::new(1, self.format.samples_per_sec as i64),
                    params: CodecParams {
                        extradata: self.format.edata.clone(),
                        bit_rate: 0,
                        delay: 0,
                        convergence_window: 0,
                        codec_id: Some(self.cname.to_owned()),
                        kind: Some(MediaKind::Audio(audio_info)),
                    },
                    user_private: None,
                };
                info.duration = Some(self.duration);
                info.add_stream(stream);
                Ok(SeekFrom::Current(buf.data().offset(i) as i64))
            }
            Err(Err::Incomplete(needed)) => {
                let sz = match needed {
                    Needed::Size(size) => buf.data().len() + usize::from(size),
                    _ => 1024,
                };
                Err(Error::MoreDataNeeded(sz))
            }
            _ => Err(Error::InvalidData),
        }
    }

    fn read_event(&mut self, buf: &mut dyn Buffered) -> Result<(SeekFrom, Event)> {
        let pts = if self.format.avg_bytes_per_sec != 0 {
            Some(
                self.data_pos as i64 * i64::from(self.format.samples_per_sec)
                    / i64::from(self.format.avg_bytes_per_sec),
            )
        } else {
            None
        };
        let time_info = TimeInfo {
            pts,
            dts: None,
            duration: None,
            timebase: None,
            user_private: None,
        };

        let block_size = if self.is_pcm {
            self.format.block_align << self.format.block_align.leading_zeros().saturating_sub(8)
        } else {
            self.format.block_align
        };

        match get_data(buf.data(), block_size as usize) {
            Ok((i, data)) => {
                let packet = Packet {
                    data: data.into(),
                    t: time_info,
                    pos: None,
                    stream_index: 0,
                    is_key: false,
                    is_corrupted: false,
                };

                self.data_pos = buf.data().offset(i);
                let seek = SeekFrom::Current(self.data_pos as i64);
                Ok((seek, Event::NewPacket(packet)))
            }
            Err(Err::Error(e)) => {
                if let ErrorKind::Nom(nom::error::ErrorKind::Eof) = e.kind {
                    Ok((SeekFrom::Current(0), Event::Eof))
                } else {
                    Err(Error::InvalidData)
                }
            }
            _ => Err(Error::InvalidData),
        }
    }
}

struct Des {
    d: Descr,
}

impl Descriptor for Des {
    type OutputDemuxer = WavDemuxer;

    fn create(&self) -> Self::OutputDemuxer {
        WavDemuxer::new()
    }
    fn describe(&self) -> &Descr {
        &self.d
    }
    fn probe(&self, data: &[u8]) -> u8 {
        header(&data[..12]).map_or(0, |_| 12)
    }
}

pub const WAV_DESC: &dyn Descriptor<OutputDemuxer = WavDemuxer> = &Des {
    d: Descr {
        name: "wav",
        demuxer: "wav",
        description: "Nom-based WAV demuxer",
        extensions: &["wav"],
        mime: &["audio/x-wav"],
    },
};

#[cfg(test)]
#[allow(non_upper_case_globals)]
mod tests {
    use std::io::Cursor;

    use av_format::{buffer::*, demuxer::Context};

    use super::*;

    const wav: &[u8] = include_bytes!("../assets/scatter.wav");

    #[test]
    fn context() {
        let mut context = Context::new(WavDemuxer::new(), AccReader::new(Cursor::new(wav)));

        println!("{:?}", context.read_headers().unwrap());

        while let Ok(event) = context.read_event() {
            println!("event: {:?}", event);
            if let Event::Eof = event {
                break;
            }
        }
    }
}
