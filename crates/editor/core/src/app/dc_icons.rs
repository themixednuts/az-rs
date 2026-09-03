use gpui_component::{Icon, IconName};

const MATERIAL_SYMBOLS: &[&str] = &[
    "accessibility_new",
    "account_tree",
    "add_box",
    "add_circle",
    "add",
    "ads_click",
    "align_horizontal_left",
    "animation",
    "architecture",
    "arrow_back",
    "arrow_drop_down",
    "arrow_forward",
    "arrow_right_alt",
    "arrow_upward",
    "attachment",
    "auto_awesome",
    "autorenew",
    "block",
    "bolt",
    "build",
    "category",
    "change_history",
    "check_box",
    "check_circle",
    "check",
    "chevron_left",
    "chevron_right",
    "close",
    "code",
    "comment",
    "commit",
    "conversion_path",
    "conveyor_belt",
    "crop_free",
    "crop_square",
    "dashboard",
    "data_object",
    "dataset",
    "delete",
    "deployed_code",
    "description",
    "deselect",
    "directions_run",
    "dock_to_bottom",
    "dock_to_left",
    "dock_to_right",
    "done_all",
    "download",
    "drive_file_rename_outline",
    "edit",
    "error",
    "extension",
    "favorite",
    "fiber_manual_record",
    "filter_alt",
    "filter_list",
    "filter_none",
    "first_page",
    "fit_screen",
    "fitness_center",
    "flag",
    "flip",
    "folder_open",
    "folder",
    "format_align_left",
    "gradient",
    "graphic_eq",
    "grid_4x4",
    "grid_on",
    "grid_view",
    "help",
    "history",
    "hub",
    "image",
    "info",
    "input",
    "inventory_2",
    "key",
    "keyboard",
    "lan",
    "last_page",
    "layers",
    "lens",
    "library_add",
    "link",
    "list",
    "lock_open",
    "lock",
    "logout",
    "map",
    "memory",
    "menu_book",
    "more_horiz",
    "more_vert",
    "movie",
    "note_add",
    "open_in_full",
    "open_in_new",
    "open_with",
    "output",
    "palette",
    "percent",
    "pest_control",
    "play_arrow",
    "priority_high",
    "push_pin",
    "radio_button_unchecked",
    "redeem",
    "redo",
    "remove",
    "repeat",
    "restart_alt",
    "rocket_launch",
    "route",
    "save_as",
    "save",
    "schema",
    "science",
    "search",
    "select_all",
    "settings",
    "show_chart",
    "skip_next",
    "smart_toy",
    "smartphone",
    "space_bar",
    "sports_esports",
    "stop",
    "storage",
    "subdirectory_arrow_right",
    "subject",
    "swords",
    "sync_problem",
    "sync",
    "table_chart",
    "table_rows",
    "tag",
    "title",
    "toggle_on",
    "tune",
    "undo",
    "unfold_more",
    "upload_file",
    "upload",
    "verified",
    "videocam",
    "view_column",
    "view_in_ar",
    "view_list",
    "visibility",
    "warning",
    "wb_incandescent",
    "wb_sunny",
    "whatshot",
    "widgets",
];

pub fn material_symbol_icon(name: impl AsRef<str>) -> Icon {
    let requested = name.as_ref().trim();
    if let Some(path) = symbol_asset_path(requested) {
        return Icon::empty().path(path);
    }

    let symbol = material_symbol_alias(requested);
    if let Some(path) = symbol_asset_path(symbol) {
        return Icon::empty().path(path);
    }

    if MATERIAL_SYMBOLS.contains(&symbol) {
        return Icon::empty().path(format!("icons/dc/{symbol}.svg"));
    }

    fallback_icon(requested)
}

