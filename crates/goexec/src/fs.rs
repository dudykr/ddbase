//! Synchronous file APIs whose blocking regions participate in goexec handoff.
//!
//! These calls do not return futures. They borrow buffers normally and preserve
//! the standard library's I/O results. Whole-file helpers track the whole
//! standard-library operation (which may contain multiple syscalls).

use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

use crate::blocking;

/// Read an entire file within a blocking region.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    blocking(|| fs::read(path))
}

/// Read an entire UTF-8 file within a blocking region.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    blocking(|| fs::read_to_string(path))
}

/// Write an entire file, creating or truncating it.
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    blocking(|| fs::write(path, contents))
}

/// Query file metadata, following symbolic links.
pub fn metadata(path: impl AsRef<Path>) -> io::Result<fs::Metadata> {
    blocking(|| fs::metadata(path))
}

/// A standard file with explicitly tracked I/O and close operations.
#[derive(Debug)]
pub struct File(Option<fs::File>);

impl File {
    /// Open a file for reading.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        blocking(|| fs::File::open(path)).map(Self::from_std)
    }

    /// Open a file for writing, creating or truncating it.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        blocking(|| fs::File::create(path)).map(Self::from_std)
    }

    /// Wrap an existing file without reopening it.
    pub fn from_std(file: fs::File) -> Self {
        Self(Some(file))
    }

    /// Remove tracking. Subsequent operations and closing the returned file do
    /// not automatically enter a goexec blocking region.
    pub fn into_std(mut self) -> fs::File {
        self.0.take().expect("file already taken")
    }

    /// Query this file's metadata.
    pub fn metadata(&self) -> io::Result<fs::Metadata> {
        blocking(|| self.inner().metadata())
    }

    /// Synchronize file data and metadata to the filesystem.
    pub fn sync_all(&self) -> io::Result<()> {
        blocking(|| self.inner().sync_all())
    }

    fn inner(&self) -> &fs::File {
        self.0.as_ref().expect("file already taken")
    }
}

impl From<fs::File> for File {
    fn from(file: fs::File) -> Self {
        Self::from_std(file)
    }
}

impl From<File> for fs::File {
    fn from(file: File) -> Self {
        file.into_std()
    }
}

impl Read for File {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        blocking(|| self.inner().read(buffer))
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        blocking(|| self.inner().read_exact(buffer))
    }

    fn read_to_end(&mut self, buffer: &mut Vec<u8>) -> io::Result<usize> {
        blocking(|| self.inner().read_to_end(buffer))
    }

    fn read_to_string(&mut self, buffer: &mut String) -> io::Result<usize> {
        blocking(|| self.inner().read_to_string(buffer))
    }
}

impl Write for File {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        blocking(|| self.inner().write(buffer))
    }

    fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        blocking(|| self.inner().write_all(buffer))
    }

    fn flush(&mut self) -> io::Result<()> {
        blocking(|| self.inner().flush())
    }
}

impl Seek for File {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        blocking(|| self.inner().seek(position))
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if let Some(file) = self.0.take() {
            blocking(|| drop(file));
        }
    }
}
