//! A minimal ZIP archive writer: stored entries, std only.
//!
//! The retired `.hnmod` bundle used the `zip` crate. An SDK mod may not add a registry
//! dependency (it would have to be pinned with `horizon-launcher native-lock`, which the
//! campaign cannot run here), so this module writes the archive the port needs with nothing
//! but `std`: local file headers, a central directory, the end-of-central-directory record,
//! and ZIP64 records when a file or the archive passes 4 GiB or 65535 entries.
//!
//! Entries are **stored** (method 0, no compression): deflate needs a compressor, and a stored
//! entry is what every ZIP reader accepts. The behaviour difference from 0.1.0 — a backup is the
//! size of the world instead of deflated — is recorded in `notes.md`.

use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::Path;

/// Signature of a local file header.
const LOCAL_HEADER: u32 = 0x0403_4b50;
/// Signature of a central-directory entry.
const CENTRAL_HEADER: u32 = 0x0201_4b50;
/// Signature of the end-of-central-directory record.
const END_OF_CENTRAL: u32 = 0x0605_4b50;
/// Signature of the ZIP64 end-of-central-directory record.
const ZIP64_END_OF_CENTRAL: u32 = 0x0606_4b50;
/// Signature of the ZIP64 end-of-central-directory locator.
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
/// The ZIP64 extended-information extra field.
const ZIP64_EXTRA: u16 = 0x0001;
/// General-purpose flag: the name is UTF-8 (`EFS`, bit 11).
const FLAG_UTF8: u16 = 0x0800;
/// Stored: no compression.
const METHOD_STORED: u16 = 0;
/// The entry's CRC and sizes follow the data instead of the local header (bit 3), which is
/// what makes a one-pass copy of a file that is still being written sound.
const FLAG_DATA_DESCRIPTOR: u16 = 0x0008;
/// The data descriptor's signature.
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;
/// 1980-01-01, the DOS epoch: the oldest date a ZIP field can carry.
const DOS_DATE_1980: u16 = 0x0021;
/// The largest size or offset a 32-bit field holds.
const MAX_32: u64 = 0xFFFF_FFFF;
/// The largest entry count a 16-bit field holds.
const MAX_16: u16 = 0xFFFF;

/// One entry, as the central directory needs it.
struct Entry {
    name: String,
    crc: u32,
    bytes: u64,
    offset: u64,
}

/// A ZIP archive builder over a seekable writer.
pub struct ZipWriter<W: Write + Seek> {
    out: W,
    entries: Vec<Entry>,
    offset: u64,
    /// Write ZIP64 records regardless of size, so the path a >4 GiB world takes is testable.
    force_zip64: bool,
}

impl<W: Write + Seek> ZipWriter<W> {
    /// Start an archive at the writer's current position.
    pub fn new(out: W) -> Self {
        Self {
            out,
            entries: Vec::new(),
            offset: 0,
            force_zip64: false,
        }
    }

    /// Ask for ZIP64 records even when every field would fit in 32 bits (tests only).
    #[cfg(test)]
    pub fn force_zip64(&mut self) {
        self.force_zip64 = true;
    }

