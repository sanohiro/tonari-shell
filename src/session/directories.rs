use super::Session;
use std::io::{self, Write};
use std::path::Path;

fn index(word: &str, len: usize) -> Option<usize> {
    let n = word.get(1..)?.parse::<usize>().ok()?;
    if n >= len {
        return None;
    }
    match word.as_bytes()[0] {
        b'+' => Some(n),
        b'-' => Some(len - 1 - n),
        _ => None,
    }
}

impl Session {
    fn stack_path(&self, i: usize) -> Option<&str> {
        if i == 0 {
            return self.logical_directory_ref().ok()?.to_str();
        }
        self.dir_stack
            .len()
            .checked_sub(i)
            .map(|n| self.dir_stack[n].as_str())
    }

    pub(super) fn stack_entry(&self, i: usize) -> Option<String> {
        self.stack_path(i).map(str::to_owned)
    }

    fn show_stack(
        &self,
        out: &mut dyn Write,
        only: Option<usize>,
        long: bool,
        vertical: bool,
        numbered: bool,
    ) -> u8 {
        let home = (!long).then(|| self.get("HOME")).flatten();
        let first = only.unwrap_or(0);
        let last = only.unwrap_or(self.dir_stack.len());
        for i in first..=last {
            let Some(path) = self.stack_path(i) else {
                return 1;
            };
            let name = home
                .as_deref()
                .filter(|h| !h.is_empty() && *h != "/")
                .and_then(|h| path.strip_prefix(h))
                .filter(|s| s.is_empty() || s.starts_with('/'));
            let result = (|| {
                if numbered {
                    write!(out, "{i:2}  ")?;
                }
                if let Some(tail) = name {
                    write!(out, "~{tail}")?;
                } else {
                    write!(out, "{path}")?;
                }
                if vertical || numbered || i == last {
                    writeln!(out)
                } else {
                    write!(out, " ")
                }
            })();
            if result.is_err() {
                return 1;
            }
        }
        u8::from(out.flush().is_err())
    }

    pub fn directory_stack(&mut self, command: &str, args: &[String], out: &mut dyn Write) -> u8 {
        let (mut long, mut vertical, mut numbered, mut clear, mut no_cd) =
            (false, false, false, false, false);
        let mut args = args.iter().peekable();
        while let Some(arg) =
            args.next_if(|a| a.starts_with('-') && a.len() > 1 && !a.as_bytes()[1].is_ascii_digit())
        {
            if arg == "--" {
                break;
            }
            for flag in arg[1..].chars() {
                match (command, flag) {
                    ("dirs", 'l') => long = true,
                    ("dirs", 'p') => vertical = true,
                    ("dirs", 'v') => numbered = true,
                    ("dirs", 'c') => clear = true,
                    ("pushd" | "popd", 'n') => no_cd = true,
                    _ => {
                        eprintln!("tonari: {command}: invalid option -{flag}");
                        return 2;
                    }
                }
            }
        }
        let operand = args.next().map(String::as_str);
        if args.next().is_some() {
            eprintln!("tonari: {command}: too many arguments");
            return 2;
        }
        if clear {
            self.dir_stack.clear();
            return 0;
        }
        let len = 1 + self.dir_stack.len();
        let slot = match operand {
            Some(text)
                if text.len() > 1
                    && text.starts_with(['-', '+'])
                    && text.as_bytes()[1..].iter().all(u8::is_ascii_digit) =>
            {
                let Some(i) = index(text, len) else {
                    eprintln!("tonari: {command}: {text}: directory stack index out of range");
                    return 1;
                };
                Some(i)
            }
            Some(_) if command != "pushd" => {
                eprintln!("tonari: {command}: invalid directory stack index");
                return 1;
            }
            _ => None,
        };
        if command == "dirs" {
            return self.show_stack(out, slot, long, vertical, numbered);
        }
        let result = (|| -> io::Result<()> {
            if command == "popd" {
                let i = slot.unwrap_or(0);
                if self.dir_stack.is_empty() {
                    return Err(io::Error::other("directory stack empty"));
                }
                if i == 0 {
                    if !no_cd {
                        let target = self.dir_stack.last().unwrap().clone();
                        self.change_directory(Path::new(&target), false, false, true)?;
                    }
                    self.dir_stack.pop();
                } else {
                    self.dir_stack.remove(len - 1 - i);
                }
            } else if let Some(i) = slot {
                let cwd = self
                    .logical_directory()?
                    .into_os_string()
                    .into_string()
                    .map_err(|_| io::Error::other("expected UTF-8 directory name"))?;
                let mut paths: Vec<_> = std::iter::once(cwd)
                    .chain(self.dir_stack.iter().rev().cloned())
                    .collect();
                paths.rotate_left(i);
                if !no_cd {
                    self.change_directory(Path::new(&paths[0]), false, false, true)?;
                }
                self.dir_stack = paths.into_iter().skip(1).rev().collect();
            } else if let Some(dir) = operand {
                if no_cd {
                    self.dir_stack.push(dir.to_owned());
                } else {
                    let cwd = self
                        .stack_entry(0)
                        .ok_or_else(|| io::Error::other("cannot read current directory"))?;
                    self.change_directory(Path::new(dir), false, false, true)?;
                    self.dir_stack.push(cwd);
                }
            } else if !no_cd {
                let cwd = self
                    .stack_entry(0)
                    .ok_or_else(|| io::Error::other("cannot read current directory"))?;
                let next = self
                    .dir_stack
                    .last()
                    .ok_or_else(|| io::Error::other("no other directory"))?
                    .clone();
                self.change_directory(Path::new(&next), false, false, true)?;
                *self.dir_stack.last_mut().unwrap() = cwd;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("tonari: {command}: {e}");
            return 1;
        }
        if command == "pushd" && no_cd && (operand.is_none() || slot.is_some()) {
            return 0;
        }
        self.show_stack(out, None, false, false, false)
    }
}
