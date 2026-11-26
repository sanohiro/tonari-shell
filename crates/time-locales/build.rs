// The upstream crate exposes the LC_TIME values as pointer-rich Rust slices.
// Intern their strings and lists once so the static PIE only needs offsets.
use pure_rust_locales::{Locale, locale_match};
use std::{collections::HashMap, env, fmt::Write, fs, path::PathBuf};

#[derive(Default)]
struct Tables {
    text: String,
    offsets: Vec<u32>,
    ids: HashMap<&'static str, u16>,
    lists: HashMap<Vec<u16>, u16>,
    entries: Vec<u16>,
}

impl Tables {
    fn intern(&mut self, s: &'static str) -> u16 {
        *self.ids.entry(s).or_insert_with(|| {
            let id = u16::try_from(self.offsets.len()).unwrap();
            self.offsets.push(u32::try_from(self.text.len()).unwrap());
            self.text.push_str(s);
            id
        })
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
    let mut names = Vec::new();
    let mut records = Vec::new();
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
        names.push(tables.intern(name));
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
        field!(DTFmt, &[locale_match!(locale => LC_TIME::D_T_FMT)]);
        field!(TFmt, &[locale_match!(locale => LC_TIME::T_FMT)]);
        field!(TFmtAmpm, &[locale_match!(locale => LC_TIME::T_FMT_AMPM)]);
        records.push(row);
    }
    tables
        .offsets
        .push(u32::try_from(tables.text.len()).unwrap());
    fs::write(out.join("text"), tables.text).unwrap();
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
    writeln!(
        code,
        "static RECORDS: &[[u16; {}]] = &{records:?};",
        fields.len()
    )
    .unwrap();
    writeln!(code, "static NAMES: &[u16] = &{names:?};").unwrap();
    writeln!(code, "static ENTRIES: &[u16] = &{:?};", tables.entries).unwrap();
    writeln!(code, "static OFFSETS: &[u32] = &{:?};", tables.offsets).unwrap();
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
