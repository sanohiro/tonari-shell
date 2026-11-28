// Bash quotes individual bracket members. libc fnmatch implementations disagree
// on backslashes in these lists and on how to resume after an unclosed '['.
use std::ffi::{CStr, CString};

pub(crate) fn posix_list_item(input: &[u8]) -> Option<(usize,u8)> {
    let [b'[', mark @ (b':' | b'=' | b'.'), rest @ ..] = input else { return None; };
    let end=rest.windows(2).position(|p|p==[*mark,b']'])?;
    Some((end+4,*mark))
}

fn posix_item_len(input: &[u8]) -> Option<usize> {
    let (end,mark)=posix_list_item(input)?;
    // Bash treats an escaped equivalence operand as ordinary list members.
    (!(mark==b'=' && input[2..end-2].contains(&b'\\'))).then_some(end)
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

// Directory enumeration amortizes compilation across all entries.
pub(super) fn simple_list(pattern: &str) -> bool {
    let Some((prefix, rest)) = pattern.split_once('[') else {
        return false;
    };
    let Some((members, suffix)) = rest.split_once(']') else {
        return false;
    };
    !members.is_empty()
        && members.bytes().all(|b| b.is_ascii_alphanumeric())
        && !prefix.contains('\\')
        && !suffix.contains(['[', '\\'])
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
                    || body[..end].split(|&b| b == b'[').skip(1).any(|item| {
                        matches!(item.first(), Some(b'.' | b'='))
                            || item.starts_with(b":ascii:]")
                            || (item.starts_with(b":") && !item.ends_with(b":]"))
                    })
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

// Names in the portable character set identify one collating element even on
// systems whose native fnmatch does not supply the corresponding locale table.
pub(crate) fn portable_symbol(name: &[u8]) -> Option<u8> {
    Some(match name {
        b"NUL" => 0,
        b"SOH" => 1,
        b"STX" => 2,
        b"ETX" => 3,
        b"EOT" => 4,
        b"ENQ" => 5,
        b"ACK" => 6,
        b"alert" => 7,
        b"BS" | b"backspace" => 8,
        b"HT" | b"tab" => 9,
        b"LF" | b"newline" => 10,
        b"VT" | b"vertical-tab" => 11,
        b"FF" | b"form-feed" => 12,
        b"CR" | b"carriage-return" => 13,
        b"SO" => 14,
        b"SI" => 15,
        b"DLE" => 16,
        b"DC1" => 17,
        b"DC2" => 18,
        b"DC3" => 19,
        b"DC4" => 20,
        b"NAK" => 21,
        b"SYN" => 22,
        b"ETB" => 23,
        b"CAN" => 24,
        b"EM" => 25,
        b"SUB" => 26,
        b"ESC" => 27,
        b"IS4" | b"FS" => 28,
        b"IS3" | b"GS" => 29,
        b"IS2" | b"RS" => 30,
        b"IS1" | b"US" => 31,
        b"space" => b' ',
        b"exclamation-mark" => b'!',
        b"quotation-mark" => b'"',
        b"number-sign" => b'#',
        b"dollar-sign" => b'$',
        b"percent-sign" => b'%',
        b"ampersand" => b'&',
        b"apostrophe" => b'\'',
        b"left-parenthesis" => b'(',
        b"right-parenthesis" => b')',
        b"asterisk" => b'*',
        b"plus-sign" => b'+',
        b"comma" => b',',
        b"hyphen" | b"hyphen-minus" | b"minus" | b"dash" => b'-',
        b"period" | b"full-stop" => b'.',
        b"slash" | b"solidus" => b'/',
        b"zero" => b'0',
        b"one" => b'1',
        b"two" => b'2',
        b"three" => b'3',
        b"four" => b'4',
        b"five" => b'5',
        b"six" => b'6',
        b"seven" => b'7',
        b"eight" => b'8',
        b"nine" => b'9',
        b"colon" => b':',
        b"semicolon" => b';',
        b"less-than-sign" => b'<',
        b"equals-sign" => b'=',
        b"greater-than-sign" => b'>',
        b"question-mark" => b'?',
        b"commercial-at" => b'@',
        b"left-square-bracket" => b'[',
        b"backslash" | b"reverse-solidus" => b'\\',
        b"right-square-bracket" => b']',
        b"circumflex" | b"circumflex-accent" => b'^',
        b"underline" | b"underscore" | b"low-line" => b'_',
        b"grave-accent" => b'`',
        b"left-brace" | b"left-curly-bracket" => b'{',
        b"vertical-line" => b'|',
        b"right-brace" | b"right-curly-bracket" => b'}',
        b"tilde" => b'~',
        b"DEL" => 127,
        _ => return None,
    })
}

#[derive(Debug)]
enum Member {
    Literal([u8; 4]),
    Range([u8; 4], [u8; 4]),
    Native(CString),
    Ascii,
    Never,
}

impl Member {
    fn read(input: &[u8], utf8: bool) -> (Self, &[u8]) {
        if let Some(len) = posix_item_len(input) {
            let mut pattern = vec![b'['];
            let item = &input[..len];
            match item[1] {
                b'.' if item.contains(&b'\\') => return (Self::Never, &input[len..]),
                b':' if &item[2..len - 2] == b"ascii" => return (Self::Ascii, &input[len..]),
                b'.' if let Some(byte) = portable_symbol(&item[2..len - 2]) => {
                    return (Self::Literal(packed(&[byte])), &input[len..]);
                }
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
            Self::Range(lo, hi) => super::glob_locale::in_range(lo, value, hi),
            Self::Never => false,
            Self::Ascii => value[0].is_ascii(),
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
    AsciiList(u128),
    List { negated: bool, members: Vec<Member> },
}

fn ascii_member(mask: u128, byte: u8) -> bool {
    byte.is_ascii() && mask & (1u128 << byte) != 0
}

fn parse_members(mut body: &[u8], utf8: bool) -> (bool, Vec<Member>) {
    let negated = matches!(body.first(), Some(b'!' | b'^'));
    body = &body[usize::from(negated)..];
    let mut members = Vec::new();
    while !body.is_empty() {
        let (mut member, rest) = Member::read(body, utf8);
        body = rest;
        if let (Member::Literal(lo), [b'-', _, ..]) = (&member, rest)
            && let (Member::Literal(hi), after) = Member::read(&rest[1..], utf8)
        {
            member = Member::Range(*lo, hi);
            body = after;
        }
        members.push(member);
    }
    (negated, members)
}

fn ascii_list(body: &[u8], fold: bool) -> Option<u128> {
    if body.is_empty() {
        return None;
    }
    body.iter().try_fold(0, |mask, &byte| {
        byte.is_ascii_alphanumeric().then(|| {
            let member = if fold {
                byte.to_ascii_lowercase()
            } else {
                byte
            };
            mask | (1u128 << member)
        })
    })
}

impl Token {
    fn list(body: &[u8], utf8: bool) -> Self {
        if let Some(mask) = ascii_list(body, false) {
            return Self::AsciiList(mask);
        }
        let (negated, members) = parse_members(body, utf8);
        Self::List { negated, members }
    }

    fn accepts(&self, value: &[u8; 4]) -> bool {
        match self {
            Self::Any => true,
            Self::Literal(literal) => literal == value,
            Self::AsciiList(mask) => ascii_member(*mask, value[0]),
            Self::List { negated, members } => members.iter().any(|m| m.accepts(value)) != *negated,
            Self::Star => unreachable!(),
        }
    }
}

// POSIX named classes test the original character, even when literal
// members and range endpoints use case-insensitive comparison.
#[derive(Debug)]
pub(super) struct CharacterClass {
    members: ClassMembers,
    fold: Option<super::CharacterLocale>,
}

#[derive(Debug)]
enum ClassMembers {
    Ascii(u128),
    General { negated: bool, members: Vec<Member> },
}

impl CharacterClass {
    pub(super) fn new(pattern: &str, fold: Option<super::CharacterLocale>) -> Self {
        let body = &pattern.as_bytes()[1..pattern.len() - 1];
        // Turkish I can fold outside ASCII even when every member is ASCII.
        let members = if !matches!(fold, Some(super::CharacterLocale::Turkic))
            && let Some(mask) = ascii_list(body, fold.is_some())
        {
            ClassMembers::Ascii(mask)
        } else {
            let (negated, members) = parse_members(body, true);
            ClassMembers::General { negated, members }
        };
        Self { members, fold }
    }

    fn compare(&self, a: char, b: char) -> std::cmp::Ordering {
        if super::glob_locale::collated_ranges() {
            let (mut left, mut right) = ([0u8; 4], [0u8; 4]);
            a.encode_utf8(&mut left);
            b.encode_utf8(&mut right);
            return super::glob_locale::range_order(&left, &right);
        }
        a.cmp(&b)
    }

    pub(super) fn accepts(&self, c: char) -> bool {
        let (negated, members) = match &self.members {
            ClassMembers::Ascii(mask) => {
                let folded = self
                    .fold
                    .map_or(c, |locale| super::mapped_char(c, false, locale));
                return folded.is_ascii() && ascii_member(*mask, folded as u8);
            }
            ClassMembers::General { negated, members } => (*negated, members),
        };
        let scalar = |bytes: &[u8; 4]| std::str::from_utf8(bytes).ok()?.chars().next();
        let mut buffer = [0; 4];
        let original = packed(c.encode_utf8(&mut buffer).as_bytes());
        let Some(locale) = self.fold else {
            return members.iter().any(|m| m.accepts(&original)) != negated;
        };
        let lower = |c| super::mapped_char(c, false, locale);
        let folded = lower(c);
        members.iter().any(|m| match m {
            Member::Literal(bytes) => scalar(bytes).is_some_and(|v| lower(v) == folded),
            Member::Range(lo, hi) => scalar(lo).zip(scalar(hi)).is_some_and(|(lo, hi)| {
                !self.compare(lower(lo), folded).is_gt() && !self.compare(folded, lower(hi)).is_gt()
            }),
            other => other.accepts(&original),
        }) != negated
    }
}

#[derive(Debug)]
pub(crate) struct BracketPattern {
    tokens: Vec<Token>,
    edges: Vec<u8>,
    prefix_len: usize,
    pub(super) needs_locale: bool,
    pub(super) utf8: bool,
}

impl BracketPattern {
    pub(super) fn utf8_locale() -> bool {
        super::localized(|| {
            let codeset = unsafe { libc::nl_langinfo(libc::CODESET) };
            !codeset.is_null()
                && unsafe { CStr::from_ptr(codeset) }
                    .to_bytes()
                    .eq_ignore_ascii_case(b"UTF-8")
        })
    }

    pub(super) fn new(mut source: &[u8]) -> Option<Self> {
        if source.contains(&0) {
            return None;
        }
        let utf8 = Self::utf8_locale();
        let mut tokens = Vec::new();
        let last_close = memchr::memrchr(b']', source).map(|i| source.len() - i);
        while !source.is_empty() {
            let (c, rest) = unit(source, utf8);
            source = rest;
            tokens.push(match c {
                b"*" => Token::Star,
                b"?" => Token::Any,
                b"[" => match last_close.filter(|&distance| distance <= rest.len()).and_then(|_|class_end(rest)) {
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

    // The caller has verified ASCII input and a star-free fixed-width pattern.
    pub(super) fn accepts_fixed_ascii(&self, text: &[u8]) -> bool {
        let (prefix, suffix) = self.edges.split_at(self.prefix_len);
        text.strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(suffix))
            .is_some_and(|middle| {
                middle.len() == self.tokens.len()
                    && self
                        .tokens
                        .iter()
                        .zip(middle)
                        .all(|(token, &byte)| token.accepts(&[byte, 0, 0, 0]))
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
        let utf8 = self.utf8 || super::glob_locale::collated_ranges();
        if utf8 && std::str::from_utf8(text).is_err() {
            return false;
        }
        // The literal prefix and suffix have already been consumed. In this
        // common pathname shape the star accepts everything after the member.
        if let [Token::AsciiList(mask), Token::Star] = self.tokens.as_slice() {
            return text.first().is_some_and(|&b| ascii_member(*mask, b));
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
                    let (value, rest) = unit(text, utf8);
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
            (_, text) = unit(start, utf8);
            retry = Some((after, text));
            tokens = after;
        }
    }
}
