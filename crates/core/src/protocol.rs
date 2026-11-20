use crate::Id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_NEGOTIATION: usize = 512;
pub const MAX_REFS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Refs {
    pub version: u32,
    pub default_branch: String,
    pub branches: BTreeMap<String, Id>,
    pub tags: BTreeMap<String, Id>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub expected: Option<Id>,
    pub target: Option<Id>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Objects {
    pub ids: Vec<Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lock {
    pub owner: String,
    pub expires: i64,
}
