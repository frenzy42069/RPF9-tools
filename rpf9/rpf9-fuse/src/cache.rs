use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub(crate) enum Namespace {
    Index,
    Media,
}

pub(crate) struct Cache {
    root: PathBuf,
    max_bytes: u64,
}

impl Cache {
    pub(crate) fn new(root: PathBuf, max_bytes: u64) -> io::Result<Self> {
        for name in ["index", "media", "locks", "tmp"] {
            fs::create_dir_all(root.join(name))?;
        }
        Ok(Self { root, max_bytes })
    }

    fn entry(
        &self,
        namespace: Namespace,
        source: &Path,
        selector: &str,
    ) -> io::Result<(PathBuf, PathBuf)> {
        let canonical = source.canonicalize()?;
        let metadata = canonical.metadata()?;
        let mut hash = Sha256::new();
        hash.update(b"rpf9-fuse-cache-v2\0");
        hash.update(canonical.as_os_str().as_encoded_bytes());
        hash.update(metadata.dev().to_le_bytes());
        hash.update(metadata.ino().to_le_bytes());
        hash.update(metadata.len().to_le_bytes());
        hash.update(metadata.ctime().to_le_bytes());
        hash.update(metadata.ctime_nsec().to_le_bytes());
        let modified = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(io::Error::other)?;
        hash.update(modified.as_nanos().to_le_bytes());
        hash.update([0]);
        hash.update(selector.as_bytes());
        let key = format!("{:x}", hash.finalize());
        let dir = match namespace {
            Namespace::Index => "index",
            Namespace::Media => "media",
        };
        Ok((
            self.root.join(dir).join(&key),
            self.root.join("locks").join(format!("{dir}-{key}")),
        ))
    }

    pub(crate) fn get(
        &self,
        namespace: Namespace,
        source: &Path,
        selector: &str,
    ) -> io::Result<Option<File>> {
        let (entry, _) = self.entry(namespace, source, selector)?;
        match File::open(entry) {
            Ok(file) => {
                if matches!(namespace, Namespace::Media) {
                    let _ =
                        file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()));
                }
                Ok(Some(file))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(crate) fn get_or_create(
        &self,
        namespace: Namespace,
        source: &Path,
        selector: &str,
        create: impl FnOnce(&Path) -> io::Result<()>,
    ) -> io::Result<File> {
        let (entry, lock_path) = self.entry(namespace, source, selector)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)?;
        lock.lock()?;
        match File::open(&entry) {
            Ok(file) => {
                if matches!(namespace, Namespace::Media) {
                    let _ =
                        file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()));
                }
                return Ok(file);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let temp = tempfile::Builder::new()
            .prefix("build-")
            .tempdir_in(self.root.join("tmp"))?;
        let output = temp.path().join("output");
        create(&output)?;
        let file = File::open(&output)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other(
                "cache producer did not create a regular file",
            ));
        }
        if self.entry(namespace, source, selector)?.0 != entry {
            return Err(io::Error::other(
                "source changed while producing cache entry",
            ));
        }
        fs::rename(&output, &entry)?;
        if matches!(namespace, Namespace::Media) {
            self.prune(&entry)?;
        }
        Ok(file)
    }

    fn prune(&self, protected: &Path) -> io::Result<()> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join("locks").join("media-prune"))?;
        lock.lock()?;
        let mut files = Vec::new();
        let mut total = 0_u64;
        for item in fs::read_dir(self.root.join("media"))? {
            let item = item?;
            let meta = item.metadata()?;
            if meta.is_file() {
                total = total.saturating_add(meta.len());
                files.push((
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    meta.len(),
                    item.path(),
                ));
            }
        }
        files.sort_by_key(|entry| entry.0);
        for (_, size, path) in files {
            if total <= self.max_bytes {
                break;
            }
            if path == protected {
                continue;
            }
            match fs::remove_file(path) {
                Ok(()) => total = total.saturating_sub(size),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
    };

    #[test]
    fn concurrent_creation_is_single_flight_and_source_changes_invalidate() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        fs::write(&source, b"first").unwrap();
        let cache = Arc::new(Cache::new(dir.path().join("cache"), 100).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let cache = cache.clone();
                let source = source.clone();
                let calls = calls.clone();
                thread::spawn(move || {
                    let mut file = cache
                        .get_or_create(Namespace::Media, &source, "png", |path| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            fs::write(path, b"cached")
                        })
                        .unwrap();
                    let mut result = String::new();
                    file.read_to_string(&mut result).unwrap();
                    assert_eq!(result, "cached");
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        fs::write(&source, b"changed length").unwrap();
        assert!(cache
            .get(Namespace::Media, &source, "png")
            .unwrap()
            .is_none());
    }
}
