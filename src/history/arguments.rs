// Argument completion is demand-driven. Read a bounded snapshot of the existing
// journal; never source a history line or expand one of its words.
use super::{decode, file};
use crate::{Session, parser};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

fn project(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|p| p.join(".git").exists())
}

pub(crate) fn candidates(
    session: &Session,
    words: &[String],
    current: usize,
    dirs: bool,
) -> Vec<String> {
    const WINDOW: u64 = 256 * 1024;
    let prefix = &words[current];
    let command = &words[0];
    let option = current
        .checked_sub(1)
        .filter(|&n| n > 0)
        .map(|n| words[n].as_str())
        .filter(|s| s.starts_with('-') && *s != "--");
    if current == 0 || prefix.starts_with('-') || (prefix.is_empty() && option.is_none()) {
        return Vec::new();
    }
    let Some(path) = file(session) else {
        return Vec::new();
    };
    let Ok(mut input) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    else {
        return Vec::new();
    };
    let Ok(meta) = input.metadata() else {
        return Vec::new();
    };
    if !meta.is_file() || meta.len() == 0 {
        return Vec::new();
    }
    let start = meta.len().saturating_sub(WINDOW);
    let mut data = Vec::with_capacity((meta.len() - start) as usize);
    if input.seek(SeekFrom::Start(start)).is_err()
        || input
            .take(meta.len() - start)
            .read_to_end(&mut data)
            .is_err()
    {
        return Vec::new();
    }
    let Some(last) = memchr::memrchr(b'\n', &data) else {
        return Vec::new();
    };
    let bytes = &data[..last];
    let bytes = if start == 0 {
        bytes
    } else {
        memchr::memchr(b'\n', bytes).map_or(&[][..], |n| &bytes[n + 1..])
    };
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let logical = session.get("PWD").unwrap_or_default();
    let root = project(&cwd);
    let mut directories = HashMap::new();
    let mut best = 0;
    let mut scores = HashMap::new();
    for (i, row) in bytes.rsplit(|&b| b == b'\n').enumerate() {
        if i % 64 == 0 {
            crate::editor::preview::poll();
        }
        let Ok(row) = std::str::from_utf8(row) else {
            continue;
        };
        let mut fields = row.splitn(5, '\t');
        if fields.next() != Some("1") || fields.nth(1) != Some("0") {
            continue;
        }
        let (Some(raw_dir), Some(raw_line)) = (fields.next(), fields.next()) else {
            continue;
        };
        // An ordinary literal can be rejected before allocation, path lookups
        // and parsing. Quotes or escapes can join pieces of a matching word.
        let needle = if prefix.is_empty() {
            option.unwrap_or("")
        } else {
            prefix
        };
        if !raw_line.contains(needle) && !raw_line.contains(['\\', '\'', '"']) {
            continue;
        }
        let Some(dir) = decode(raw_dir) else { continue };
        let dir = Path::new(&dir);
        let dir = if dir == cwd || dir == Path::new(&logical) {
            cwd.as_path()
        } else {
            let resolved = directories.entry(dir.to_owned()).or_insert_with(|| {
                let resolved = dir.canonicalize().ok()?;
                (resolved == cwd
                    || root
                        .is_some_and(|r| resolved.starts_with(r) && project(&resolved) == Some(r)))
                .then_some(resolved)
            });
            let Some(path) = resolved.as_deref() else {
                continue;
            };
            path
        };
        let same_dir = dir == cwd;
        let Some(line) = decode(raw_line) else {
            continue;
        };
        let Ok(statements) = parser::parse_final(&line) else {
            continue;
        };
        for statement in statements {
            let parser::Command::Pipeline(pipeline) = statement.command else {
                continue;
            };
            for stage in pipeline {
                let Some(program) = stage.words.first().and_then(|w| w.as_literal()) else {
                    continue;
                };
                for pair in stage.words.windows(2) {
                    let Some(arg) = pair[1].as_literal() else {
                        continue;
                    };
                    if arg.is_empty() || arg.starts_with('-') {
                        continue;
                    }
                    let same_option =
                        program == command && option.is_some() && pair[0].as_literal() == option;
                    if prefix.is_empty() && !same_option {
                        continue;
                    }
                    if !arg.starts_with(prefix) && !arg.split('/').any(|s| s.starts_with(prefix)) {
                        continue;
                    }
                    let path = Path::new(arg);
                    let absolute = dir.join(path);
                    if dirs && !absolute.is_dir() {
                        continue;
                    }
                    let mut value = PathBuf::new();
                    let rank =
                        if same_option && (same_dir || !arg.contains('/') || path.is_absolute()) {
                            4
                        } else {
                            if !absolute.exists() {
                                continue;
                            }
                            if !same_dir && !path.is_absolute() {
                                value = absolute.strip_prefix(&cwd).unwrap_or(&absolute).to_owned();
                            }
                            if program == command && same_dir {
                                3
                            } else if same_dir {
                                2
                            } else {
                                1
                            }
                        };
                    if rank < best {
                        continue;
                    }
                    if rank > best {
                        scores.clear();
                        best = rank;
                    }
                    let value = value.to_str().filter(|s| !s.is_empty()).unwrap_or(arg);
                    let mut value = value.to_owned();
                    if absolute.is_dir() && !value.ends_with('/') {
                        value.push('/');
                    }
                    let order = scores.len();
                    let (count, _, last_row) =
                        scores.entry(value).or_insert((0usize, order, usize::MAX));
                    if *last_row != i {
                        *count += 1;
                        *last_row = i;
                    }
                }
            }
        }
    }
    let mut ranked: Vec<_> = scores.into_iter().collect();
    ranked.sort_unstable_by_key(|(_, (count, order, _))| (std::cmp::Reverse(*count), *order));
    ranked.into_iter().map(|(word, _)| word).collect()
}
