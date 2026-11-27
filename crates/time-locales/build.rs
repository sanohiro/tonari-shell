// The upstream crate exposes the LC_TIME values as pointer-rich Rust slices.
// Intern their strings and lists once so the static PIE only needs offsets.
use pure_rust_locales::{Locale, locale_match};
use std::{collections::HashMap, env, fmt::Write, fs, path::PathBuf};

#[derive(Default)]
struct Tables {
    ids: HashMap<&'static str, u16>,
    lists: HashMap<Vec<u16>, u16>,
    entries: Vec<u16>,
}

impl Tables {
    fn intern(&mut self, s: &'static str) -> u16 {
        let next = self.ids.len();
        *self
            .ids
            .entry(s)
            .or_insert_with(|| u16::try_from(next).unwrap())
    }

    fn pack_text(&self) -> (String, Vec<u16>, Vec<u8>) {
        let mut items: Vec<_> = self.ids.iter().map(|(&s, &id)| (s, id)).collect();
        // Longer strings can contain shorter ones. Break ties by ID for reproducible builds.
        items.sort_unstable_by_key(|&(s, id)| (std::cmp::Reverse(s.len()), id));
        let mut text = String::new();
        let mut offsets = vec![0; items.len()];
        let mut lengths = vec![0; items.len()];
        for (s, id) in items {
            let start = text.find(s).unwrap_or_else(|| {
                let at = text.len();
                text.push_str(s);
                at
            });
            offsets[usize::from(id)] = u16::try_from(start).expect("locale text offset");
            lengths[usize::from(id)] = u8::try_from(s.len()).expect("locale string length");
        }
        (text, offsets, lengths)
    }