fn material_symbol_alias(name: &str) -> &str {
    match name {
        "3d_rotation" | "transform" => "open_with",
        "all_inclusive" => "repeat",
        "android" => "smart_toy",
        "arrow_right" | "east" => "arrow_forward",
        "arrow_selector_tool" | "my_location" => "ads_click",
        "arrow_up" => "arrow_upward",
        "alt_route" => "route",
        "apps" => "widgets",
        "blur_on" | "grain" => "gradient",
        "boxes" | "package" => "box",
        "bug_report" => "warning",
        "cancel" => "close",
        "chat_bubble" => "comment",
        "check_box_outline_blank" | "crop_landscape" => "crop_square",
        "circle" | "radio_button_checked" => "fiber_manual_record",
        "cleaning_services" => "auto_awesome",
        "cloud" | "publish" => "upload",
        "construction" => "hammer",
        "content_cut" => "deselect",
        "content_paste" => "note_add",
        "desktop_windows" | "laptop_mac" => "dashboard",
        "devices" | "phone_iphone" => "smartphone",
        "directions_walk" | "person-running" => "directions_run",
        "dns" => "storage",
        "done" => "check_circle",
        "edit_document" => "edit",
        "expand_more" => "arrow_drop_down",
        "fullscreen" => "open_in_full",
        "function" | "functions" => "code",
        "insert_drive_file" => "description",
        "language" | "public" => "lan",
        "label" => "tag",
        "link_off" => "link",
        "login" => "input",
        "lightbulb" => "wb_incandescent",
        "monitoring" => "show_chart",
        "mountain" | "terrain" => "map",
        "play_circle" => "play_arrow",
        "precision_manufacturing" => "build",
        "progress_activity" => "autorenew",
        "refresh" => "restart_alt",
        "rule" => "check_box",
        "schedule" | "work_history" => "history",
        "self_improvement" => "accessibility_new",
        "short_text" => "subject",
        "sliders-horizontal" => "tune",
        "space" => "space_bar",
        "sports_martial_arts" => "fitness_center",
        "stop_circle" => "stop",
        "tab" => "title",
        "test-tube" => "science",
        "texture" => "image",
        "volume_up" => "graphic_eq",
        "workflow" => "account_tree",
        _ => name,
    }
}

fn symbol_asset_path(name: &str) -> Option<&'static str> {
    material_symbol_asset_path(name).or_else(|| lucide_symbol_asset_path(name))
}

fn material_symbol_asset_path(name: &str) -> Option<&'static str> {
    match name {
        // Toolbar tools use the established bundled icon path so every DC
        // Material Symbol has a distinct, present glyph in native GPUI.
        "3d_rotation" => Some("icons/rotate-cw.svg"),
        "arrow_selector_tool" => Some("icons/mouse-pointer-2.svg"),
        "content_copy" => Some("icons/copy.svg"),
        "pause" => Some("icons/pause.svg"),
        "speed" => Some("icons/gauge.svg"),
        "terminal" => Some("icons/square-terminal.svg"),
        "toggle_off" => Some("icons/dc/radio_button_unchecked.svg"),
        "transform" => Some("icons/move-3d.svg"),
        "visibility_off" => Some("icons/eye-off.svg"),
        _ => None,
    }
}

fn lucide_symbol_asset_path(name: &str) -> Option<&'static str> {
    match name {
        "box" => Some("icons/box.svg"),
        "file" => Some("icons/file.svg"),
        "hammer" => Some("icons/hammer.svg"),
        "loader" => Some("icons/loader.svg"),
        "loader-circle" => Some("icons/loader-circle.svg"),
        "network" => Some("icons/network.svg"),
        _ => None,
    }
}

