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
fn class_end(body: &[u8]) -> Option<usize> {
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
                if body[..end].contains(&b'\\') {
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

#[derive(Debug)]
pub(crate) struct BracketPattern {
    tokens: Vec<Token>,
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
        Some(Self { tokens, utf8 })
    }

    pub(super) fn accepts(&self, mut text: &[u8], period: bool) -> bool {
        if period
            && text.starts_with(b".")
            && !matches!(self.tokens.first(), Some(Token::Literal(c)) if c == &[b'.', 0, 0, 0])
        {
            return false;
        }
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
