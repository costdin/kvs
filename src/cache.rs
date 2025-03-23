use std::collections::HashMap;
use std::hash::Hash;

pub struct Cache<K, V> {
    map: HashMap<K, (V, usize)>,
    fifo: Vec<(usize, K)>,
    max_size: usize,
    count: usize,
}

impl<K: Eq + Hash + Clone + Ord, V> Cache<K, V> {
    pub fn new(size: usize) -> Cache<K, V> {
        Cache {
            map: HashMap::new(),
            fifo: vec![],
            max_size: size,
            count: 0,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&mut V> {
        self.map.get_mut(key).map(|(v, _)| v)
    }

    pub fn set(&mut self, key: K, value: V) {
        match self.map.get_mut(&key) {
            Some(v) => {
                v.0 = value;
            }
            None => {
                self.count += 1;

                self.fifo.push((self.count, key.clone()));
                if self.fifo.len() > self.max_size {
                    let removed_entry = self.fifo.remove(0);
                    self.map.remove(&removed_entry.1);
                }

                self.map.insert(key, (value, self.count));
            }
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some((v, n)) = self.map.remove(key) {
            if let Ok(ix) = self.fifo.binary_search(&(n, key.clone())) {
                self.fifo.remove(ix);
            }

            Some(v)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_retrieve() {
        let mut cache = Cache::new(2);
        cache.set(1, "one");
        cache.set(2, "two");

        assert_eq!(cache.get(&1), Some(&mut "one"));
        assert_eq!(cache.get(&2), Some(&mut "two"));
    }

    #[test]
    fn test_cache_eviction() {
        let mut cache = Cache::new(2);
        cache.set(1, "one");
        cache.set(2, "two");
        cache.set(3, "three"); // Should evict key 1

        assert!(cache.get(&1).is_none());
        assert_eq!(cache.get(&2), Some(&mut "two"));
        assert_eq!(cache.get(&3), Some(&mut "three"));
    }

    #[test]
    fn test_cache_update_existing_key() {
        let mut cache = Cache::new(2);
        cache.set(1, "one");
        cache.set(1, "uno");

        assert_eq!(cache.get(&1), Some(&mut "uno"));
    }

    #[test]
    fn test_cache_ordering() {
        let mut cache = Cache::new(2);
        cache.set(1, "one");
        cache.set(2, "two");
        cache.set(1, "uno");
        cache.set(3, "three"); // Should evict key 1

        assert!(cache.get(&1).is_none());
        assert_eq!(cache.get(&2), Some(&mut "two"));
        assert_eq!(cache.get(&3), Some(&mut "three"));
    }

    #[test]
    fn test_cache_remove() {
        let mut cache = Cache::new(2);
        cache.set(1, "one");
        cache.set(2, "two");
        cache.remove(&2);
        cache.set(3, "three"); // Should evict no key

        assert!(cache.get(&2).is_none());
        assert_eq!(cache.get(&1), Some(&mut "one"));
        assert_eq!(cache.get(&3), Some(&mut "three"));
    }
}
