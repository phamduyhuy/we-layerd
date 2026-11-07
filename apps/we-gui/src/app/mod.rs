mod detail_update;
mod init;
mod scene_update;
mod settings;
mod signal;
mod state;
mod subscription;
mod update;
mod view;
pub(crate) use state::{App, Message};
pub fn run() -> iced::Result {
    signal::install().expect("install GUI shutdown signal handler");
    iced::daemon(init::initialize, update::update, view::daemon_view)
        .title("we-layerd")
        .theme(|app: &App, _window| app.theme.clone())
        .subscription(subscription::subscription)
        .run()
}

pub fn was_interrupted() -> bool {
    signal::was_interrupted()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    #[cfg(unix)]
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use iced::{widget::pane_grid, Theme};

    use super::{settings::sync_from_ui, update, App, Message};
    use crate::{
        domain::{
            i18n::Language,
            playlist_editor::LegacyShuffleMigration,
            runtime_status::RuntimeStatus,
            settings::{ScaleModeOption, UiSettings},
            ui_state::Pane,
        },
        ui::sidebar::detail::DetailTab,
    };
    use we_core::{
        config::{LaunchSettings, ScaleMode},
        wallpaper::properties::UserPropertySchema,
    };

    #[cfg(unix)]
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_ui_settings() -> UiSettings {
        UiSettings {
            assets_path: "/opt/wallpaper_engine".to_string(),
            workshop_path: "/tmp/workshop/content/431960".to_string(),
            renderer_library_path: "/opt/libwallpaper-engine-renderer.so".to_string(),
            renderer_cache_path: "~/.cache/we-layerd/test".to_string(),
            prefer_dmabuf: false,
            allow_shm_fallback: true,
            interactive: false,
            input_mode: we_core::config::InputMode::BackgroundProxy,
            force_scene_audio_loop: true,
            fps_limit: "144".to_string(),
            show_fps: true,
            scale_mode: ScaleModeOption::Stretch,
            media_integration: true,
            audio_spectrum: false,
            audio_source: "@DEFAULT_MONITOR@".to_string(),
            rule_focused: we_core::config::RuntimeRuleAction::Keep,
            rule_maximized: we_core::config::RuntimeRuleAction::Mute,
            rule_fullscreen: we_core::config::RuntimeRuleAction::Pause,
        }
    }

    fn legacy_shuffle() -> LegacyShuffleMigration {
        LegacyShuffleMigration {
            enabled: false,
            interval_ms: 1_800_000,
            include_video: true,
            include_scene: true,
            include_web: true,
        }
    }

    #[test]
    fn sync_launch_settings_copies_workshop_path_from_ui() {
        let ui_settings = test_ui_settings();
        let mut launch_settings = LaunchSettings::default();

        sync_from_ui(&ui_settings, &mut launch_settings);

        assert_eq!(launch_settings.workshop_path, "/tmp/workshop/content/431960");
        assert_eq!(launch_settings.assets_path, "/opt/wallpaper_engine");
        assert_eq!(launch_settings.renderer_library_path, "/opt/libwallpaper-engine-renderer.so");
        assert_eq!(launch_settings.renderer_cache_path, "~/.cache/we-layerd/test");
        assert!(!launch_settings.prefer_dmabuf);
        assert!(launch_settings.allow_shm_fallback);
        assert!(!launch_settings.interactive);
        assert_eq!(launch_settings.input_mode, we_core::config::InputMode::BackgroundProxy);
        assert_eq!(launch_settings.backend, we_core::config::Backend::Auto);
        assert!(launch_settings.force_scene_audio_loop);
        assert_eq!(launch_settings.scale_mode, ScaleMode::Stretch);
    }

    fn test_app() -> App {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let temp = std::env::temp_dir().join(format!("we-gui-language-{suffix}"));
        let preferences_path = temp.join("gui.toml");
        App {
            entries: Vec::new(),
            selected_id: None,
            selected_schema: UserPropertySchema { entries: Vec::new() },
            selected_content: None,
            resolution_width: String::new(),
            resolution_height: String::new(),
            config_path: temp.join("config.toml"),
            runtime_child: None,
            viewport_width: 1280.0,
            layerd_available: true,
            launch_settings: LaunchSettings::default(),
            ui_settings: test_ui_settings(),
            show_settings: true,
            sidebar: None,
            detail_tab: DetailTab::Actions,
            property_filter: Default::default(),
            playback_paused: false,
            playback_running: false,
            search_query: String::new(),
            type_filter: None,
            filtered_entry_indices: Vec::new(),
            panes: pane_grid::State::with_configuration(pane_grid::Configuration::Pane(
                Pane::Library,
            )),
            animated_previews: HashMap::new(),
            gif_preview_desired: Default::default(),
            gif_preview_loading: Default::default(),
            gif_preview_failed: Default::default(),
            gif_preview_generation: 0,
            library_scroll_y: 0.0,
            library_viewport_width: 860.0,
            library_viewport_height: 720.0,
            library_scan: Default::default(),
            tray: None,
            main_window_id: None,
            theme: Theme::Dark,
            runtime_shutdown: true,
            outputs: Vec::new(),
            selected_outputs: Default::default(),
            running_source: None,
            language: Language::English,
            preferences_path: Some(preferences_path.clone()),
            runtime_status: RuntimeStatus::DaemonNotRunning,
            autostart_enabled: false,
            autostart_system_managed: false,
            autostart_pending: false,
            autostart_error: None,
            preferences_generation: 0,
            playlist_selected: None,
            playlist_new_name_input: String::new(),
            playlist_name_input: String::new(),
            playlist_default_duration_input: String::new(),
            playlist_entry_duration_inputs: Vec::new(),
            runtime_playlist_active: None,
            runtime_playlist_index: None,
            runtime_outputs: Default::default(),
            profile_selected: None,
            profile_new_name_input: String::new(),
            profile_name_input: String::new(),
            scene_editor: crate::domain::scene_editor::SceneEditor::new(&LaunchSettings::default()),
            legacy_shuffle: legacy_shuffle(),
            playlist_migration_completed: true,
        }
    }

    #[test]
    fn language_switch_updates_headless_state_and_queues_independent_save() {
        let mut app = test_app();

        let _task =
            update::update(&mut app, Message::LanguageSelected(Language::SimplifiedChinese));

        assert_eq!(app.language, Language::SimplifiedChinese);
        assert_eq!(app.preferences_generation, 1);
        assert!(!app.config_path.exists());

        let _newest_task = update::update(&mut app, Message::LanguageSelected(Language::English));
        let _retry_task = update::update(
            &mut app,
            Message::PreferencesSaved { generation: 1, result: Err("stale failure".to_string()) },
        );
        assert_eq!(app.language, Language::English);
        assert_eq!(app.preferences_generation, 2);
        assert_eq!(app.runtime_status, RuntimeStatus::DaemonNotRunning);

        let _completed_task = update::update(
            &mut app,
            Message::PreferencesSaved { generation: 2, result: Err("disk full".to_string()) },
        );
        assert_eq!(
            app.runtime_status,
            RuntimeStatus::PreferencesSaveFailed("disk full".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn dropping_gui_leaves_unowned_layerd_running() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock").as_nanos();
        let temp = std::env::temp_dir().join(format!("we-gui-exit-{suffix}"));
        let command = temp.join("we-layerd");
        let log = temp.join("commands.log");
        fs::create_dir_all(&temp).expect("create test directory");
        fs::write(&command, "#!/bin/sh\nprintf '%s\n' \"$*\" >> \"$WE_GUI_TEST_LOG\"\n")
            .expect("write fake we-layerd");
        let mut permissions = fs::metadata(&command).expect("fake command metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&command, permissions).expect("make fake command executable");

        let old_path = std::env::var_os("PATH");
        let old_log = std::env::var_os("WE_GUI_TEST_LOG");
        std::env::set_var("PATH", &temp);
        std::env::set_var("WE_GUI_TEST_LOG", &log);

        let app = App {
            entries: Vec::new(),
            selected_id: None,
            selected_schema: UserPropertySchema { entries: Vec::new() },
            selected_content: None,
            resolution_width: String::new(),
            resolution_height: String::new(),
            config_path: temp.join("config.toml"),
            runtime_child: None,
            viewport_width: 1280.0,
            layerd_available: true,
            launch_settings: LaunchSettings::default(),
            ui_settings: test_ui_settings(),
            show_settings: false,
            sidebar: None,
            detail_tab: DetailTab::Actions,
            property_filter: Default::default(),
            playback_paused: false,
            playback_running: true,
            search_query: String::new(),
            type_filter: None,
            filtered_entry_indices: Vec::new(),
            panes: pane_grid::State::with_configuration(pane_grid::Configuration::Pane(
                Pane::Library,
            )),
            animated_previews: HashMap::new(),
            gif_preview_desired: Default::default(),
            gif_preview_loading: Default::default(),
            gif_preview_failed: Default::default(),
            gif_preview_generation: 0,
            library_scroll_y: 0.0,
            library_viewport_width: 860.0,
            library_viewport_height: 720.0,
            library_scan: Default::default(),
            tray: None,
            main_window_id: None,
            theme: Theme::Dark,
            runtime_shutdown: false,
            outputs: Vec::new(),
            selected_outputs: Default::default(),
            running_source: None,
            language: Language::English,
            preferences_path: None,
            runtime_status: RuntimeStatus::DaemonNotRunning,
            autostart_enabled: false,
            autostart_system_managed: false,
            autostart_pending: false,
            autostart_error: None,
            preferences_generation: 0,
            playlist_selected: None,
            playlist_new_name_input: String::new(),
            playlist_name_input: String::new(),
            playlist_default_duration_input: String::new(),
            playlist_entry_duration_inputs: Vec::new(),
            runtime_playlist_active: None,
            runtime_playlist_index: None,
            runtime_outputs: Default::default(),
            profile_selected: None,
            profile_new_name_input: String::new(),
            profile_name_input: String::new(),
            scene_editor: crate::domain::scene_editor::SceneEditor::new(&LaunchSettings::default()),
            legacy_shuffle: legacy_shuffle(),
            playlist_migration_completed: true,
        };
        drop(app);

        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        match old_log {
            Some(value) => std::env::set_var("WE_GUI_TEST_LOG", value),
            None => std::env::remove_var("WE_GUI_TEST_LOG"),
        }

        let commands = fs::read_to_string(&log).unwrap_or_default();
        assert!(commands.trim().is_empty());
        fs::remove_dir_all(temp).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_restart_preserves_systemd_ownership() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock").as_nanos();
        let temp = std::env::temp_dir().join(format!("we-gui-systemd-restart-{suffix}"));
        let systemctl = temp.join("systemctl");
        let layerd = temp.join("we-layerd");
        let systemctl_log = temp.join("systemctl.log");
        let layerd_log = temp.join("layerd.log");
        fs::create_dir_all(&temp).expect("create test directory");
        fs::write(
            &systemctl,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$WE_GUI_SYSTEMCTL_LOG\"\nif [ \"$1 $2 $3\" = \"--user is-active we-layerd.service\" ]; then printf 'active\\n'; exit 0; fi\nif [ \"$1 $2 $3\" = \"--user restart we-layerd.service\" ]; then exit 0; fi\nexit 1\n",
        )
        .expect("write fake systemctl");
        fs::write(&layerd, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$WE_GUI_LAYERD_LOG\"\nexit 1\n")
            .expect("write fake we-layerd");
        for command in [&systemctl, &layerd] {
            let mut permissions =
                fs::metadata(command).expect("fake command metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(command, permissions).expect("make fake command executable");
        }

        let old_path = std::env::var_os("PATH");
        let old_systemctl_log = std::env::var_os("WE_GUI_SYSTEMCTL_LOG");
        let old_layerd_log = std::env::var_os("WE_GUI_LAYERD_LOG");
        std::env::set_var("PATH", &temp);
        std::env::set_var("WE_GUI_SYSTEMCTL_LOG", &systemctl_log);
        std::env::set_var("WE_GUI_LAYERD_LOG", &layerd_log);

        let mut owned_child = None;
        let replacement =
            crate::services::runtime::restart(&temp.join("config.toml"), &mut owned_child)
                .expect("restart systemd-managed daemon");

        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        match old_systemctl_log {
            Some(value) => std::env::set_var("WE_GUI_SYSTEMCTL_LOG", value),
            None => std::env::remove_var("WE_GUI_SYSTEMCTL_LOG"),
        }
        match old_layerd_log {
            Some(value) => std::env::set_var("WE_GUI_LAYERD_LOG", value),
            None => std::env::remove_var("WE_GUI_LAYERD_LOG"),
        }

        assert!(replacement.is_none());
        assert!(owned_child.is_none());
        let systemctl_commands = fs::read_to_string(&systemctl_log).expect("read systemctl log");
        assert_eq!(
            systemctl_commands.lines().collect::<Vec<_>>(),
            ["--user is-active we-layerd.service", "--user restart we-layerd.service"]
        );
        assert!(!layerd_log.exists());
        fs::remove_dir_all(temp).expect("remove test directory");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_restart_stops_old_daemon_before_starting_new_one() {
        let _lock = ENV_LOCK.lock().expect("environment lock");
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock").as_nanos();
        let temp = std::env::temp_dir().join(format!("we-gui-restart-{suffix}"));
        let command = temp.join("we-layerd");
        let config = temp.join("config.toml");
        let log = temp.join("commands.log");
        fs::create_dir_all(&temp).expect("create test directory");
        fs::write(
            &command,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$WE_GUI_TEST_LOG\"\nif [ \"$1 $2\" = \"ctl status\" ]; then exit 1; fi\nif [ \"$1\" = \"run\" ]; then /usr/bin/sleep 30; fi\n",
        )
        .expect("write fake we-layerd");
        let mut permissions = fs::metadata(&command).expect("fake command metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&command, permissions).expect("make fake command executable");

        let old_path = std::env::var_os("PATH");
        let old_log = std::env::var_os("WE_GUI_TEST_LOG");
        std::env::set_var("PATH", &temp);
        std::env::set_var("WE_GUI_TEST_LOG", &log);

        let mut owned_child = None;
        let mut replacement = crate::services::runtime::restart(&config, &mut owned_child)
            .expect("restart daemon")
            .expect("direct restart should return an owned child");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = replacement.kill();
        let _ = replacement.wait();

        match old_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        match old_log {
            Some(value) => std::env::set_var("WE_GUI_TEST_LOG", value),
            None => std::env::remove_var("WE_GUI_TEST_LOG"),
        }

        let commands = fs::read_to_string(&log).expect("read fake command log");
        let lines = commands.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "ctl stop");
        assert_eq!(lines[1], "ctl status");
        assert_eq!(lines[2], format!("run --config {}", config.display()));
        fs::remove_dir_all(temp).expect("remove test directory");
    }
    #[test]
    fn effective_pause_is_output_scoped_and_loaded_source_uses_resume() {
        let mut app = test_app();
        app.selected_id = Some("one".into());
        app.entries.push(we_core::wallpaper::WallpaperEntry {
            id: "one".into(),
            title: "One".into(),
            project_json: "/wallpapers/one/project.json".into(),
            ty: we_core::wallpaper::WallpaperType::Scene,
            preview: None,
            source_file: None,
        });
        let raw = r#"[orchestrator]
phase = "running"
[output_runtime.DP-1.runtime]
source = "/wallpapers/one"
[output_runtime.DP-1.presentation]
paused = true
manual_paused = false
rule_paused = true
[output_runtime.DP-2.runtime]
source = "/wallpapers/one"
[output_runtime.DP-2.presentation]
paused = false
"#;
        let _ = update::update(
            &mut app,
            Message::StatusLoaded(Ok(crate::services::runtime::DaemonStatus::Running(raw.into()))),
        );
        app.selected_outputs = ["DP-1".into()].into_iter().collect();
        assert!(app.runtime_outputs["DP-1"].rule_paused);
        assert!(app.selected_wallpaper_is_loaded());
        assert!(app.selected_wallpaper_is_paused());
        assert_eq!(app.selected_playback_control(), Some("resume"));
        app.selected_outputs = ["DP-2".into()].into_iter().collect();
        assert!(!app.selected_wallpaper_is_paused());
        assert_eq!(app.selected_playback_control(), Some("pause"));
        app.selected_id = None;
        assert_eq!(app.selected_playback_control(), None);
    }

    #[test]
    fn apply_in_output_mode_saves_global_ui_settings() {
        let mut app = test_app();
        app.outputs = vec!["DP-1".into()];
        super::settings::sync(&mut app);
        super::detail_update::persist_playback_config(&app).unwrap();
        let loaded = we_core::config::load_launch_settings(&app.config_path).unwrap();
        assert_eq!(loaded.fps_limit, 144);
        assert_eq!(loaded.renderer_library_path, app.ui_settings.renderer_library_path);
        assert_eq!(loaded.assets_path, app.ui_settings.assets_path);
        assert_eq!(loaded.workshop_path, app.ui_settings.workshop_path);
        assert!(!loaded.prefer_dmabuf);
        assert!(!loaded.interactive);
        fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn paused_detail_play_sends_resume_without_switch_or_restart() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut app = test_app();
        let temp = app.config_path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&temp).unwrap();
        let command = temp.join("we-layerd");
        let log = temp.join("commands.log");
        fs::write(&command, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$WE_GUI_TEST_LOG\"\n").unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
        let previous_path = std::env::var_os("PATH");
        let previous_log = std::env::var_os("WE_GUI_TEST_LOG");
        std::env::set_var("PATH", &temp);
        std::env::set_var("WE_GUI_TEST_LOG", &log);
        app.selected_id = Some("one".into());
        app.entries.push(we_core::wallpaper::WallpaperEntry {
            id: "one".into(),
            title: "One".into(),
            project_json: "/wallpapers/one/project.json".into(),
            ty: we_core::wallpaper::WallpaperType::Scene,
            preview: None,
            source_file: None,
        });
        app.playback_paused = true;
        app.running_source = Some("/wallpapers/one".into());
        let _ = super::detail_update::update(
            &mut app,
            crate::ui::sidebar::detail::DetailMessage::TogglePlayback,
        );
        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
        if let Some(value) = previous_log {
            std::env::set_var("WE_GUI_TEST_LOG", value);
        } else {
            std::env::remove_var("WE_GUI_TEST_LOG");
        }
        assert_eq!(fs::read_to_string(&log).unwrap().trim(), "ctl resume");
        assert!(!app.config_path.exists(), "resume must not rewrite playback config");
        fs::remove_dir_all(temp).unwrap();
    }

    fn content_fixture(app: &mut App) -> std::path::PathBuf {
        let root = app.config_path.parent().unwrap().join("workshop");
        for (id, metadata) in [
            (
                "100",
                r#"{"type":"scene","file":"scene.pkg","general":{"properties":{"amount":{"type":"slider","value":1},"image":{"type":"file","value":"base.png"}}}}"#,
            ),
            ("200", r#"{"dependency":"100","preset":{"amount":2,"image":"preset.png"}}"#),
            ("300", r#"{"dependency":"999","preset":{}}"#),
            ("400", r#"{"type":"scene","category":"asset","file":"assets.json"}"#),
            ("500", r#"{"type":"unsupported"}"#),
            ("600", r#"{"type":"web","file":"index.html"}"#),
            ("700", r#"{"dependency":"600","preset":{}}"#),
        ] {
            let item = root.join(id);
            std::fs::create_dir_all(&item).unwrap();
            std::fs::write(item.join("project.json"), metadata).unwrap();
        }
        app.entries = we_core::wallpaper::scan_workshop_wallpapers(&root).unwrap();
        root
    }

    fn select_content(app: &mut App, id: &str) {
        let index = app.entries.iter().position(|entry| entry.id == id).unwrap();
        // Do not execute the returned status-fetch task: these tests never contact a daemon.
        let _ = update::update(app, Message::SelectWallpaper(index));
    }

    #[test]
    fn property_filters_are_transient_and_clear_when_the_wallpaper_changes() {
        use crate::ui::sidebar::detail::DetailMessage;
        let mut app = test_app();
        content_fixture(&mut app);
        select_content(&mut app, "100");
        let before = std::fs::read(&app.config_path).unwrap();
        let _ = super::detail_update::update(
            &mut app,
            DetailMessage::PropertySearchChanged("amount".into()),
        );
        let _ =
            super::detail_update::update(&mut app, DetailMessage::ModifiedPropertiesToggled(true));
        let _ =
            super::detail_update::update(&mut app, DetailMessage::InternalPropertiesToggled(true));
        assert_eq!(std::fs::read(&app.config_path).unwrap(), before);
        assert_eq!(app.property_filter.query, "amount");
        assert!(app.property_filter.modified_only);
        assert!(app.property_filter.show_internal);
        let _ = super::detail_update::update(&mut app, DetailMessage::ClearPropertyFilters);
        assert!(app.property_filter.query.is_empty());
        assert!(!app.property_filter.modified_only);
        assert!(app.property_filter.show_internal, "Clear must preserve the explicit display mode");
        assert_eq!(std::fs::read(&app.config_path).unwrap(), before);
        app.detail_tab = DetailTab::UserProperties;
        select_content(&mut app, "200");
        assert!(app.property_filter.query.is_empty());
        assert!(!app.property_filter.modified_only);
        assert!(!app.property_filter.show_internal);
        assert_eq!(app.detail_tab, DetailTab::UserProperties);
        std::fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn shrinking_the_window_keeps_the_property_sidebar_usable() {
        let mut app = test_app();
        app.sidebar = Some(crate::domain::ui_state::Sidebar::Detail);
        app.panes = pane_grid::State::with_configuration(pane_grid::Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.82,
            a: Box::new(pane_grid::Configuration::Pane(Pane::Library)),
            b: Box::new(pane_grid::Configuration::Pane(Pane::Sidebar)),
        });
        let _ = update::update(&mut app, Message::WindowResized(iced::Size::new(1040.0, 680.0)));
        let pane_grid::Node::Split { ratio, .. } = app.panes.layout() else {
            panic!("missing split")
        };
        assert!((1.0 - ratio) * app.viewport_width >= 319.9);
        assert!(app.library_viewport_width >= 468.0);
    }

    #[test]
    fn resetting_one_property_keeps_other_edits_and_inherited_preset_defaults() {
        use crate::ui::sidebar::detail::DetailMessage;
        let mut app = test_app();
        content_fixture(&mut app);
        select_content(&mut app, "200");
        for (key, value) in
            [("amount", serde_json::json!(5)), ("image", serde_json::json!("custom.png"))]
        {
            let _ = super::detail_update::update(
                &mut app,
                DetailMessage::PropertyChanged { key: key.into(), value },
            );
        }
        let _ =
            super::detail_update::update(&mut app, DetailMessage::ResetProperty("amount".into()));
        let profile = &app.launch_settings.wallpapers["200"];
        assert!(!profile.user_properties.contains_key("amount"));
        assert_eq!(profile.user_properties["image"], "custom.png");
        assert_eq!(app.selected_schema.values_json(&profile.user_properties)["amount"], 2);
        let persisted = we_core::config::load_launch_settings(&app.config_path).unwrap();
        assert_eq!(persisted.wallpapers["200"].user_properties, profile.user_properties);
        std::fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn preset_gui_settings_inherit_scene_msaa_and_keep_overrides_independent() {
        let mut app = test_app();
        let root = content_fixture(&mut app);
        app.launch_settings.msaa_samples = 8;
        select_content(&mut app, "200");
        assert_eq!(app.launch_settings.wallpapers["200"].msaa_samples, 8);
        assert_eq!(app.selected_schema.values_json(&Default::default())["amount"], 2);
        assert_eq!(
            app.selected_schema.values_json(&Default::default())["image"],
            root.join("200/preset.png").display().to_string()
        );
        let _ = super::detail_update::update(
            &mut app,
            crate::ui::sidebar::detail::DetailMessage::PropertyChanged {
                key: "amount".into(),
                value: serde_json::json!(3),
            },
        );
        select_content(&mut app, "100");
        assert_eq!(app.selected_schema.values_json(&Default::default())["amount"], 1);
        assert!(app.launch_settings.wallpapers["100"].user_properties.is_empty());
        select_content(&mut app, "200");
        assert_eq!(
            app.selected_schema.values_json(&app.launch_settings.wallpapers["200"].user_properties)
                ["amount"],
            3
        );
        let _ = super::detail_update::update(
            &mut app,
            crate::ui::sidebar::detail::DetailMessage::ResetProperties,
        );
        assert_eq!(
            app.selected_schema.values_json(&app.launch_settings.wallpapers["200"].user_properties)
                ["amount"],
            2
        );
        select_content(&mut app, "700");
        assert_eq!(app.launch_settings.wallpapers["700"].msaa_samples, 1);
        std::fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn preset_playback_controls_match_resolved_base_for_single_and_multiple_outputs() {
        let mut app = test_app();
        let root = content_fixture(&mut app);
        select_content(&mut app, "200");
        app.running_source = Some(root.join("100").display().to_string());
        app.playback_running = true;
        assert_eq!(app.selected_playback_control(), Some("pause"));
        app.runtime_outputs.insert(
            "DP-1".into(),
            super::state::OutputRuntimeState {
                source: root.join("100").display().to_string(),
                paused: true,
                ..Default::default()
            },
        );
        app.selected_outputs.insert("DP-1".into());
        assert_eq!(app.selected_playback_control(), Some("resume"));
        assert_eq!(app.selected_id.as_deref(), Some("200"));
        std::fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn preset_redraw_uses_cached_source_until_an_explicit_refresh_or_apply() {
        let mut app = test_app();
        let root = content_fixture(&mut app);
        select_content(&mut app, "200");
        app.running_source = Some(root.join("100").display().to_string());
        app.playback_running = true;
        std::fs::rename(root.join("100/project.json"), root.join("100/project.offline")).unwrap();
        for _ in 0..20 {
            assert!(app.selected_wallpaper_is_loaded(), "redraw resolved the dependency again");
            let _ = super::view::view(&app);
        }
        std::fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn invalid_content_is_explained_before_apply_and_preserves_playback_and_config() {
        let _lock = ENV_LOCK.lock().unwrap();
        let mut app = test_app();
        let root = content_fixture(&mut app);
        let commands = root.join("commands");
        std::fs::create_dir_all(&commands).unwrap();
        let fake_daemon = commands.join("we-layerd");
        std::fs::write(&fake_daemon, "#!/bin/sh\nprintf 'called' > \"$0.called\"\nexit 99\n")
            .unwrap();
        std::fs::set_permissions(&fake_daemon, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Isolate command resolution even if validation regresses: never touch a real daemon.
        struct RestorePath(Option<std::ffi::OsString>);
        impl Drop for RestorePath {
            fn drop(&mut self) {
                if let Some(path) = &self.0 {
                    std::env::set_var("PATH", path);
                } else {
                    std::env::remove_var("PATH");
                }
            }
        }
        let _restore = RestorePath(std::env::var_os("PATH"));
        std::env::set_var("PATH", &commands);
        app.layerd_available = true;
        app.playback_running = true;
        app.running_source = Some("/working/wallpaper".into());
        app.runtime_playlist_active = Some("Working".into());
        app.launch_settings.playlists.active = Some("Working".into());
        let original = "# preserve the existing wallpaper config\n";
        std::fs::write(&app.config_path, original).unwrap();
        for multi_output in [false, true] {
            if multi_output {
                app.outputs = vec!["DP-1".into()];
                app.selected_outputs.insert("DP-1".into());
                app.launch_settings.outputs.insert(
                    "DP-1".into(),
                    we_core::config::OutputBinding::wallpaper("working", "/working/wallpaper"),
                );
            }
            let outputs = app.launch_settings.outputs.clone();
            for id in ["300", "400", "500"] {
                select_content(&mut app, id);
                assert!(
                    matches!(&app.runtime_status, RuntimeStatus::Raw(error) if !error.is_empty()),
                    "selection must explain unavailable content: {:?}",
                    app.runtime_status
                );
                assert!(app.selected_schema.entries.is_empty());
                let _ = update::update(&mut app, Message::PlayPressed);
                assert!(
                    matches!(&app.runtime_status, RuntimeStatus::Raw(error) if !error.is_empty())
                );
                assert!(app.runtime_shutdown, "invalid Apply must not change runtime lifecycle");
                assert!(app.playback_running);
                assert_eq!(app.running_source.as_deref(), Some("/working/wallpaper"));
                assert_eq!(app.runtime_playlist_active.as_deref(), Some("Working"));
                assert_eq!(app.launch_settings.playlists.active.as_deref(), Some("Working"));
                assert_eq!(app.launch_settings.outputs, outputs);
                assert_eq!(std::fs::read_to_string(&app.config_path).unwrap(), original);
                assert!(
                    !commands.join("we-layerd.called").exists(),
                    "invalid Apply must not issue commands"
                );
            }
        }
        // Revalidate at Apply time if a previously valid preset loses its dependency.
        select_content(&mut app, "200");
        let before = std::fs::read_to_string(&app.config_path).unwrap();
        std::fs::remove_file(root.join("100/project.json")).unwrap();
        let _ = update::update(&mut app, Message::PlayPressed);
        assert!(
            matches!(&app.runtime_status, RuntimeStatus::Raw(error) if error.contains("999") || error.contains("100"))
        );
        assert!(!commands.join("we-layerd.called").exists());
        assert_eq!(std::fs::read_to_string(&app.config_path).unwrap(), before);
        assert_eq!(app.running_source.as_deref(), Some("/working/wallpaper"));
        std::fs::remove_dir_all(app.config_path.parent().unwrap()).unwrap();
    }
}
