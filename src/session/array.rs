use super::{Session, Variable};
use crate::parser::Subscript;
use std::collections::BTreeMap;
use std::rc::Rc;

pub(crate) fn subscript(name: &str) -> Option<(&str, &str)> {
    let (base, expr) = name.split_once('[')?;
    Some((base, expr.strip_suffix(']')?))
}

pub(crate) fn absolute_index(offset: i64, last: impl FnOnce() -> Option<i64>) -> Option<i64> {
    if offset < 0 {
        // Adding one to the offset also works when the last index is i64::MAX.
        last()?.checked_add(offset + 1).filter(|&i| i >= 0)
    } else {
        Some(offset)
    }
}

pub(crate) enum Index {
    Literal(i64),
    Expression(String),
}

impl Index {
    pub(crate) fn new(text: String) -> Self {
        if let Some(number) = Session::plain_decimal(&text) {
            return Self::Literal(number);
        }
        Self::Expression(text)
    }
}

pub(crate) enum ArrayItems {
    Indexed(Vec<(Option<Subscript<Index>>, String)>),
    Associative(Vec<(String, String, bool)>, Option<&'static str>),
}

impl ArrayItems {
    pub(crate) fn initializer_error(&self) -> Option<&'static str> {
        match self {
            Self::Associative(_, error) => *error,
            Self::Indexed(_) => None,
        }
    }
}

// A full mapfile read produces consecutive UTF-8 records. Keep their storage
// together until the first indexed write needs independently owned strings.
#[derive(Clone, Default)]
pub(super) struct Records {
    text: String,
    ends: Vec<usize>,
}

impl Records {
    pub(super) fn push(&mut self, value: &str) {
        self.text.push_str(value);
        self.ends.push(self.text.len());
    }

    fn get(&self, index: usize) -> Option<&str> {
        let end = *self.ends.get(index)?;
        let start = if index == 0 { 0 } else { self.ends[index - 1] };
        Some(&self.text[start..end])
    }

    fn values(&self) -> impl Iterator<Item = &str> {
        let mut start = 0;
        self.ends.iter().map(move |&end| {
            let value = &self.text[start..end];
            start = end;
            value
        })
    }
}

#[derive(Clone)]
pub(super) enum Elements {
    Records(Records),
    Dense(Vec<String>),
    Associative(super::assoc::Associative),
    Sparse(BTreeMap<i64, String>),
}

impl Elements {
    pub(super) fn get(&self, index: &i64) -> Option<&str> {
        match self {
            Self::Records(r) => r.get(usize::try_from(*index).ok()?),
            Self::Dense(values) => values
                .get(usize::try_from(*index).ok()?)
                .map(String::as_str),
            Self::Sparse(values) => values.get(index).map(String::as_str),
            Self::Associative(map) => map.get(&index.to_string()).map(String::as_str),
        }
    }

    fn last_index(&self) -> Option<i64> {
        match self {
            Self::Associative(_) => None,
            Self::Records(r) => r.ends.len().checked_sub(1).map(|i| i as i64),
            Self::Dense(values) => values.len().checked_sub(1).map(|i| i as i64),
            Self::Sparse(values) => values.last_key_value().map(|(&i, _)| i),
        }
    }

    fn indices(&self) -> Vec<i64> {
        match self {
            Self::Associative(_) => Vec::new(),
            Self::Records(r) => (0..r.ends.len() as i64).collect(),
            Self::Dense(values) => (0..values.len() as i64).collect(),
            Self::Sparse(values) => values.keys().copied().collect(),
        }
    }

