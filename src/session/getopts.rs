use super::{Session, is_identifier};

impl Session {
    pub fn getopts(&mut self, args: &[String]) -> u8 {
        let args = if args.first().is_some_and(|a| a == "--") {
            &args[1..]
        } else {
            args
        };
        let [spec, name, explicit @ ..] = args else {
            eprintln!("getopts: usage: getopts optstring name [arg ...]");
            return 2;
        };
        if spec.starts_with('-') && spec.len() > 1 {
            eprintln!("getopts: invalid option: {spec}");
            return 2;
        }
        let inputs = if explicit.is_empty() {
            &self.positional
        } else {
            explicit
        };
        let index = self
            .get("OPTIND")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);
        let mut pos = index.saturating_sub(1).min(inputs.len());
        let mut offset = self.opt_offset.max(1);
        let mut option = None;
        let mut value = None;
        let mut problem = None;
        if let Some(word) = inputs.get(pos) {
            if word == "--" {
                pos += 1;
            } else if word.starts_with('-') && word.len() > 1 {
                if let Some(c) = word.get(offset..).and_then(|s| s.chars().next()) {
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
        }
        self.assign("OPTIND", &(pos + 1).to_string());
        if option.is_some() {
            self.opt_offset = offset;
        }
        if !is_identifier(name) {
            eprintln!("tonari: getopts: {name}: not a valid identifier");
            return 1;
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
                    eprintln!("{}: {message} -- {c}", self.arg0);
                }
            }
        }
        if let Some(value) = value {
            self.assign("OPTARG", &value);
        } else {
            self.unset_variable("OPTARG");
        }
        if self.readonly(name) {
            eprintln!("tonari: getopts: {name}: readonly variable");
            return 2;
        }
        self.assign(name, &result.to_string());
        u8::from(option.is_none())
    }
}
