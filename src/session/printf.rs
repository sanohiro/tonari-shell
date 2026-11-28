use crate::word::quote::{ansi_quote, needs_ansi};
use std::ffi::CString;
use std::io::Write;

use super::Session;
use crate::builtins::escapes::{self, Mode};
use crate::parser::is_identifier;
use tonari_time_locales::numeric::{self, Numeric};

// Reads during a capture share the shell state. A captured %n or -v gets a
// private session on its first assignment, visible to the rest of that printf.
struct Variables<'a> {
    shell: &'a mut Session,
    capture: bool,
    has_target: bool,
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
    while let Some(option) = crate::builtins::flags::next(&mut words) {
        let Some(attached) = option.strip_prefix("-v") else {
            diagnostic!("printf: invalid option");
            return 2;
        };
        let Some(name) = crate::builtins::flags::value(attached, &mut words) else {
            diagnostic!("printf: -v requires a variable");
            return 2;
        };
        if !session.valid_builtin_target(name) {
            diagnostic!("printf: {name}: invalid identifier");
            return 2;
        }
        target = Some(name);
    }
    let Some((format, values)) = words.split_first() else {
        diagnostic!("printf: format required");
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
        if matches!(format.as_str(), "%s\\n" | "%s\n") && values.len() <= 1 {
            let text = values.first().map_or("", String::as_str);
            let mut line = [0u8; 256];
            if text.len() < line.len() {
                line[..text.len()].copy_from_slice(text.as_bytes());
                line[text.len()] = b'\n';
                return crate::builtins::write_stdout_for("printf", &line[..=text.len()]);
            }
        }
        let borrowed = if format == "%s" && values.len() <= 1 {
            Some(values.first().map_or("", String::as_str))
        } else if !format.contains(['%', '\\']) {
            Some(format.as_str())
        } else {
            None
        };
        if let Some(text) = borrowed {
            return crate::builtins::write_stdout_for("printf", text.as_bytes());
        }
    }
    let mut variables = Variables {
        shell: session,
        capture: capture.is_some(),
        has_target: target.is_some(),
        private: None,
    };
    let mut args = Arguments {
        stdout: (capture.is_none() && target.is_none()).then(StdoutProgress::default),
        values,
        status: 0,
        locale: None,
        byte_char_codes: values
            .iter()
            .any(|s| matches!(s.as_bytes().first(), Some(b'\'' | b'"')))
            && variables.read().ascii_quoting(),
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
                    args.flush_lines(&output);
                    diagnostic!("printf: {message}");
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
            diagnostic!("printf: expected UTF-8 variable value");
            return 1;
        };
        if variables.capture && variables.read().capture_local(name) {
            return args.status;
        }
        if let Err(e) = variables.write().assign_builtin_target(name, value) {
            diagnostic!("printf: {e}");
            return 1;
        }
    } else if let Some(capture) = capture {
        if capture.is_empty() {
            *capture = output;
        } else {
            capture.extend_from_slice(&output);
        }
    } else if crate::builtins::write_stdout_for(
        "printf",
        &output[args.stdout.as_ref().map_or(0, |s| s.written)..],
    ) != 0
    {
        return 1;
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

#[derive(Default)]
struct StdoutProgress {
    scanned: usize,
    written: usize,
}

struct Arguments<'a> {
    stdout: Option<StdoutProgress>,
    values: &'a [String],
    status: u8,
    locale: Option<Numeric>,
    byte_char_codes: bool,
}

impl<'a> Arguments<'a> {
    // Bash flushes completed lines before a diagnostic on merged stdout/stderr.
    // Track both offsets so repeated errors on one long line only scan new bytes.
    #[cold]
    fn flush_lines(&mut self, output: &[u8]) {
        if let Some(state) = &mut self.stdout {
            if let Some(offset) = memchr::memrchr(b'\n', &output[state.scanned..]) {
                let end = state.scanned + offset + 1;
                self.status |=
                    crate::builtins::write_stdout_for("printf", &output[state.written..end]);
                state.written = end;
            }
            state.scanned = output.len();
        }
    }

