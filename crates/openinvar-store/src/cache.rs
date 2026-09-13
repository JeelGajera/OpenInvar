use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub size: usize,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    value: String,
    last_access_tick: u64,
}

pub struct HotQueryCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
    max_entries: usize,
    tick: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl HotQueryCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            max_entries,
            tick: AtomicU64::new(1),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    pub fn put(&self, key: String, value: String) {
        let current_tick = self.tick.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(
                key,
                CacheEntry {
                    value,
                    last_access_tick: current_tick,
                },
            );
            evict_if_needed(&mut entries, self.max_entries, &self.evictions);
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.get_mut(key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                entry.last_access_tick = self.tick.fetch_add(1, Ordering::Relaxed);
                return Some(entry.value.clone());
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn invalidate(&self, key: &str) {
        if let Ok(mut entries) = self.entries.write() {
            entries.remove(key);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
    }

    pub fn stats(&self) -> CacheStats {
        let size = self
            .entries
            .read()
            .map(|entries| entries.len())
            .unwrap_or_default();

        CacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            size,
        }
    }
}

fn evict_if_needed(
    entries: &mut HashMap<String, CacheEntry>,
    max_entries: usize,
    evictions: &AtomicU64,
) {
    while entries.len() > max_entries {
        let mut oldest_key = None;
        let mut oldest_tick = u64::MAX;

        for (key, entry) in entries.iter() {
            if entry.last_access_tick < oldest_tick {
                oldest_tick = entry.last_access_tick;
                oldest_key = Some(key.clone());
            }
        }

        let Some(oldest_key) = oldest_key else {
            break;
        };

        if entries.remove(&oldest_key).is_some() {
            evictions.fetch_add(1, Ordering::Relaxed);
        } else {
            break;
        }
    }
}
