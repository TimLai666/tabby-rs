use crate::ssh::model::SshError;

pub fn normalize(path: &str) -> Result<String, SshError> {
    if path.len() > 16 * 1024 || path.bytes().any(|byte| byte == 0) {
        return Err(SshError::InvalidRequest(
            "remote path is too long or contains a NUL byte".into(),
        ));
    }

    let absolute = path.starts_with('/');
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if absolute => {
                if parts.pop().is_none() {
                    return Err(SshError::InvalidRequest(
                        "remote path escapes the POSIX root".into(),
                    ));
                }
            }
            ".." if parts.last().is_some_and(|value| *value != "..") => {
                parts.pop();
            }
            ".." => parts.push(".."),
            value => parts.push(value),
        }
    }

    let result = parts.join("/");
    if absolute {
        Ok(if result.is_empty() {
            "/".into()
        } else {
            format!("/{result}")
        })
    } else if result.is_empty() {
        Ok(".".into())
    } else {
        Ok(result)
    }
}

pub fn join(parent: &str, name: &str) -> Result<String, SshError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(SshError::InvalidRequest(
            "remote entry name is not a single POSIX path component".into(),
        ));
    }
    normalize(&format!("{}/{}", parent.trim_end_matches('/'), name))
}

pub fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("/")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{basename, join, normalize};

    #[test]
    fn normalizes_posix_components_without_using_local_path_rules() {
        assert_eq!(normalize("/var//tmp/./file").unwrap(), "/var/tmp/file");
        assert_eq!(normalize("/var/tmp/../file").unwrap(), "/var/file");
        assert_eq!(normalize(".").unwrap(), ".");
        assert_eq!(normalize("a/../../file").unwrap(), "../file");
        assert!(normalize("/../../file").is_err());
    }

    #[test]
    fn joins_only_single_remote_names() {
        assert_eq!(join("/tmp/", "file").unwrap(), "/tmp/file");
        assert!(join("/tmp", "../file").is_err());
        assert_eq!(basename("/tmp/file"), "file");
    }
}
