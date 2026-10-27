//! The response body: in memory while small, in a temporary file once it is big, so a
//! 500 MB answer costs disk, not RAM. Decompression happens on the way in.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use tempfile::TempPath;

#[derive(Debug, Clone)]
pub enum ResponseBody {
    Memory(Bytes),
    /// The file is deleted when the last clone of the body is dropped.
    File { path: Arc<TempPath>, len: u64 },
}

impl ResponseBody {
    pub fn len(&self) -> u64 {
        match self {
            ResponseBody::Memory(b) => b.len() as u64,
            ResponseBody::File { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn in_memory(&self) -> Option<&[u8]> {
        match self {
            ResponseBody::Memory(b) => Some(b),
            ResponseBody::File { .. } => None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            ResponseBody::Memory(_) => None,
            ResponseBody::File { path, .. } => Some(path),
        }
    }

    pub fn reader(&self) -> io::Result<Box<dyn Read + '_>> {
        Ok(match self {
            ResponseBody::Memory(b) => Box::new(&b[..]),
            ResponseBody::File { path, .. } => Box::new(io::BufReader::new(File::open(path.as_ref() as &Path)?)),
        })
    }

    /// The first `limit` bytes: enough for a preview without reading a huge file.
    pub fn head(&self, limit: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.reader()?.take(limit as u64).read_to_end(&mut out)?;
        Ok(out)
    }
}

/// Collects bytes in memory and moves them to a temporary file past `threshold`.
pub(crate) struct Sink {
    mem: Vec<u8>,
    file: Option<(BufWriter<File>, TempPath)>,
    len: u64,
    threshold: usize,
    dir: PathBuf,
}

impl Sink {
    pub(crate) fn new(threshold: usize, dir: PathBuf) -> Self {
        Sink { mem: Vec::new(), file: None, len: 0, threshold, dir }
    }

    fn spill(&mut self) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let (file, path) = tempfile::Builder::new().prefix("response-").tempfile_in(&self.dir)?.into_parts();
        let mut file = BufWriter::with_capacity(1 << 20, file);
        file.write_all(&self.mem)?;
        self.mem = Vec::new();
        self.file = Some((file, path));
        Ok(())
    }

    pub(crate) fn finish(self) -> io::Result<ResponseBody> {
        Ok(match self.file {
            None => ResponseBody::Memory(Bytes::from(self.mem)),
            Some((mut file, path)) => {
                file.flush()?;
                ResponseBody::File { path: Arc::new(path), len: self.len }
            }
        })
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.file.is_none() && self.mem.len() + buf.len() > self.threshold {
            self.spill()?;
        }
        match &mut self.file {
            Some((file, _)) => file.write_all(buf)?,
            None => self.mem.extend_from_slice(buf),
        }
        self.len += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.file {
            Some((file, _)) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Undoes `Content-Encoding` while writing into the sink.
pub(crate) enum Decoder {
    Identity(Sink),
    Gzip(flate2::write::MultiGzDecoder<Sink>),
    Deflate(flate2::write::ZlibDecoder<Sink>),
    Brotli(Box<brotli::DecompressorWriter<Sink>>),
}

impl Decoder {
    /// Unknown encodings are kept as they came.
    pub(crate) fn new(encoding: &str, sink: Sink) -> Self {
        match encoding {
            "gzip" | "x-gzip" => Decoder::Gzip(flate2::write::MultiGzDecoder::new(sink)),
            "deflate" => Decoder::Deflate(flate2::write::ZlibDecoder::new(sink)),
            "br" => Decoder::Brotli(Box::new(brotli::DecompressorWriter::new(sink, 64 * 1024))),
            _ => Decoder::Identity(sink),
        }
    }

    pub(crate) fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            Decoder::Identity(w) => w.write_all(data),
            Decoder::Gzip(w) => w.write_all(data),
            Decoder::Deflate(w) => w.write_all(data),
            Decoder::Brotli(w) => w.write_all(data),
        }
    }

    pub(crate) fn finish(self) -> io::Result<ResponseBody> {
        let sink = match self {
            Decoder::Identity(sink) => sink,
            Decoder::Gzip(w) => w.finish()?,
            Decoder::Deflate(w) => w.finish()?,
            Decoder::Brotli(mut w) => {
                w.close()?;
                w.into_inner().map_err(|_| io::Error::other("поток brotli оборван"))?
            }
        };
        sink.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spills_past_threshold_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = Sink::new(10, dir.path().to_path_buf());
        sink.write_all(b"12345").unwrap();
        sink.write_all(b"678901234").unwrap();
        let body = sink.finish().unwrap();
        let path = body.path().unwrap().to_path_buf();
        assert_eq!(std::fs::read(&path).unwrap(), b"12345678901234");
        assert_eq!(body.len(), 14);
        assert_eq!(body.head(3).unwrap(), b"123");
        drop(body);
        assert!(!path.exists());
    }

    #[test]
    fn decodes_gzip_in_pieces() {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all("привет ".repeat(1000).as_bytes()).unwrap();
        let packed = enc.finish().unwrap();
        let mut dec = Decoder::new("gzip", Sink::new(1 << 20, std::env::temp_dir()));
        for chunk in packed.chunks(7) {
            dec.write_all(chunk).unwrap();
        }
        let body = dec.finish().unwrap();
        assert_eq!(body.in_memory().unwrap(), "привет ".repeat(1000).as_bytes());
    }
}
