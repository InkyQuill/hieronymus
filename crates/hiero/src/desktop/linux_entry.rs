//! XDG desktop entries use two escaping layers; they are never shell scripts.
use std::path::Path;

pub fn argument(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("Desktop paths must be absolute".into());
    }
    let value = path.to_str().ok_or("Desktop paths must be UTF-8")?;
    if value.chars().any(char::is_control) {
        return Err("Desktop paths cannot contain newlines or control characters".into());
    }
    // Exec quoting is decoded after desktop string escaping. Percent is a
    // field-code introducer even inside a quoted argument.
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '%' => quoted.push_str("%%"),
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    Ok(quoted.replace('\\', "\\\\"))
}

pub fn render(binary: &Path, root: &Path) -> Result<String, String> {
    if binary.as_os_str().to_string_lossy().contains('=') {
        return Err("Desktop executable paths cannot contain an equal sign".into());
    }
    // GLib checks executable existence before expanding %%. Keep the actual
    // percent-bearing path in argv, using a fixed non-shell absolute dispatcher.
    let prefix = if binary.as_os_str().to_string_lossy().contains('%') {
        if !Path::new("/usr/bin/env").is_file() {
            return Err("Percent-containing desktop executable paths require /usr/bin/env; install coreutils or choose another application directory".into());
        }
        "\"/usr/bin/env\" -- "
    } else {
        ""
    };
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=Hieronymus\nTerminal=false\nIcon=hieronymus\nExec={prefix}{} tray --data-root {}\n",
        argument(binary)?,
        argument(root)?
    ))
}
