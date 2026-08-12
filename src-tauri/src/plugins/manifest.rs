use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::error::AppError;

use super::npm;

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub name: String,
    pub description: String,
    pub package_name: String,
    pub is_builtin: bool,
    pub is_legacy: bool,
    pub version: String,
    pub author: String,
    pub homepage: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDescriptor {
    pub name: String,
    pub package_name: String,
    pub version: String,
    pub path: String,
    pub entry: String,
    pub is_builtin: bool,
    pub is_legacy: bool,
    pub manifest: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSource {
    pub package_name: String,
    pub entry: String,
    pub code: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct PackageManifest {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: serde_json::Value,
    homepage: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    main: Option<String>,
    module: Option<String>,
    browser: Option<String>,
}

pub fn list_installed(root: &Path) -> Result<Vec<InstalledPlugin>, AppError> {
    let node_modules = match regular_directory(root, "plugin root")? {
        Some(_) => root.join("node_modules"),
        None => return Ok(Vec::new()),
    };
    let Some(_) = regular_directory(&node_modules, "plugin node_modules")? else {
        return Ok(Vec::new());
    };

    let mut package_paths = Vec::new();
    collect_package_paths(&node_modules, &mut package_paths)?;
    let mut plugins = package_paths
        .into_iter()
        .filter_map(|(path, package_name)| read_plugin(&path, &package_name).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    plugins.sort_by(|left, right| left.package_name.cmp(&right.package_name));
    Ok(plugins)
}

pub fn discover(root: &Path) -> Result<Vec<PluginDescriptor>, AppError> {
    let node_modules = match regular_directory(root, "plugin root")? {
        Some(_) => root.join("node_modules"),
        None => return Ok(Vec::new()),
    };
    let Some(_) = regular_directory(&node_modules, "plugin node_modules")? else {
        return Ok(Vec::new());
    };

    let mut package_paths = Vec::new();
    collect_package_paths(&node_modules, &mut package_paths)?;
    let mut plugins = package_paths
        .into_iter()
        .filter_map(|(path, package_name)| read_descriptor(&path, &package_name).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    plugins.sort_by(|left, right| left.package_name.cmp(&right.package_name));
    Ok(plugins)
}

pub fn read_entry(root: &Path, package_name: &str) -> Result<PluginSource, AppError> {
    if npm::validate_package_name(package_name).is_err()
        || (!package_name.starts_with("tabby-") && !package_name.starts_with("terminus-"))
    {
        return Err(AppError::InvalidArgument(
            "plugin package name is invalid".into(),
        ));
    }
    let node_modules = root.join("node_modules");
    let mut package_paths = Vec::new();
    if regular_directory(root, "plugin root")?.is_some()
        && regular_directory(&node_modules, "plugin node_modules")?.is_some()
    {
        collect_package_paths(&node_modules, &mut package_paths)?;
    }
    let Some((path, name)) = package_paths
        .into_iter()
        .find(|(_, name)| name == package_name)
    else {
        return Err(AppError::NotFound("plugin package was not found".into()));
    };
    let Some(descriptor) = read_descriptor(&path, &name)? else {
        return Err(AppError::InvalidData("plugin manifest is invalid".into()));
    };
    let entry_path = descriptor_entry_path(&path, &descriptor.entry)?;
    let Some(bytes) = regular_file(&entry_path, "plugin entry", MAX_ENTRY_BYTES)? else {
        return Err(AppError::InvalidData(
            "plugin entry is missing or too large".into(),
        ));
    };
    let code = String::from_utf8(bytes)
        .map_err(|_| AppError::InvalidData("plugin entry is not UTF-8".into()))?;
    Ok(PluginSource {
        package_name: descriptor.package_name,
        entry: descriptor.entry,
        code,
    })
}

fn collect_package_paths(
    node_modules: &Path,
    package_paths: &mut Vec<(PathBuf, String)>,
) -> Result<(), AppError> {
    for entry in fs::read_dir(node_modules)? {
        let entry = entry?;
        let path = entry.path();
        let Some(metadata) = regular_directory(&path, "plugin package directory")? else {
            continue;
        };
        let _ = metadata;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".bin" {
            continue;
        }
        if name.starts_with('@') {
            for scoped_entry in fs::read_dir(&path)? {
                let scoped_entry = scoped_entry?;
                let scoped_path = scoped_entry.path();
                if regular_directory(&scoped_path, "scoped plugin package directory")?.is_none() {
                    continue;
                }
                let package_name = format!("{name}/{}", scoped_entry.file_name().to_string_lossy());
                package_paths.push((scoped_path, package_name));
            }
        } else {
            package_paths.push((path, name));
        }
    }
    Ok(())
}

fn read_plugin(path: &Path, package_name: &str) -> Result<Option<InstalledPlugin>, AppError> {
    let Some((manifest, _)) = read_package_manifest(path, package_name)? else {
        return Ok(None);
    };
    let legacy = is_legacy(&manifest);
    let name = package_name
        .strip_prefix("tabby-")
        .or_else(|| package_name.strip_prefix("terminus-"))
        .unwrap_or(package_name)
        .into();
    Ok(Some(InstalledPlugin {
        name,
        description: manifest.description,
        package_name: package_name.into(),
        is_builtin: false,
        is_legacy: legacy,
        version: manifest.version,
        author: author_name(&manifest.author),
        homepage: manifest.homepage,
        path: Some(path.to_string_lossy().into_owned()),
    }))
}

fn read_descriptor(path: &Path, package_name: &str) -> Result<Option<PluginDescriptor>, AppError> {
    let Some((manifest, raw)) = read_package_manifest(path, package_name)? else {
        return Ok(None);
    };
    let legacy = is_legacy(&manifest);
    let entry = manifest
        .main
        .clone()
        .or_else(|| manifest.module.clone())
        .or_else(|| manifest.browser.clone())
        .unwrap_or_else(|| "index.js".into());
    Ok(Some(PluginDescriptor {
        name: plugin_name(package_name),
        package_name: package_name.into(),
        version: manifest.version,
        path: path.to_string_lossy().into_owned(),
        entry,
        is_builtin: false,
        is_legacy: legacy,
        manifest: raw,
    }))
}

fn read_package_manifest(
    path: &Path,
    package_name: &str,
) -> Result<Option<(PackageManifest, serde_json::Value)>, AppError> {
    if npm::validate_package_name(package_name).is_err()
        || (!package_name.starts_with("tabby-") && !package_name.starts_with("terminus-"))
    {
        return Ok(None);
    }
    let manifest_path = path.join("package.json");
    let Some(bytes) = regular_file(&manifest_path, "plugin manifest", MAX_MANIFEST_BYTES)? else {
        return Ok(None);
    };
    let raw: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };
    let manifest: PackageManifest = match serde_json::from_value(raw.clone()) {
        Ok(manifest) => manifest,
        Err(_) => return Ok(None),
    };
    if manifest.name != package_name
        || npm::validate_version(&manifest.version).is_err()
        || !manifest
            .keywords
            .iter()
            .any(|keyword| is_plugin_keyword(keyword))
    {
        return Ok(None);
    }
    Ok(Some((manifest, raw)))
}

fn descriptor_entry_path(package_path: &Path, entry: &str) -> Result<PathBuf, AppError> {
    validate_entry_name(entry)?;
    let package_path = package_path
        .canonicalize()
        .map_err(|_| AppError::InvalidData("plugin package path is unavailable".into()))?;
    let entry_path = package_path.join(entry);
    let canonical_entry = entry_path
        .canonicalize()
        .map_err(|_| AppError::InvalidData("plugin entry is unavailable".into()))?;
    if !canonical_entry.starts_with(&package_path) {
        return Err(AppError::PermissionDenied(
            "plugin entry escapes its package root".into(),
        ));
    }
    Ok(canonical_entry)
}

fn validate_entry_name(entry: &str) -> Result<(), AppError> {
    if entry.is_empty()
        || entry.len() > 512
        || entry.chars().any(char::is_control)
        || Path::new(entry).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AppError::InvalidData("plugin entry path is invalid".into()));
    }
    Ok(())
}

fn is_plugin_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "tabby-plugin" | "tabby-builtin-plugin" | "terminus-plugin" | "terminus-builtin-plugin"
    )
}

