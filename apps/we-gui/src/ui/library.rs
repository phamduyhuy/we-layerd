use iced::{
    alignment::Vertical,
    widget::{button, column, container, image, responsive, row, scrollable, text, text_input},
    Background, Border, Color, ContentFit, Element, Fill, Theme,
};
use we_core::wallpaper::{WallpaperEntry, WallpaperType};

use crate::{
    app::{App, Message},
    domain::{
        i18n::{Language, Text},
        library_grid::{grid_window, GridWindow, CARD_CAPTION_HEIGHT},
        ui_state::AnimatedPreview,
    },
};

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let language = app.language;
    let entries = &app.entries;
    let filtered_entry_indices = &app.filtered_entry_indices;
    let selected_id = app.selected_id.as_ref();
    let animated_previews = &app.animated_previews;
    let playlist_selected = app.playlist_selected.as_deref();
    let scroll_y = app.library_scroll_y;
    let viewport_height = app.library_viewport_height;
    let grid = responsive(move |size| {
        if filtered_entry_indices.is_empty() {
            return container(text(language.text(Text::NoMatchingWallpapers)).size(16))
                .width(Fill)
                .align_x(iced::alignment::Horizontal::Center)
                .padding(32)
                .into();
        }
        let layout = grid_window(
            filtered_entry_indices.len(),
            size.width,
            scroll_y,
            viewport_height,
            playlist_selected.is_some(),
        );
        build_wallpaper_grid(
            filtered_entry_indices[layout.start_item..layout.end_item]
                .iter()
                .copied()
                .filter_map(|index| entries.get(index).map(|entry| (index, entry))),
            layout,
            selected_id,
            animated_previews,
            playlist_selected,
            language,
        )
    });
    let filters = row![
        filter_chip(
            language.text(Text::FilterAll),
            "library.filter.all",
            app.type_filter.is_none(),
            None
        ),
        filter_chip(
            language.text(Text::FilterWeb),
            "library.filter.web",
            app.type_filter == Some(WallpaperType::Web),
            Some(WallpaperType::Web)
        ),
        filter_chip(
            language.text(Text::FilterScene),
            "library.filter.scene",
            app.type_filter == Some(WallpaperType::Scene),
            Some(WallpaperType::Scene)
        ),
        filter_chip(
            language.text(Text::FilterVideo),
            "library.filter.video",
            app.type_filter == Some(WallpaperType::Video),
            Some(WallpaperType::Video)
        ),
    ]
    .spacing(8);
    let toolbar = row![
        column![
            text(language.text(Text::Wallpapers)).size(22),
            text(if app.filtered_entry_indices.len() == app.entries.len() {
                language.item_count(app.entries.len())
            } else {
                format!("{} / {}", app.filtered_entry_indices.len(), app.entries.len())
            })
            .size(12)
            .color(crate::ui::theme::MUTED)
        ]
        .spacing(2)
        .width(Fill),
        container(
            button(text(language.text(Text::RefreshLibrary)).size(13))
                .on_press(Message::RefreshLibrary)
                .style(top_bar_button_style),
        )
        .id("library.refresh"),
        container(
            button(text(language.text(Text::Playlists)).size(13))
                .on_press(Message::PlaylistsPressed)
                .style(top_bar_button_style),
        )
        .id("library.playlists"),
        container(
            button(text(language.text(Text::Profiles)).size(13))
                .on_press(Message::ProfilesPressed)
                .style(top_bar_button_style),
        )
        .id("library.profiles"),
        container(
            button(text(language.text(Text::SmartScenes)).size(13))
                .on_press(Message::ScenesPressed)
                .style(top_bar_button_style),
        )
        .id("library.scenes"),
        container(
            button(text(language.text(Text::Settings)).size(13))
                .on_press(Message::SettingsPressed)
                .style(top_bar_button_style),
        )
        .id("library.settings"),
    ]
    .align_y(Vertical::Center);
    container(
        column![
            toolbar,
            row![
                crate::ui::search_input::clearable(
                    text_input(language.text(Text::SearchWallpapers), &app.search_query)
                        .id("library.search")
                        .on_input(Message::SearchChanged)
                        .padding([10, 12])
                        .size(14)
                        .style(search_style)
                        .width(Fill),
                    Message::SearchChanged(String::new())
                ),
                filters,
            ]
            .spacing(12)
            .align_y(Vertical::Center),
            scrollable(
                container(grid)
                    .padding(iced::Padding { right: 12.0, ..Default::default() })
                    .width(Fill)
            )
            .id("library.scroll")
            .on_scroll(|viewport| {
                let bounds = viewport.bounds();
                Message::LibraryScrolled {
                    offset_y: viewport.absolute_offset().y,
                    viewport_width: bounds.width,
                    viewport_height: bounds.height,
                }
            })
            .width(Fill)
            .height(Fill),
        ]
        .spacing(16),
    )
    .padding(16)
    .width(Fill)
    .height(Fill)
    .style(library_style)
    .into()
}

fn build_wallpaper_grid<'a>(
    entries: impl Iterator<Item = (usize, &'a WallpaperEntry)>,
    layout: GridWindow,
    selected_id: Option<&String>,
    animated_previews: &'a std::collections::HashMap<std::path::PathBuf, AnimatedPreview>,
    playlist_selected: Option<&'a str>,
    language: Language,
) -> Element<'a, Message> {
    let spacing = 12.0;
    let mut root = column!().width(Fill);
    if layout.top_spacer > 0.0 {
        root = root.push(container(text("")).width(Fill).height(layout.top_spacer));
    }
    let entries = entries.collect::<Vec<_>>();

    for chunk in entries.chunks(layout.cols) {
        let mut row = row!().spacing(spacing);
        for (index, entry) in chunk.iter() {
            let selected = selected_id.is_some_and(|id| id == &entry.id);
            row = row.push(make_wallpaper_card(
                entry,
                *index,
                layout.card_width,
                selected,
                animated_previews,
                playlist_selected,
                language,
            ));
        }
        root = root.push(container(row).width(Fill).height(layout.row_pitch));
    }
    if layout.bottom_spacer > 0.0 {
        root = root.push(container(text("")).width(Fill).height(layout.bottom_spacer));
    }
    container(root).width(Fill).into()
}

