//! File access, injected so that the command logic is tested without touching the disk.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// What the commands need from the file system.
pub trait Files {
    /// Reads a UTF-8 text file of at most `limit` bytes, without reading more than one
    /// byte past the limit: an endless or huge file is refused, not loaded.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, is larger than `limit` bytes
    /// ([`io::ErrorKind::FileTooLarge`]) or is not UTF-8.
    fn read(&mut self, path: &Path, limit: usize) -> io::Result<String>;

    /// Writes a file, replacing it.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Creates a directory and its missing parents; does nothing if it exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created.
    fn create_dir_all(&mut self, dir: &Path) -> io::Result<()>;

    /// Lists the entries of a directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read.
    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>>;
}

/// The real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct Disk;

impl Files for Disk {
    fn read(&mut self, path: &Path, limit: usize) -> io::Result<String> {
        let mut bytes = Vec::new();
        let past_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
        let _: usize = std::fs::File::open(path)?
            .take(past_limit)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                format!("larger than {limit} bytes"),
            ));
        }
        String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn create_dir_all(&mut self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(dir)?
            .map(|entry| entry.map(|dir_entry| dir_entry.path()))
            .collect()
    }
}