    fn list(&mut self, values: &[&'static str]) -> u16 {
        let ids: Vec<_> = values.iter().map(|s| self.intern(s)).collect();
        *self.lists.entry(ids.clone()).or_insert_with(|| {
            let start = u16::try_from(self.entries.len()).unwrap();
            self.entries.push(u16::try_from(ids.len()).unwrap());
            self.entries.extend(ids);
            start
        })
    }
}

fn main() {
    println!("cargo:rerun-if-changed=names.txt");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut tables = Tables::default();
    let mut records = Vec::new();
    let mut locale_rows = Vec::new();
    let mut fields = Vec::new();
    let mut numeric_names = String::new();
    let mut numeric_offsets = vec![0u16];
    let mut numeric_kinds = Vec::new();
    let mut profiles = Vec::new();
    let mut previous = "";
    for name in include_str!("names.txt").lines() {
        assert!(name > previous, "locale names must be unique and sorted");
        previous = name;
        let locale: Locale = name.parse().unwrap();
        numeric_names.push_str(name);
        numeric_offsets.push(u16::try_from(numeric_names.len()).unwrap());
        let profile = (
            locale_match!(locale => LC_NUMERIC::DECIMAL_POINT),
            locale_match!(locale => LC_NUMERIC::THOUSANDS_SEP),
            locale_match!(locale => LC_NUMERIC::GROUPING),
        );
        let id = profiles
            .iter()
            .position(|p| p == &profile)
            .unwrap_or_else(|| {
                profiles.push(profile);
                profiles.len() - 1
            });
        numeric_kinds.push(u8::try_from(id).unwrap());
        let mut row = Vec::new();
        macro_rules! field {
            ($id:ident, $value:expr) => {{
                if records.is_empty() {
                    fields.push(stringify!($id));
                }
                row.push(tables.list($value));
            }};
        }
        field!(Abday, locale_match!(locale => LC_TIME::ABDAY));
        field!(Day, locale_match!(locale => LC_TIME::DAY));
        field!(Abmon, locale_match!(locale => LC_TIME::ABMON));
        field!(Mon, locale_match!(locale => LC_TIME::MON));
        field!(
            AbAltMon,
            locale_match!(locale => LC_TIME::AB_ALT_MON).unwrap_or_default()
        );
        field!(
            AltMon,
            locale_match!(locale => LC_TIME::ALT_MON).unwrap_or_default()
        );
        field!(AmPm, locale_match!(locale => LC_TIME::AM_PM));
        field!(
            AltDigits,
            locale_match!(locale => LC_TIME::ALT_DIGITS).unwrap_or_default()
        );
        field!(
            Era,
            locale_match!(locale => LC_TIME::ERA).unwrap_or_default()
        );
        field!(
            EraDFmt,
            locale_match!(locale => LC_TIME::ERA_D_FMT).as_slice()
        );
        field!(
            EraDTFmt,
            locale_match!(locale => LC_TIME::ERA_D_T_FMT).as_slice()
        );
        field!(
            EraTFmt,
            locale_match!(locale => LC_TIME::ERA_T_FMT).as_slice()
        );
        field!(DFmt, &[locale_match!(locale => LC_TIME::D_FMT)]);
        // pure-rust-locales 0.8.2 carries the older French d_t_fmt.
        // Modern fr_FR uses %Z only in date_fmt. The euro variant copies LC_TIME.
        let datetime = match locale {
            Locale::fr_FR | Locale::fr_FR_euro => "%a %d %b %Y %T",
            _ => locale_match!(locale => LC_TIME::D_T_FMT),
        };
        field!(DTFmt, &[datetime]);
        field!(TFmt, &[locale_match!(locale => LC_TIME::T_FMT)]);
        field!(TFmtAmpm, &[locale_match!(locale => LC_TIME::T_FMT_AMPM)]);
        let index = records
            .iter()
            .position(|old| *old == row)
            .unwrap_or_else(|| {
                records.push(row);
                records.len() - 1
            });
        locale_rows.push(u16::try_from(index).unwrap());
    }
    let (text, offsets, lengths) = tables.pack_text();
    fs::write(out.join("text"), text).unwrap();
    let mut code = String::new();
    let posix = include_str!("names.txt")
        .lines()
        .position(|s| s == "POSIX")
        .unwrap();
    writeln!(code, "pub const POSIX: Locale = Locale({posix});").unwrap();
    writeln!(
        code,
        "#[derive(Clone, Copy)] pub enum Field {{ {} }}",
        fields.join(", ")
    )
    .unwrap();
    let mut list_ids = Vec::new();
    let mut field_offsets = Vec::new();
    let mut compact = vec![Vec::with_capacity(fields.len()); records.len()];
    for column in 0..fields.len() {
        field_offsets.push(u16::try_from(list_ids.len()).unwrap());
        let mut column_lists = Vec::new();
        for (i, record) in records.iter().enumerate() {
            let list = record[column];
            let id = column_lists
                .iter()
                .position(|&v| v == list)
                .unwrap_or_else(|| {
                    column_lists.push(list);
                    column_lists.len() - 1
                });
            compact[i].push(u8::try_from(id).expect("too many distinct lists for one field"));
        }
        list_ids.extend(column_lists);
    }
    writeln!(
        code,
        "static RECORDS: &[[u8; {}]] = &{compact:?};",
        fields.len()
    )
    .unwrap();
    writeln!(code, "static FIELD_OFFSETS: &[u16] = &{field_offsets:?};").unwrap();
    writeln!(code, "static LIST_IDS: &[u16] = &{list_ids:?};").unwrap();
    writeln!(code, "static LOCALE_ROWS: &[u16] = &{locale_rows:?};").unwrap();
    writeln!(code, "static ENTRIES: &[u16] = &{:?};", tables.entries).unwrap();
    writeln!(code, "static OFFSETS: &[u16] = &{offsets:?};").unwrap();
    writeln!(code, "static LENGTHS: &[u8] = &{lengths:?};").unwrap();
    fs::write(out.join("tables.rs"), code).unwrap();
    let mut numeric = format!(
        "const NAMES: &str = {numeric_names:?};\nconst OFFSETS: &[u16] = &{numeric_offsets:?};\nconst KINDS: &[u8] = &{numeric_kinds:?};\nconst PROFILES: &[Numeric] = &[\n"
    );
    for (decimal, separator, groups) in profiles {
        let groups: Vec<u8> = groups
            .iter()
            .copied()
            .take_while(|&n| n > 0)
            .map(|n| u8::try_from(n).unwrap())
            .collect();
        writeln!(
            numeric,
            "Numeric {{ decimal: {decimal:?}, separator: {separator:?}, groups: &{groups:?} }},"
        )
        .unwrap();
    }
    numeric.push_str("];");
    fs::write(out.join("numeric.rs"), numeric).unwrap();
}
