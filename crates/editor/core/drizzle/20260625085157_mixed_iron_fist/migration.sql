CREATE TABLE `db_info` (
	`row_id` INTEGER PRIMARY KEY,
	`version` INTEGER NOT NULL
);

--> statement-breakpoint
CREATE TABLE `project_manager_preferences` (
	`row_id` INTEGER PRIMARY KEY,
	`recent_layout` TEXT NOT NULL,
	`recent_sort` TEXT NOT NULL,
	`pinned_project_ids_json` TEXT NOT NULL,
	`updated_unix_ms` INTEGER NOT NULL
);
