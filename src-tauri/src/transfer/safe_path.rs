use std::path::{Component, Path, PathBuf};

use crate::error::AppError;

pub fn resolve_inside(base: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let candidate = Path::new(relative);
    if candidate.is_absolute()
        || relative.starts_with('\\')
        || relative.starts_with('/')
        || relative.as_bytes().get(1) == Some(&b':')
    {
        return Err(AppError::InvalidArgument(
            "transfer path must be relative".into(),
        ));
    }

    let mut result = PathBuf::from(base);
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text
                    .chars()
                    .any(|character| character.is_control() || character == '\0')
                {
                    return Err(AppError::InvalidArgument(
                        "transfer path contains control characters".into(),
                    ));
                }
                result.push(part);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::InvalidArgument(
                    "transfer path escapes its destination".into(),
                ));
            }
        }
    }
    Ok(result)
}

pub fn safe_file_name(name: &str) -> String {
    let mut value = name
        .replace(['/', '\\'], "_")
        .chars()
        .filter(|character| !character.is_control() && *character != '\0')
        .collect::<String>()
        .trim()
        .to_owned();
    if value.is_empty() || value == "." || value == ".." {
        value = "download".into();
    }
    let reserved = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(reserved.as_str(), "con" | "prn" | "aux" | "nul")
        || (reserved.len() == 4
            && (reserved.starts_with("com") || reserved.starts_with("lpt"))
            && reserved.as_bytes()[3].is_ascii_digit())
    {
        value.insert(0, '_');
    }
    value.chars().take(255).collect()
}

#[cfg(test)]
mod tests {
    use super::{resolve_inside, safe_file_name};
    use std::path::Path;

    #[test]
    fn rejects_parent_and_absolute_paths() {
        assert!(resolve_inside(Path::new("/tmp/destination"), "../escape").is_err());
        assert!(resolve_inside(Path::new("/tmp/destination"), "/tmp/escape").is_err());
        assert!(resolve_inside(Path::new("/tmp/destination"), "C:\\escape").is_err());
    }

    #[test]
    fn sanitizes_names_and_reserved_devices() {
        assert_eq!(safe_file_name("../report.txt"), ".._report.txt");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("\0\n"), "download");
    }
}