    pub(super) fn iter(&self) -> Box<dyn Iterator<Item = (i64, &str)> + '_> {
        match self {
            Self::Associative(_) => unreachable!("indexed iterator on associative array"),
            Self::Records(r) => Box::new((0..).zip(r.values())),
            Self::Dense(values) => Box::new((0..).zip(values.iter().map(String::as_str))),
            Self::Sparse(values) => Box::new(values.iter().map(|(&i, v)| (i, v.as_str()))),
        }
    }

    fn make_writable(&mut self) {
        if let Self::Records(records) = self {
            *self = Self::Dense(records.values().map(str::to_owned).collect());
        }
    }

    fn sparse(&mut self) -> &mut BTreeMap<i64, String> {
        if let Self::Dense(values) = self {
            *self = Self::Sparse((0..).zip(std::mem::take(values)).collect());
        }
        let Self::Sparse(values) = self else {
            unreachable!()
        };
        values
    }

    pub(super) fn insert(&mut self, index: i64, value: String) {
        self.make_writable();
        if let Self::Associative(map) = self {
            map.insert(index.to_string(), value);
            return;
        }
        if let Self::Dense(values) = self
            && let Ok(i) = usize::try_from(index)
        {
            if let Some(slot) = values.get_mut(i) {
                *slot = value;
                return;
            }
            if i == values.len() {
                values.push(value);
                return;
            }
        }
        self.sparse().insert(index, value);
    }

    fn remove(&mut self, index: i64) {
        self.make_writable();
        if let Self::Dense(values) = self {
            if index < 0 || index as usize >= values.len() {
                return;
            }
            if index as usize + 1 == values.len() {
                values.pop();
                return;
            }
        }
        self.sparse().remove(&index);
    }
}

impl Variable {
    pub(super) fn make_array(&mut self) -> &mut Elements {
        let initial = self.scalar_mut().take();
        self.array_assigned |= initial.is_some();
        Rc::make_mut(
            self.elements
                .get_or_insert_with(|| Rc::new(Elements::Dense(initial.into_iter().collect()))),
        )
    }
}

impl Session {
    pub(super) fn active_groups(&self, name: &str) -> bool {
        name == "GROUPS" && !self.groups_unset
    }

    pub(super) fn protected_array(&self, name: &str) -> bool {
        self.active_groups(name) || name == "FUNCNAME" && !self.funcname_unset
    }

    pub(crate) fn array_parameter(&self, name: &str) -> bool {
        self.variables.get(name).map_or_else(
            || {
                matches!(
                    name,
                    "BASH_ALIASES"
                        | "BASH_CMDS"
                        | "BASH_ARGC"
                        | "BASH_ARGV"
                        | "BASH_LINENO"
                        | "BASH_SOURCE"
                        | "BASH_VERSINFO"
                        | "DIRSTACK"
                        | "FUNCNAME"
                        | "GROUPS"
                        | "PIPESTATUS"
                )
            },
            |var| var.elements.is_some(),
        )
    }

