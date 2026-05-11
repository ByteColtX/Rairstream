pub const OS_NAME: &str = "windows";

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "Rairstream";

#[must_use]
pub const fn supports_system_audio_capture() -> bool {
    true
}

pub fn is_start_at_login_enabled() -> Result<bool, String> {
    let run_key = windows_registry::CURRENT_USER
        .open(RUN_KEY)
        .map_err(|error| format!("failed to open Windows Run registry key: {error}"))?;

    match run_key.get_string(RUN_VALUE_NAME) {
        Ok(command) => Ok(command == start_at_login_command()?),
        Err(_) => Ok(false),
    }
}

pub fn set_start_at_login_enabled(enabled: bool) -> Result<(), String> {
    let run_key = windows_registry::CURRENT_USER
        .create(RUN_KEY)
        .map_err(|error| format!("failed to open Windows Run registry key: {error}"))?;

    if enabled {
        run_key
            .set_string(RUN_VALUE_NAME, start_at_login_command()?)
            .map_err(|error| format!("failed to enable Windows start at login: {error}"))?;
    } else if is_start_at_login_enabled()? {
        run_key
            .remove_value(RUN_VALUE_NAME)
            .map_err(|error| format!("failed to disable Windows start at login: {error}"))?;
    }

    Ok(())
}

fn start_at_login_command() -> Result<String, String> {
    let exe_path = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;
    Ok(quote_command_path(&exe_path.to_string_lossy()))
}

fn quote_command_path(path: &str) -> String {
    format!("\"{}\"", path.replace('"', r#"\""#))
}

#[cfg(test)]
mod tests {
    use super::quote_command_path;

    #[test]
    fn quote_command_path_wraps_executable_path() {
        assert_eq!(
            quote_command_path(r"C:\Program Files\Rairstream\rairstream.exe"),
            r#""C:\Program Files\Rairstream\rairstream.exe""#
        );
    }

    #[test]
    fn quote_command_path_escapes_embedded_quotes() {
        assert_eq!(
            quote_command_path(r#"C:\Apps\Rain"stream.exe"#),
            r#""C:\Apps\Rain\"stream.exe""#
        );
    }
}
