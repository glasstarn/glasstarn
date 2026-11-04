//! A kept copy of the rows of a file that is slow to read.
//!
//! Reading a large XES file means parsing XML several times per question
//! (find the columns, sample, check, prepare). The first complete read writes
//! the rows to a compact file in the scratch folder; later reads of the same
//! unchanged file stream that instead. The copy holds exactly the rows the
//! reader produced, row numbers included, so answers are identical.
//!
//! A copy is kept only once a read has run to the end, so a sample or a
//! failed read never leaves a partial copy behind.

use crate::read::{ReadError, Row, RowStream, Schema};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const MAGIC: &[u8; 8] = b"PMROWS\0\x01";
/// Copies kept at once; each is a large file.
const KEEP: usize = 3;

/// The scratch folder for kept copies.
pub fn default_folder() -> PathBuf {
    std::env::temp_dir().join("glasstarn-rows")
}

/// Streams `path` through a kept copy in `folder`, or reads it with `open`
/// and keeps a copy for next time. `revision` changes whenever the reader
/// would produce different rows, so old copies are not reused.
pub fn stream(
    path: &Path,
    folder: &Path,
    revision: u32,
    open: impl FnOnce() -> Result<RowStream, ReadError>,
) -> Result<RowStream, ReadError> {
    let Some(file) = copy_name(path, folder, revision) else {
        return open();
    };
    if file.is_file() {
        if let Ok(stream) = read_copy(&file) {
            let _ = touch(&file);
            return Ok(stream);
        }
        let _ = std::fs::remove_file(&file);
    }
    let (schema, rows) = open()?;
    match Writer::start(folder, &file, &schema) {
        Some(writer) => Ok((
            Rc::clone(&schema),
            Box::new(Keeping {
                rows,
                writer: Some(writer),
            }),
        )),
        None => Ok((schema, rows)),
    }
}

/// The copy's file name: a hash of the path, size, change time and revision.
fn copy_name(path: &Path, folder: &Path, revision: u32) -> Option<PathBuf> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    // FNV-1a.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in format!(
        "{}|{}|{modified}|{revision}",
        canonical.display(),
        metadata.len()
    )
    .bytes()
    {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    Some(folder.join(format!("pm-rows-{hash:016x}.rows")))
}

/// Passes rows through while writing them to the copy.
struct Keeping {
    rows: Box<dyn Iterator<Item = Result<Row, ReadError>>>,
    writer: Option<Writer>,
}

impl Iterator for Keeping {
    type Item = Result<Row, ReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.rows.next();
        match &next {
            Some(Ok(row)) => {
                if let Some(writer) = &mut self.writer
                    && writer.row(row).is_err()
                {
                    self.writer = None;
                }
            }
            // A read error means the copy would be incomplete.
            Some(Err(_)) => self.writer = None,
            None => {
                if let Some(writer) = self.writer.take() {
                    writer.finish();
                }
            }
        }
        next
    }
}

/// Writes a copy under a temporary name, renamed into place when complete.
/// Dropped before then, the temporary file is removed.
struct Writer {
    out: BufWriter<File>,
    building: PathBuf,
    file: PathBuf,
    folder: PathBuf,
    previous: Vec<String>,
}

