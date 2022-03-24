use nom::{
    bytes::complete::{tag, take},
    combinator::{cond, map, verify},
    number::complete::{le_u16, le_u32},
    sequence::{pair, tuple},
    Err, IResult,
};

const WAVEFORMAT: usize = 16;
const WAVEFORMATEX: usize = 18;

#[derive(Debug, PartialEq)]
pub struct Error<'a> {
    input: &'a [u8],
    pub(crate) kind: ErrorKind,
}

#[derive(Debug, PartialEq)]
pub enum ErrorKind {
    Nom(nom::error::ErrorKind),
    Custom(u8),
}

impl<'a> nom::error::ParseError<&'a [u8]> for Error<'a> {
    fn from_error_kind(input: &'a [u8], kind: nom::error::ErrorKind) -> Self {
        Error {
            input,
            kind: ErrorKind::Nom(kind),
        }
    }

    fn append(_input: &'a [u8], _kind: nom::error::ErrorKind, other: Self) -> Self {
        other
    }
}

pub(crate) fn custom_error(input: &[u8], code: u8) -> Error {
    Error {
        input,
        kind: ErrorKind::Custom(code),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header<'a> {
    magic1: &'a [u8],
    pub file_size: u32,
    magic2: &'a [u8],
}

pub(crate) fn header(input: &[u8]) -> IResult<&[u8], Header, Error> {
    map(
        tuple((tag(b"RIFF"), le_u32, tag(b"WAVE"))),
        |(magic1, file_size, magic2)| Header {
            magic1,
            file_size,
            magic2,
        },
    )(input)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Format {
    pub format_tag: u16,
    pub channels: u16,
    pub samples_per_sec: u32,
    pub avg_bytes_per_sec: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    pub edata: Option<Vec<u8>>,
}

fn extradata(chunk_size: usize) -> impl Fn(&[u8]) -> IResult<&[u8], Option<Vec<u8>>, Error> {
    move |input| {
        if chunk_size < WAVEFORMATEX - 1 {
            return Err(Err::Error(custom_error(input, 100)));
        }
        le_u16(input).and_then(|(i, size)| {
            cond(chunk_size >= WAVEFORMATEX, take(size))(i)
                .map(|(i, data)| (i, data.map(|v| v.to_owned())))
        })
    }
}

fn bits_per_sample(chunk_size: usize) -> impl Fn(&[u8]) -> IResult<&[u8], u16, Error> {
    move |input| {
        if chunk_size >= WAVEFORMAT {
            le_u16(input)
        } else {
            Ok((input, 8))
        }
    }
}

fn parse_fmt(input: &[u8]) -> IResult<&[u8], Format, Error> {
    verify(read_chunks_type, |t| t.0 == b"fmt ")(input).and_then(|(i, (_, chunk_size))| {
        map(
            tuple((
                le_u16,
                le_u16,
                le_u32,
                le_u32,
                le_u16,
                bits_per_sample(chunk_size as usize),
                extradata(chunk_size as usize),
            )),
            |t| Format {
                format_tag: t.0,
                channels: t.1,
                samples_per_sec: t.2,
                avg_bytes_per_sec: t.3,
                block_align: t.4,
                bits_per_sample: t.5,
                edata: t.6,
            },
        )(i)
    })
}

pub(crate) fn parse_header_fmt(input: &[u8]) -> IResult<&[u8], Format, Error> {
    pair(header, parse_fmt)(input).map(|(i, (_, format))| (i, format))
}

pub(crate) fn read_duration(input: &[u8]) -> IResult<&[u8], u32, Error> {
    le_u32(input)
}

pub(crate) fn skip_chunk(input: &[u8], chunk_size: usize) -> IResult<&[u8], &[u8], Error> {
    take(chunk_size)(input)
}

pub(crate) fn get_data(input: &[u8], data_size: usize) -> IResult<&[u8], &[u8], Error> {
    take(data_size)(input)
}

pub(crate) fn read_chunks_type(input: &[u8]) -> IResult<&[u8], (&[u8], u32), Error> {
    pair(take(4usize), le_u32)(input)
}
