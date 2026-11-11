//! Disk cache for downloaded tiles.

use crate::wtime::{Duration, SystemTime};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What was used this recently is never removed.
const KEEP_RECENT: Duration = Duration::from_secs(6 * 3600);
/// How long to remember that a server has no such tile.
const MISSING_FOR: Duration = Duration::from_secs(30 * 24 * 3600);
const MISSING_SUFFIX: &str = ".none";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct DiskCache {
    root: PathBuf,
}

impl DiskCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_root() -> PathBuf {
        crate::paths::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("orbis")
            .join("tiles")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Reads an entry and marks it as used.
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.path(name);
        let data = fs::read(&path).ok()?;
        if let Ok(file) = File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(data)
    }

    /// Stores an entry. Readers never see a partly written file.
    pub fn put(&self, name: &str, data: &[u8]) -> std::io::Result<()> {
        let path = self.path(name);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_extension(format!("tmp{}-{id}", std::process::id()));
        fs::write(&temp, data)?;
        fs::rename(&temp, &path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }

    pub fn remove(&self, name: &str) {
        let _ = fs::remove_file(self.path(name));
    }

    fn missing_path(&self, name: &str) -> PathBuf {
        let mut path = self.path(name).into_os_string();
        path.push(MISSING_SUFFIX);
        path.into()
    }

    /// Remembers that a server has no such tile.
    pub fn mark_missing(&self, name: &str) {
        let path = self.missing_path(name);
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(e) = fs::write(&path, b"") {
            eprintln!("cache: could not note {name}: {e}");
        }
    }

    /// Whether a server recently had no such tile. The note expires, as data gets added.
    pub fn is_known_missing(&self, name: &str) -> bool {
        fs::metadata(self.missing_path(name))
            .and_then(|m| m.modified())
            .is_ok_and(|noted| {
                SystemTime::now().duration_since(noted).unwrap_or_default() < MISSING_FOR
            })
    }

    /// A place for this cache's own state, which size limits do not remove.
    pub fn state_path(&self, name: &str) -> PathBuf {
        self.root.join(format!(".{name}"))
    }

    /// How many entries the cache holds, and their size in bytes.
    pub fn usage(&self) -> (u64, u64) {
        let mut entries = Vec::new();
        collect(&self.root, &mut entries);
        (entries.len() as u64, entries.iter().map(|e| e.1).sum())
    }

    /// Removes the longest-unused entries until the cache is under 90 % of `limit`.
    /// Returns the number of bytes freed.
    pub fn trim(&self, limit: u64) -> u64 {
        let mut entries = Vec::new();
        collect(&self.root, &mut entries);
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        if total <= limit {
            return 0;
        }
        entries.sort_by_key(|e| e.2);
        let goal = limit / 10 * 9;
        let now = SystemTime::now();
        let mut freed = 0;
        for (path, size, used) in entries {
            if total <= goal || now.duration_since(used).unwrap_or_default() < KEEP_RECENT {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
                freed += size;
            }
        }
        freed
    }
}

fn collect(dir: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        // Dot files hold state rather than cached data.
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            collect(&path, out);
        } else if meta.is_file() {
            out.push((
                path,
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache(tag: &str) -> DiskCache {
        let root = std::env::temp_dir().join(format!("orbis-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        DiskCache::new(root)
    }

    fn age(c: &DiskCache, name: &str, hours: u64) {
        let f = File::options().write(true).open(c.path(name)).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600))
            .unwrap();
    }

    #[test]
    fn stores_and_reads() {
        let c = temp_cache("rw");
        assert_eq!(c.get("a/b/c.jpg"), None);
        c.put("a/b/c.jpg", b"hello").unwrap();
        assert_eq!(c.get("a/b/c.jpg").as_deref(), Some(&b"hello"[..]));
        let _ = fs::remove_dir_all(&c.root);
    }

    #[test]
    fn reading_marks_as_used() {
        let c = temp_cache("touch");
        c.put("a.bin", b"x").unwrap();
        age(&c, "a.bin", 100);
        c.get("a.bin").unwrap();
        let m = fs::metadata(c.path("a.bin")).unwrap().modified().unwrap();
        assert!(SystemTime::now().duration_since(m).unwrap() < Duration::from_secs(60));
        let _ = fs::remove_dir_all(&c.root);
    }

    #[test]
    fn missing_notes_are_remembered_and_expire() {
        let c = temp_cache("missing");
        assert!(!c.is_known_missing("a/1.jpg"));
        c.mark_missing("a/1.jpg");
        assert!(c.is_known_missing("a/1.jpg"));
        assert_eq!(c.get("a/1.jpg"), None, "a note is not a tile");
        let note = File::options()
            .write(true)
            .open(c.missing_path("a/1.jpg"))
            .unwrap();
        note.set_modified(SystemTime::now() - Duration::from_secs(31 * 24 * 3600))
            .unwrap();
        assert!(!c.is_known_missing("a/1.jpg"));
        let _ = fs::remove_dir_all(&c.root);
    }

    #[test]
    fn usage_counts_data_but_not_state() {
        let c = temp_cache("usage");
        c.put("a/1.bin", &[0; 100]).unwrap();
        c.put("b/2.bin", &[0; 50]).unwrap();
        fs::write(c.state_path("servers.txt"), b"state").unwrap();
        assert_eq!(c.usage(), (2, 150));
        assert_eq!(
            c.trim(10),
            0,
            "recent data stays, and state is not even considered"
        );
        assert!(c.state_path("servers.txt").exists());
        let _ = fs::remove_dir_all(&c.root);
    }

    #[test]
    fn trim_removes_oldest_first_and_keeps_recent() {
        let c = temp_cache("trim");
        for (name, hours) in [("old", 100), ("mid", 50), ("newer", 10), ("recent", 1)] {
            c.put(name, &[0; 1000]).unwrap();
            age(&c, name, hours);
        }
        assert_eq!(c.trim(4000), 0, "under the limit nothing goes");
        assert_eq!(c.trim(2500), 2000);
        assert!(c.get("old").is_none() && c.get("mid").is_none());
        assert!(c.get("newer").is_some());

        // Entries used just now survive even when the cache is over its limit.
        let c = temp_cache("trim2");
        c.put("recent", &[0; 1000]).unwrap();
        assert_eq!(c.trim(10), 0);
        assert!(c.get("recent").is_some());
        let _ = fs::remove_dir_all(&c.root);
    }
}