impl Writer {
    fn start(folder: &Path, file: &Path, schema: &Schema) -> Option<Self> {
        std::fs::create_dir_all(folder).ok()?;
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let building = file.with_extension(format!(
            "{}-{}.building",
            std::process::id(),
            RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let mut out = BufWriter::with_capacity(1 << 20, File::create(&building).ok()?);
        let mut header = || -> std::io::Result<()> {
            out.write_all(MAGIC)?;
            varint(&mut out, schema.headers().len() as u64)?;
            for name in schema.headers() {
                text(&mut out, name)?;
            }
            Ok(())
        };
        if header().is_err() {
            let _ = std::fs::remove_file(&building);
            return None;
        }
        Some(Self {
            out,
            building,
            file: file.to_owned(),
            folder: folder.to_owned(),
            previous: vec![String::new(); schema.headers().len()],
        })
    }

    /// One row: its number plus one, then per column either 0 for "same as
    /// the row before" or the length plus one and the text. Trace attributes
    /// repeat on every step, so most columns cost one byte.
    fn row(&mut self, row: &Row) -> std::io::Result<()> {
        varint(&mut self.out, row.source_row as u64 + 1)?;
        for (index, previous) in self.previous.iter_mut().enumerate() {
            let value = row.at(index);
            if value == previous.as_str() {
                varint(&mut self.out, 0)?;
            } else {
                varint(&mut self.out, value.len() as u64 + 1)?;
                self.out.write_all(value.as_bytes())?;
                value.clone_into(previous);
            }
        }
        Ok(())
    }

    fn finish(mut self) {
        let done = varint(&mut self.out, 0).and_then(|()| self.out.flush());
        if done.is_ok() && std::fs::rename(&self.building, &self.file).is_ok() {
            evict(&self.folder, &self.file);
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // After a successful rename this finds nothing to remove.
        let _ = std::fs::remove_file(&self.building);
    }
}

fn read_copy(file: &Path) -> Result<RowStream, ReadError> {
    let display = file.display().to_string();
    let io = |source| ReadError::Io {
        path: display.clone(),
        source,
    };
    let mut input = BufReader::with_capacity(1 << 20, File::open(file).map_err(io)?);
    let mut magic = [0u8; 8];
    input.read_exact(&mut magic).map_err(io)?;
    if &magic != MAGIC {
        return Err(io(std::io::Error::other("not a kept copy of rows")));
    }
    let columns = read_varint(&mut input).map_err(io)? as usize;
    let headers = (0..columns)
        .map(|_| read_text(&mut input))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(io)?;
    let schema = Rc::new(Schema::new(headers));
    let rows = CopyRows {
        input,
        schema: Rc::clone(&schema),
        previous: vec![String::new(); columns],
        display,
        done: false,
    };
    Ok((schema, Box::new(rows)))
}

struct CopyRows {
    input: BufReader<File>,
    schema: Rc<Schema>,
    previous: Vec<String>,
    display: String,
    done: bool,
}

impl CopyRows {
    fn read_row(&mut self) -> std::io::Result<Option<Row>> {
        let number = read_varint(&mut self.input)?;
        if number == 0 {
            return Ok(None);
        }
        for previous in &mut self.previous {
            let length = read_varint(&mut self.input)?;
            if length > 0 {
                *previous = read_exact_text(&mut self.input, length as usize - 1)?;
            }
        }
        Ok(Some(Row::new(
            Rc::clone(&self.schema),
            self.previous.clone(),
            number as usize - 1,
        )))
    }
}

impl Iterator for CopyRows {
    type Item = Result<Row, ReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.read_row() {
            Ok(Some(row)) => Some(Ok(row)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(source) => {
                self.done = true;
                Some(Err(ReadError::Io {
                    path: self.display.clone(),
                    source,
                }))
            }
        }
    }
}

fn varint(out: &mut impl Write, mut value: u64) -> std::io::Result<()> {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            return out.write_all(&[byte]);
        }
        out.write_all(&[byte | 0x80])?;
    }
}

fn read_varint(input: &mut impl Read) -> std::io::Result<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let mut byte = [0u8];
        input.read_exact(&mut byte)?;
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(std::io::Error::other(
        "a number in the kept copy is too long",
    ))
}

fn text(out: &mut impl Write, value: &str) -> std::io::Result<()> {
    varint(out, value.len() as u64)?;
    out.write_all(value.as_bytes())
}

fn read_text(input: &mut impl Read) -> std::io::Result<String> {
    let length = read_varint(input)? as usize;
    read_exact_text(input, length)
}

fn read_exact_text(input: &mut impl Read, length: usize) -> std::io::Result<String> {
    let mut bytes = vec![0u8; length];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(std::io::Error::other)
}

fn touch(file: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .append(true)
        .open(file)?
        .set_modified(std::time::SystemTime::now())
}

