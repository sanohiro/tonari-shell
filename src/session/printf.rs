use crate::word::quote::{ansi_quote, needs_ansi};
use std::ffi::CString;
use std::io::{self, Write};

use super::Session;
use crate::builtins::escapes::{self, Mode};
use crate::parser::is_identifier;
use tonari_time_locales::numeric::{self, Numeric};

// Reads during a capture share the shell state. A captured %n or -v gets a
// private session on its first assignment, visible to the rest of that printf.
struct Variables<'a> {
    shell: &'a mut Session,
    capture: bool,
    private: Option<Box<Session>>,
}

impl Variables<'_> {
    fn read(&self) -> &Session {
        self.private.as_deref().unwrap_or(self.shell)
    }

    fn write(&mut self) -> &mut Session {
        if self.capture {
            self.private
                .get_or_insert_with(|| Box::new(self.shell.clone()))
        } else {
            self.shell
        }
    }
}

pub(crate) fn run(
    session: &mut Session,
    mut words: &[String],
    mut capture: Option<&mut Vec<u8>>,
) -> u8 {
    let mut target = None;
    if words.first().is_some_and(|arg| arg == "-v") {
        let Some(name) = words.get(1) else {
            eprintln!("tonari: printf: -v requires a variable");
            return 2;
        };
        if !session.valid_builtin_target(name) {
            eprintln!("tonari: printf: {name}: invalid identifier");
            return 2;
        }
        target = Some(name);
        words = &words[2..];
    }
    if words.first().is_some_and(|arg| arg == "--") {
        words = &words[1..];
    } else if words
        .first()
        .is_some_and(|arg| arg.starts_with('-') && arg != "-")
    {
        eprintln!("tonari: printf: invalid option");
        return 2;
    }
    let Some((format, values)) = words.split_first() else {
        eprintln!("tonari: printf: format required");
        return 2;
    };
    if target.is_none()
        && let Some(out) = capture.as_deref_mut()
    {
        match format.as_str() {
            "%s" => {
                for value in values {
                    out.extend_from_slice(value.as_bytes());
                }
                return 0;
            }
            "%s\n" | "%s\\n" => {
                string_lines(out, values);
                return 0;
            }
            text if !text.contains(['%', '\\']) => {
                out.extend_from_slice(text.as_bytes());
                return 0;
            }
            _ => {}
        }
    }
    if target.is_none() && capture.is_none() {
        let borrowed = if format == "%s" && values.len() <= 1 {
            Some(values.first().map_or("", String::as_str))
        } else if !format.contains(['%', '\\']) {
            Some(format.as_str())
        } else {
            None
        };
        if let Some(text) = borrowed {
            let mut stdout = io::stdout().lock();
            return u8::from(
                stdout
                    .write_all(text.as_bytes())
                    .and_then(|()| stdout.flush())
                    .is_err(),
            );
        }
    }
    let mut variables = Variables {
        shell: session,
        capture: capture.is_some(),
        private: None,
    };
    let mut args = Arguments {
        values,
        status: 0,
        locale: None,
    };
    let mut output = Vec::new();
    if matches!(format.as_str(), "%s\\n" | "%s\n") {
        string_lines(&mut output, values);
    } else {
        loop {
            let count = args.values.len();
            match render(format.as_bytes(), &mut args, &mut output, &mut variables) {
                Ok(true) if !args.values.is_empty() && count != args.values.len() => continue,
                Err(message) => {
                    eprintln!("tonari: printf: {message}");
                    args.status = 1;
                }
                _ => {}
            }
            break;
        }
    }
    if let Some(name) = target {
        let bytes = output.split(|&v| v == 0).next().unwrap_or_default();
        let Ok(value) = std::str::from_utf8(bytes) else {
            eprintln!("tonari: printf: expected UTF-8 variable value");
            return 1;
        };
        if let Err(e) = variables.write().assign_builtin_target(name, value) {
            eprintln!("tonari: printf: {e}");
            return 1;
        }
    } else if let Some(capture) = capture {
        if capture.is_empty() {
            *capture = output;
        } else {
            capture.extend_from_slice(&output);
        }
    } else {
        let mut stdout = io::stdout().lock();
        if stdout
            .write_all(&output)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 1;
        }
    }
    args.status
}

