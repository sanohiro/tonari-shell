use super::*;
use std::collections::BTreeMap;

impl Session {
    pub fn export(&mut self, args: &[String], bindings: &mut Bindings) -> u8 {
        let mut functions = false;
        let mut remove = false;
        let mut args = args;
        while let Some(opt) = crate::builtins::flags::next(&mut args) {
            for flag in opt[1..].chars() {
                match flag {
                    'n' => remove = true,
                    'f' => functions = true,
                    'p' => {}
                    _ => {
                        diagnostic!("export: unsupported option {opt}");
                        return 2;
                    }
                }
            }
        }
        if functions {
            let mut status = 0;
            if args.is_empty() {
                let mut out = crate::builtins::listing_stdout("export");
                for name in self.functions.names("") {
                    let flags = self.function_attributes(name);
                    if flags.ends_with('x')
                        && self.functions[name].print(&mut out, name, flags).is_err()
                    {
                        return 1;
                    }
                }
                if out.flush().is_err() {
                    return 1;
                }
            } else {
                for name in args {
                    if !remove && name.contains('/') && self.functions.contains_key(name) {
                        diagnostic!("export: {name}: cannot export");
                        status = 1;
                        continue;
                    }
                    if let Err(e) = self.export_function(name, !remove) {
                        diagnostic!("export: {name}: {e}");
                        status = 1;
                    }
                }
            }
            return status;
        }
        if args.is_empty() {
            return self.list_exported().map_or(1, |()| 0);
        }
        let mut status = 0;
        for arg in args {
            let (key, value) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
            let (name, append) = if value.is_some() {
                key.strip_suffix('+').map_or((key, false), |k| (k, true))
            } else {
                (key, false)
            };
            if !is_identifier(name) {
                diagnostic!("export: {arg}: invalid identifier");
                status = 1;
                continue;
            }
            let resolved = match self.reference_name(name) {
                Ok(resolved) => resolved,
                Err(e) => {
                    diagnostic!("export: {name}: {e}");
                    status = 1;
                    continue;
                }
            };
            let target = resolved.as_ref();
            // An array element assignment uses its resolved subscript. The
            // export attribute itself belongs to the array variable.
            let element = array::subscript(target);
            let name = element.map_or(target, |(base, _)| base);
            let readonly = self.readonly(name);
            if readonly && value.is_some() {
                diagnostic!("export: {name}: readonly variable");
                status = 1;
                continue;
            }
            if name == "PATH" && value.is_some() {
                self.hashes.clear();
            }
            let value = if element.is_some() {
                if let Some(value) = value {
                    let value = if append {
                        Cow::Owned(self.get(target).unwrap_or_default() + value)
                    } else {
                        Cow::Borrowed(value)
                    };
                    if let Err(e) = self.assign_builtin_target(target, &value) {
                        diagnostic!("export: {target}: {e}");
                        status = 1;
                        continue;
                    }
                }
                None
            } else {
                value
            };
            let integer = self.integer(name);
            let normalized;
            let (value, append) = if integer && let Some(value) = value {
                let original = append.then(|| self.get(name)).flatten();
                match self.integer_value(original.as_deref(), value, append) {
                    Ok(v) => {
                        normalized = v;
                        (Some(normalized.as_str()), false)
                    }
                    Err(e) => {
                        diagnostic!("export: {e}");
                        status = 1;
                        continue;
                    }
                }
            } else {
                (value, append)
            };
            if let Some(text) = value {
                let text = if append {
                    Cow::Owned(self.get(name).unwrap_or_default() + text)
                } else {
                    Cow::Borrowed(text)
                };
                self.assign_value(name, text);
            }
            // Export takes a snapshot. Launching successive child processes
            // must not advance the random sequence on behalf of the parent.
            let snapshot = self
                .variables
                .get(name)
                .filter(|var| var.dynamic)
                .and_then(|_| self.dynamic_number(name));
            if let Some(var) = self.variables.get_mut(name) {
                if let Some(number) = snapshot {
                    *var.scalar_mut() = Some(number.to_string());
                }
                var.exported = !remove;
                var.declared = true;
            } else {
                let mut var = match self.declared_variable(name, false) {
                    Some(var) => var,
                    None if remove => continue,
                    None => Default::default(),
                };
                var.exported = !remove;
                var.declared = true;
                self.variables.insert(name.into(), var);
            }
            if !remove {
                // An explicit export inside this invocation promotes its prefix
                // binding into the surrounding shell.
                bindings.variables.remove(name);
            }
        }
        status
    }

