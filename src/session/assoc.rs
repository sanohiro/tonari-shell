use std::collections::HashMap;
use std::rc::Rc;

use super::array::Elements;
use super::{Session, Variable};

// Bash enumerates the initial 1024 buckets in ascending order and visits the
// newest key first within a bucket. Keep ordinary lookup independent of this
// presentation order, including for keys containing spaces or shell syntax.
#[derive(Clone, Default)]
pub(crate) struct Associative {
    entries: HashMap<String, (String, usize)>,
    serial: usize,
}

impl Associative {
    pub(super) fn get(&self, key: &str) -> Option<&String> {
        self.entries.get(key).map(|(value, _)| value)
    }

    pub(super) fn insert(&mut self, key: String, value: String) {
        if let Some((slot, _)) = self.entries.get_mut(&key) {
            *slot = value;
        } else {
            self.entries.insert(key, (value, self.serial));
            self.serial += 1;
        }
    }

    pub(super) fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }

    pub(super) fn ordered(&self) -> Vec<(&str, &str)> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_unstable_by_key(|(key, (_, serial))| {
            let hash = key.bytes().fold(2166136261u32, |hash, c| {
                hash.wrapping_mul(16777619) ^ (c as libc::c_char as u32)
            });
            (hash & 1023, std::cmp::Reverse(*serial))
        });
        entries
            .into_iter()
            .map(|(k, (v, _))| (k.as_str(), v.as_str()))
            .collect()
    }
}

impl Variable {
    pub(super) fn associative(&self) -> Option<&Associative> {
        match self.elements.as_deref()? {
            Elements::Associative(map) => Some(map),
            _ => None,
        }
    }

    pub(super) fn make_associative(&mut self) -> Result<&mut Associative, &'static str> {
        if self.elements.is_none() {
            let mut map = Associative::default();
            if let Some(value) = self.value.take() {
                map.insert("0".into(), value);
                self.array_assigned = true;
            }
            self.elements = Some(Rc::new(Elements::Associative(map)));
        }
        match Rc::make_mut(self.elements.as_mut().unwrap()) {
            Elements::Associative(map) => Ok(map),
            _ => Err("cannot convert indexed to associative array"),
        }
    }
}

impl Session {
    pub fn is_associative(&self, name: &str) -> bool {
        self.variables
            .get(name)
            .and_then(Variable::associative)
            .is_some()
    }

    pub fn assoc_value(&self, name: &str, key: &str) -> Option<&str> {
        self.variables
            .get(name)?
            .associative()?
            .get(key)
            .map(String::as_str)
    }

    pub fn assign_key(
        &mut self,
        name: &str,
        key: String,
        value: String,
    ) -> Result<(), &'static str> {
        if self.readonly(name) {
            return Err("readonly variable");
        }
        let var = self
            .variables
            .get_mut(name)
            .ok_or("missing associative array")?;
        var.make_associative()?.insert(key, value);
        var.array_assigned = true;
        Ok(())
    }

    pub fn subscript_key(&mut self, source: &str) -> Result<String, &'static str> {
        let captured = std::cell::Cell::new(None);
        crate::expansion::Expansion::new(self, 0, &captured)
            .key(source)
            .map_err(|_| "bad array subscript")
    }

    pub(super) fn remove_key(&mut self, name: &str, key: &str) {
        if let Some(var) = self.variables.get_mut(name)
            && let Ok(map) = var.make_associative()
        {
            map.remove(key);
        }
    }
}