fn fallback_icon(name: &str) -> Icon {
    match name {
        "add" | "add_box" => Icon::new(IconName::Plus),
        "arrow_back" => Icon::new(IconName::ArrowLeft),
        "arrow_drop_down" => Icon::new(IconName::ChevronDown),
        "arrow_forward" => Icon::new(IconName::ArrowRight),
        "auto_stories" | "menu_book" => Icon::new(IconName::BookOpen),
        "check" => Icon::new(IconName::Check),
        "check_circle" => Icon::new(IconName::CircleCheck),
        "close" => Icon::new(IconName::Close),
        "folder_open" => Icon::new(IconName::FolderOpen),
        "palette" => Icon::new(IconName::Palette),
        "play_arrow" => Icon::new(IconName::Play),
        "remove" => Icon::new(IconName::WindowMinimize),
        "save" => Icon::new(IconName::Save),
        "search" => Icon::new(IconName::Search),
        "settings" => Icon::new(IconName::Settings),
        "stop" => Icon::new(IconName::Stop),
        "undo" => Icon::new(IconName::Undo),
        "redo" => Icon::new(IconName::Redo),
        "view_list" => Icon::new(IconName::Menu),
        _ => Icon::empty().path("icons/file.svg"),
    }
}

#[cfg(test)]
mod tests {
    use super::{MATERIAL_SYMBOLS, material_symbol_alias, symbol_asset_path};
    use gpui::AssetSource;

    #[test]
    fn declared_dc_material_symbols_are_embedded_svg_assets() {
        let assets = gpui_component_assets::Assets;
        for symbol in MATERIAL_SYMBOLS {
            let path = format!("icons/dc/{symbol}.svg");
            assert!(
                assets.load(&path).unwrap().is_some(),
                "{path} must be embedded in gpui-component assets"
            );
        }
    }

    #[test]
    fn current_editor_material_symbols_do_not_fall_back_to_generic_file() {
        let assets = gpui_component_assets::Assets;
        let extra_symbols = [
            "3d_rotation",
            "alt_route",
            "all_inclusive",
            "apps",
            "arrow_right",
            "arrow_selector_tool",
            "blur_on",
            "boxes",
            "cancel",
            "chat_bubble",
            "check_box_outline_blank",
            "circle",
            "cleaning_services",
            "cloud",
            "construction",
            "content_copy",
            "content_cut",
            "content_paste",
            "crop_landscape",
            "desktop_windows",
            "devices",
            "directions_walk",
            "dns",
            "done",
            "drive_file_rename_outline",
            "east",
            "edit_document",
            "expand_more",
            "fullscreen",
            "function",
            "functions",
            "grain",
            "insert_drive_file",
            "label",
            "laptop_mac",
            "lightbulb",
            "link_off",
            "login",
            "monitoring",
            "mountain",
            "my_location",
            "package",
            "pause",
            "person-running",
            "play_circle",
            "precision_manufacturing",
            "progress_activity",
            "publish",
            "refresh",
            "rule",
            "schedule",
            "self_improvement",
            "short_text",
            "sliders-horizontal",
            "speed",
            "sports_martial_arts",
            "stop_circle",
            "tab",
            "terminal",
            "toggle_off",
            "terrain",
            "test-tube",
            "texture",
            "transform",
            "visibility_off",
            "volume_up",
            "work_history",
            "workflow",
        ];

        for symbol in extra_symbols {
            let asset_path = symbol_asset_path(symbol)
                .or_else(|| symbol_asset_path(material_symbol_alias(symbol)))
                .map(std::borrow::ToOwned::to_owned);
            let resolves_to_asset = asset_path.as_ref().is_some_and(|path| {
                assets
                    .load(path)
                    .unwrap_or_else(|error| panic!("failed to load {path}: {error}"))
                    .is_some()
            });
            let dc_symbol = material_symbol_alias(symbol);
            let dc_path = format!("icons/dc/{dc_symbol}.svg");
            let resolves_to_dc_svg = MATERIAL_SYMBOLS.contains(&dc_symbol)
                && assets
                    .load(&dc_path)
                    .unwrap_or_else(|error| panic!("failed to load {dc_path}: {error}"))
                    .is_some();
            assert!(
                resolves_to_asset || resolves_to_dc_svg,
                "{symbol} must resolve to a real icon asset"
            );
        }
    }
}
