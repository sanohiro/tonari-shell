pub(super) fn tilde(s: &str, env: &crate::expansion::Expansion<'_>) -> String {
    let prefix = s.strip_suffix('/').unwrap_or(s);
    let home = match prefix {
        "~" => env.lookup("HOME").or_else(account_home),
        "~+" => env.lookup("PWD"),
        "~-" => env.lookup("OLDPWD"),
        numbered
            if numbered[1..]
                .trim_start_matches(['+', '-'])
                .starts_with(|c: char| c.is_ascii_digit()) =>
        {
            env.session.stack_index(&numbered[1..]).and_then(|i| {
                if i == 0 {
                    env.lookup("PWD")
                } else {
                    env.session.stack_entry(i)
                }
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
pub(crate) fn named_home(name: &str) -> Option<String> {
    account_field(Some(name), Field::Home)
}

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