    /// Add the file at `path` as the stored entry `name`.
    ///
    /// The file is copied in **one** pass and its CRC and sizes travel in a data descriptor
    /// after it (flag bit 3), so a file that changes between a checksum pass and a copy pass
    /// cannot put a wrong CRC in the header: the archive always matches the bytes it holds.
    /// (0.2.0 read the file twice and a save or a chunk write in between produced an entry
    /// whose CRC failed on read-back.)
    ///
    /// # Errors
    ///
    /// The file could not be read, or the writer failed.
    pub fn add_file(&mut self, name: &str, path: &Path) -> io::Result<()> {
        let name_len = u16::try_from(name.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "the entry name is too long")
        })?;
        let force = self.force_zip64;
        let local_offset = self.offset;
        let mut header = Vec::with_capacity(30 + name.len());
        put_u32(&mut header, LOCAL_HEADER);
        put_u16(&mut header, if force { 45 } else { 20 });
        put_u16(&mut header, FLAG_UTF8 | FLAG_DATA_DESCRIPTOR);
        put_u16(&mut header, METHOD_STORED);
        put_u16(&mut header, 0);
        put_u16(&mut header, DOS_DATE_1980);
        put_u32(&mut header, 0);
        let placeholder = if force { MAX_32 as u32 } else { 0 };
        put_u32(&mut header, placeholder);
        put_u32(&mut header, placeholder);
        put_u16(&mut header, name_len);
        put_u16(&mut header, 0);
        header.extend_from_slice(name.as_bytes());
        self.write(&header)?;
        let (crc, bytes) = self.copy_computing(path)?;
        let zip64 = force || bytes >= MAX_32;
        let mut descriptor = Vec::with_capacity(24);
        put_u32(&mut descriptor, DATA_DESCRIPTOR);
        put_u32(&mut descriptor, crc);
        if zip64 {
            put_u64(&mut descriptor, bytes);
            put_u64(&mut descriptor, bytes);
        } else {
            put_u32(&mut descriptor, bytes as u32);
            put_u32(&mut descriptor, bytes as u32);
        }
        self.write(&descriptor)?;

        self.entries.push(Entry {
            name: name.to_owned(),
            crc,
            bytes,
            offset: local_offset,
        });
        Ok(())
    }

    /// Write the central directory and the end record, and return the writer.
    ///
    /// # Errors
    ///
    /// The writer failed.
    pub fn finish(mut self) -> io::Result<W> {
        let cd_offset = self.offset;
        for index in 0..self.entries.len() {
            let entry = &self.entries[index];
            let name = entry.name.clone();
            let name_len = name.len() as u16;
            let zip64 = self.force_zip64 || entry.bytes >= MAX_32 || entry.offset >= MAX_32;
            let mut record = Vec::with_capacity(46 + name.len() + 28);
            put_u32(&mut record, CENTRAL_HEADER);
            put_u16(&mut record, 20);
            put_u16(&mut record, if zip64 { 45 } else { 20 });
            put_u16(&mut record, FLAG_UTF8);
            put_u16(&mut record, METHOD_STORED);
            put_u16(&mut record, 0);
            put_u16(&mut record, DOS_DATE_1980);
            put_u32(&mut record, entry.crc);
            let sizes = if zip64 {
                MAX_32 as u32
            } else {
                entry.bytes as u32
            };
            put_u32(&mut record, sizes);
            put_u32(&mut record, sizes);
            put_u16(&mut record, name_len);
            put_u16(&mut record, if zip64 { 28 } else { 0 });
            put_u16(&mut record, 0);
            put_u16(&mut record, 0);
            put_u16(&mut record, 0);
            put_u32(&mut record, 0);
            put_u32(
                &mut record,
                if zip64 {
                    MAX_32 as u32
                } else {
                    entry.offset as u32
                },
            );
            record.extend_from_slice(name.as_bytes());
            if zip64 {
                put_u16(&mut record, ZIP64_EXTRA);
                put_u16(&mut record, 24);
                put_u64(&mut record, entry.bytes);
                put_u64(&mut record, entry.bytes);
                put_u64(&mut record, entry.offset);
            }
            self.write(&record)?;
        }
        let cd_size = self.offset - cd_offset;
        let count = u64::try_from(self.entries.len()).unwrap_or(u64::MAX);
        let needs_zip64 = self.force_zip64
            || count > u64::from(MAX_16)
            || cd_offset >= MAX_32
            || cd_size >= MAX_32;

        if needs_zip64 {
            let zip64_end = self.offset;
            let mut record = Vec::with_capacity(56);
            put_u32(&mut record, ZIP64_END_OF_CENTRAL);
            put_u64(&mut record, 44);
            put_u16(&mut record, 45);
            put_u16(&mut record, 45);
            put_u32(&mut record, 0);
            put_u32(&mut record, 0);
            put_u64(&mut record, count);
            put_u64(&mut record, count);
            put_u64(&mut record, cd_size);
            put_u64(&mut record, cd_offset);
            self.write(&record)?;

            let mut locator = Vec::with_capacity(20);
            put_u32(&mut locator, ZIP64_LOCATOR);
            put_u32(&mut locator, 0);
            put_u64(&mut locator, zip64_end);
            put_u32(&mut locator, 1);
            self.write(&locator)?;
        }

        let mut end = Vec::with_capacity(22);
        put_u32(&mut end, END_OF_CENTRAL);
        put_u16(&mut end, 0);
        put_u16(&mut end, 0);
        put_u16(&mut end, if needs_zip64 { MAX_16 } else { count as u16 });
        put_u16(&mut end, if needs_zip64 { MAX_16 } else { count as u16 });
        put_u32(
            &mut end,
            if needs_zip64 {
                MAX_32 as u32
            } else {
                cd_size as u32
            },
        );
        put_u32(
            &mut end,
            if needs_zip64 {
                MAX_32 as u32
            } else {
                cd_offset as u32
            },
        );
        put_u16(&mut end, 0);
        self.write(&end)?;
        self.out.flush()?;
        Ok(self.out)
    }

    /// Write bytes and count them.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.offset += bytes.len() as u64;
        Ok(())
    }

    /// Copy `path` after the writer in one pass, returning its CRC-32 and length. A file that
    /// grows is copied to its end; one that shrinks cannot make the header lie, because the
    /// header carries no size.
    fn copy_computing(&mut self, path: &Path) -> io::Result<(u32, u64)> {
        let mut source = File::open(path)?;
        let mut buffer = [0u8; 64 * 1024];
        let mut crc = 0xFFFF_FFFFu32;
        let mut bytes = 0u64;
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            crc = crc_update(crc, &buffer[..read]);
            bytes += read as u64;
            self.write(&buffer[..read])?;
        }
        Ok((crc ^ 0xFFFF_FFFF, bytes))
    }
}