    #[cold]
    fn literal_field_overflow(
        &mut self,
        digits: &[u8],
        conversion: u8,
        fallback: i32,
        output: &[u8],
    ) -> Result<i32, &'static str> {
        // %n stores the number already written and ignores the field format.
        if conversion == b'n' {
            return Ok(fallback);
        }
        if !matches!(conversion, b'b' | b'q' | b'Q' | b'(') {
            return Err("field width or precision out of range");
        }
        self.flush_lines(output);
        let value = std::str::from_utf8(digits).expect("decimal field digits");
        let error = std::io::Error::from_raw_os_error(libc::ERANGE);
        diagnostic!("printf: {value}: {}", crate::builtins::os_message(&error));
        self.status = 1;
        Ok(fallback)
    }

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
        output: &[u8],
    ) -> T {
        if self.values.is_empty() {
            return from_char(0);
        }
        let value = self.next();
        if value.is_empty() {
            self.flush_lines(output);
            diagnostic!("printf: : invalid number");
            self.status = 1;
            return from_char(0);
        }
        if let Some(rest) = value.strip_prefix(['\'', '"']) {
            return from_char(if self.byte_char_codes {
                rest.as_bytes().first().copied().unwrap_or(0).into()
            } else {
                rest.chars().next().map_or(0, u32::from)
            });
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
            self.flush_lines(output);
            diagnostic!("printf: invalid number");
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
        let (error, remaining) = unsafe { (*errno, *end) };
        if remaining != 0 || error != 0 && error != libc::ERANGE {
            self.flush_lines(output);
            diagnostic!("printf: {value}: invalid number");
            self.status = 1;
        } else if error == libc::ERANGE {
            self.flush_lines(output);
            diagnostic!("printf: {value}: value out of range");
            self.status = 1;
        }
        result
    }

    fn signed(&mut self, output: &[u8]) -> i64 {
        // Common decimal widths and operands need neither a C string nor errno.
        if let Some((word, rest)) = self.values.split_first()
            && (word == "0"
                || word
                    .strip_prefix(['-', '+'])
                    .unwrap_or(word)
                    .as_bytes()
                    .first()
                    .is_some_and(|b| matches!(b, b'1'..=b'9')))
            && let Ok(value) = word.parse::<i64>()
        {
            self.values = rest;
            return value;
        }
        self.number(
            |p, end| unsafe { libc::strtoll(p, end, 0) },
            i64::from,
            ".",
            output,
        )
    }
    fn unsigned(&mut self, output: &[u8]) -> u64 {
        if let Some((word, rest)) = self.values.split_first()
            && (word == "0"
                || word
                    .as_bytes()
                    .first()
                    .is_some_and(|b| matches!(b, b'1'..=b'9')))
            && let Ok(value) = word.parse::<u64>()
        {
            self.values = rest;
            return value;
        }
        self.number(
            |p, end| unsafe { libc::strtoull(p, end, 0) },
            u64::from,
            ".",
            output,
        )
    }
    fn float(&mut self, decimal: &str, output: &[u8]) -> f64 {
        self.number(
            |p, end| unsafe { libc::strtod(p, end) },
            f64::from,
            decimal,
            output,
        )
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

fn flag_bit(flag: u8) -> u8 {
    match flag {
        b'#' => 1,
        b'0' => 2,
        b'-' => 4,
        b' ' => 8,
        b'+' => 16,
        b'\'' => 32,
        _ => 0,
    }
}

struct Spec {
    flags: u8,
    width: i32,
    precision: Option<i32>,
}

impl Spec {
    fn has(&self, flag: u8) -> bool {
        self.flags & flag_bit(flag) != 0
    }

    fn parse(
        input: &mut &[u8],
        args: &mut Arguments<'_>,
        output: &[u8],
    ) -> Result<(Self, u8), &'static str> {
        let mut flags = 0;
        while let Some((&flag, rest)) = input.split_first() {
            let bit = flag_bit(flag);
            if bit == 0 {
                break;
            }
            flags |= bit;
            *input = rest;
        }
        let width = amount(input, args, 0, output);
        let precision = input.strip_prefix(b".").map(|rest| {
            *input = rest;
            amount(input, args, -1, output)
        });
        let mut wide = false;
        while let Some((&len, rest)) = input.split_first() {
            if !LENGTH_MODIFIERS.contains(&len) {
                break;
            }
            wide |= len == b'l';
            *input = rest;
        }
        let (conversion, rest) = input.split_first().ok_or("`%': missing format character")?;
        *input = rest;
        let conversion = if wide && matches!(conversion, b'c' | b's') {
            conversion.to_ascii_uppercase()
        } else {
            *conversion
        };
        let mut field = |amount, fallback| match amount {
            Ok(n) => Ok(n),
            Err(digits) => args.literal_field_overflow(digits, conversion, fallback, output),
        };
        let mut width = field(width, 0)?;
        if width < 0 {
            flags |= flag_bit(b'-');
            width = width.checked_abs().ok_or("field width too large")?;
        }
        // Bash's %Q applies precision before quoting. An overflowing literal
        // leaves its input intact. The other shell string conversions use zero.
        let precision = precision
            .map(|p| field(p, if conversion == b'Q' { -1 } else { 0 }))
            .transpose()?
            .filter(|&p| p >= 0);
        Ok((
            Self {
                flags,
                width,
                precision,
            },
            conversion,
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
        if !self.has(b'-') {
            output.resize(output.len() + padding, b' ');
        }
        output.extend_from_slice(&value[..len]);
        if self.has(b'-') {
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
        let left = self.has(b'-');
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
        let left = self.has(b'-');
        let zero = self.has(b'0');
        let integer = b"diu".contains(&conversion);
        let precision = self.precision;
        if integer && self.precision != Some(0) {
            self.precision = None;
        }
        self.flags &= !(flag_bit(b'\'') | flag_bit(b'-') | flag_bit(b'0'));
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
        if self.flags == 0 && self.width == 0 && self.precision.is_none() {
            // The default integer conversions have no padding, grouping or
            // alternate prefixes. Write directly into the existing byte buffer.
            match conversion {
                b'd' | b'i' => write!(output, "{}", args.signed(output)),
                b'u' => write!(output, "{}", args.unsigned(output)),
                b'x' => write!(output, "{:x}", args.unsigned(output)),
                b'X' => write!(output, "{:X}", args.unsigned(output)),
                b'o' => write!(output, "{:o}", args.unsigned(output)),
                _ => {
                    let fmt = [b'%', conversion, 0];
                    let n = args.float(decimal, output);
                    return append_number(output, |dst, cap| unsafe {
                        libc::snprintf(dst, cap, fmt.as_ptr().cast(), n)
                    });
                }
            }
            .expect("writing to Vec cannot fail");
            return Ok(());
        }
        if self.width <= 4096
            && self.precision.is_none()
            && self.has(b'0')
            && self.flags & !(flag_bit(b'0') | flag_bit(b'+')) == 0
            && b"diuxXo".contains(&conversion)
        {
            let signed = matches!(conversion, b'd' | b'i');
            let (value, sign) = if signed {
                let value = args.signed(output);
                (
                    value.unsigned_abs(),
                    if value < 0 {
                        Some(b'-')
                    } else if self.has(b'+') {
                        Some(b'+')
                    } else {
                        None
                    },
                )
            } else {
                (args.unsigned(output), None)
            };
            let mut decimal = itoa::Buffer::new();
            let mut digits = [0; 22]; // ceil(64/3) octal digits
            let number = if signed || conversion == b'u' {
                decimal.format(value).as_bytes()
            } else {
                let alphabet = if conversion == b'X' {
                    b"0123456789ABCDEF"
                } else {
                    b"0123456789abcdef"
                };
                let shift = if conversion == b'o' { 3 } else { 4 };
                let mask = (1 << shift) - 1;
                let mut n = value;
                let mut i = digits.len();
                loop {
                    i -= 1;
                    digits[i] = alphabet[(n & mask) as usize];
                    n >>= shift;
                    if n == 0 {
                        break;
                    }
                }
                &digits[i..]
            };
            let length = number.len() + usize::from(sign.is_some());
            let padding = (self.width as usize).saturating_sub(length);
            output
                .try_reserve(length + padding)
                .map_err(|_| "field too large")?;
            if let Some(sign) = sign {
                output.push(sign);
            }
            output.resize(output.len() + padding, b'0');
            output.extend_from_slice(number);
            return Ok(());
        }
        // Only a validated numeric conversion reaches libc. Build its exact
        // argument type explicitly; user-provided C length modifiers are discarded.
        // Six distinct flags and two nonnegative i32 fields fit in 32 bytes,
        // including %, the decimal point, ll, conversion and terminating NUL.
        let mut format = [0u8; 32];
        let mut fmt = &mut format[..];
        fmt.write_all(b"%").unwrap();
        for &flag in b"#0- +" {
            if self.has(flag) {
                fmt.write_all(&[flag]).unwrap();
            }
        }
        if self.width != 0 {
            write!(fmt, "{}", self.width).unwrap();
        }
        if let Some(p) = self.precision {
            write!(fmt, ".{p}").unwrap();
        }
        if b"diouxX".contains(&conversion) {
            fmt.write_all(b"ll").unwrap();
        }
        fmt[0] = conversion;
        let fmt = format.as_ptr().cast::<libc::c_char>();
        match conversion {
            b'd' | b'i' => {
                let n = args.signed(output);
                append_number(output, |dst, cap| unsafe {
                    libc::snprintf(dst, cap, fmt, n)
                })
            }
            b'o' | b'u' | b'x' | b'X' => {
                let n = args.unsigned(output);
                append_number(output, |dst, cap| unsafe {
                    libc::snprintf(dst, cap, fmt, n)
                })
            }
            _ => {
                let n = args.float(decimal, output);
                append_number(output, |dst, cap| unsafe {
                    libc::snprintf(dst, cap, fmt, n)
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

fn amount<'a>(
    input: &mut &'a [u8],
    args: &mut Arguments<'_>,
    overflow: i32,
    output: &[u8],
) -> Result<i32, &'a [u8]> {
    if input.first() == Some(&b'*') {
        *input = &input[1..];
        return Ok(i32::try_from(args.signed(output)).unwrap_or_else(|_| {
            args.flush_lines(output);
            diagnostic!("printf: field width or precision out of range");
            args.status = 1;
            overflow
        }));
    }
    let digits = *input;
    let mut amount = 0i32;
    while let Some((&digit, rest)) = input.split_first() {
        if !digit.is_ascii_digit() {
            break;
        }
        *input = rest;
        let Some(next) = amount
            .checked_mul(10)
            .and_then(|n| n.checked_add(i32::from(digit - b'0')))
        else {
            let tail = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            *input = &rest[tail..];
            return Err(&digits[..digits.len() - input.len()]);
        };
        amount = next;
    }
    Ok(amount)
}

fn render(
    mut input: &[u8],
    args: &mut Arguments<'_>,
    out: &mut Vec<u8>,
    variables: &mut Variables,
) -> Result<bool, std::borrow::Cow<'static, str>> {
    let start = out.len();
    while let Some((&byte, rest)) = input.split_first() {
        input = rest;
        match byte {
            b'\\' => {
                escapes::one(&mut input, out, Mode::Format, |output| {
                    args.flush_lines(output)
                });
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
                let (mut spec, conversion) = Spec::parse(&mut input, args, out)?;
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
                                args.signed(out)
                            };
                            let seconds = match timestamp {
                                -1 => unsafe { libc::time(std::ptr::null_mut()) },
                                -2 => variables.read().started_at,
                                t => t,
                            };
                            let stamp = crate::shell_time::try_format(
                                variables.read(),
                                if format.is_empty() { "%X" } else { format },
                                seconds,
                            )
                            .ok_or("time or format out of range")?;
                            spec.bytes(stamp.as_bytes(), out)?;
                        } else {
                            args.flush_lines(out);
                            diagnostic!("printf: warning: invalid time format specification");
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
                        let complete = escapes::decode(
                            args.next().as_bytes(),
                            &mut bytes,
                            Mode::Argument,
                            |_| args.flush_lines(out),
                        );
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
                        spec.bytes(&quote(data, variables.read(), spec.has(b'#')), out)?;
                    }
                    b'n' => {
                        let name = args.next();
                        if !name.is_empty() {
                            if !is_identifier(name) {
                                return Err("invalid identifier for %n".into());
                            }
                            if variables.read().readonly(name) {
                                args.flush_lines(out);
                                diagnostic!("printf: {name}: readonly variable");
                                args.status = 1;
                            } else if !(variables.capture
                                && !variables.has_target
                                && input.is_empty()
                                && args.values.is_empty()
                                && variables.read().capture_local(name))
                            {
                                variables
                                    .write()
                                    .assign(name, itoa::Buffer::new().format(out.len() - start));
                            }
                        }
                    }
                    k if b"diouxXaAeEfFgG".contains(&k) => {
                        let float = b"aAeEfFgG".contains(&k);
                        let mut locale = numeric::C;
                        if float || spec.has(b'\'') {
                            locale = *args.locale.get_or_insert_with(|| {
                                variables
                                    .read()
                                    .locale_category("LC_NUMERIC")
                                    .as_deref()
                                    .map(numeric::find)
                                    .unwrap_or(numeric::C)
                            });
                        }
                        let grouped = spec.has(b'\'')
                            && !locale.groups.is_empty()
                            && !locale.separator.is_empty()
                            && b"diufFgG".contains(&k);
                        if (float && locale.decimal != ".") || grouped {
                            spec.localized(k, locale, grouped, args, out)?;
                        } else {
                            spec.numeric(k, ".", args, out)?;
                        }
                    }
                    _ => {
                        return Err(format!(
                            "`{}': invalid format character",
                            char::from(conversion)
                        )
                        .into());
                    }
                }
            }
            _ => {
                out.push(byte);
                let n = memchr::memchr2(b'%', b'\\', input).unwrap_or(input.len());
                out.extend_from_slice(&input[..n]);
                input = &input[n..];
            }
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
            || i == 0 && b == b'#'
            || crate::word::quote::tilde_at(data, i)
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
