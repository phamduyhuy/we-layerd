use std::path::Path;

use we_core::{
    config::{
        save_force_scene_audio_loop, save_integrations_and_rules,
        save_playlists_profiles_and_outputs, save_profiles_outputs_and_scenes, save_scene_settings,
        save_wallpaper_launch_settings, save_wallpapers, AdaptiveConfig, IntegrationsConfig,
        LaunchSettings, OutputBinding, RuntimeRulesConfig, SceneConfig,
    },
    playlist::PlaylistConfig,
    profile::ProfileConfig,
    wallpaper::WallpaperEntry,
};

pub(crate) fn persist_selected(
    config_path: &Path,
    launch_settings: &LaunchSettings,
    entry: &WallpaperEntry,
) -> Result<(), String> {
    save_wallpaper_launch_settings(config_path, launch_settings, &entry.id, &entry.project_json)
        .map_err(|error| error.to_string())
}

pub(crate) fn persist_force_scene_audio_loop(
    config_path: &Path,
    enabled: bool,
) -> Result<(), String> {
    save_force_scene_audio_loop(config_path, enabled).map_err(|error| error.to_string())
}

pub(crate) fn persist_integrations_and_rules(
    config_path: &Path,
    integrations: &IntegrationsConfig,
    rules: &RuntimeRulesConfig,
) -> Result<(), String> {
    save_integrations_and_rules(config_path, integrations, rules).map_err(|error| error.to_string())
}

pub(crate) fn persist_scene_settings(
    config_path: &Path,
    scenes: &SceneConfig,
    adaptive: &AdaptiveConfig,
) -> Result<(), String> {
    save_scene_settings(config_path, scenes, adaptive).map_err(|error| error.to_string())
}

pub(crate) fn persist_wallpapers(
    config_path: &Path,
    wallpapers: &std::collections::BTreeMap<
        String,
        we_core::wallpaper::settings::WallpaperSettings,
    >,
) -> Result<(), String> {
    save_wallpapers(config_path, wallpapers).map_err(|error| error.to_string())
}

pub(crate) fn persist_profiles_outputs_and_scenes(
    config_path: &Path,
    profiles: &ProfileConfig,
    outputs: &std::collections::BTreeMap<String, OutputBinding>,
    scenes: &SceneConfig,
) -> Result<(), String> {
    save_profiles_outputs_and_scenes(config_path, profiles, outputs, scenes)
        .map_err(|error| error.to_string())
}

pub(crate) fn persist_playlists_profiles_and_outputs(
    config_path: &Path,
    playlists: &PlaylistConfig,
    profiles: &ProfileConfig,
    outputs: &std::collections::BTreeMap<String, OutputBinding>,
) -> Result<(), String> {
    save_playlists_profiles_and_outputs(config_path, playlists, profiles, outputs)
        .map_err(|error| error.to_string())
}

pub(crate) fn persist_output_launch_settings(
    path: &Path,
    settings: &LaunchSettings,
) -> Result<(), String> {
    we_core::config::save_output_launch_settings(path, settings).map_err(|error| error.to_string())
}
