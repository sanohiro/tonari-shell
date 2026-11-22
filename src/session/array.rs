use super::{Session, Variable};
use std::collections::BTreeMap;
use std::rc::Rc;

pub(crate) fn subscript(name: &str) -> Option<(&str, &str)> {
    let (base, expr) = name.split_once('[')?;
    Some((base, expr.strip_suffix(']')?))
}

impl Variable {
    pub(super) fn make_array(&mut self) -> &mut BTreeMap<i64, String> {
        let initial = self.value.take();
        self.array_assigned |= initial.is_some();
        Rc::make_mut(
            self.elements
                .get_or_insert_with(|| Rc::new(initial.into_iter().map(|v| (0, v)).collect())),
        )
    }
}

impl Session {
    pub fn valid_target(&self, target: &str) -> bool {
        if let Some((base, expr)) = subscript(target) {
            super::is_identifier(base) && self.resolve_index(base, expr).is_ok()
        } else {
            super::is_identifier(target)
        }
    }

    pub fn assign_target(&mut self, target: &str, value: &str) -> Result<(), &'static str> {
        if let Some((base, expr)) = subscript(target) {
            if !super::is_identifier(base) {
                return Err("invalid variable name");
            }
            let index = self.resolve_index(base, expr)?;
            self.assign_element(base, index, value.into());
        } else {
            if !super::is_identifier(target) {
                return Err("invalid variable name");
            }
            self.assign(target, value);
        }
        Ok(())
    }

    pub(super) fn indices(&self, name: &str) -> Vec<i64> {
        if name == "PIPESTATUS" {
            let len = usize::from(self.pipe_status.first.is_some()) + self.pipe_status.rest.len();
            return (0..len as i64).collect();
        }
        if let Some(v) = self.variables.get(name) {
            return v.elements.as_ref().map_or_else(
                || (v.value.is_some()).then_some(0).into_iter().collect(),
                |elements| elements.keys().copied().collect(),
            );
        }
        match name {
            "FUNCNAME" if self.function_depth == 0 => Vec::new(),
            "FUNCNAME" => (0..self.function_names.len() as i64).collect(),
            "BASH_SOURCE" => (0..self.source_names.len() as i64).collect(),
            _ => self.inherited_value(name).map(|_| 0).into_iter().collect(),
        }
    }

    pub(super) fn element(&self, name: &str, index: i64) -> Option<String> {
        if name == "PIPESTATUS" {
            return self
                .pipe_status
                .first
                .iter()
                .chain(&self.pipe_status.rest)
                .nth(index as usize)
                .map(u8::to_string);
        }
        if let Some(v) = self.variables.get(name) {
            return match v.elements.as_ref() {
                Some(a) => a.get(&index).cloned(),
                None if index == 0 => v.value.clone(),
                _ => None,
            };
        }
        match name {
            "FUNCNAME" if self.function_depth == 0 => None,
            "FUNCNAME" => self
                .function_names
                .iter()
                .rev()
                .nth(index as usize)
                .cloned(),
            "BASH_SOURCE" => self.source_names.iter().rev().nth(index as usize).cloned(),
            _ if index == 0 => self.inherited_value(name),
            _ => None,
        }
    }

    pub fn array_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .variables
            .iter()
            .filter(|(_, v)| v.elements.is_some() && v.array_assigned)
            .map(|(key, _)| key.clone())
            .collect();
        names.push("BASH_SOURCE".into());
        if self.pipe_status.first.is_some() {
            names.push("PIPESTATUS".into());
        }
        if self.function_depth > 0 {
            names.push("FUNCNAME".into());
        }
        names.sort_unstable();
        names.dedup();
        names
    }

    pub fn array_values(&self, name: &str) -> Vec<String> {
        self.indices(name)
            .into_iter()
            .filter_map(|index| self.element(name, index))
            .collect()
    }

    pub fn replace_array(&mut self, name: &str, values: Vec<String>) {
        let values = (0i64..).zip(values).collect();
        self.store_array(name, values, false);
    }

    pub fn assign_array(
        &mut self,
        name: &str,
        append: bool,
        items: &[(Option<i64>, String)],
        global: bool,
    ) -> Result<(), &'static str> {
        let mut map = if append {
            self.declared_variable(name, global)
                .map_or_else(BTreeMap::new, |v| {
                    v.elements.map_or_else(
                        || v.value.into_iter().map(|s| (0, s)).collect(),
                        Rc::unwrap_or_clone,
                    )
                })
        } else {
            BTreeMap::new()
        };
        let mut next = map
            .last_key_value()
            .map_or(Some(0), |(&i, _)| i.checked_add(1));
        for (index, value) in items {
            let i = match index {
                Some(n) if *n < 0 => map
                    .last_key_value()
                    .and_then(|(&i, _)| i.checked_add(*n + 1))
                    .filter(|i| *i >= 0),
                Some(n) => Some(*n),
                None => next,
            }
            .ok_or("array subscript out of range")?;
            map.insert(i, value.clone());
            next = i.checked_add(1);
        }
        self.store_array(name, map, global);
        Ok(())
    }

    pub(crate) fn store_array(&mut self, name: &str, values: BTreeMap<i64, String>, global: bool) {
        let mut variable = self.declared_variable(name, global).unwrap_or(Variable {
            array_assigned: false,
            elements: None,
            declared: true,
            value: None,
            exported: false,
        });
        variable.declared = true;
        variable.value = None;
        variable.array_assigned = true;
        variable.elements = Some(Rc::new(values));
        if global {
            if let Some(entry) = self
                .locals
                .iter_mut()
                .find_map(|scope| scope.0.get_mut(name))
            {
                *entry = Some(variable);
                return;
            }
        }
        self.variables.insert(name.into(), variable);
    }

    pub fn resolve_index(&self, name: &str, expr: &str) -> Result<i64, &'static str> {
        if expr.trim().is_empty() {
            return Err("empty array subscript");
        }
        let n = crate::arithmetic::evaluate(expr, &|k| self.get(k))?;
        if n < 0 {
            self.last_index(name)
                .and_then(|i| i.checked_add(n + 1))
                .filter(|i| *i >= 0)
                .ok_or("array subscript out of range")
        } else {
            Ok(n)
        }
    }

    pub fn last_index(&self, name: &str) -> Option<i64> {
        if let Some(v) = self.variables.get(name) {
            return match &v.elements {
                Some(map) => map.last_key_value().map(|(&k, _)| k),
                None => v.value.as_ref().map(|_| 0),
            };
        }
        self.indices(name).last().copied()
    }

    pub fn assign_element(&mut self, name: &str, index: i64, value: String) {
        if !self.variables.contains_key(name) {
            let inherited = self.inherited_value(name);
            let exported = self.is_exported(name);
            self.variables.insert(
                name.into(),
                Variable {
                    declared: true,
                    value: inherited,
                    array_assigned: false,
                    elements: None,
                    exported,
                },
            );
        }
        let var = self.variables.get_mut(name).unwrap();
        var.declared = true;
        var.array_assigned = true;
        var.make_array().insert(index, value);
    }

    pub(super) fn unset_element(&mut self, name: &str, index: i64) {
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(array) = var.elements.as_mut().map(Rc::make_mut) {
                array.remove(&index);
            } else if index == 0 {
                var.value = None;
                var.declared = false;
            }
        } else if index == 0 && self.inherited_value(name).is_some() {
            self.variables.insert(
                name.into(),
                Variable {
                    declared: false,
                    value: None,
                    array_assigned: false,
                    elements: None,
                    exported: false,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_arrays_survive_scope_changes_and_restore_the_outer_scalar() {
        let mut shell = Session::default();
        shell.assign("a", "outer");
        shell.enter_function();
        assert_eq!(shell.declare("local", &["-a".into(), "a".into()]), 0);
        shell
            .assign_array(
                "a",
                false,
                &[(Some(3), "third".into()), (None, "fourth".into())],
                false,
            )
            .unwrap();
        shell.assign_target("a[-1]", "last").unwrap();
        assert_eq!(shell.get("a[4]").as_deref(), Some("last"));
        assert_eq!(shell.get("\u{1}a").as_deref(), Some("3 4"));
        assert_eq!(shell.get("a"), None);
        let saved = shell.save_variables(&["a"]);
        shell.assign_element("a", 3, "changed".into());
        shell.restore(saved);
        assert_eq!(shell.get("a[3]").as_deref(), Some("third"));
        shell.leave_function();
        assert_eq!(shell.get("a").as_deref(), Some("outer"));
    }
}
