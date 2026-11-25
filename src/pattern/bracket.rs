// Bash quotes individual bracket members. libc fnmatch implementations disagree
// on backslashes in these lists and on how to resume after an unclosed '['.
use std::ffi::{CStr, CString};

fn posix_item_len(input: &[u8]) -> Option<usize> {
    let [b'[', mark @ (b':' | b'=' | b'.'), rest @ ..] = input else {
        return None;
    };
    let end = rest.windows(2).position(|p| p == [*mark, b']'])?;
    // A quoted equivalence operand is read as ordinary list members by Bash.
    if *mark == b'=' && rest[..end].contains(&b'\\') {
        return None;
    }
    Some(end + 4)
}

// Input starts immediately after '['. The initial ']' is a literal member.
pub(super) fn class_end(body: &[u8]) -> Option<usize> {
    let mut i = usize::from(matches!(body.first(), Some(b'!' | b'^')));
    i += usize::from(body.get(i) == Some(&b']'));
    while i < body.len() {
        match body[i] {
            b']' => return Some(i),
            b'\\' => i += 2,
            b'[' => i += posix_item_len(&body[i..]).unwrap_or(1),
            _ => i += 1,
        }
    }
    None
}

pub(super) fn needs_parser(source: &[u8]) -> bool {
    let mut i = 0;
    while i < source.len() {
        match source[i] {
            b'\\' => i += 2,
            b'[' => {
                let body = &source[i + 1..];
                let Some(end) = class_end(body) else {
                    return true;
                };
                // The reference shells default to globasciiranges. Native
                // fnmatch may put accented letters between ASCII endpoints.
                if body[..end].iter().any(|b| matches!(b, b'\\' | b'-'))
                    || body[..end].windows(2).any(|w| matches!(w, b"[." | b"[="))
                {
                    return true;
                }
                i += end + 2;
            }
            _ => i += 1,
        }
    }
    false
}

// Each unit is either a UTF-8 scalar or one byte in the C character locale.
// UTF-8 byte order also preserves scalar order for Bash's character ranges.
fn unit(input: &[u8], utf8: bool) -> (&[u8], &[u8]) {
    let len = if utf8 && input[0] >= 0xc0 {
        input[0].leading_ones() as usize
    } else {
        1
    };
    input.split_at(len)
}

fn packed(value: &[u8]) -> [u8; 4] {
    let mut bytes = [0; 4];
    bytes[..value.len()].copy_from_slice(value);
    bytes
}

#[derive(Debug)]
enum Member {
    Literal([u8; 4]),
    Range([u8; 4], [u8; 4]),
    Native(CString),
    Never,
}

impl Member {
    fn read(input: &[u8], utf8: bool) -> (Self, &[u8]) {
        if let Some(len) = posix_item_len(input) {
            let mut pattern = vec![b'['];
            let item = &input[..len];
            match item[1] {
                b'.' if item.contains(&b'\\') => return (Self::Never, &input[len..]),
                b'.' | b'=' if len > 4 => {
                    let (first, rest) = unit(&item[2..len - 2], utf8);
                    if rest.is_empty() {
                        return (Self::Literal(packed(first)), &input[len..]);
                    }
                    pattern.extend_from_slice(item);
                }
                b':' => pattern.extend(item.iter().filter(|&&b| b != b'\\')),
                _ => pattern.extend_from_slice(item),
            }
            pattern.push(b']');
            return (Self::Native(CString::new(pattern).unwrap()), &input[len..]);
        }
        let (value, rest) = unit(input.strip_prefix(b"\\").unwrap_or(input), utf8);
        (Self::Literal(packed(value)), rest)
    }

    fn accepts(&self, value: &[u8; 4]) -> bool {
        match self {
            Self::Literal(v) => v == value,
            Self::Range(lo, hi) => lo <= value && value <= hi,
            Self::Never => false,
            Self::Native(pattern) => {
                let mut name = [0u8; 5];
                name[..value.len()].copy_from_slice(value);
                unsafe { libc::fnmatch(pattern.as_ptr(), name.as_ptr().cast(), 0) == 0 }
            }
        }
    }
}

#[derive(Debug)]
enum Token {
    Star,
    Any,
    Literal([u8; 4]),
    List { negated: bool, members: Vec<Member> },
}

impl Token {
    fn list(mut body: &[u8], utf8: bool) -> Self {
        let negated = matches!(body.first(), Some(b'!' | b'^'));
        body = &body[usize::from(negated)..];
        let mut members = Vec::new();
        while !body.is_empty() {
            let (mut member, rest) = Member::read(body, utf8);
            body = rest;
            if let (Member::Literal(lo), [b'-', _, ..]) = (&member, rest) {
                if let (Member::Literal(hi), after) = Member::read(&rest[1..], utf8) {
                    member = Member::Range(*lo, hi);
                    body = after;
                }
            }
            members.push(member);
        }
        Self::List { negated, members }
    }

    fn accepts(&self, value: &[u8; 4]) -> bool {
        match self {
            Self::Any => true,
            Self::Literal(literal) => literal == value,
            Self::List { negated, members } => members.iter().any(|m| m.accepts(value)) != *negated,
            Self::Star => unreachable!(),
        }
    }
}

// POSIX named classes test the original character, even when literal
// members and range endpoints use case-insensitive comparison.
#[derive(Debug)]
pub(super) struct CharacterClass {
    negated: bool,
    members: Vec<Member>,
    fold: Option<super::CharacterLocale>,
}

