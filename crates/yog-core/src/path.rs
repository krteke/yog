use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub mod native {
    use super::*;

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        OsString::deserialize(deserializer).map(PathBuf::from)
    }
}
