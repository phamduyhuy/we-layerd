use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::{parse_project_json, WallpaperType};

#[derive(Debug, Clone)]
pub struct ResolvedWallpaper {
    pub project_json: PathBuf,
    pub ty: WallpaperType,
    pub user_properties: BTreeMap<String, Value>,
}

/// Resolve a Workshop preset without changing the selected item's identity or files.
pub fn resolve_wallpaper_source(project_json: &Path) -> Result<ResolvedWallpaper> {
    let project_json = if project_json.is_dir() {
        project_json.join("project.json")
    } else {
        project_json.to_path_buf()
    };
    let root = project_json
        .parent()
        .and_then(Path::parent)
        .context("wallpaper must have a project directory")?
        .canonicalize()
        .context("cannot resolve Workshop directory")?;
    resolve(&project_json, &root, &mut BTreeSet::new())
}

fn resolve(
    project: &Path,
    root: &Path,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<ResolvedWallpaper> {
    let project = project
        .canonicalize()
        .with_context(|| format!("missing wallpaper dependency: {}", project.display()))?;
    if !project.starts_with(root) {
        bail!("wallpaper dependency escapes Workshop directory");
    }
    if visited.len() >= 32 || !visited.insert(project.clone()) {
        bail!("wallpaper preset dependency cycle or chain too deep: {}", project.display());
    }
    let metadata = parse_project_json(&project)?;
    let ty = metadata.wallpaper_type();
    if ty != WallpaperType::Preset {
        if !ty.is_playable() {
            bail!("this item is {:?}, not a standalone playable wallpaper", ty);
        }
        return Ok(ResolvedWallpaper {
            project_json: project,
            ty,
            user_properties: BTreeMap::new(),
        });
    }
    let dependency = metadata.dependency.as_ref().context("preset has no dependency")?;
    let dependency = dependency
        .as_str()
        .map(str::to_owned)
        .or_else(|| dependency.as_u64().map(|value| value.to_string()))
        .context("preset dependency must be a Workshop item ID")?;
    if dependency.is_empty() || !dependency.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("preset dependency must be a numeric Workshop item ID");
    }
    let mut resolved = resolve(&root.join(&dependency).join("project.json"), root, visited)?;
    let values = metadata
        .preset
        .as_ref()
        .and_then(Value::as_object)
        .context("preset properties must be an object")?;
    // File properties are relative to the preset that owns their override, not the base scene.
    let base: Value = serde_json::from_str(&fs::read_to_string(&resolved.project_json)?)?;
    for (key, value) in values {
        let descriptor = &base["general"]["properties"][key];
        let is_path = matches!(
            descriptor["type"].as_str(),
            Some("file" | "directory" | "replacetexture" | "texture" | "scenetexture")
        );
        let mut value = value.clone();
        if is_path {
            if let Some(raw) = value.as_str().filter(|raw| !raw.is_empty()) {
                let path = Path::new(raw);
                if path.is_relative() && !raw.contains(":\\") {
                    if path.components().any(|part| matches!(part, Component::ParentDir)) {
                        bail!("preset file property '{key}' escapes its item directory");
                    }
                    value =
                        Value::String(project.parent().unwrap().join(path).display().to_string());
                }
            }
        }
        resolved.user_properties.insert(key.clone(), value);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Library(PathBuf);
    impl Library {
        fn new() -> Self {
            let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("we-preset-resolver-{suffix}"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn item(&self, id: &str, metadata: Value) -> PathBuf {
            let directory = self.0.join(id);
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("project.json");
            fs::write(&path, metadata.to_string()).unwrap();
            path
        }
    }
    impl Drop for Library {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nested_presets_merge_inner_then_outer_and_rebase_file_properties() {
        let library = Library::new();
        let base = library.item("100", serde_json::json!({"type":"scene","general":{"properties":{"image":{"type":"file","value":""}}}}));
        library.item("200", serde_json::json!({"dependency":"100","preset":{"speed":2,"color":"red","image":"files/picture.png"}}));
        let preset =
            library.item("300", serde_json::json!({"dependency":200,"preset":{"speed":3}}));
        let resolved = resolve_wallpaper_source(&preset).unwrap();
        assert_eq!(resolved.project_json, base);
        assert_eq!(resolved.ty, WallpaperType::Scene);
        assert_eq!(resolved.user_properties["speed"], 3);
        assert_eq!(resolved.user_properties["color"], "red");
        assert_eq!(
            resolved.user_properties["image"],
            library.0.join("200/files/picture.png").display().to_string()
        );
    }

    #[test]
    fn dependency_errors_are_reported_before_a_renderer_source_is_selected() {
        let library = Library::new();
        let first = library.item("100", serde_json::json!({"dependency":"200","preset":{}}));
        assert!(resolve_wallpaper_source(&first)
            .unwrap_err()
            .to_string()
            .contains("missing wallpaper dependency"));
        library.item("200", serde_json::json!({"dependency":"100","preset":{}}));
        assert!(resolve_wallpaper_source(&first).unwrap_err().to_string().contains("cycle"));
        let escape = library.item("300", serde_json::json!({"dependency":"../other","preset":{}}));
        assert!(resolve_wallpaper_source(&escape)
            .unwrap_err()
            .to_string()
            .contains("numeric Workshop"));
    }

    #[test]
    fn preset_file_paths_do_not_escape_the_selected_item() {
        let library = Library::new();
        library.item(
            "100",
            serde_json::json!({"type":"scene","general":{"properties":{"image":{"type":"file"}}}}),
        );
        let preset = library.item(
            "200",
            serde_json::json!({"dependency":"100","preset":{"image":"../../elsewhere"}}),
        );
        assert!(resolve_wallpaper_source(&preset).unwrap_err().to_string().contains("escapes"));
    }

    #[test]
    fn asset_rejection_does_not_guess_a_scene_backend() {
        let library = Library::new();
        let asset =
            library.item("100", serde_json::json!({"category":"Asset","file":"assets.json"}));
        assert!(resolve_wallpaper_source(&asset)
            .unwrap_err()
            .to_string()
            .contains("not a standalone"));
    }
}
