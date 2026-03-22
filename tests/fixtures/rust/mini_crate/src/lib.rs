use crate::model::{Record, RecordId};
use crate::service::Store;

mod model;
pub mod service;

pub const DEFAULT_BATCH_SIZE: usize = 64;

pub fn bootstrap(id: RecordId) -> Store {
    Store::new(Record::new(id, "bootstrap"))
}
