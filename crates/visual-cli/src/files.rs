//! File access, injected so that the command logic is tested without touching the disk.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// What the commands need from the file system.
///
/// ```
/// use std::io;
/// use std::path::{Path, PathBuf};
///
/// use visual_cli::Files;
///
/// /// A file system holding nothing: [`Files::absolute`] has a default.
/// struct Nothing;
///
/// impl Files for Nothing {
///     fn read(&mut self, _: &Path, _: usize) -> io::Result<String> {
///         Err(io::ErrorKind::NotFound.into())
///     }
///
///     fn write(&mut self, _: &Path, _: &[u8]) -> io::Result<()> {
///         Err(io::ErrorKind::ReadOnlyFilesystem.into())
///     }
///
///     fn list(&mut self, _: &Path) -> io::Result<Vec<PathBuf>> {
///         Ok(Vec::new())
///     }
/// }
///
/// let command = ["check", "/nothing"].map(str::to_owned);
/// let refused = visual_cli::run(&command, &mut Nothing, &mut Vec::new());
/// assert_eq!(
///     refused.map_err(|error| error.to_string()),
///     Err("cannot read /nothing/references.json: entity not found".to_owned())
/// );
/// ```
pub trait Files {
    /// Reads a UTF-8 text file of at most `limit` bytes, without reading more than one
    /// byte past the limit: an endless or huge file is refused, not loaded.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, is larger than `limit` bytes
    /// ([`io::ErrorKind::FileTooLarge`]) or is not UTF-8 ([`io::ErrorKind::InvalidData`]).
    fn read(&mut self, path: &Path, limit: usize) -> io::Result<String>;

    /// Writes a file, replacing it.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Lists the entries of a directory, in no particular order.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read.
    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>>;

    /// `path` from the root of the file system, as a hint names it, so that the command it
    /// names runs from any directory of the repository: by default the process's current
    /// directory before a relative path, or the path as given when it has none to stand on (an
    /// empty path, a current directory gone). An implementation whose files stand elsewhere
    /// overrides it.
    fn absolute(&mut self, path: &Path) -> PathBuf {
        absolute(path)
    }
}

/// [`Files::absolute`]'s default, apart so that its one copy is the one the tests cover.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
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

    fn list(&mut self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(dir)?
            .map(|entry| entry.map(|dir_entry| dir_entry.path()))
            .collect()
    }
}