fn string_lines(out: &mut Vec<u8>, values: &[String]) {
    if values.is_empty() {
        out.push(b'\n');
    }
    for value in values {
        out.extend_from_slice(value.as_bytes());
        out.push(b'\n');
    }
}

struct Arguments<'a> {
    values: &'a [String],
    status: u8,
    locale: Option<Numeric>,
}

impl<'a> Arguments<'a> {
    fn next(&mut self) -> &'a str {
        if let Some((first, rest)) = self.values.split_first() {
            self.values = rest;
            first
        } else {
            ""
        }
    }

    fn number<T>(
        &mut self,
        parse: impl FnOnce(*const libc::c_char, *mut *mut libc::c_char) -> T,
        from_char: impl Fn(u32) -> T,
        decimal: &str,
    ) -> T {
        let value = self.next();
        if value.is_empty() {
            return from_char(0);
        }
        if let Some(rest) = value.strip_prefix(['\'', '"']) {
            return from_char(rest.chars().next().unwrap_or('\0') as u32);
        }
        let translated;
        let input = if decimal == "." {
            value
        } else if value.contains('.') {
            // The C conversion must stop at an input dot in a comma locale.
            translated = value.replace('.', "?").replace(decimal, ".");
            translated.as_str()
        } else if value.contains(decimal) {
            translated = value.replace(decimal, ".");
            translated.as_str()
        } else {
            value
        };
        let Ok(input) = CString::new(input) else {
            eprintln!("tonari: printf: invalid number");
            self.status = 1;
            return from_char(0);
        };
        let mut end = std::ptr::null_mut();
        let errno = unsafe {
            #[cfg(target_os = "macos")]
            {
                libc::__error()
            }
            #[cfg(target_os = "linux")]
            {
                libc::__errno_location()
            }
        };
        unsafe {
            *errno = 0;
        }
        let result = parse(input.as_ptr(), &mut end);
        if unsafe { *errno != 0 || *end != 0 } {
            eprintln!("tonari: printf: {value}: invalid or out-of-range number");
            self.status = 1;
        }
        result
    }

    fn signed(&mut self) -> i64 {
        // Common decimal widths and operands need neither a C string nor errno.
        if let Some((word, rest)) = self.values.split_first()
            && word
                .as_bytes()
                .first()
                .is_some_and(|b| matches!(b, b'1'..=b'9'))
            && let Ok(value) = word.parse::<i64>()
        {
            self.values = rest;
            return value;
        }
        self.number(|p, end| unsafe { libc::strtoll(p, end, 0) }, i64::from, ".")
    }
    fn unsigned(&mut self) -> u64 {
        if let Some((word, rest)) = self.values.split_first()
            && word
                .as_bytes()
                .first()
                .is_some_and(|b| matches!(b, b'1'..=b'9'))
            && let Ok(value) = word.parse::<u64>()
        {
            self.values = rest;
            return value;
        }
        self.number(
            |p, end| unsafe { libc::strtoull(p, end, 0) },
            u64::from,
            ".",
        )
    }
    fn float(&mut self, decimal: &str) -> f64 {
        self.number(|p, end| unsafe { libc::strtod(p, end) }, f64::from, decimal)
    }
}

const FLAGS: &[u8] = b"#0- +'";
const LENGTH_MODIFIERS: &[u8] = b"hljztL";

// Inspect every percent, including escaped ones. A false positive keeps the
// real child; a false negative would hide the assignment from the next command.
pub(crate) fn may_assign_count(format: &str) -> bool {
    format.as_bytes().contains(&b'n')
        && format.split('%').skip(1).any(|after| {
            after.bytes().find(|b| {
                !(b.is_ascii_digit()
                    || matches!(b, b'*' | b'.')
                    || FLAGS.contains(b)
                    || LENGTH_MODIFIERS.contains(b))
            }) == Some(b'n')
        })
}

