//! The tile cache in a browser: kept in memory for the visit, since a page has no disk.
//! It has the same interface as the disk cache, and drops the longest-unused tiles first.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What the page may hold before old tiles are dropped. The whole program lives in a 4 GiB
/// address space, and the decoded tiles on the GPU side need room too.
const BUDGET: u64 = 384 << 20;

#[derive(Default)]
struct Store {
    entries: HashMap<String, (Vec<u8>, u64)>,
    missing: HashSet<String>,
    bytes: u64,
    clock: u64,
}

impl Store {
    fn trim_to(&mut self, limit: u64) -> u64 {
        if self.bytes <= limit {
            return 0;
        }
        let mut order: Vec<(u64, String)> =
            self.entries.iter().map(|(k, v)| (v.1, k.clone())).collect();
        order.sort_unstable();
        let mut freed = 0;
        for (_, name) in order {
            if self.bytes <= limit * 9 / 10 {
                break;
            }
            if let Some((data, _)) = self.entries.remove(&name) {
                self.bytes -= data.len() as u64;
                freed += data.len() as u64;
            }
        }
        freed
    }
}

#[derive(Clone, Default)]
pub struct DiskCache {
    store: Arc<Mutex<Store>>,
}

impl DiskCache {
    pub fn new(_root: PathBuf) -> Self {
        Self::default()
    }

    pub fn default_root() -> PathBuf {
        PathBuf::from("/memory")
    }

    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        let mut s = self.store.lock().unwrap();
        s.clock += 1;
        let clock = s.clock;
        let entry = s.entries.get_mut(name)?;
        entry.1 = clock;
        Some(entry.0.clone())
    }

    pub fn put(&self, name: &str, data: &[u8]) -> std::io::Result<()> {
        let mut s = self.store.lock().unwrap();
        s.clock += 1;
        let clock = s.clock;
        s.missing.remove(name);
        if let Some((old, _)) = s.entries.insert(name.to_string(), (data.to_vec(), clock)) {
            s.bytes -= old.len() as u64;
        }
        s.bytes += data.len() as u64;
        s.trim_to(BUDGET);
        Ok(())
    }

    pub fn remove(&self, name: &str) {
        let mut s = self.store.lock().unwrap();
        if let Some((old, _)) = s.entries.remove(name) {
            s.bytes -= old.len() as u64;
        }
    }

    pub fn mark_missing(&self, name: &str) {
        self.store.lock().unwrap().missing.insert(name.to_string());
    }

    pub fn is_known_missing(&self, name: &str) -> bool {
        self.store.lock().unwrap().missing.contains(name)
    }

    /// Nothing is written, so state files simply fail to open, as for a read-only disk.
    pub fn state_path(&self, name: &str) -> PathBuf {
        Path::new("/memory").join(format!(".{name}"))
    }

    pub fn usage(&self) -> (u64, u64) {
        let s = self.store.lock().unwrap();
        (s.entries.len() as u64, s.bytes)
    }

    pub fn trim(&self, limit: u64) -> u64 {
        self.store.lock().unwrap().trim_to(limit.min(BUDGET))
    }
}
