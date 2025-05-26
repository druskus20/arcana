use std::{error::Error, fmt::Display};

pub enum HashMapExtError {
    KeyAlreadyExists,
}

impl Display for HashMapExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashMapExtError::KeyAlreadyExists => write!(f, "Key already exists in the HashMap"),
        }
    }
}

impl std::fmt::Debug for HashMapExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HashMapExtError: {self}")
    }
}
impl Error for HashMapExtError {}

pub trait HashmapExt<K, V> {
    fn fallible_insert(&mut self, key: K, value: V) -> Result<&mut V, HashMapExtError>
    where
        K: std::hash::Hash + Eq;
}

impl<K, V> HashmapExt<K, V> for std::collections::HashMap<K, V> {
    fn fallible_insert(&mut self, key: K, value: V) -> Result<&mut V, HashMapExtError>
    where
        K: std::hash::Hash + Eq,
    {
        if self.contains_key(&key) {
            return Err(HashMapExtError::KeyAlreadyExists);
        }
        Ok(self.entry(key).or_insert(value))
    }
}