fn make_wallpaper_card<'a>(
    entry: &'a WallpaperEntry,
    index: usize,
    card_width: f32,
    selected: bool,
    animated_previews: &'a std::collections::HashMap<std::path::PathBuf, AnimatedPreview>,
    playlist_selected: Option<&'a str>,
    language: Language,
) -> Element<'a, Message> {
    let card_height = (card_width * 9.0 / 16.0).round();
    let media: Element<'a, Message> = if let Some(path) = &entry.preview {
        let handle = animated_previews
            .get(path)
            .map(|preview| preview.frames[preview.current].handle.clone())
            .unwrap_or_else(|| image::Handle::from_path(path));
        image(handle).width(card_width).height(card_height).content_fit(ContentFit::Cover).into()
    } else {
        container(text(""))
            .width(card_width)
            .height(card_height)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgb8(18, 18, 18))),
                ..Default::default()
            })
            .into()
    };

    let caption = container(
        row![
            container(text(&entry.title).size(13).color(crate::ui::theme::TEXT))
                .width(Fill)
                .height(34)
                .clip(true),
            text(wallpaper_type_name(entry.ty, language)).size(11).color(crate::ui::theme::MUTED),
        ]
        .spacing(8)
        .align_y(Vertical::Center),
    )
    .padding([5, 10])
    .width(Fill)
    .height(CARD_CAPTION_HEIGHT);
    let frame = container(column![media, caption].spacing(0))
        .width(card_width)
        .height(card_height + CARD_CAPTION_HEIGHT)
        .clip(true)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if selected {
                crate::ui::theme::RAISED
            } else {
                crate::ui::theme::PANEL
            })),
            border: Border {
                radius: 6.0.into(),
                width: if selected { 2.0 } else { 1.0 },
                color: if selected { crate::ui::theme::ACCENT } else { Color::TRANSPARENT },
            },
            ..Default::default()
        });
    let card = button(frame)
        .padding(0)
        .on_press(Message::SelectWallpaper(index))
        .style(image_card_button_style);
    let mut content = column![card].spacing(6).width(card_width);
    if let Some(playlist_name) = playlist_selected {
        content = content.push(
            button(
                text(format!("+ {} · {}", language.text(Text::AddToPlaylist), playlist_name))
                    .size(12),
            )
            .on_press_maybe(
                entry.ty.is_playable().then_some(Message::AddWallpaperToSelectedPlaylist(index)),
            )
            .width(Fill)
            .style(top_bar_button_style),
        );
    }
    container(content).id(format!("library.wallpaper.{}", entry.id)).into()
}

fn wallpaper_type_name(ty: WallpaperType, language: Language) -> &'static str {
    match ty {
        WallpaperType::Video => language.text(Text::TypeVideo),
        WallpaperType::Scene => language.text(Text::TypeScene),
        WallpaperType::Web => language.text(Text::TypeWeb),
        WallpaperType::Asset => language.text(Text::TypeAsset),
        WallpaperType::Preset => language.text(Text::TypePreset),
        WallpaperType::Unknown => language.text(Text::TypeUnknown),
    }
}

fn image_card_button_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: Color::WHITE,
        border: Border::default(),
        shadow: iced::Shadow::default(),
        ..Default::default()
    }
}

fn library_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(crate::ui::theme::BACKGROUND)),
        ..Default::default()
    }
}

fn search_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let border = if matches!(status, text_input::Status::Focused { .. }) {
        Color::from_rgb8(174, 198, 255)
    } else {
        crate::ui::theme::BORDER
    };
    text_input::Style {
        background: Background::Color(crate::ui::theme::SURFACE),
        border: Border { radius: 8.0.into(), width: 1.0, color: border },
        icon: Color::from_rgb8(196, 199, 204),
        placeholder: Color::from_rgb8(196, 199, 204),
        value: Color::from_rgb8(230, 225, 229),
        selection: Color::from_rgb8(78, 99, 139),
    }
}

fn filter_chip<'a>(
    label: &'a str,
    id: &'static str,
    selected: bool,
    value: Option<WallpaperType>,
) -> Element<'a, Message> {
    container(
        button(text(label).size(13))
            .on_press(Message::TypeFilterSelected(value))
            .padding([9, 12])
            .style(move |_theme, status| {
                let background = if selected {
                    crate::ui::theme::RAISED
                } else if matches!(status, button::Status::Hovered) {
                    Color::from_rgb8(54, 56, 62)
                } else {
                    Color::TRANSPARENT
                };
                button::Style {
                    background: Some(Background::Color(background)),
                    text_color: if selected {
                        Color::from_rgb8(222, 231, 255)
                    } else {
                        Color::from_rgb8(201, 203, 209)
                    },
                    border: Border {
                        radius: 6.0.into(),
                        width: if selected { 0.0 } else { 1.0 },
                        color: crate::ui::theme::BORDER,
                    },
                    ..Default::default()
                }
            }),
    )
    .id(id)
    .into()
}

fn top_bar_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = if matches!(status, button::Status::Hovered) {
        Color::from_rgb8(56, 58, 63)
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::from_rgb8(220, 225, 235),
        border: Border { radius: 6.0.into(), ..Default::default() },
        ..Default::default()
    }
}