    fn group_ids() -> &'static [String] {
        static GROUPS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        GROUPS.get_or_init(|| {
            let primary = unsafe { libc::getgid() };
            let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
            let mut ids = vec![0; n.max(0) as usize];
            if n > 0 {
                let used = unsafe { libc::getgroups(n, ids.as_mut_ptr()) };
                ids.truncate(used.max(0) as usize);
            }
            if let Some(i) = ids.iter().position(|&id| id == primary) {
                ids.swap(0, i);
            } else {
                ids.insert(0, primary);
            }
            ids.into_iter().map(|id| id.to_string()).collect()
        })
    }

    pub fn valid_target(&self, target: &str) -> bool {
        self.check_target(target, false)
    }

    pub fn valid_builtin_target(&self, target: &str) -> bool {
        self.check_target(target, !self.shopt.array_expand_once)
    }

    fn check_target(&self, target: &str, check_syntax: bool) -> bool {
        let Some((base, expr)) = subscript(target) else {
            return super::is_identifier(target);
        };
        if !super::is_identifier(base) {
            return false;
        }
        if !self.is_associative(base) {
            return self.resolve_index(base, expr).is_ok();
        }
        !expr.is_empty()
            && (!check_syntax
                || crate::parser::index_end(&target[base.len()..]).ok()
                    == Some(target.len() - base.len()))
    }

    pub fn assign_target(&mut self, target: &str, value: &str) -> Result<(), &'static str> {
        self.assign_target_value(target, value, false)
    }

    pub fn assign_builtin_target(&mut self, target: &str, value: &str) -> Result<(), &'static str> {
        self.assign_target_value(
            target,
            value,
            self.shopt.array_expand_once && target.as_bytes().last() == Some(&b']'),
        )
    }

    fn assign_target_value(
        &mut self,
        target: &str,
        value: &str,
        literal_key: bool,
    ) -> Result<(), &'static str> {
        let resolved = self.reference_name(target)?;
        let target = resolved.as_ref();
        let element = subscript(target);
        let base = element.map_or(target, |(base, _)| base);
        if self.readonly(base) {
            return Err(crate::session::READONLY);
        }
        if self.is_nameref(base) && !Self::valid_reference_target(value) {
            return Err("invalid name reference");
        }
        let number;
        let value = if self.integer(base) {
            number = crate::arithmetic::run(value, self)?.to_string();
            number.as_str()
        } else {
            value
        };
        if let Some((base, expr)) = element {
            if !super::is_identifier(base) {
                return Err("invalid variable name");
            }
            if self.is_associative(base) {
                let key = if literal_key {
                    if expr.is_empty() {
                        return Err("bad array subscript");
                    }
                    expr.to_owned()
                } else {
                    self.subscript_key(expr)?
                };
                return self.assign_key(base, key, value.into());
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

    fn debug_frames(&self) -> impl DoubleEndedIterator<Item = &Vec<String>> {
        if self.function_depth == 0 {
            self.debug_base
                .get_or_init(|| self.positional.as_slice().to_vec());
        }
        self.debug_base.get().into_iter().chain(&self.debug_args)
    }

    pub(crate) fn indices(&self, name: &str) -> Vec<i64> {
        if name == "PIPESTATUS" {
            let len = usize::from(self.pipe_status.first.is_some()) + self.pipe_status.rest.len();
            return (0..len as i64).collect();
        }
        if let Some(v) = self.variables.get(name) {
            return v.elements.as_ref().map_or_else(
                || (v.value.is_some()).then_some(0).into_iter().collect(),
                |elements| elements.indices(),
            );
        }
        match name {
            "BASH_ARGV" => (0..self.debug_frames().map(Vec::len).sum::<usize>() as i64).collect(),
            "BASH_ARGC" => (0..self.debug_frames().count() as i64).collect(),
            "GROUPS" => {
                self.dynamic.groups_read.set(true);
                (0..Self::group_ids().len() as i64).collect()
            }
            "BASH_VERSINFO" => (0..6).collect(),
            "DIRSTACK" => {
                self.dynamic.stack_read.set(true);
                (0..=self.dir_stack.len() as i64).collect()
            }
            "FUNCNAME" if self.function_depth == 0 => Vec::new(),
            "FUNCNAME" => (0..self.function_names.len() as i64).collect(),
            "BASH_LINENO" => (0..self.call_lines.len() as i64).collect(),
            "BASH_SOURCE" => (0..self.source_names.len() as i64).collect(),
            _ => self.inherited_value(name).map(|_| 0).into_iter().collect(),
        }
    }

    pub(crate) fn element(&self, name: &str, index: i64) -> Option<String> {
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
            if v.dynamic && index == 0 {
                return self.get(name);
            }
            return match v.elements.as_ref() {
                Some(a) => a.get(&index).map(str::to_owned),
                None if index == 0 => v.value.clone(),
                _ => None,
            };
        }
        match name {
            "BASH_ARGV" => {
                let mut offset = usize::try_from(index).ok()?;
                self.debug_frames().rev().find_map(|args| {
                    if offset < args.len() {
                        Some(args[args.len() - 1 - offset].clone())
                    } else {
                        offset -= args.len();
                        None
                    }
                })
            }
            "BASH_ARGC" => self
                .debug_frames()
                .rev()
                .nth(index as usize)
                .map(|v| v.len().to_string()),
            "GROUPS" => {
                self.dynamic.groups_read.set(true);
                Self::group_ids().get(usize::try_from(index).ok()?).cloned()
            }
            "BASH_VERSINFO" => match index {
                5 => Some(Self::machine_type()),
                i => super::BASH_VERSINFO
                    .get(usize::try_from(i).ok()?)
                    .map(|s| (*s).into()),
            },
            "DIRSTACK" => {
                self.dynamic.stack_read.set(true);
                self.stack_entry(index.try_into().ok()?)
            }
            "FUNCNAME" if self.function_depth == 0 => None,
            "FUNCNAME" => self
                .function_names
                .iter()
                .rev()
                .nth(index as usize)
                .cloned(),
            "BASH_LINENO" => self
                .call_lines
                .iter()
                .rev()
                .nth(index as usize)
                .map(usize::to_string),
            "BASH_SOURCE" => self
                .source_names
                .iter()
                .rev()
                .nth(index as usize)
                .map(ToString::to_string),
            _ if index == 0 => self.inherited_value(name),
            _ => None,
        }
    }

    pub(crate) fn stored_array(&self, name: &str) -> bool {
        name != "PIPESTATUS"
            && self
                .variables
                .get(name)
                .is_some_and(|v| !v.nameref && v.elements.is_some())
    }

    pub fn array_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .variables
            .iter()
            .filter(|(_, v)| v.elements.is_some() && v.array_assigned)
            .map(|(key, _)| key.clone())
            .collect();
        if !self.variables.contains_key("DIRSTACK") {
            names.push("DIRSTACK".into());
        }
        for name in ["BASH_ALIASES", "BASH_CMDS"] {
            if self.live_table(name) {
                names.push(name.into());
            }
        }
        names.push("BASH_SOURCE".into());
        names.push("BASH_LINENO".into());
        names.push("BASH_VERSINFO".into());
        if self.active_groups("GROUPS") {
            names.push("GROUPS".into());
        }
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

    pub(crate) fn stored_array_len(&self, name: &str) -> Option<usize> {
        if self.active_groups(name) && !self.variables.contains_key(name) {
            return Some(Self::group_ids().len());
        }
        if self.live_table(name) {
            return Some(if name == "BASH_ALIASES" {
                self.aliases.0.as_ref().map_or(0, |m| m.len())
            } else {
                self.hashes.items().count()
            });
        }
        if name == "PIPESTATUS" {
            return None;
        }
        match self.variables.get(name)?.elements.as_deref()? {
            Elements::Dense(values) => Some(values.len()),
            Elements::Sparse(values) => Some(values.len()),
            Elements::Records(records) => Some(records.ends.len()),
            Elements::Associative(map) => Some(map.len()),
        }
    }

    pub(crate) fn stored_indexed_slice(
        &self,
        name: &str,
        offset: i64,
        count: usize,
    ) -> Option<Vec<String>> {
        let resolved = self.reference_name(name).ok()?;
        let name = resolved.as_ref();
        let variable = self.variables.get(name)?;
        if variable.dynamic || name == "PIPESTATUS" {
            return None;
        }
        let elements = variable.elements.as_deref()?;
        let start = self.relative_index(name, offset).ok();
        let range = |len: usize| {
            let from = start.map_or(len, |i| (i as usize).min(len));
            from..from.saturating_add(count).min(len)
        };
        match elements {
            Elements::Associative(_) => None,
            Elements::Sparse(values) => Some(start.map_or_else(Vec::new, |i| {
                values
                    .range(i..)
                    .take(count)
                    .map(|(_, value)| value.clone())
                    .collect()
            })),
            Elements::Dense(values) => Some(values[range(values.len())].to_vec()),
            Elements::Records(records) => Some(
                range(records.ends.len())
                    .filter_map(|i| records.get(i).map(str::to_owned))
                    .collect(),
            ),
        }
    }

    pub fn append_array_fields(&self, name: &str, keys: bool, fields: &mut Vec<String>) -> bool {
        let Ok(resolved) = self.reference_name(name) else {
            return false;
        };
        let name = resolved.as_ref();
        if !self.live_table(name)
            && let Some(map) = self.variables.get(name).and_then(Variable::associative)
        {
            if keys {
                fields.extend_from_slice(map.ordered_keys());
            } else {
                fields.extend(map.ordered().map(|(_, v)| v.to_owned()));
            }
            return true;
        }
        if keys {
            if !self.stored_array(name) || self.is_associative(name) {
                return false;
            }
            let mut decimal = itoa::Buffer::new();
            fields.extend(
                self.indices(name)
                    .into_iter()
                    .map(|i| decimal.format(i).to_owned()),
            );
            return true;
        }
        if self.active_groups(name) && !self.variables.contains_key(name) {
            fields.extend_from_slice(Self::group_ids());
            return true;
        }
        if name == "PIPESTATUS" {
            return false;
        }
        match self.variables.get(name).and_then(|v| v.elements.as_deref()) {
            Some(Elements::Dense(values)) => fields.extend_from_slice(values),
            Some(Elements::Sparse(values)) => fields.extend(values.values().cloned()),
            Some(Elements::Records(records)) => fields.extend(records.values().map(str::to_owned)),
            _ => return false,
        }
        true
    }

    pub fn array_values(&self, name: &str) -> Vec<String> {
        let Ok(resolved) = self.reference_name(name) else {
            return Vec::new();
        };
        let name = resolved.as_ref();
        if self.active_groups(name) && !self.variables.contains_key(name) {
            self.dynamic.groups_read.set(true);
            return Self::group_ids().to_vec();
        }
        if self.live_table(name) {
            return self
                .table_snapshot(name)
                .ordered()
                .map(|(_, v)| v.to_owned())
                .collect();
        }
        if let Some(map) = self.variables.get(name).and_then(Variable::associative) {
            return map.ordered().map(|(_, v)| v.to_owned()).collect();
        }
        self.indices(name)
            .into_iter()
            .filter_map(|index| self.element(name, index))
            .collect()
    }

    // The caller restores the original COMPREPLY binding immediately afterwards.
    // A callback may redirect the name through a nameref or share its storage.
    pub(crate) fn take_completion_reply(&mut self) -> Vec<String> {
        if let Some(variable) = self.variables.get_mut("COMPREPLY")
            && !variable.nameref
            && let Some(Elements::Dense(values)) = variable.elements.as_mut().and_then(Rc::get_mut)
        {
            return std::mem::take(values);
        }
        self.array_values("COMPREPLY")
    }

    pub(crate) fn store_completions(
        &mut self,
        name: &str,
        words: Vec<String>,
    ) -> Result<(), &'static str> {
        let target = self.reference_name(name)?;
        if self.readonly(&target) {
            return Err(crate::session::READONLY);
        }
        if self.is_associative(&target) || !super::is_identifier(&target) {
            return Err("not an indexed array");
        }
        if let Some(variable) = self.variables.get_mut(target.as_ref()) {
            variable.declared = true;
            variable.array_assigned = true;
            *variable.scalar_mut() = None;
            variable.elements = Some(Rc::new(Elements::Dense(words)));
        } else {
            self.store_elements(&target, Elements::Dense(words), false);
        }
        Ok(())
    }

    pub(crate) fn indexed_receiver<'a>(
        &mut self,
        name: &'a str,
        replace_associative: bool,
    ) -> Result<std::borrow::Cow<'a, str>, &'static str> {
        let target = self.reference_name(name)?;
        if !super::is_identifier(&target) {
            return Err("array destination must be a variable name");
        }
        if Self::builtin_readonly(&target) {
            return Err(crate::session::READONLY);
        }
        if !replace_associative && self.live_table(&target) {
            return Err("not an indexed array");
        }
        if let Some(var) = self.variables.get_mut(target.as_ref()) {
            if var.readonly {
                return Err(crate::session::READONLY);
            }
            if !replace_associative && var.associative().is_some() {
                return Err("not an indexed array");
            }
            if var.nameref {
                diagnostic!("warning: {target}: removing nameref attribute");
                var.nameref = false;
            }
        }
        Ok(target)
    }

    // Store each converted integer immediately: the next read field can refer to it.
    pub(super) fn assign_input_element(
        &mut self,
        name: &str,
        index: i64,
        text: &str,
    ) -> Result<(), &'static str> {
        let value = if self.integer(name) && Self::plain_decimal(text).is_none() {
            crate::arithmetic::run(text, self)?.to_string()
        } else {
            text.to_owned()
        };
        self.assign_element(name, index, value);
        Ok(())
    }

    pub fn replace_array(&mut self, name: &str, mut values: Vec<String>) {
        if let Some(upper) = self.variables.get(name).and_then(|v| v.letter_case) {
            for v in &mut values {
                *v = self.convert_case(std::mem::take(v), Some(upper));
            }
        }
        self.store_elements(name, Elements::Dense(values), false);
    }

    pub(crate) fn assign_expanded_array(
        &mut self,
        name: &str,
        append: bool,
        items: ArrayItems,
    ) -> Result<(), &'static str> {
        if let ArrayItems::Indexed(words) = &items
            && !append
            && super::is_identifier(name)
            && !self.protected_array(name)
            && self.variables.get(name).is_none_or(|v| {
                !v.readonly
                    && !v.dynamic
                    && !v.integer
                    && !v.nameref
                    && v.letter_case.is_none()
                    && v.associative().is_none()
            })
            && words.iter().all(|(index, _)| index.is_none())
        {
            let ArrayItems::Indexed(words) = items else {
                unreachable!()
            };
            let values = words.into_iter().map(|(_, value)| value).collect();
            self.store_elements(name, Elements::Dense(values), false);
            return Ok(());
        }
        self.assign_array(name, append, &items, false)
    }

    pub fn assign_array(
        &mut self,
        name: &str,
        append: bool,
        items: &ArrayItems,
        global: bool,
    ) -> Result<(), &'static str> {
        let resolved = self.reference_name(name)?;
        let name = resolved.as_ref();
        if self.protected_array(name) {
            return Ok(());
        }
        if subscript(name).is_some() {
            return Err("cannot assign an array to an array element");
        }
        // Expanded, unnumbered words can extend an ordinary array in place.
        // Check the last index before writing so an overflowing append is atomic.
        if append
            && !global
            && let ArrayItems::Indexed(words) = items
            && let Some(var) = self.variables.get_mut(name)
            && !var.readonly
            && !var.integer
            && !var.nameref
            && var.letter_case.is_none()
            && var.associative().is_none()
            && words.iter().all(|(index, _)| index.is_none())
        {
            let last = var
                .elements
                .as_deref()
                .and_then(Elements::last_index)
                .unwrap_or(if var.value.is_some() { 0 } else { -1 });
            last.checked_add(words.len() as i64)
                .ok_or(crate::session::SUBSCRIPT_RANGE)?;
            let array = var.make_array();
            for (offset, (_, word)) in words.iter().enumerate() {
                array.insert(last + (offset as i64 + 1), word.clone());
            }
            var.declared = true;
            var.array_assigned = true;
            return Ok(());
        }
        let mut var = self.declared_variable(name, global).unwrap_or_default();
        if var.readonly {
            return Err(crate::session::READONLY);
        }
        var.nameref = false;
        let result = self.array_items(name, global, &mut var, append, items);
        self.store_variable(name, var, global);
        result
    }

    // Arithmetic can read or modify earlier entries, including through a nameref.
    // Temporarily expose the in-progress value to the ordinary arithmetic evaluator.
    fn initializer_number(
        &mut self,
        name: &str,
        var: &Variable,
        map: &mut Elements,
        visible: bool,
        expression: &str,
    ) -> Result<i64, &'static str> {
        if let Some(number) = Self::plain_decimal(expression) {
            return Ok(number);
        }
        if !visible {
            return crate::arithmetic::run(expression, self);
        }
        if let Some((base, index)) = subscript(expression)
            && base == name
            && let Some(index) = Self::plain_decimal(index)
            && let Some(value) = map.get(&index).and_then(Self::plain_decimal)
        {
            return Ok(value);
        }
        let mut current = var.clone();
        *current.scalar_mut() = None;
        current.elements = Some(Rc::new(std::mem::replace(map, Elements::Dense(Vec::new()))));
        let previous = self.variables.insert(name.into(), current);
        let result = crate::arithmetic::run(expression, self);
        let mut current = self.variables.remove(name).expect("active initializer");
        if let Some(previous) = previous {
            self.variables.insert(name.into(), previous);
        }
        *map = Rc::unwrap_or_clone(current.elements.take().expect("indexed initializer"));
        result
    }

    pub(super) fn array_items(
        &mut self,
        name: &str,
        global: bool,
        var: &mut Variable,
        append: bool,
        items: &ArrayItems,
    ) -> Result<(), &'static str> {
        if var.dynamic
            && matches!(name, "BASH_ALIASES" | "BASH_CMDS")
            && let ArrayItems::Associative(items, _) = items
        {
            for (key, value, concat) in items {
                let value = if *concat {
                    self.assoc_value(name, key).unwrap_or_default().to_owned() + value
                } else {
                    value.clone()
                };
                self.assign_key(name, key.clone(), value)?;
            }
            return Ok(());
        }
        let items = match items {
            ArrayItems::Associative(items, _) => {
                var.array_assigned = true;
                var.declared = true;
                let integer = var.integer;
                let letter_case = var.letter_case;
                let map = var.make_associative()?;
                if !append {
                    *map = Default::default();
                }
                for (key, value, concat) in items {
                    let old = map.get(key).map(String::as_str);
                    let value = if integer {
                        self.integer_value(old, value, *concat)?
                    } else if *concat {
                        old.unwrap_or_default().to_owned() + value
                    } else {
                        value.clone()
                    };
                    map.insert(key.clone(), self.convert_case(value, letter_case));
                }
                return Ok(());
            }
            ArrayItems::Indexed(items) if var.associative().is_none() => items,
            _ => return Err("cannot convert associative to indexed array"),
        };
        let mut map = if append {
            var.elements.take().map_or_else(
                || Elements::Dense(var.scalar_mut().take().into_iter().collect()),
                Rc::unwrap_or_clone,
            )
        } else {
            Elements::Dense(Vec::with_capacity(items.len()))
        };
        // With declare -g, an existing local still controls name resolution.
        let visible = !global
            || !self
                .locals
                .iter()
                .any(|scope| scope.variables.contains_key(name));
        let result = (|| {
            let mut next = map.last_index().map_or(Some(0), |i| i.checked_add(1));
            for (index, value) in items {
                let concat = index.as_ref().is_some_and(|s| s.append);
                let index = match index.as_ref().map(|s| &s.index) {
                    Some(Index::Literal(i)) => Some(*i),
                    Some(Index::Expression(expr)) if expr.is_empty() => {
                        return Err("bad array subscript");
                    }
                    Some(Index::Expression(expr)) if matches!(expr.as_str(), "*" | "@") => {
                        return Err("cannot assign to non-numeric index");
                    }
                    Some(Index::Expression(expr)) => {
                        Some(self.initializer_number(name, var, &mut map, visible, expr)?)
                    }
                    None => None,
                };
                let i = match index {
                    Some(n) if n < 0 => map
                        .last_index()
                        .and_then(|i| i.checked_add(n + 1))
                        .filter(|i| *i >= 0),
                    Some(n) => Some(n),
                    None => next,
                }
                .ok_or(crate::session::SUBSCRIPT_RANGE)?;
                let value = if var.integer {
                    let old = if concat {
                        map.get(&i).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0)
                    } else {
                        0
                    };
                    old.wrapping_add(self.initializer_number(name, var, &mut map, visible, value)?)
                        .to_string()
                } else if concat {
                    map.get(&i).unwrap_or_default().to_owned() + value
                } else {
                    value.clone()
                };
                let value = self.convert_case(value, var.letter_case);
                if let Elements::Dense(values) = &mut map
                    && i == values.len() as i64
                {
                    values.push(value);
                } else {
                    map.insert(i, value);
                }
                next = i.checked_add(1);
            }
            Ok(())
        })();
        var.elements = Some(Rc::new(map));
        *var.scalar_mut() = None;
        var.declared = true;
        var.array_assigned = true;
        result
    }

    pub(super) fn store_elements(&mut self, name: &str, values: Elements, global: bool) {
        let mut variable = self.declared_variable(name, global).unwrap_or_default();
        if variable.readonly {
            return;
        }
        variable.declared = true;
        *variable.scalar_mut() = None;
        variable.array_assigned = true;
        variable.elements = Some(Rc::new(values));
        self.store_variable(name, variable, global);
    }

    pub(super) fn declaration_index(
        &mut self,
        name: &str,
        expr: &str,
        global: bool,
        var: &mut Variable,
    ) -> Result<i64, &'static str> {
        if let Some(n) = Self::plain_decimal(expr) {
            return self.relative_index(name, n);
        }
        if expr.trim().is_empty() {
            return Err("empty array subscript");
        }
        var.make_array();
        let mut elements = Rc::unwrap_or_clone(var.elements.take().unwrap());
        let visible = !global
            || !self
                .locals
                .iter()
                .any(|scope| scope.variables.contains_key(name));
        let result = self
            .initializer_number(
                name,
                var,
                &mut elements,
                visible,
                crate::arithmetic::numeric_subscript(expr),
            )
            .and_then(|n| {
                if n >= 0 {
                    Ok(n)
                } else {
                    elements
                        .last_index()
                        .and_then(|i| i.checked_add(n + 1))
                        .filter(|i| *i >= 0)
                        .ok_or(crate::session::SUBSCRIPT_RANGE)
                }
            });
        var.elements = Some(Rc::new(elements));
        result
    }

    pub fn resolve_index(&self, name: &str, expr: &str) -> Result<i64, &'static str> {
        if expr.trim().is_empty() {
            return Err("empty array subscript");
        }
        if let Some(value) = Self::plain_decimal(expr)
            && value >= 0
        {
            return Ok(value);
        }
        let n = crate::arithmetic::index(expr, &|k| self.get(k))?;
        self.relative_index(name, n)
    }

    pub(crate) fn relative_index(&self, name: &str, n: i64) -> Result<i64, &'static str> {
        absolute_index(n, || self.last_index(name)).ok_or(crate::session::SUBSCRIPT_RANGE)
    }

    pub fn last_index(&self, name: &str) -> Option<i64> {
        if let Some(v) = self.variables.get(name) {
            return match &v.elements {
                Some(map) => map.last_index(),
                None => v.value.as_ref().map(|_| 0),
            };
        }
        if name == "DIRSTACK" {
            return Some(self.dir_stack.len() as i64);
        }
        self.indices(name).last().copied()
    }

    pub(crate) fn indexed_scalar_decimal(&self, name: &str, index: i64) -> Option<i64> {
        let var = self.variables.get(name)?;
        if var.dynamic || var.nameref {
            return None;
        }
        match var.elements.as_deref()? {
            Elements::Dense(values) => {
                Self::plain_decimal(values.get(usize::try_from(index).ok()?)?)
            }
            Elements::Sparse(values) => Self::plain_decimal(values.get(&index)?),
            _ => None,
        }
    }

    pub(crate) fn replace_indexed_decimal(&mut self, name: &str, index: i64, value: i64) -> bool {
        if index < 0 || Self::builtin_readonly(name) || self.protected_array(name) {
            return false;
        }
        let Some(var) = self.variables.entries.get_mut(name) else {
            return false;
        };
        if var.dynamic || var.readonly || var.nameref {
            return false;
        }
        // Only an existing indexed element changes. Array shape and export membership stay fixed.
        let Some(elements) = var.elements.as_mut() else {
            return false;
        };
        let slot = match Rc::make_mut(elements) {
            Elements::Dense(values) => values.get_mut(index as usize),
            Elements::Sparse(values) => values.get_mut(&index),
            _ => None,
        };
        let Some(slot) = slot else { return false };
        itoa::Buffer::new().format(value).clone_into(slot);
        var.array_assigned = true;
        var.declared = true;
        true
    }

    pub fn assign_element(&mut self, name: &str, index: i64, value: String) {
        if self.protected_array(name) || self.readonly(name) {
            return;
        }
        if name == "DIRSTACK" && !self.variables.contains_key(name) {
            if index > 0
                && let Some(i) = self.dir_stack.len().checked_sub(index as usize)
            {
                self.dir_stack[i] = value;
            }
            return;
        }
        if !self.variables.contains_key(name) {
            let inherited = self.inherited_value(name);
            let exported = self.is_exported(name);
            self.variables
                .insert(name.into(), Variable::scalar(inherited, exported));
        }
        let value = self.convert_case(value, self.variables[name].letter_case);
        let var = self.variables.get_mut(name).unwrap();
        var.declared = true;
        var.array_assigned = true;
        var.make_array().insert(index, value);
    }

    pub(super) fn unset_element(&mut self, name: &str, index: i64) {
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(array) = var.elements.as_mut().map(Rc::make_mut) {
                array.remove(index);
            } else if index == 0 {
                *var.scalar_mut() = None;
                var.declared = false;
            }
        } else if index == 0 && self.inherited_value(name).is_some() {
            self.variables.insert(name.into(), Variable::default());
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
        shell.enter_function(&mut Default::default());
        assert_eq!(
            shell.declare(
                "local",
                &["-a".into(), "a".into()],
                &[],
                0,
                &mut Default::default()
            ),
            crate::Flow::Continue(0)
        );
        shell
            .assign_array(
                "a",
                false,
                &ArrayItems::Indexed(vec![
                    (
                        Some(Subscript {
                            index: Index::Literal(3),
                            append: false,
                        }),
                        "third".into(),
                    ),
                    (None, "fourth".into()),
                ]),
                false,
            )
            .unwrap();
        shell.assign_target("a[-1]", "last").unwrap();
        assert_eq!(shell.get("a[4]").as_deref(), Some("last"));
        assert_eq!(shell.get("\u{1}a").as_deref(), Some("3\x004"));
        assert_eq!(shell.get("a"), None);
        let saved = shell.save_variables(&["a"]);
        shell.assign_element("a", 3, "changed".into());
        shell.restore(saved);
        assert_eq!(shell.get("a[3]").as_deref(), Some("third"));
        shell.leave_function();
        assert_eq!(shell.get("a").as_deref(), Some("outer"));
    }
}