fn is_legacy(manifest: &PackageManifest) -> bool {
    manifest
        .keywords
        .iter()
        .any(|keyword| keyword.starts_with("terminus-"))
        || manifest.name.starts_with("terminus-")
}

fn plugin_name(package_name: &str) -> String {
    package_name
        .strip_prefix("tabby-")
        .or_else(|| package_name.strip_prefix("terminus-"))
        .unwrap_or(package_name)
        .into()
}

fn author_name(author: &serde_json::Value) -> String {
    match author {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Object(value) => value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into(),
        _ => String::new(),
    }
}

fn regular_directory(path: &Path, description: &str) -> Result<Option<()>, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::PermissionDenied(
            format!("{description} must not be a symbolic link"),
        )),
        Ok(metadata) if metadata.is_dir() => Ok(Some(())),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn regular_file(
    path: &Path,
    description: &str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::PermissionDenied(
            format!("{description} must not be a symbolic link"),
        )),
        Ok(metadata) if metadata.is_file() && metadata.len() <= max_bytes as u64 => {
            Ok(Some(fs::read(path)?))
        }
        Ok(metadata) if metadata.is_file() => Ok(None),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{discover, list_installed, read_entry};

    #[test]
    fn lists_only_valid_tabby_plugin_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("node_modules/tabby-demo");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("package.json"),
            r#"{
                "name": "tabby-demo",
                "version": "1.2.3",
                "description": "Demo",
                "author": {"name": "Tabby"},
                "keywords": ["tabby-plugin"]
            }"#,
        )
        .unwrap();
        let invalid = temp.path().join("node_modules/tabby-invalid");
        fs::create_dir_all(&invalid).unwrap();
        fs::write(
            invalid.join("package.json"),
            r#"{"name":"tabby-invalid","version":"1.2.3","keywords":[]}"#,
        )
        .unwrap();

        let plugins = list_installed(temp.path()).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "demo");
        assert_eq!(plugins[0].author, "Tabby");
    }

    #[test]
    fn discovers_and_reads_a_bounded_plugin_entry() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("node_modules/tabby-demo");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("package.json"),
            r#"{"name":"tabby-demo","version":"1.2.3","main":"dist/index.js","keywords":["tabby-builtin-plugin"]}"#,
        )
        .unwrap();
        fs::create_dir_all(plugin.join("dist")).unwrap();
        fs::write(
            plugin.join("dist/index.js"),
            "module.exports = { default: {} }",
        )
        .unwrap();

        let descriptors = discover(temp.path()).unwrap();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].entry, "dist/index.js");
        let source = read_entry(temp.path(), "tabby-demo").unwrap();
        assert_eq!(source.package_name, "tabby-demo");
        assert!(source.code.contains("module.exports"));
    }

    #[test]
    fn rejects_entry_paths_outside_the_package_root() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("node_modules/tabby-demo");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("package.json"),
            r#"{"name":"tabby-demo","version":"1.2.3","main":"../outside.js","keywords":["tabby-plugin"]}"#,
        )
        .unwrap();

        assert_eq!(discover(temp.path()).unwrap().len(), 1);
        assert!(read_entry(temp.path(), "tabby-demo").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_plugin_manifest() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("node_modules/tabby-demo");
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            outside.path().join("package.json"),
            r#"{"name":"tabby-demo","version":"1.2.3","keywords":["tabby-plugin"]}"#,
        )
        .unwrap();
        symlink(
            outside.path().join("package.json"),
            plugin.join("package.json"),
        )
        .unwrap();

        assert!(list_installed(temp.path()).is_err());
    }
}