    fn list_exported(&self) -> io::Result<()> {
        let mut vars: BTreeMap<Cow<'_, str>, Option<String>> = std::env::vars_os()
            .filter_map(|(k, v)| {
                let name = k.into_string().ok()?;
                is_identifier(&name).then(|| (Cow::Owned(name), v.into_string().ok()))
            })
            .collect();
        vars.insert("PWD".into(), self.get("PWD"));
        vars.entry("OLDPWD".into()).or_insert(None);
        vars.remove("_");
        for (name, var) in self.variables.iter() {
            if var.exported {
                vars.insert(Cow::Borrowed(name), None);
            } else {
                vars.remove(name.as_str());
            }
        }
        let mut out = crate::builtins::listing_stdout("export");
        let command = self.posix.then_some("export");
        for (name, value) in vars {
            if let Some(var) = self.variables.get(name.as_ref()) {
                if matches!(name.as_ref(), "BASHOPTS" | "SHELLOPTS") {
                    let current = self.declared_variable(&name, false).unwrap();
                    print_attribute(&mut out, &name, &current, command)?;
                } else {
                    print_attribute(&mut out, &name, var, command)?;
                }
                continue;
            }
            print_attribute(&mut out, &name, &Variable::scalar(value, true), command)?;
        }
        out.flush()
    }

    pub(crate) fn unset_variable(&mut self, name: &str) {
        if Self::builtin_readonly(name) || self.variables.get(name).is_some_and(|v| v.readonly) {
            return;
        }
        if name == "BASH_COMPAT" {
            self.apply_compat("");
        }
        if name == "POSIXLY_CORRECT" {
            self.apply_posix_mode(false);
        }
        if name == "COMP_WORDBREAKS" {
            self.word_breaks_unset = true;
        }
        if name == "GROUPS" {
            self.groups_unset = true;
        }
        if name == "FUNCNAME" {
            self.funcname_unset = true;
        }
        if matches!(name, "IGNOREEOF" | "ignoreeof") {
            self.set_eof_variable(None);
        }
        if name == "GLOBIGNORE" {
            self.shopt.glob.set_ignore(None);
        }
        if name == "OPTIND" {
            self.opt_offset = 0;
        }
        if name == "PATH" {
            self.hashes.clear();
        }
        let scope_index = self
            .locals
            .iter()
            .rposition(|scope| scope.variables.contains_key(name));
        let current = self.variables.get(name);
        if let Some(index) = scope_index
            && (current.is_some_and(|v| v.origin == super::BindingOrigin::CommandPrefix)
                || !self.shopt.localvar_unset && index + 1 < self.locals.len())
        {
            // Unsetting a caller's local exposes the binding it was shadowing.
            let scope = &mut self.locals[index];
            if let Some(original) = scope.variables.remove(name).flatten() {
                self.variables.insert(name.into(), original);
            } else {
                self.variables.remove(name);
            }
            if name == "OPTIND"
                && let Some(offset) = scope.opt_offset.take()
            {
                self.opt_offset = offset;
            }
            if matches!(name, "IGNOREEOF" | "ignoreeof") {
                self.set_eof_variable(self.get(name).as_deref());
            }
            if name == "GLOBIGNORE" {
                self.shopt.glob.set_ignore(self.get(name).as_deref());
            }
            return;
        }
        let declared =
            scope_index.is_some_and(|i| self.shopt.localvar_unset || i + 1 == self.locals.len());
        // Unset clears a local's attributes. A local made from a command
        // prefix keeps export for the next assignment, without exposing the
        // value of the previously shadowed binding.
        let exported = declared
            && current
                .is_some_and(|v| v.exported && v.origin == super::BindingOrigin::LocalFromPrefix);
        self.variables.insert(
            name.into(),
            Variable {
                declared,
                exported,
                ..Variable::default()
            },
        );
    }

