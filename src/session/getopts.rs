use super::{Session, is_identifier};
use crate::Flow;

impl Session {
    pub fn getopts(&mut self, args: &[String]) -> Flow {
        let terminated = args.first().is_some_and(|a| a == "--");
        let args = if terminated { &args[1..] } else { args };
        let [spec, name, explicit @ ..] = args else {
            diagnostic!("getopts: usage: getopts optstring name [arg ...]");
            return Flow::Continue(2);
        };
        if !terminated && spec.starts_with('-') && spec.len() > 1 {
            diagnostic!("getopts: invalid option: {spec}");
            return Flow::Continue(2);
        }
        if self.posix && self.readonly("OPTIND") {
            diagnostic!("getopts: OPTIND: readonly variable");
            return Flow::Continue(1);
        }
        let inputs = if explicit.is_empty() {
            self.positional.as_slice()
        } else {
            explicit
        };
        let ordinary = self
            .variables
            .get("OPTIND")
            .filter(|v| !v.nameref && v.elements.is_none());
        let index = match ordinary {
            Some(v) => v.value.as_deref().and_then(|s| s.parse::<usize>().ok()),
            None => self.get("OPTIND").and_then(|s| s.parse::<usize>().ok()),
        };
        let mut pos = index.unwrap_or(1).saturating_sub(1).min(inputs.len());
        let mut offset = self.opt_offset.max(1);
        let mut option = None;
        let mut value = None;
        let mut problem = None;
        if let Some(word) = inputs.get(pos) {
            if word == "--" {
                pos += 1;
            } else if word.starts_with('-')
                && word.len() > 1
                && let Some(c) = word.get(offset..).and_then(|s| s.chars().next())
            {
                offset += c.len_utf8();
                option = Some(c);
                let rest = &word[offset..];
                match spec.trim_start_matches(':').split_once(c) {
                    Some((_, tail)) if c != ':' && tail.starts_with(':') => {
                        pos += 1;
                        if rest.is_empty() {
                            if let Some(argument) = inputs.get(pos) {
                                value = Some(argument.clone());
                                pos += 1;
                            } else {
                                problem = Some(':');
                            }
                        } else {
                            value = Some(rest.to_owned());
                        }
                        offset = 1;
                    }
                    matched => {
                        if matched.is_none() || c == ':' {
                            problem = Some('?');
                        }
                        if rest.is_empty() {
                            pos += 1;
                            offset = 1;
                        }
                    }
                }
            }
        }
        if ordinary.is_some_and(|v| !v.readonly) {
            let var = self.variables.get_mut("OPTIND").unwrap();
            if index != Some(pos + 1) {
                use std::fmt::Write;
                let text = var.scalar_mut().get_or_insert_with(String::new);
                text.clear();
                write!(text, "{}", pos + 1).unwrap();
            }
            var.declared = true;
            var.exported |= self.allexport;
            self.opt_offset = 0;
        } else if self.readonly("OPTIND") {
            diagnostic!("getopts: OPTIND: readonly variable");
        } else {
            self.assign("OPTIND", &(pos + 1).to_string());
        }
        if option.is_some() {
            self.opt_offset = offset;
        }
        let mut result = option.unwrap_or('?');
        if let (Some(c), Some(error)) = (option, problem) {
            if spec.starts_with(':') {
                result = error;
                value = Some(c.to_string());
            } else {
                result = '?';
                if self.get("OPTERR").as_deref() != Some("0") {
                    let message = if error == ':' {
                        "option requires an argument"
                    } else {
                        "illegal option"
                    };
                    stderr_line!("{}: {message} -- {c}", self.arg0);
                }
            }
        }
        if self.readonly("OPTARG") {
            diagnostic!("getopts: OPTARG: readonly variable");
            if self.posix {
                return Flow::Continue(2);
            }
        } else if let Some(value) = value {
            if let Err(error) = self.assign_target("OPTARG", &value) {
                diagnostic!("getopts: OPTARG: {error}");
                return Flow::Discard(1);
            }
        } else {
            self.unset_variable("OPTARG");
        }
        if !is_identifier(name) {
            diagnostic!("getopts: {name}: not a valid identifier");
            return Flow::Continue(1);
        }
        if self.readonly(name) {
            diagnostic!("getopts: {name}: readonly variable");
            return Flow::Continue(if option.is_none() { 1 } else { 2 });
        }
        let mut bytes = [0; 4];
        let value = result.encode_utf8(&mut bytes);
        if !self.assign_existing_literal(name, value)
            && let Err(error) = self.assign_target(name, value)
        {
            diagnostic!("getopts: {name}: {error}");
            return Flow::Discard(1);
        }
        Flow::Continue(u8::from(option.is_none()))
    }
}
