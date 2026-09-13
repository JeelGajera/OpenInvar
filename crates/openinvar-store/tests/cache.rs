use openinvar_store::HotQueryCache;

#[test]
fn test_cache_hit_miss_and_stats() {
    let cache = HotQueryCache::new(8);

    assert_eq!(cache.get("missing"), None);
    cache.put("q1".to_string(), "result1".to_string());

    assert_eq!(cache.get("q1"), Some("result1".to_string()));

    let stats = cache.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.size, 1);
}

#[test]
fn test_cache_eviction_lru_like_behavior() {
    let cache = HotQueryCache::new(2);

    cache.put("a".to_string(), "A".to_string());
    cache.put("b".to_string(), "B".to_string());
    let _ = cache.get("a");
    cache.put("c".to_string(), "C".to_string());

    assert_eq!(cache.get("a"), Some("A".to_string()));
    assert_eq!(cache.get("b"), None);
    assert_eq!(cache.get("c"), Some("C".to_string()));

    let stats = cache.stats();
    assert!(stats.evictions >= 1);
}

#[test]
fn test_cache_invalidate_and_clear() {
    let cache = HotQueryCache::new(4);

    cache.put("x".to_string(), "X".to_string());
    cache.put("y".to_string(), "Y".to_string());

    cache.invalidate("x");
    assert_eq!(cache.get("x"), None);

    cache.clear();
    let stats = cache.stats();
    assert_eq!(stats.size, 0);
}
