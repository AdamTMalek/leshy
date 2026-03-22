use std::fmt;

pub struct RecordId(pub u64);

pub struct Record {
    pub id: RecordId,
    pub name: &'static str,
}

pub enum Status {
    Ready,
    Archived,
}

pub const DEFAULT_NAME: &str = "pending";

impl Record {
    pub fn new(id: RecordId, name: &'static str) -> Self {
        Self { id, name }
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