struct Spec {
    flags: Vec<u8>,
    width: i32,
    precision: Option<i32>,
}

impl Spec {
    fn parse(input: &mut &[u8], args: &mut Arguments<'_>) -> Result<(Self, u8), &'static str> {
        let mut flags = Vec::new();
        while let Some((&flag, rest)) = input.split_first() {
            if !FLAGS.contains(&flag) {
                break;
            }
            flags.push(flag);
            *input = rest;
        }
        let mut width = amount(input, args, 0);
        if width < 0 {
            flags.push(b'-');
            width = width.checked_abs().ok_or("field width too large")?;
        }
        let mut precision = None;
        if input.first() == Some(&b'.') {
            *input = &input[1..];
            let p = amount(input, args, -1);
            if p >= 0 {
                precision = Some(p);
            }
        }
        let mut wide = false;
        while let Some((&len, rest)) = input.split_first() {
            if !LENGTH_MODIFIERS.contains(&len) {
                break;
            }
            wide |= len == b'l';
            *input = rest;
        }
        let (conversion, rest) = input.split_first().ok_or("missing format character")?;
        *input = rest;
        Ok((
            Self {
                flags,
                width,
                precision,
            },
            if wide && matches!(conversion, b'c' | b's') {
                conversion.to_ascii_uppercase()
            } else {
                *conversion
            },
        ))
    }

    fn bytes(&self, value: &[u8], output: &mut Vec<u8>) -> Result<(), &'static str> {
        let len = self
            .precision
            .map_or(value.len(), |p| value.len().min(p as usize));
        let padding = (self.width as usize).saturating_sub(len);
        output
            .try_reserve(len + padding)
            .map_err(|_| "field too large")?;
        if !self.flags.contains(&b'-') {
            output.resize(output.len() + padding, b' ');
        }
        output.extend_from_slice(&value[..len]);
        if self.flags.contains(&b'-') {
            output.resize(output.len() + padding, b' ');
        }
        Ok(())
    }

