use crate::model::RecordId;

pub struct CacheEntry {
    pub id: RecordId,
}

pub static CACHE_CAPACITY: usize = 32;

impl CacheEntry {
    pub fn new(id: RecordId) -> Self {
        Self { id }
    }
}
