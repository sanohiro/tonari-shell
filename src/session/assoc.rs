use foldhash::HashMap;
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
    order: std::cell::OnceCell<Vec<String>>,
}

impl Associative {
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn get(&self, key: &str) -> Option<&String> {
        self.entries.get(key).map(|(value, _)| value)
    }

    pub(super) fn get_mut(&mut self, key: &str) -> Option<&mut String> {
        self.entries.get_mut(key).map(|(value, _)| value)
    }

    pub(super) fn insert(&mut self, key: String, value: String) {
        use std::collections::hash_map::Entry;
        match self.entries.entry(key) {
            Entry::Occupied(mut entry) => entry.get_mut().0 = value,
            Entry::Vacant(entry) => {
                self.order.take();
                entry.insert((value, self.serial));
                self.serial += 1;
            }
        }
    }

    pub(super) fn remove(&mut self, key: &str) {
        if self.entries.remove(key).is_some() {
            self.order.take();
        }
    }

    pub(super) fn ordered_keys(&self) -> &[String] {
        self.order.get_or_init(|| {
            let mut keys: Vec<_> = self.entries.iter().collect();
            keys.sort_by_cached_key(|(key, (_, serial))| {
                let hash = key.bytes().fold(2166136261u32, |hash, c| {
                    hash.wrapping_mul(16777619) ^ (c as libc::c_char as u32)
                });
                (hash & 1023, std::cmp::Reverse(*serial))
            });
            keys.into_iter().map(|(k, _)| k.clone()).collect()
        })
    }
    pub(super) fn ordered(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.ordered_keys()
            .iter()
            .map(|k| (k.as_str(), self.entries[k].0.as_str()))
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
            if let Some(value) = self.scalar_mut().take() {
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
    pub(super) fn live_table(&self, name: &str) -> bool {
        matches!(name, "BASH_ALIASES" | "BASH_CMDS")
            && self.variables.get(name).is_none_or(|v| v.dynamic)
    }

    pub(super) fn table_snapshot(&self, name: &str) -> Associative {
        let mut map = Associative::default();
        if name == "BASH_ALIASES" {
            if let Some(aliases) = &self.aliases.0 {
                for (key, value) in aliases.iter() {
                    map.insert(key.clone(), value.clone());
                }
            }
        } else {
            for (key, value) in self.hashes.items() {
                map.insert(key.into(), value.into());
            }
        }
        map
    }

    pub fn is_associative(&self, name: &str) -> bool {
        let Ok(resolved) = self.reference_name(name) else {
            return false;
        };
        let name = resolved.as_ref();
        self.live_table(name)
            || self
                .variables
                .get(name)
                .and_then(Variable::associative)
                .is_some()
    }

    pub fn assoc_value(&self, name: &str, key: &str) -> Option<&str> {
        let resolved = self.reference_name(name).ok()?;
        let name = resolved.as_ref();
        if self.live_table(name) {
            return if name == "BASH_ALIASES" {
                self.aliases.get(key)
            } else {
                self.hashes.path_value(key)
            };
        }
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
        let resolved = self.reference_name(name)?;
        let name = resolved.as_ref();
        if self.readonly(name) {
            return Err(crate::session::READONLY);
        }
        if self.live_table(name) {
            if name == "BASH_ALIASES" {
                return self.aliases.insert(&key, &value);
            }
            self.hashes.bind_path(key, value);
            return Ok(());
        }
        let value = self.convert_case(value, self.variables.get(name).and_then(|v| v.letter_case));
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