    fn wide(&self, value: &str, single: bool, out: &mut Vec<u8>) -> Result<(), &'static str> {
        let limit = self
            .precision
            .map_or(usize::MAX, |n| n as usize)
            .min(if single { 1 } else { usize::MAX });
        if !single && self.width == 0 && self.precision.is_none() {
            out.extend_from_slice(value.as_bytes());
            return Ok(());
        }
        let mut bytes = 0;
        let mut chars = 0;
        for c in value.chars().take(limit) {
            bytes += c.len_utf8();
            chars += 1;
        }
        let pad = (self.width as usize).saturating_sub(chars);
        out.try_reserve(bytes + pad)
            .map_err(|_| "field too large")?;
        let left = self.flags.contains(&b'-');
        if !left {
            out.resize(out.len() + pad, b' ');
        }
        out.extend_from_slice(&value.as_bytes()[..bytes]);
        if left {
            out.resize(out.len() + pad, b' ');
        }
        Ok(())
    }

    fn localized(
        &mut self,
        conversion: u8,
        locale: Numeric,
        grouped: bool,
        args: &mut Arguments<'_>,
        out: &mut Vec<u8>,
    ) -> Result<(), &'static str> {
        let width = std::mem::take(&mut self.width) as usize;
        let left = self.flags.contains(&b'-');
        let zero = self.flags.contains(&b'0');
        let integer = b"diu".contains(&conversion);
        let precision = self.precision;
        if integer && self.precision != Some(0) {
            self.precision = None;
        }
        self.flags
            .retain(|flag| !matches!(flag, b'\'' | b'-' | b'0'));
        let mut raw = Vec::new();
        self.numeric(conversion, locale.decimal, args, &mut raw)?;
        let sign = usize::from(raw.first().is_some_and(|c| matches!(c, b'-' | b'+' | b' ')));
        let mut prefix = sign;
        if raw[sign..].starts_with(b"0x") || raw[sign..].starts_with(b"0X") {
            prefix += 2;
        }
        let mut digits = raw[prefix..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let finite = digits != 0 || prefix != sign;
        let mut cuts = Vec::new();
        if grouped {
            let mut group = locale.groups.iter().copied();
            let mut size = group.next().unwrap() as usize;
            while digits > size {
                digits -= size;
                cuts.push(prefix + digits);
                size = group.next().map_or(size, usize::from);
            }
        }
        let point = raw.iter().position(|&b| b == b'.');
        let len = raw.len()
            + cuts.len() * locale.separator.len()
            + point.map_or(0, |_| locale.decimal.len().saturating_sub(1));
        let zeros = if integer {
            (precision.unwrap_or(0) as usize).saturating_sub(raw.len() - sign)
        } else {
            0
        };
        let padding = width.saturating_sub(len + zeros);
        let pad_zero = zero && !left && finite && !(integer && precision.is_some());
        out.try_reserve(len + zeros + padding)
            .map_err(|_| "field too large")?;
        if !left && !pad_zero {
            out.resize(out.len() + padding, b' ');
        }
        out.extend_from_slice(&raw[..prefix]);
        out.resize(out.len() + zeros + if pad_zero { padding } else { 0 }, b'0');
        let mut start = prefix;
        for cut in cuts.into_iter().rev() {
            out.extend_from_slice(&raw[start..cut]);
            out.extend_from_slice(locale.separator.as_bytes());
            start = cut;
        }
        if let Some(point) = point {
            out.extend_from_slice(&raw[start..point]);
            out.extend_from_slice(locale.decimal.as_bytes());
            start = point + 1;
        }
        out.extend_from_slice(&raw[start..]);
        if left {
            out.resize(out.len() + padding, b' ');
        }
        Ok(())
    }

    fn numeric(
        &self,
        conversion: u8,
        decimal: &str,
        args: &mut Arguments<'_>,
        output: &mut Vec<u8>,
    ) -> Result<(), &'static str> {
        if self.flags.is_empty() && self.width == 0 && self.precision.is_none() {
            // The default integer conversions have no padding, grouping or
            // alternate prefixes. Write directly into the existing byte buffer.
            match conversion {
                b'd' | b'i' => write!(output, "{}", args.signed()),
                b'u' => write!(output, "{}", args.unsigned()),
                b'x' => write!(output, "{:x}", args.unsigned()),
                b'X' => write!(output, "{:X}", args.unsigned()),
                b'o' => write!(output, "{:o}", args.unsigned()),
                _ => {
                    let fmt = [b'%', conversion, 0];
                    let n = args.float(decimal);
                    return append_number(output, |dst, cap| unsafe {
                        libc::snprintf(dst, cap, fmt.as_ptr().cast(), n)
                    });
                }
            }
            .expect("writing to Vec cannot fail");
            return Ok(());
        }
        // Only a validated numeric conversion reaches libc. Build its exact
        // argument type explicitly; user-provided C length modifiers are discarded.
        let mut fmt = vec![b'%'];
        fmt.extend(self.flags.iter().copied().filter(|&f| f != b'\''));
        if self.width != 0 {
            fmt.extend_from_slice(self.width.to_string().as_bytes());
        }
        if let Some(p) = self.precision {
            fmt.push(b'.');
            fmt.extend_from_slice(p.to_string().as_bytes());
        }
        if b"diouxX".contains(&conversion) {
            fmt.extend_from_slice(b"ll");
        }
        fmt.push(conversion);
        let fmt = CString::new(fmt).unwrap();
        match conversion {
            b'd' | b'i' => {
                let n = args.signed();
                append_number(output, |dst, cap| unsafe {
                    libc::snprintf(dst, cap, fmt.as_ptr(), n)
                })
            }
            b'o' | b'u' | b'x' | b'X' => {
                let n = args.unsigned();
                append_number(output, |dst, cap| unsafe {
                    libc::snprintf(dst, cap, fmt.as_ptr(), n)
                })
            }
            _ => {
                let n = args.float(decimal);
                append_number(output, |dst, cap| unsafe {
                    libc::snprintf(dst, cap, fmt.as_ptr(), n)
                })
            }
        }
    }
}

