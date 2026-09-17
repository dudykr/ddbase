use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    time::Duration,
};

use goexec::{fs, Runtime};

fn main() -> io::Result<()> {
    let directory = std::env::temp_dir().join(format!("goexec-example-{}", std::process::id()));
    std::fs::create_dir(&directory)?;
    let path = directory.join("hello.txt");
    let runtime = Runtime::builder().parallelism(2).build()?;
    let result = runtime.block_on(async move {
        {
            let mut file = fs::File::create(&path)?;
            file.write_all(b"hello from goexec\n")?;
            file.sync_all()?;
        } // Closing the file also enters a blocking region.
        let mut file = fs::File::open(&path)?;
        file.seek(SeekFrom::Start(6))?;
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let other = goexec::spawn(async move { fs::metadata(path).map(|m| m.len()) });
        println!("Read {text:?}; file length: {}", other.await.unwrap()?);
        Ok::<_, io::Error>(())
    });
    let finished = runtime.shutdown_timeout(Duration::from_secs(5));
    std::fs::remove_dir_all(directory)?;
    assert!(finished);
    result
}
