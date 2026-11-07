use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

pub mod properties;
pub mod settings;
mod source;
pub use source::{resolve_wallpaper_source, ResolvedWallpaper};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperType {
    Video,
    Scene,
    Web,
    Asset,
    Preset,
    Unknown,
}

impl WallpaperType {
    pub fn is_playable(self) -> bool {
        matches!(self, Self::Video | Self::Scene | Self::Web | Self::Preset)
    }
}

#[derive(Debug, Clone)]
pub struct WallpaperEntry {
    pub id: String,
    pub project_json: PathBuf,
    pub title: String,
    pub ty: WallpaperType,
    pub preview: Option<PathBuf>,
    pub source_file: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ProjectJson {
    #[serde(default)]
    title: String,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    dependency: Option<serde_json::Value>,
    #[serde(default)]
    preset: Option<serde_json::Value>,
}

impl ProjectJson {
    fn wallpaper_type(&self) -> WallpaperType {
        if self.category.eq_ignore_ascii_case("asset") || self.file == "assets.json" {
            WallpaperType::Asset
        } else if self.dependency.is_some() && self.preset.is_some() {
            WallpaperType::Preset
        } else {
            parse_type(&self.r#type)
        }
    }
}

pub fn scan_workshop_wallpapers(workshop_app_root: &Path) -> Result<Vec<WallpaperEntry>> {
    Ok(scan_workshop_wallpapers_with_warnings(workshop_app_root)?.entries)
}

#[derive(Debug, Default)]
pub struct WorkshopScan {
    pub entries: Vec<WallpaperEntry>,
    pub warnings: Vec<String>,
}

pub fn scan_workshop_wallpapers_with_warnings(workshop_app_root: &Path) -> Result<WorkshopScan> {
    let mut result = Vec::new();
    let mut warnings = Vec::new();

    for dir in fs::read_dir(workshop_app_root)
        .with_context(|| format!("failed to read {}", workshop_app_root.display()))?
    {
        let dir = match dir {
            Ok(dir) => dir,
            Err(error) => {
                warnings.push(error.to_string());
                continue;
            }
        };
        let path = dir.path();
        if !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let id = name.to_string();
        let project_json = path.join("project.json");
        if !project_json.is_file() {
            continue;
        }

        let meta = match parse_project_json(&project_json) {
            Ok(meta) => meta,
            Err(error) => {
                warnings.push(format!("{error:#}"));
                continue;
            }
        };
        let ty = meta.wallpaper_type();
        let source_file =
            if meta.file.trim().is_empty() { None } else { Some(path.join(meta.file)) };
        let preview = detect_preview_image(&path);
        result.push(WallpaperEntry {
            id,
            project_json,
            title: if meta.title.trim().is_empty() { "Untitled".to_string() } else { meta.title },
            ty,
            preview,
            source_file,
        });
    }

    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(WorkshopScan { entries: result, warnings })
}

fn parse_project_json(path: &Path) -> Result<ProjectJson> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let parsed = serde_json::from_str::<ProjectJson>(&raw)
        .with_context(|| format!("invalid JSON: {}", path.display()))?;
    Ok(parsed)
}

pub fn wallpaper_type_from_source(source: &Path) -> Result<WallpaperType> {
    Ok(parse_project_json(&source.join("project.json"))?.wallpaper_type())
}

fn parse_type(value: &str) -> WallpaperType {
    match value.to_ascii_lowercase().as_str() {
        "video" => WallpaperType::Video,
        "scene" => WallpaperType::Scene,
        "web" => WallpaperType::Web,
        _ => WallpaperType::Unknown,
    }
}

fn detect_preview_image(wallpaper_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        "preview.gif",
        "preview.jpg",
        "preview.jpeg",
        "preview.png",
        "thumbnail.jpg",
        "thumbnail.png",
    ];

    for name in candidates {
        let path = wallpaper_dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::detect_preview_image;

    #[test]
    fn gif_preview_takes_priority_over_static_preview() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("we-layerd-preview-{suffix}"));
        fs::create_dir_all(&dir).expect("temporary wallpaper directory must be created");
        fs::write(dir.join("preview.jpg"), []).expect("static preview fixture must be created");
        fs::write(dir.join("preview.gif"), []).expect("gif preview fixture must be created");

        assert_eq!(detect_preview_image(&dir), Some(dir.join("preview.gif")));

        fs::remove_dir_all(dir).expect("temporary wallpaper directory must be removed");
    }
    #[test]
    fn corrupt_item_does_not_hide_valid_wallpapers() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("we-layerd-scan-{suffix}"));
        fs::create_dir_all(dir.join("good")).unwrap();
        fs::create_dir_all(dir.join("bad")).unwrap();
        fs::write(dir.join("good/project.json"), r#"{"title":"Good","type":"scene"}"#).unwrap();
        fs::write(dir.join("bad/project.json"), "{").unwrap();
        let scan = super::scan_workshop_wallpapers_with_warnings(&dir).unwrap();
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].id, "good");
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.warnings[0].contains("bad/project.json"));
        assert!(super::scan_workshop_wallpapers(&dir.join("missing")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preset_schema_inherits_base_and_keeps_selected_overrides() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("we-preset-{suffix}"));
        fs::create_dir_all(root.join("100")).unwrap();
        fs::create_dir_all(root.join("200")).unwrap();
        fs::write(root.join("100/project.json"), r#"{"type":"scene","file":"scene.json","general":{"properties":{"speed":{"type":"slider","value":1},"color":{"type":"color","value":"1 1 1"}}}}"#).unwrap();
        fs::write(
            root.join("200/project.json"),
            r#"{"title":"Preset","dependency":"100","preset":{"speed":2}}"#,
        )
        .unwrap();
        let schema = super::properties::UserPropertySchema::from_project_file(
            &root.join("200/project.json"),
        )
        .unwrap();
        assert_eq!(
            schema.values_json(&Default::default()),
            serde_json::json!({"speed":2,"color":"1 1 1"})
        );
        let config = crate::config::build_config_for_wallpaper(
            &Default::default(),
            "200",
            &root.join("200/project.json"),
        )
        .unwrap();
        assert_eq!(std::path::Path::new(&config.renderer.source), root.join("100"));
        let options: serde_json::Value =
            serde_json::from_str(config.renderer.options_json.as_deref().unwrap()).unwrap();
        assert_eq!(options["scene"]["userProperties"]["speed"], 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_distinguishes_assets_and_presets_from_unknown_content() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("we-content-kinds-{suffix}"));
        for (id, raw) in [
            ("100", r#"{"category":"Asset","file":"assets.json"}"#),
            ("200", r#"{"dependency":"300","preset":{}}"#),
        ] {
            fs::create_dir_all(root.join(id)).unwrap();
            fs::write(root.join(id).join("project.json"), raw).unwrap();
        }
        let entries = super::scan_workshop_wallpapers(&root).unwrap();
        assert_eq!(format!("{:?}", entries[0].ty), "Asset");
        assert_eq!(format!("{:?}", entries[1].ty), "Preset");
        fs::remove_dir_all(root).unwrap();
    }
}
