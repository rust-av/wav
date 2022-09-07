use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;

use av_data::{packet::Packet, value::Value};
use av_format::{common::GlobalInfo, error::*, muxer::*};

use crate::parser::Format;
use crate::{find_codec_from_wav_twocc, PCM_FLOAT_FORMAT_ID};

#[derive(Debug, Clone, PartialEq)]
pub struct WavMuxer {
    format: Format,
    data_pos: u64,
}

impl WavMuxer {
    pub fn new(format: Format) -> Self {
        Self {
            data_pos: 0,
            format,
        }
    }

    fn patch_size<W: Write>(bw: &mut Writer<W>, pos: u64) -> Result<()> {
        let size = bw.position() as u64 - pos;
        bw.seek(SeekFrom::Current(-((size + 4) as i64)))?;
        bw.write_all(&(size as u32).to_le_bytes())?;
        bw.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

impl Muxer for WavMuxer {
    fn configure(&mut self) -> Result<()> {
        Ok(())
    }

    fn write_header<W: Write>(&mut self, out: &mut Writer<W>) -> Result<()> {
        let edata_len = self.format.edata.as_ref().map(|buf| buf.len()).unwrap_or(0);

        if edata_len >= (1 << 16) {
            return Err(Error::InvalidData);
        }

        let codec_name = find_codec_from_wav_twocc(self.format.format_tag).unwrap_or("unknown");
        let twocc = if codec_name == "pcm" {
            if self.format.format_tag != PCM_FLOAT_FORMAT_ID {
                0x0001
            } else {
                PCM_FLOAT_FORMAT_ID
            }
        } else {
            self.format.format_tag
        };

        let avg_bytes_per_sec = if codec_name == "pcm" {
            (u32::from(self.format.channels)
                * self.format.samples_per_sec
                * u32::from(self.format.bits_per_sample))
                >> 3
        } else {
            0
        };

        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        buf.write_all(&((if edata_len == 0 { 16 } else { 18 + edata_len }) as u32).to_le_bytes())?;
        buf.write_all(&twocc.to_le_bytes())?;
        buf.write_all(&self.format.channels.to_le_bytes())?;
        buf.write_all(&self.format.samples_per_sec.to_le_bytes())?;
        buf.write_all(&avg_bytes_per_sec.to_le_bytes())?;
        buf.write_all(&self.format.block_align.to_le_bytes())?;
        buf.write_all(&self.format.bits_per_sample.to_le_bytes())?;
        if let Some(ref edata_buf) = self.format.edata {
            buf.write_all(&(edata_len as u16).to_le_bytes())?;
            buf.extend_from_slice(edata_buf);
        }
        buf.extend_from_slice(b"data\0\0\0\0");

        out.write_all(&buf)?;

        self.data_pos = out.position() as u64;
        Ok(())
    }

    fn write_packet<W: Write>(&mut self, out: &mut Writer<W>, pkt: Arc<Packet>) -> Result<()> {
        out.write_all(&pkt.data)?;
        Ok(())
    }

    fn write_trailer<W: Write>(&mut self, out: &mut Writer<W>) -> Result<()> {
        Self::patch_size(out, self.data_pos)?;
        Self::patch_size(out, 8)?;
        Ok(())
    }

    fn set_global_info(&mut self, _info: GlobalInfo) -> Result<()> {
        Ok(())
    }

    fn set_option<'a>(&mut self, _key: &str, _val: Value<'a>) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(non_upper_case_globals)]
mod tests {
    use std::io::Cursor;

    use av_format::{
        buffer::*,
        demuxer::{Context as DemuxerContext, Event},
        muxer::Context,
    };

    use crate::demuxer::WavDemuxer;

    use super::*;

    const wav: &[u8] = include_bytes!("../assets/scatter.wav");

    // Returns the remuxed stream
    fn demux_mux(data: &[u8]) -> Result<Vec<u8>> {
        let mut demuxer = DemuxerContext::new(WavDemuxer::new(), AccReader::new(Cursor::new(data)));

        println!("read headers: {:?}", demuxer.read_headers().unwrap());

        let mux = WavMuxer::new(demuxer.demuxer().format.clone());
        let writer = Writer::new(Cursor::new(Vec::new()));

        let mut muxer = Context::new(mux, writer);
        muxer.configure().unwrap();
        muxer.set_global_info(demuxer.info.clone()).unwrap();
        muxer.write_header().unwrap();

        loop {
            match demuxer.read_event() {
                Ok(event) => {
                    println!("event: {:?}", event);
                    match event {
                        Event::MoreDataNeeded(sz) => panic!("we needed more data: {} bytes", sz),
                        Event::NewStream(s) => panic!("new stream :{:?}", s),
                        Event::NewPacket(packet) => {
                            println!("writing packet {:?}", packet);
                            muxer.write_packet(Arc::new(packet)).unwrap();
                        }
                        Event::Continue => {
                            continue;
                        }
                        Event::Eof => {
                            muxer.write_trailer().unwrap();
                            break;
                        }
                        _ => break,
                    }
                }
                Err(e) => {
                    println!("error: {:?}", e);
                    break;
                }
            }
        }
        Ok(muxer.writer().as_ref().0.into_inner().to_owned())
    }

    #[test]
    fn remux() {
        let first_mux = demux_mux(wav).unwrap();
        assert_eq!(first_mux, demux_mux(&first_mux).unwrap());
    }
}