fn append_number(
    out: &mut Vec<u8>,
    format: impl Fn(*mut libc::c_char, usize) -> i32,
) -> Result<(), &'static str> {
    let mut buf = [0u8; 128];
    let n = format(buf.as_mut_ptr().cast(), buf.len());
    let len = usize::try_from(n).map_err(|_| "invalid numeric format")?;
    if len < buf.len() {
        out.extend_from_slice(&buf[..len]);
    } else {
        let start = out.len();
        out.try_reserve(len + 1).map_err(|_| "field too large")?;
        out.resize(start + len + 1, 0);
        format(out[start..].as_mut_ptr().cast(), len + 1);
        out.pop();
    }
    Ok(())
}

fn amount(input: &mut &[u8], args: &mut Arguments<'_>, overflow: i32) -> i32 {
    let value = if input.first() == Some(&b'*') {
        *input = &input[1..];
        i32::try_from(args.signed()).ok()
    } else {
        let mut amount = Some(0i32);
        while let Some((&digit, rest)) = input.split_first() {
            if !digit.is_ascii_digit() {
                break;
            }
            amount = amount.and_then(|n| n.checked_mul(10)?.checked_add(i32::from(digit - b'0')));
            *input = rest;
        }
        amount
    };
    value.unwrap_or_else(|| {
        eprintln!("tonari: printf: field width or precision out of range");
        args.status = 1;
        overflow
    })
}

fn render(
    mut input: &[u8],
    args: &mut Arguments<'_>,
    out: &mut Vec<u8>,
    variables: &mut Variables,
) -> Result<bool, &'static str> {
    while let Some((&byte, rest)) = input.split_first() {
        input = rest;
        match byte {
            b'\\' => {
                escapes::one(&mut input, out, Mode::Format);
            }
            b'%' if input.first() == Some(&b's') => {
                out.extend_from_slice(args.next().as_bytes());
                input = &input[1..];
            }
            b'%' if input.first() == Some(&b'%') => {
                out.push(b'%');
                input = &input[1..];
            }
            b'%' => {
                let original = input;
                let (mut spec, conversion) = Spec::parse(&mut input, args)?;
                match conversion {
                    b's' => spec.bytes(args.next().as_bytes(), out)?,
                    b'(' => {
                        let mut depth = 1;
                        let close = input.iter().position(|&b| {
                            match b {
                                b'(' => depth += 1,
                                b')' => depth -= 1,
                                _ => {}
                            }
                            depth == 0
                        });
                        if let Some(end) = close.filter(|i| input.get(i + 1) == Some(&b'T')) {
                            let format = std::str::from_utf8(&input[..end]).unwrap();
                            input = &input[end + 2..];
                            let timestamp = if args.values.is_empty() {
                                -1
                            } else {
                                args.signed()
                            };
                            let seconds = match timestamp {
                                -1 => unsafe { libc::time(std::ptr::null_mut()) },
                                -2 => variables.read().started_at,
                                t => t,
                            };
                            let stamp = crate::shell_time::format(
                                variables.read(),
                                if format.is_empty() { "%X" } else { format },
                                seconds,
                            );
                            spec.bytes(stamp.as_bytes(), out)?;
                        } else {
                            eprintln!("tonari: printf: warning: invalid time format specification");
                            out.push(b'%');
                            out.extend_from_slice(&original[..original.len() - input.len()]);
                        }
                    }
                    b'S' | b'C' => {
                        let arg = args.next();
                        let single = conversion == b'C';
                        spec.wide(
                            if single && arg.is_empty() { "\0" } else { arg },
                            single,
                            out,
                        )?;
                    }
                    b'c' => {
                        let byte = args.next().as_bytes().first().copied().unwrap_or(0);
                        spec.precision = None;
                        spec.bytes(&[byte], out)?;
                    }
                    b'b' => {
                        let mut bytes = Vec::new();
                        let complete =
                            escapes::decode(args.next().as_bytes(), &mut bytes, Mode::Argument);
                        spec.bytes(&bytes, out)?;
                        if !complete {
                            return Ok(false);
                        }
                    }
                    b'q' | b'Q' => {
                        let mut data = args.next().as_bytes();
                        if conversion == b'Q'
                            && let Some(p) = spec.precision.take()
                        {
                            data = &data[..data.len().min(p as usize)];
                        }
                        spec.bytes(
                            &quote(data, variables.read(), spec.flags.contains(&b'#')),
                            out,
                        )?;
                    }
                    b'n' => {
                        let name = args.next();
                        if !name.is_empty() {
                            if !is_identifier(name) {
                                return Err("invalid identifier for %n");
                            }
                            if variables.read().readonly(name) {
                                eprintln!("tonari: printf: {name}: readonly variable");
                                args.status = 1;
                            } else {
                                variables.write().assign(name, &out.len().to_string());
                            }
                        }
                    }
                    k if b"diouxXaAeEfFgG".contains(&k) => {
                        let float = b"aAeEfFgG".contains(&k);
                        let mut locale = numeric::C;
                        if float || spec.flags.contains(&b'\'') {
                            locale = *args.locale.get_or_insert_with(|| {
                                ["LC_ALL", "LC_NUMERIC", "LANG"]
                                    .into_iter()
                                    .find_map(|key| {
                                        variables.read().get(key).filter(|s| !s.is_empty())
                                    })
                                    .as_deref()
                                    .map(numeric::find)
                                    .unwrap_or(numeric::C)
                            });
                        }
                        let grouped = spec.flags.contains(&b'\'')
                            && !locale.groups.is_empty()
                            && !locale.separator.is_empty()
                            && b"diufFgG".contains(&k);
                        if (float && locale.decimal != ".") || grouped {
                            spec.localized(k, locale, grouped, args, out)?;
                        } else {
                            spec.numeric(k, ".", args, out)?;
                        }
                    }
                    _ => return Err("unsupported conversion"),
                }
            }
            _ => out.push(byte),
        }
    }
    Ok(true)
}

