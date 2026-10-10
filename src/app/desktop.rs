//! Ярлык запускает `cm app run`, а не программу в сети хоста.

use crate::profiles::Id;

use super::spec::SpecError;

pub fn desktop_entry(id: &str, name: &str, icon: Option<&str>) -> Result<String, SpecError> {
    Id::new(id).map_err(|_| SpecError::BadId)?;
    if name.chars().any(|ch| ch.is_control()) {
        return Err(SpecError::HasNul);
    }
    let mut text = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=cm app run {}\n",
        escape_value(name),
        exec_quote(id)
    );
    if let Some(icon) = icon {
        text.push_str(&format!("Icon={icon}\n"));
    }
    text.push_str(&format!("Terminal=false\nX-CM-Application={id}\n"));
    Ok(text)
}

pub fn exec_quote(arg: &str) -> String {
    let quote = arg.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\''
                    | '\\'
                    | '>'
                    | '<'
                    | '~'
                    | '|'
                    | '&'
                    | ';'
                    | '$'
                    | '*'
                    | '?'
                    | '#'
                    | '('
                    | ')'
                    | '`'
            )
    });
    let mut body = String::new();
    for ch in arg.chars() {
        if quote && matches!(ch, '"' | '`' | '$' | '\\') {
            body.push('\\');
        }
        body.push(ch);
    }
    let body = body.replace('%', "%%");
    if quote { format!("\"{body}\"") } else { body }
}

fn escape_value(name: &str) -> String {
    name.replace('\\', "\\\\")
}
