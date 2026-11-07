//! File access, injected so that the command logic is tested without touching the disk.

use std::io;
use std::path::{Path, PathBuf};

/// What the commands need from the file system.
pub trait Files {
    /// Reads a text file.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read.
    fn read(&mut self, path: &Path) -> io::Result<String>;

    /// Writes a file, replacing it.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;

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
    fn read(&mut self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.path()))
            .collect()
    }
}
