#![cfg(not(feature = "loom"))]

use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use goexec::{fs, Runtime};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "goexec-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn file_helpers_traits_conversions_and_errors() {
    let dir = TempDir::new();
    let path = dir.0.join("data");
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    rt.block_on(async move {
        fs::write(&path, "hello").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello");
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert_eq!(fs::metadata(&path).unwrap().len(), 5);
        let mut file = fs::File::open(&path).unwrap();
        file.seek(SeekFrom::Start(1)).unwrap();
        let mut tail = String::new();
        file.read_to_string(&mut tail).unwrap();
        assert_eq!(tail, "ello");
        assert_eq!(file.metadata().unwrap().len(), 5);
        drop(file);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"abc").unwrap();
        file.flush().unwrap();
        file.sync_all().unwrap();
        drop(file);
        let standard = goexec::blocking(|| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
        })
        .unwrap();
        let mut file = fs::File::from_std(standard);
        file.seek(SeekFrom::Start(1)).unwrap();
        assert_eq!(file.write(b"z").unwrap(), 1);
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = [0; 3];
        file.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"azc");
        assert_eq!(file.read(&mut bytes).unwrap(), 0);
        let standard: std::fs::File = file.into();
        let mut file: fs::File = standard.into();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        file.read_to_end(&mut all).unwrap();
        assert_eq!(all, b"azc");
        drop(file);
        fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(
            fs::read(path.with_file_name("missing")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    });
    assert!(rt.shutdown_timeout(Duration::from_secs(5)));
}

#[test]
fn helpers_work_without_a_runtime() {
    let dir = TempDir::new();
    let path = dir.0.join("outside");
    fs::write(&path, b"outside").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"outside");
}
