pub(super) fn tilde(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let prefix = s.strip_suffix('/').unwrap_or(s);
    let home = match prefix {
        "~" => lookup("HOME").or_else(account_home),
        "~+" => lookup("PWD"),
        "~-" => lookup("OLDPWD"),
        numbered
            if numbered[1..]
                .trim_start_matches(['+', '-'])
                .bytes()
                .next()
                .is_some_and(|c| c.is_ascii_digit()) =>
        {
            let digits = &numbered[1..];
            let (reverse, digits) = if let Some(n) = digits.strip_prefix('-') {
                (true, n)
            } else {
                (false, digits.strip_prefix('+').unwrap_or(digits))
            };
            digits
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit)
                .then(|| digits.parse::<u32>().ok())
                .flatten()
                .and_then(|n| {
                    let i = if reverse {
                        -1 - i64::from(n)
                    } else {
                        i64::from(n)
                    };
                    lookup(&format!("DIRSTACK[{i}]"))
                })
        }
        _ => account_field(
            Some(prefix.strip_prefix('~').unwrap_or(prefix)),
            Field::Home,
        ),
    };
    let Some(mut home) = home else {
        return s.to_owned();
    };
    if prefix.len() != s.len() && !home.ends_with('/') {
        home.push('/');
    }
    home
}

// Query the OS account database for each named expansion. Keep the common
// result on the stack and retry only when a larger passwd entry needs space.
pub(crate) fn account_home() -> Option<String> {
    static HOME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    HOME.get_or_init(|| account_field(None, Field::Home))
        .clone()
}

pub(crate) fn account_shell() -> Option<String> {
    static SHELL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SHELL
        .get_or_init(|| account_field(None, Field::Shell))
        .clone()
}

enum Field {
    Home,
    Shell,
    User,
}

pub(crate) fn effective_user() -> Option<&'static str> {
    static USER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    USER.get_or_init(|| account_field(None, Field::User))
        .as_deref()
}

fn account_field(login: Option<&str>, field: Field) -> Option<String> {
    let name = login.map(std::ffi::CString::new).transpose().ok()?;
    let mut local = [0u8; 1024];
    let mut large = Vec::new();
    let mut buf = local.as_mut_slice();
    loop {
        let mut entry = std::mem::MaybeUninit::uninit();
        let mut found = std::ptr::null_mut();
        let status = unsafe {
            if let Some(name) = &name {
                libc::getpwnam_r(
                    name.as_ptr(),
                    entry.as_mut_ptr(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    &mut found,
                )
            } else {
                libc::getpwuid_r(
                    if matches!(field, Field::User) {
                        libc::geteuid()
                    } else {
                        libc::getuid()
                    },
                    entry.as_mut_ptr(),
                    buf.as_mut_ptr().cast(),
                    buf.len(),
                    &mut found,
                )
            }
        };
        if status == libc::ERANGE {
            let size = buf.len().checked_mul(2)?;
            large.try_reserve(size - large.len()).ok()?;
            large.resize(size, 0);
            buf = &mut large;
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        let field = unsafe {
            match field {
                Field::Shell => (*found).pw_shell,
                Field::Home => (*found).pw_dir,
                Field::User => (*found).pw_name,
            }
        };
        if field.is_null() {
            return None;
        }
        return unsafe { std::ffi::CStr::from_ptr(field) }
            .to_str()
            .ok()
            .map(str::to_owned);
    }
}
