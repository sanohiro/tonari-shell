// Argument completion is demand-driven. Read a bounded snapshot of the existing
// journal; never source a history line or expand one of its words.
use super::{decode, file};
use crate::{Session, parser};
use foldhash::{HashMap, HashMapExt as _};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn project(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|p| p.join(".git").exists())
}

fn recent_journal(session: &Session) -> Option<(Vec<u8>, usize)> {
    const WINDOW: u64 = 256 * 1024;
    let (mut input, meta) = super::open_journal(&file(session)?).ok()?;
    let count = meta.len().min(WINDOW);
    if count == 0 {
        return None;
    }
    let start = meta.len() - count;
    let mut data = Vec::with_capacity(count as usize);
    input.seek(SeekFrom::Start(start)).ok()?;
    input.take(count).read_to_end(&mut data).ok()?;
    data.truncate(memchr::memrchr(b'\n', &data)?);
    let first = if start == 0 {
        0
    } else {
        memchr::memchr(b'\n', &data)? + 1
    };
    Some((data, first))
}

fn successful_rows<'a>(bytes: &'a [u8], needle: &str) -> Vec<(&'a str, &'a str, usize)> {
    // Group identical journal commands before parsing words and probing paths.
    // The first occurrence in the reverse scan sets the recency tie-break.
    let mut groups = Vec::new();
    let mut positions = HashMap::new();
    for (i, row) in bytes.rsplit(|&b| b == b'\n').enumerate() {
        if i % 64 == 0 {
            crate::editor::preview::poll();
        }
        let Ok(row) = std::str::from_utf8(row) else {
            continue;
        };
        let Some(record) = super::journal_record(row) else {
            continue;
        };
        if record.status != "0" {
            continue;
        }
        let (raw_dir, raw_line) = (record.dir, record.command);
        // An ordinary literal can be rejected before allocation, path lookups
        // and parsing. Quotes or escapes can join pieces of a matching word.
        if !raw_line.contains(needle) && !raw_line.contains(['\\', '\'', '"']) {
            continue;
        }
        let index = *positions.entry((raw_dir, raw_line)).or_insert_with(|| {
            groups.push((raw_dir, raw_line, 0usize));
            groups.len() - 1
        });
        groups[index].2 += 1;
    }
    groups
}

pub(crate) fn candidates(
    session: &Session,
    words: &[String],
    current: usize,
    dirs: bool,
) -> Vec<String> {
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
    let Some((data, first)) = recent_journal(session) else {
        return Vec::new();
    };
    let bytes = &data[first..];
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let logical = session.get("PWD").unwrap_or_default();
    let root = project(&cwd);
    let needle = if prefix.is_empty() {
        option.unwrap_or("")
    } else {
        prefix
    };
    let groups = successful_rows(bytes, needle);
    let mut directories = HashMap::new();
    let mut path_kinds = HashMap::new();
    let mut best = 0;
    let mut scores = HashMap::new();
    for (i, (raw_dir, raw_line, occurrences)) in groups.into_iter().enumerate() {
        if i % 64 == 0 {
            crate::editor::preview::poll();
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
                    let rank =
                        if same_option && (same_dir || !arg.contains('/') || path.is_absolute()) {
                            4
                        } else if program == command && same_dir {
                            3
                        } else if same_dir {
                            2
                        } else {
                            1
                        };
                    if rank < best {
                        continue;
                    }
                    let absolute = dir.join(path);
                    let is_dir = if let Some(kind) = path_kinds.get(&absolute) {
                        *kind
                    } else {
                        let kind = absolute.metadata().ok().map(|m| m.is_dir());
                        path_kinds.insert(absolute.clone(), kind);
                        kind
                    };
                    if (dirs && is_dir != Some(true)) || (rank < 4 && is_dir.is_none()) {
                        continue;
                    }
                    if rank > best {
                        scores.clear();
                        best = rank;
                    }
                    let mut value = if rank < 4 && !same_dir && !path.is_absolute() {
                        absolute
                            .strip_prefix(&cwd)
                            .unwrap_or(&absolute)
                            .to_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(arg)
                            .to_owned()
                    } else {
                        arg.to_owned()
                    };
                    if is_dir == Some(true) && !value.ends_with('/') {
                        value.push('/');
                    }
                    let order = scores.len();
                    let (count, _, last_row) =
                        scores.entry(value).or_insert((0usize, order, usize::MAX));
                    if *last_row != i {
                        *count += occurrences;
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
