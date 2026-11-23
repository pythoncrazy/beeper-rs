//! `BEEPER_CORE_SOURCE`: a hash of the sources of this crate and of the versions of its dependencies. What a run keeps
//! of the rooms for the next start (see `matrix::start_pass`) holds only for the build that made it.
use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read the source folder") {
        let path = e.expect("a source entry").path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    let lock = Path::new("../../Cargo.lock");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed={}", lock.display());
    let mut paths = vec![PathBuf::from("Cargo.toml")];
    files(Path::new("src"), &mut paths);
    paths.extend(lock.exists().then(|| lock.to_path_buf()));
    paths.sort();
    // FNV-1a over the path and the bytes of each file.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &paths {
        let bytes = std::fs::read(p).expect("read a source file");
        for b in p.to_string_lossy().bytes().chain(bytes) {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    println!("cargo:rustc-env=BEEPER_CORE_SOURCE={hash:016x}");
}
