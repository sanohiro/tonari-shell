include!(concat!(env!("OUT_DIR"), "/tables.rs"));
static TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/text"));

fn text(id: u16) -> &'static str {
    let i = usize::from(id);
    &TEXT[OFFSETS[i] as usize..OFFSETS[i + 1] as usize]
}

#[derive(Clone, Copy)]
pub struct Locale(usize);

impl Locale {
    pub fn find(name: &str) -> Option<Self> {
        NAMES
            .binary_search_by(|id| text(*id).cmp(name))
            .ok()
            .map(Self)
    }

    fn ids(self, field: Field) -> &'static [u16] {
        let start = usize::from(RECORDS[usize::from(LOCALE_ROWS[self.0])][field as usize]);
        let count = usize::from(ENTRIES[start]);
        &ENTRIES[start + 1..][..count]
    }

    pub fn get(self, field: Field, index: usize) -> Option<&'static str> {
        self.ids(field).get(index).map(|id| text(*id))
    }

    pub fn values(self, field: Field) -> impl Iterator<Item = &'static str> {
        self.ids(field).iter().map(|id| text(*id))
    }
}

// Keep the much larger LC_TIME data independently removable by the linker.
pub mod numeric {
    #[derive(Clone, Copy)]
    pub struct Numeric {
        pub decimal: &'static str,
        pub separator: &'static str,
        pub groups: &'static [u8],
    }
    pub const C: Numeric = Numeric {
        decimal: ".",
        separator: "",
        groups: &[],
    };
    include!(concat!(env!("OUT_DIR"), "/numeric.rs"));

    pub fn find(name: &str) -> Numeric {
        let normalized;
        let name = if let Some((base, codeset)) = name.split_once('.') {
            if let Some((_, modifier)) = codeset.split_once('@') {
                normalized = format!("{base}@{modifier}");
                &normalized
            } else {
                base
            }
        } else {
            name
        };
        if matches!(name, "C" | "POSIX" | "") {
            return C;
        }
        let mut lo = 0;
        let mut hi = KINDS.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            let candidate = &NAMES[usize::from(OFFSETS[mid])..usize::from(OFFSETS[mid + 1])];
            match candidate.cmp(name) {
                std::cmp::Ordering::Equal => return PROFILES[usize::from(KINDS[mid])],
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        C
    }
}