/// Keeps the most recently used copies in `folder`, and `keep`.
fn evict(folder: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut copies = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("pm-rows-") && name.ends_with(".rows") && e.path() != keep
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect::<Vec<_>>();
    copies.sort();
    let surplus = (copies.len() + 1).saturating_sub(KEEP);
    for (_, path) in copies.into_iter().take(surplus) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pm-row-cache-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn source(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("log.csv");
        std::fs::write(
            &path,
            "case,activity,note\nC1,Create,\"two\nlines\"\nC1,Pay,\nC2,Create,é\n",
        )
        .unwrap();
        path
    }

    fn rows(stream: RowStream) -> (Vec<String>, Vec<(usize, Vec<String>)>) {
        let (schema, rows) = stream;
        let headers = schema.headers().to_vec();
        let rows = rows
            .map(|r| {
                let r = r.unwrap();
                let values = (0..headers.len()).map(|i| r.at(i).to_owned()).collect();
                (r.source_row, values)
            })
            .collect();
        (headers, rows)
    }

    fn copies(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn a_kept_copy_gives_the_same_rows_without_reading_the_file() {
        let dir = folder("same");
        let path = source(&dir);
        let cache = dir.join("cache");
        let direct = rows(crate::read::stream_csv(&path).unwrap());

        let first = rows(stream(&path, &cache, 1, || crate::read::stream_csv(&path)).unwrap());
        assert_eq!(first, direct);
        assert_eq!(copies(&cache).len(), 1, "a complete read keeps a copy");

        let second = rows(
            stream(&path, &cache, 1, || -> Result<RowStream, ReadError> {
                panic!("the file must not be read again")
            })
            .unwrap(),
        );
        assert_eq!(second, direct, "values and row numbers are unchanged");
    }

    #[test]
    fn a_partial_read_keeps_nothing() {
        let dir = folder("partial");
        let path = source(&dir);
        let cache = dir.join("cache");
        let (_, rows) = stream(&path, &cache, 1, || crate::read::stream_csv(&path)).unwrap();
        assert_eq!(rows.take(1).count(), 1);
        assert!(copies(&cache).is_empty(), "{:?}", copies(&cache));
    }

    #[test]
    fn a_changed_file_or_reader_is_read_afresh() {
        let dir = folder("changed");
        let path = source(&dir);
        let cache = dir.join("cache");
        let _ = rows(stream(&path, &cache, 1, || crate::read::stream_csv(&path)).unwrap());

        let mut read = false;
        let _ = rows(
            stream(&path, &cache, 2, || {
                read = true;
                crate::read::stream_csv(&path)
            })
            .unwrap(),
        );
        assert!(read, "a new reader revision does not reuse the old copy");

        std::fs::write(&path, "case,activity,note\nC9,Create,x\n").unwrap();
        let after = rows(stream(&path, &cache, 2, || crate::read::stream_csv(&path)).unwrap());
        assert_eq!(after.1.len(), 1);
        assert_eq!(after.1[0].1[0], "C9");
    }

    #[test]
    fn keeps_only_a_few_copies() {
        let dir = folder("evict");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..5 {
            let path = dir.join(format!("log{n}.csv"));
            std::fs::write(&path, format!("case,activity\nC{n},A\n")).unwrap();
            let _ = rows(stream(&path, &cache, 1, || crate::read::stream_csv(&path)).unwrap());
        }
        assert_eq!(copies(&cache).len(), KEEP);
    }

    #[test]
    fn a_damaged_copy_is_replaced() {
        let dir = folder("damaged");
        let path = source(&dir);
        let cache = dir.join("cache");
        let direct = rows(crate::read::stream_csv(&path).unwrap());
        let _ = rows(stream(&path, &cache, 1, || crate::read::stream_csv(&path)).unwrap());
        let copy = cache.join(&copies(&cache)[0]);
        std::fs::write(&copy, b"junk").unwrap();
        let again = rows(stream(&path, &cache, 1, || crate::read::stream_csv(&path)).unwrap());
        assert_eq!(again, direct);
    }
}
