CREATE TABLE `workspace_ui_layouts` (
	`layout_key` TEXT PRIMARY KEY NOT NULL,
	`project_key` TEXT NOT NULL,
	`mode` TEXT NOT NULL,
	`state_version` INTEGER NOT NULL,
	`layout_json` TEXT NOT NULL,
	`updated_unix_ms` INTEGER NOT NULL
);
--> statement-breakpoint
CREATE TABLE `project_ui_states` (
	`project_key` TEXT PRIMARY KEY NOT NULL,
	`state_version` INTEGER NOT NULL,
	`last_mode` TEXT NOT NULL,
	`asset_view_mode` TEXT NOT NULL,
	`asset_folder_key` TEXT,
	`window_x` INTEGER,
	`window_y` INTEGER,
	`window_width` INTEGER,
	`window_height` INTEGER,
	`window_maximized` INTEGER NOT NULL,
	`updated_unix_ms` INTEGER NOT NULL
);