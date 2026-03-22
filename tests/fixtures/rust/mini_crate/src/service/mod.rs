use crate::model::{Record, RecordId};

pub mod cache;

pub trait Repository {
    type Error;

    fn load(&self, id: RecordId) -> Result<Record, Self::Error>;
}

pub struct Store;

impl Store {
    pub fn new(record: Record) -> Self {
        let _ = record;
        Self
    }

    pub fn fetch<R: Repository>(
        &self,
        repository: &R,
        id: RecordId,
    ) -> Result<Record, R::Error> {
        repository.load(id)
    }
}
