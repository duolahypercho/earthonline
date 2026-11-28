//! Load the desktop app's optional environment overrides at startup.

use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use std::{env, fs, io};

/// Apply the user's environment file, giving inherited variables precedence.
///
/// # Safety
/// Call at the start of main, before the game or its libraries start threads.
pub(crate) unsafe fn load() -> Result<()> {
    let Some(settings_path) = eo_ui::settings::default_path() else {
        return Ok(());
    };
    let Some(directory) = settings_path.parent() else {
        return Ok(());
    };
    let path = directory.join("environment.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let entries: BTreeMap<String, String> = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "parsing {} as an object of environment names and string values",
            path.display()
        )
    })?;
    for (name, value) in &entries {
        let valid_name = name.strip_prefix("EO_").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        });
        ensure!(valid_name, "{}: invalid EO_* name {name:?}", path.display());
        ensure!(
            !value.contains('\0'),
            "{}: {name} contains a NUL character",
            path.display()
        );
    }
    for (name, value) in entries {
        if env::var_os(&name).is_none() {
            // SAFETY: the caller runs before any worker threads start. All entries were validated above.
            unsafe { env::set_var(name, value) };
        }
    }
    Ok(())
}