fn quote(data: &[u8], session: &Session, single: bool) -> Vec<u8> {
    if data.is_empty() {
        return b"''".to_vec();
    }
    let ascii_locale = session.ascii_quoting();
    if needs_ansi(data, ascii_locale) {
        return ansi_quote(data, ascii_locale);
    }
    if single && let Ok(text) = std::str::from_utf8(data) {
        return crate::word::quote::single_quote(text).into_bytes();
    }
    let mut out = Vec::new();
    for (i, &b) in data.iter().enumerate() {
        if b.is_ascii_whitespace()
            || b"!\"$&'()*,;<>?[\\]^`{|}".contains(&b)
            || i == 0 && matches!(b, b'#' | b'~')
        {
            out.push(b'\\');
        }
        out.push(b);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatted(format: &str, args: &[&str]) -> (String, u8) {
        let mut session = Session::default();
        let words = ["-v", "result", format]
            .into_iter()
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let status = run(&mut session, &words, None);
        (session.get("result").unwrap(), status)
    }

    #[test]
    fn repeats_format_and_preserves_bash_argument_conversions() {
        assert_eq!(
            formatted("%#06x:%.*s;", &["010", "2", "abc", "15", "1", "end"]),
            ("0x0008:ab;0x000f:e;".into(), 0)
        );
        assert_eq!(formatted("%*d", &["-4", "'A"]), ("65  ".into(), 0));
        assert_eq!(formatted("%d", &["08"]), ("0".into(), 1));
    }

    #[test]
    fn quotes_shell_syntax_and_stops_at_argument_escape() {
        assert_eq!(formatted("%q", &[""]), ("''".into(), 0));
        assert_eq!(formatted("%q", &["a b*"]), ("a\\ b\\*".into(), 0));
        assert_eq!(formatted("%q", &["x\ny"]), ("$'x\\ny'".into(), 0));
        assert_eq!(
            formatted(":%b:%s", &["\\0123\\crest", "lost"]),
            (":S".into(), 0)
        );
    }
}