/// The CRC-32 table (polynomial 0xEDB88320, the ZIP/Gzip one).
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0xEDB8_8320
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
};

/// Fold one buffer into a running CRC-32 (the caller seeds and finishes it).
fn crc_update(crc: u32, bytes: &[u8]) -> u32 {
    let mut value = crc;
    for byte in bytes {
        value = CRC_TABLE[((value ^ u32::from(*byte)) & 0xFF) as usize] ^ (value >> 8);
    }
    value
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A scratch directory that removes itself.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "simplebackups-zip-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).expect("scratch file");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What an independent reader recovers from an archive.
    #[derive(Debug, PartialEq, Eq)]
    struct ReadEntry {
        name: String,
        crc: u32,
        bytes: Vec<u8>,
    }

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn u64_at(bytes: &[u8], at: usize) -> u64 {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[at..at + 8]);
        u64::from_le_bytes(word)
    }

    /// The 8-byte values a ZIP64 extra field carries, in the order the 0xFFFFFFFF placeholders
    /// named them: uncompressed size, compressed size, local-header offset.
    fn zip64_fields(extra: &[u8]) -> Vec<u64> {
        let mut at = 0;
        while at + 4 <= extra.len() {
            let id = u16::from_le_bytes([extra[at], extra[at + 1]]);
            let len = usize::from(u16::from_le_bytes([extra[at + 2], extra[at + 3]]));
            if id == ZIP64_EXTRA {
                let fields = &extra[at + 4..at + 4 + len];
                assert!(fields.len().is_multiple_of(8), "whole 8-byte values");
                return (0..fields.len() / 8)
                    .map(|index| u64_at(fields, index * 8))
                    .collect();
            }
            at += 4 + len;
        }
        Vec::new()
    }

    /// Walk an archive the way a reader does: end record, central directory, local headers.
    fn read_archive(bytes: &[u8]) -> Vec<ReadEntry> {
        let eocd = (0..=bytes.len().saturating_sub(22))
            .rev()
            .find(|at| u32_at(bytes, *at) == END_OF_CENTRAL)
            .expect("an end-of-central-directory record");
        let mut count = usize::from(u16_at(bytes, eocd + 10));
        let mut cd_offset = u64::from(u32_at(bytes, eocd + 16));
        let cd_size = u64::from(u32_at(bytes, eocd + 12));
        if count == usize::from(MAX_16) || cd_offset == MAX_32 || cd_size == MAX_32 {
            let locator = eocd - 20;
            assert_eq!(
                u32_at(bytes, locator),
                ZIP64_LOCATOR,
                "the locator precedes the end record"
            );
            let zip64 = u64_at(bytes, locator + 8) as usize;
            assert_eq!(
                u32_at(bytes, zip64),
                ZIP64_END_OF_CENTRAL,
                "the ZIP64 end record is where the locator says"
            );
            count = u64_at(bytes, zip64 + 32) as usize;
            cd_offset = u64_at(bytes, zip64 + 48);
        }
        let mut at = cd_offset as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            assert_eq!(u32_at(bytes, at), CENTRAL_HEADER, "a central entry");
            assert_eq!(u16_at(bytes, at + 10), METHOD_STORED, "stored");
            assert_ne!(u16_at(bytes, at + 8) & FLAG_UTF8, 0, "the name is UTF-8");
            let crc = u32_at(bytes, at + 16);
            let name_len = usize::from(u16_at(bytes, at + 28));
            let extra_len = usize::from(u16_at(bytes, at + 30));
            let comment_len = usize::from(u16_at(bytes, at + 32));
            let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec())
                .expect("a UTF-8 name");
            let central_extra = &bytes[at + 46 + name_len..at + 46 + name_len + extra_len];
            let overflow = |field: u32, values: &mut std::vec::IntoIter<u64>| {
                if field == MAX_32 as u32 {
                    values.next().expect("a ZIP64 value for the placeholder")
                } else {
                    u64::from(field)
                }
            };
            let mut values = zip64_fields(central_extra).into_iter();
            let uncompressed = overflow(u32_at(bytes, at + 24), &mut values);
            let _compressed = overflow(u32_at(bytes, at + 20), &mut values);
            let local = overflow(u32_at(bytes, at + 42), &mut values) as usize;
            assert_eq!(u32_at(bytes, local), LOCAL_HEADER, "a local header: {name}");
            let local_name_len = usize::from(u16_at(bytes, local + 26));
            let local_extra_len = usize::from(u16_at(bytes, local + 28));
            let start = local + 30 + local_name_len + local_extra_len;
            let flags = u16_at(bytes, local + 6);
            if flags & 0x0008 != 0 {
                // The CRC and sizes travel in the data descriptor after the entry.
                let descriptor = start + uncompressed as usize;
                assert_eq!(
                    u32_at(bytes, descriptor),
                    DATA_DESCRIPTOR,
                    "a data descriptor follows the entry: {name}"
                );
                assert_eq!(
                    u32_at(bytes, descriptor + 4),
                    crc,
                    "the descriptor carries the same CRC: {name}"
                );
            } else {
                assert_eq!(
                    u32_at(bytes, local + 14),
                    crc,
                    "the local header carries the same CRC: {name}"
                );
            }
            let data = bytes[start..start + uncompressed as usize].to_vec();
            entries.push(ReadEntry {
                name,
                crc,
                bytes: data,
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        entries
    }

    fn crc32_of(bytes: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 == 1 {
                    (value >> 1) ^ 0xEDB8_8320
                } else {
                    value >> 1
                };
            }
            *slot = value;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for byte in bytes {
            crc = table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    fn build(scratch: &Scratch, names: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (index, (name, bytes)) in names.iter().enumerate() {
            let path = scratch.file(&format!("source-{index}"), bytes);
            writer.add_file(name, &path).expect("add_file");
        }
        writer.finish().expect("finish").into_inner()
    }

    #[test]
    fn an_archive_carries_each_entry_name_and_its_bytes() {
        let scratch = Scratch::new("entries");
        let level = scratch.file("level.dat", b"level data");
        let region = scratch.file("r.0.0.mca", b"");
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_file("World/level.dat", &level)
            .expect("level.dat");
        writer
            .add_file("World/region/r.0.0.mca", &region)
            .expect("region");
        let bytes = writer.finish().expect("finish").into_inner();

        let entries = read_archive(&bytes);
        assert_eq!(entries.len(), 2, "two entries");
        assert_eq!(entries[0].name, "World/level.dat");
        assert_eq!(entries[0].bytes, b"level data");
        assert_eq!(entries[0].crc, crc32_of(b"level data"));
        assert_eq!(entries[1].name, "World/region/r.0.0.mca");
        assert!(entries[1].bytes.is_empty(), "an empty file is an entry");
        assert_eq!(entries[1].crc, 0, "the CRC of nothing is zero");
    }

    #[test]
    fn an_empty_archive_is_valid() {
        let bytes = ZipWriter::new(Cursor::new(Vec::new()))
            .finish()
            .expect("finish")
            .into_inner();
        assert!(read_archive(&bytes).is_empty(), "no entries");
        assert_eq!(&bytes[..4], &END_OF_CENTRAL.to_le_bytes(), "only the end");
    }

    #[test]
    fn the_crc_matches_the_reference_value() {
        // The CRC-32 of "123456789" is the check value every specification quotes.
        assert_eq!(crc32_of(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_of(b""), 0);
    }

    #[test]
    fn a_missing_file_is_an_error_and_writes_no_entry() {
        let scratch = Scratch::new("missing");
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let error = writer.add_file("World/gone.dat", &scratch.0.join("gone.dat"));
        assert!(error.is_err(), "a missing source is refused");
        let bytes = writer.finish().expect("finish").into_inner();
        assert!(read_archive(&bytes).is_empty(), "nothing was added");
    }

    #[test]
    fn a_forced_zip64_archive_is_well_formed() {
        // The 4 GiB path cannot be built in a test, so the writer is asked for the ZIP64
        // records anyway: a reader must follow the locator and the 8-byte fields.
        let scratch = Scratch::new("zip64");
        let path = scratch.file("payload", b"a world file");
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.force_zip64();
        writer.add_file("World/level.dat", &path).expect("add_file");
        let bytes = writer.finish().expect("finish").into_inner();
        let entries = read_archive(&bytes);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].bytes, b"a world file");
        assert_eq!(entries[0].crc, crc32_of(b"a world file"));

        let archive = scratch.0.join("zip64.zip");
        std::fs::write(&archive, &bytes).expect("write archive");
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("python3 is absent: the independent check did not run");
            return;
        }
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg("import sys, zipfile; a = zipfile.ZipFile(sys.argv[1]); print(a.testzip(), a.namelist())")
            .arg(&archive)
            .output()
            .expect("run python3");
        assert!(
            output.status.success(),
            "python3 refused the ZIP64 archive: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("World/level.dat"));
    }

    #[test]
    fn python_reads_the_archive_and_checks_every_crc() {
        // The game's own readers are not the only oracle: CPython's zipfile validates the
        // structure, the offsets and every entry's CRC. Skipped only where python3 is absent.
        let scratch = Scratch::new("python");
        let bytes = build(
            &scratch,
            &[
                ("World/level.dat", b"level data"),
                ("World/region/r.0.0.mca", &[0u8; 4096]),
                ("World/session.lock", b"not included in real backups"),
            ],
        );
        let archive = scratch.0.join("archive.zip");
        std::fs::write(&archive, &bytes).expect("write archive");

        let visible = std::process::Command::new("python3")
            .arg("--version")
            .output();
        if visible.is_err() {
            eprintln!("python3 is absent: the independent check did not run");
            return;
        }
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                "import sys, zipfile\n\
                 a = zipfile.ZipFile(sys.argv[1])\n\
                 assert a.testzip() is None, 'a CRC failed'\n\
                 print('|'.join(a.namelist()))\n",
            )
            .arg(&archive)
            .output()
            .expect("run python3");
        assert!(
            output.status.success(),
            "python3 refused the archive: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let names = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            names.trim(),
            "World/level.dat|World/region/r.0.0.mca|World/session.lock"
        );
    }

    #[test]
    fn the_archive_stays_well_formed_across_many_entries() {
        let scratch = Scratch::new("many");
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..300u32 {
            let path = scratch.file(&format!("file-{index}"), format!("{index}").as_bytes());
            writer
                .add_file(&format!("World/data/{index}.dat"), &path)
                .expect("add_file");
        }
        let entries = read_archive(&writer.finish().expect("finish").into_inner());
        assert_eq!(entries.len(), 300);
        assert_eq!(entries[299].name, "World/data/299.dat");
        assert_eq!(entries[299].bytes, b"299");
        assert_eq!(entries[299].crc, crc32_of(b"299"));
    }
}
