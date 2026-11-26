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
        let start = usize::from(RECORDS[self.0][field as usize]);
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