impl CharacterClass {
    pub(super) fn new(pattern: &str, fold: Option<super::CharacterLocale>) -> Self {
        let Token::List { negated, members } =
            Token::list(&pattern.as_bytes()[1..pattern.len() - 1], true)
        else {
            unreachable!()
        };
        Self {
            negated,
            members,
            fold,
        }
    }

    fn compare(&self, a: char, b: char) -> std::cmp::Ordering {
        let order = a.cmp(&b);
        if a == b
            || (a as u32 <= 255 && b as u32 <= 255)
            || matches!(self.fold, Some(super::CharacterLocale::Ascii))
        {
            return order;
        }
        let (mut left, mut right) = ([0u8; 5], [0u8; 5]);
        a.encode_utf8(&mut left);
        b.encode_utf8(&mut right);
        unsafe { libc::strcoll(left.as_ptr().cast(), right.as_ptr().cast()) }
            .cmp(&0)
            .then(order)
    }

    pub(super) fn accepts(&self, c: char) -> bool {
        let scalar = |bytes: &[u8; 4]| std::str::from_utf8(bytes).ok()?.chars().next();
        let mut buffer = [0; 4];
        let original = packed(c.encode_utf8(&mut buffer).as_bytes());
        let Some(locale) = self.fold else {
            return self.members.iter().any(|m| m.accepts(&original)) != self.negated;
        };
        let lower = |c| super::mapped_char(c, false, locale);
        let folded = lower(c);
        self.members.iter().any(|m| match m {
            Member::Literal(bytes) => scalar(bytes).is_some_and(|v| lower(v) == folded),
            Member::Range(lo, hi) => scalar(lo).zip(scalar(hi)).is_some_and(|(lo, hi)| {
                !self.compare(lower(lo), folded).is_gt() && !self.compare(folded, lower(hi)).is_gt()
            }),
            other => other.accepts(&original),
        }) != self.negated
    }
}

#[derive(Debug)]
pub(crate) struct BracketPattern {
    tokens: Vec<Token>,
    edges: Vec<u8>,
    prefix_len: usize,
    pub(super) needs_locale: bool,
    utf8: bool,
}

impl BracketPattern {
    pub(super) fn new(mut source: &[u8]) -> Option<Self> {
        if source.contains(&0) {
            return None;
        }
        let utf8 = super::localized(|| {
            let codeset = unsafe { libc::nl_langinfo(libc::CODESET) };
            !codeset.is_null()
                && unsafe { CStr::from_ptr(codeset) }
                    .to_bytes()
                    .eq_ignore_ascii_case(b"UTF-8")
        });
        let mut tokens = Vec::new();
        while !source.is_empty() {
            let (c, rest) = unit(source, utf8);
            source = rest;
            tokens.push(match c {
                b"*" => Token::Star,
                b"?" => Token::Any,
                b"[" => match class_end(rest) {
                    Some(end) => {
                        source = &rest[end + 1..];
                        Token::list(&rest[..end], utf8)
                    }
                    None => Token::Literal(packed(c)),
                },
                b"\\" if !rest.is_empty() => {
                    let (literal, after) = unit(rest, utf8);
                    source = after;
                    Token::Literal(packed(literal))
                }
                _ => Token::Literal(packed(c)),
            });
        }
        // Compare literal runs on both ends in bulk. A star in the middle
        // cannot consume a byte belonging to the anchored suffix.
        let head = tokens
            .iter()
            .take_while(|t| matches!(t, Token::Literal(_)))
            .count();
        let tail = tokens[head..]
            .iter()
            .rposition(|t| !matches!(t, Token::Literal(_)))
            .map_or(head, |i| head + i + 1);
        let mut edges = Vec::new();
        let mut append = |tokens: &[Token]| {
            for token in tokens {
                if let Token::Literal(bytes) = token {
                    edges.extend(bytes.iter().copied().take_while(|b| *b != 0));
                }
            }
            edges.len()
        };
        let prefix_len = append(&tokens[..head]);
        append(&tokens[tail..]);
        tokens.truncate(tail);
        tokens.drain(..head);
        let needs_locale = tokens.iter().any(|t| {
            matches!(t, Token::List { members, .. }
            if members.iter().any(|m| matches!(m, Member::Native(_))))
        });
        Some(Self {
            tokens,
            edges,
            prefix_len,
            needs_locale,
            utf8,
        })
    }

    pub(super) fn accepts(&self, text: &[u8], period: bool) -> bool {
        let (prefix, suffix) = self.edges.split_at(self.prefix_len);
        if period && text.starts_with(b".") && !prefix.starts_with(b".") {
            return false;
        }
        let Some(mut text) = text
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(suffix))
        else {
            return false;
        };
        if self.utf8 && std::str::from_utf8(text).is_err() {
            return false;
        }
        let mut tokens = self.tokens.as_slice();
        let mut retry = None;
        loop {
            match tokens.split_first() {
                Some((Token::Star, [])) => return true,
                Some((Token::Star, after)) => {
                    tokens = after;
                    retry = Some((after, text));
                    continue;
                }
                Some((token, after)) if !text.is_empty() => {
                    let (value, rest) = unit(text, self.utf8);
                    if token.accepts(&packed(value)) {
                        tokens = after;
                        text = rest;
                        continue;
                    }
                }
                None if text.is_empty() => return true,
                _ => {}
            }
            // Consume one more unit with the most recent star, then retry its suffix.
            let Some((after, start)) = retry else {
                return false;
            };
            if start.is_empty() {
                return false;
            }
            (_, text) = unit(start, self.utf8);
            retry = Some((after, text));
            tokens = after;
        }
    }
}