    pub fn unset(&mut self, args: &[String]) -> crate::Flow {
        let mut args = args;
        let mut literal_ref = false;
        let mut functions_only = false;
        let mut variables_only = false;
        while let Some(opt) = crate::builtins::flags::next(&mut args) {
            for flag in opt[1..].chars() {
                match flag {
                    'n' => literal_ref = true,
                    'v' => variables_only = true,
                    'f' => functions_only = true,
                    _ => {
                        diagnostic!("unset: unsupported option {opt}");
                        return crate::Flow::Continue(2);
                    }
                }
            }
        }
        if functions_only && variables_only {
            diagnostic!("unset: cannot combine -f and -v");
            return crate::Flow::Continue(1);
        }
        self.unset_operands(
            args.iter().map(String::as_str),
            literal_ref,
            functions_only,
            variables_only,
        )
    }

    pub(crate) fn unset_builtin_target(&mut self, name: &str) -> u8 {
        self.unset_operands(std::iter::once(name), false, false, true)
            .child_status()
    }

    fn unset_operands<'a>(
        &mut self,
        args: impl Iterator<Item = &'a str>,
        literal_ref: bool,
        functions_only: bool,
        variables_only: bool,
    ) -> crate::Flow {
        let mut status = 0;
        for original in args {
            if literal_ref && !functions_only && !self.is_nameref(original) {
                continue;
            }
            let resolved;
            let name: &str = if literal_ref || functions_only {
                original
            } else {
                match self.reference_name(original) {
                    Ok(Cow::Owned(n)) => {
                        resolved = n;
                        &resolved
                    }
                    Ok(_) => original,
                    Err(e) => {
                        diagnostic!("unset: {original}: {e}");
                        status = 1;
                        continue;
                    }
                }
            };
            let subscript = array::subscript(name);
            if variables_only && !is_identifier(subscript.map_or(name, |(base, _)| base)) {
                diagnostic!("unset: {original}: not a valid identifier");
                status = 1;
                continue;
            }
            let base = subscript.map_or(name, |(base, _)| base);
            if !functions_only
                && (Self::builtin_readonly(base)
                    || self.variables.get(base).is_some_and(|v| v.readonly))
            {
                diagnostic!("unset: {base}: readonly variable");
                status = 1;
                continue;
            }
            let resolve_function = functions_only
                || !variables_only
                    && (subscript.is_none() || self.readonly_functions.contains(name))
                    && self.variables.get(name).map_or_else(
                        || {
                            self.declared_variable(name, false)
                                .is_none_or(|v| !v.declared)
                        },
                        |var| !var.declared,
                    );
            if resolve_function && self.readonly_functions.contains(name) {
                diagnostic!("unset: {name}: readonly function");
                status = 1;
                continue;
            }
            if !functions_only && let Some((base, index)) = subscript {
                if matches!(index, "*" | "@") && self.stored_array(base) {
                    if self.shopt.compat <= 51 {
                        self.unset_variable(base);
                        continue;
                    }
                    if !self.is_associative(base) {
                        self.replace_array(base, Vec::new());
                        continue;
                    }
                }
                if self.is_associative(base) {
                    if self.shopt.array_expand_once && !index.is_empty() {
                        self.remove_key(base, index);
                        continue;
                    }
                    match crate::arithmetic::data_key(index, self) {
                        Ok(key) => self.remove_key(base, &key),
                        Err(e) => {
                            diagnostic!("unset: {e}");
                            status = 1;
                        }
                    }
                    continue;
                }
                if self.active_groups(base) && !self.variables.contains_key(base) {
                    let var = self.declared_variable(base, false).unwrap();
                    self.variables.insert(base.into(), var);
                }
                let variable = self.variables.get(base);
                let array = variable.is_some_and(|v| v.elements.is_some());
                if !array
                    && !variable
                        .map_or_else(|| self.inherited_value(base).is_some(), |v| v.declared)
                {
                    continue;
                }
                let i = match crate::arithmetic::data_index(index, self) {
                    Ok(i) => i,
                    Err(e) => {
                        diagnostic!("unset: {e}");
                        return crate::Flow::Discard(1);
                    }
                };
                if !array && i != 0 {
                    diagnostic!("unset: {base}: not an array variable");
                    status = 1;
                } else if let Ok(element) = self.relative_index(base, i) {
                    self.unset_element(base, element);
                } else {
                    diagnostic!("unset: [{index}]: bad array subscript");
                    status = 1;
                }
                continue;
            }
            if resolve_function {
                self.forget_function(name);
            }
            if !functions_only && is_identifier(name) {
                self.unset_variable(name);
            }
        }
        crate::Flow::Continue(status)
    }
}
