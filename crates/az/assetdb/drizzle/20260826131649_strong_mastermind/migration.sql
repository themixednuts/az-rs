CREATE TABLE `roots` (
	`root_id` INTEGER PRIMARY KEY,
	`key` TEXT NOT NULL UNIQUE
) STRICT;
--> statement-breakpoint
CREATE TABLE `workspaces` (
	`workspace_id` INTEGER PRIMARY KEY,
	`project` TEXT NOT NULL,
	`root` TEXT NOT NULL,
	`branch` TEXT NOT NULL,
	`builders` BLOB,
	`created` INTEGER NOT NULL,
	`updated` INTEGER NOT NULL
) STRICT;
--> statement-breakpoint
CREATE TABLE `workspace_roots` (
	`workspace_root_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`root_pk` INTEGER NOT NULL,
	`owner` TEXT NOT NULL,
	`path` TEXT NOT NULL,
	`exclusions` TEXT NOT NULL,
	CONSTRAINT `fk_workspace_roots_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_workspace_roots_root_pk_roots_root_id_fk` FOREIGN KEY (`root_pk`) REFERENCES `roots`(`root_id`) ON DELETE RESTRICT
) STRICT;
--> statement-breakpoint
CREATE TABLE `assets` (
	`asset_id` INTEGER PRIMARY KEY,
	`guid` BLOB NOT NULL UNIQUE,
	`deleted` INTEGER DEFAULT 0 NOT NULL,
	`created` INTEGER NOT NULL,
	`updated` INTEGER NOT NULL
) STRICT;
--> statement-breakpoint
CREATE TABLE `paths` (
	`path_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`asset_pk` INTEGER NOT NULL,
	`root_pk` INTEGER NOT NULL,
	`path` TEXT NOT NULL,
	`digest` BLOB NOT NULL,
	`session` TEXT,
	`from` INTEGER NOT NULL,
	`to` INTEGER,
	CONSTRAINT `fk_paths_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_paths_asset_pk_assets_asset_id_fk` FOREIGN KEY (`asset_pk`) REFERENCES `assets`(`asset_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_paths_root_pk_roots_root_id_fk` FOREIGN KEY (`root_pk`) REFERENCES `roots`(`root_id`) ON DELETE RESTRICT
) STRICT;
--> statement-breakpoint
CREATE TABLE `entries` (
	`entry_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`asset_pk` INTEGER NOT NULL,
	`root_pk` INTEGER NOT NULL,
	`path` TEXT NOT NULL,
	`schema` TEXT,
	`digest` BLOB NOT NULL,
	`diff` INTEGER NOT NULL,
	`diagnostics` INTEGER DEFAULT 0 NOT NULL,
	`updated` INTEGER NOT NULL,
	`src_bytes` INTEGER NOT NULL,
	`src_mtime` INTEGER NOT NULL,
	`meta_bytes` INTEGER NOT NULL,
	`meta_mtime` INTEGER NOT NULL,
	`observed` INTEGER NOT NULL,
	CONSTRAINT `fk_entries_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_entries_asset_pk_assets_asset_id_fk` FOREIGN KEY (`asset_pk`) REFERENCES `assets`(`asset_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_entries_root_pk_roots_root_id_fk` FOREIGN KEY (`root_pk`) REFERENCES `roots`(`root_id`) ON DELETE RESTRICT
) STRICT;
--> statement-breakpoint
CREATE TABLE `payloads` (
	`payload_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`root_pk` INTEGER NOT NULL,
	`path` TEXT NOT NULL,
	`document` TEXT NOT NULL,
	`schema` TEXT NOT NULL,
	`encoding` INTEGER NOT NULL,
	`revision` INTEGER NOT NULL,
	`saved` INTEGER,
	`digest` BLOB NOT NULL,
	`bytes` INTEGER NOT NULL,
	`payload` BLOB NOT NULL,
	`checkpoint` BLOB,
	`session` TEXT,
	`project` TEXT NOT NULL,
	`deleted` INTEGER DEFAULT 0 NOT NULL,
	`created` INTEGER NOT NULL,
	`updated` INTEGER NOT NULL,
	CONSTRAINT `fk_payloads_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_payloads_root_pk_roots_root_id_fk` FOREIGN KEY (`root_pk`) REFERENCES `roots`(`root_id`) ON DELETE RESTRICT
) STRICT;
--> statement-breakpoint
CREATE TABLE `builders` (
	`builder_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`guid` BLOB NOT NULL,
	`name` TEXT NOT NULL,
	`version` INTEGER NOT NULL,
	`digest` BLOB NOT NULL,
	CONSTRAINT `fk_builders_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE
) STRICT;
--> statement-breakpoint
CREATE TABLE `jobs` (
	`job_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`asset_pk` INTEGER NOT NULL,
	`kind` INTEGER NOT NULL,
	`builder` BLOB,
	`key` TEXT NOT NULL,
	`platform` TEXT NOT NULL,
	`status` INTEGER NOT NULL,
	`ready` INTEGER DEFAULT 0 NOT NULL,
	`attempts` INTEGER DEFAULT 0 NOT NULL,
	CONSTRAINT `fk_jobs_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_jobs_asset_pk_assets_asset_id_fk` FOREIGN KEY (`asset_pk`) REFERENCES `assets`(`asset_id`) ON DELETE CASCADE,
	CONSTRAINT `jobs_check` CHECK(((kind = 0 AND builder IS NULL) OR (kind = 1 AND builder IS NOT NULL)) AND status IN (0, 1, 2, 3))
) STRICT;
--> statement-breakpoint
CREATE TABLE `attempts` (
	`attempt_id` INTEGER PRIMARY KEY,
	`job_pk` INTEGER NOT NULL,
	`ordinal` INTEGER NOT NULL,
	`status` INTEGER NOT NULL,
	`owner` TEXT,
	`expires` INTEGER,
	`staging` TEXT,
	`finished` INTEGER,
	`errors` INTEGER DEFAULT 0 NOT NULL,
	`warnings` INTEGER DEFAULT 0 NOT NULL,
	CONSTRAINT `fk_attempts_job_pk_jobs_job_id_fk` FOREIGN KEY (`job_pk`) REFERENCES `jobs`(`job_id`) ON DELETE CASCADE,
	CONSTRAINT `attempts_check` CHECK(status IN (1, 2, 3, 4))
) STRICT;
--> statement-breakpoint
CREATE TABLE `products` (
	`product_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`asset_pk` INTEGER NOT NULL,
	`platform` TEXT NOT NULL,
	`sub_id` INTEGER NOT NULL,
	`job_pk` INTEGER NOT NULL,
	`path` TEXT NOT NULL,
	`kind` BLOB NOT NULL,
	`format` TEXT NOT NULL,
	`version` INTEGER NOT NULL,
	`aliases` TEXT NOT NULL,
	`registration` INTEGER NOT NULL,
	`digest` BLOB NOT NULL,
	`bytes` INTEGER NOT NULL,
	CONSTRAINT `fk_products_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_products_asset_pk_assets_asset_id_fk` FOREIGN KEY (`asset_pk`) REFERENCES `assets`(`asset_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_products_job_pk_jobs_job_id_fk` FOREIGN KEY (`job_pk`) REFERENCES `jobs`(`job_id`) ON DELETE CASCADE
) STRICT;
--> statement-breakpoint
CREATE TABLE `product_edges` (
	`product_edge_id` INTEGER PRIMARY KEY,
	`product_pk` INTEGER NOT NULL,
	`guid` BLOB NOT NULL,
	`sub_id` INTEGER NOT NULL,
	`flags` INTEGER NOT NULL,
	CONSTRAINT `fk_product_edges_product_pk_products_product_id_fk` FOREIGN KEY (`product_pk`) REFERENCES `products`(`product_id`) ON DELETE CASCADE
) STRICT;
--> statement-breakpoint
CREATE TABLE `job_edges` (
	`job_edge_id` INTEGER PRIMARY KEY,
	`job_pk` INTEGER NOT NULL,
	`asset_pk` INTEGER,
	`target` BLOB NOT NULL,
	`key` TEXT NOT NULL,
	`platform` TEXT NOT NULL,
	`coupling` INTEGER NOT NULL,
	CONSTRAINT `fk_job_edges_job_pk_jobs_job_id_fk` FOREIGN KEY (`job_pk`) REFERENCES `jobs`(`job_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_job_edges_asset_pk_assets_asset_id_fk` FOREIGN KEY (`asset_pk`) REFERENCES `assets`(`asset_id`) ON DELETE RESTRICT
) STRICT;
--> statement-breakpoint
CREATE TABLE `source_edges` (
	`source_edge_id` INTEGER PRIMARY KEY,
	`workspace_pk` INTEGER NOT NULL,
	`builder` BLOB NOT NULL,
	`asset_pk` INTEGER NOT NULL,
	`depends_pk` INTEGER,
	`target` BLOB NOT NULL,
	`relation` INTEGER NOT NULL,
	CONSTRAINT `fk_source_edges_workspace_pk_workspaces_workspace_id_fk` FOREIGN KEY (`workspace_pk`) REFERENCES `workspaces`(`workspace_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_source_edges_asset_pk_assets_asset_id_fk` FOREIGN KEY (`asset_pk`) REFERENCES `assets`(`asset_id`) ON DELETE CASCADE,
	CONSTRAINT `fk_source_edges_depends_pk_assets_asset_id_fk` FOREIGN KEY (`depends_pk`) REFERENCES `assets`(`asset_id`) ON DELETE RESTRICT
) STRICT;
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_workspaces_key` ON `workspaces`(`project`, `root`, `branch`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_workspace_roots_workspace_root` ON `workspace_roots`(`workspace_pk`, `root_pk`);
--> statement-breakpoint
CREATE INDEX `idx_workspace_roots_root` ON `workspace_roots`(`root_pk`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_paths_asset_open` ON `paths`(`workspace_pk`, `asset_pk`) WHERE "to" IS NULL;
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_paths_locator_open` ON `paths`(`workspace_pk`, `root_pk`, `path`) WHERE "to" IS NULL;
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_entries_path` ON `entries`(`workspace_pk`, `root_pk`, `path`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_entries_asset` ON `entries`(`workspace_pk`, `asset_pk`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_payloads_document` ON `payloads`(`workspace_pk`, `document`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_payloads_path` ON `payloads`(`workspace_pk`, `root_pk`, `path`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_builders_workspace_guid` ON `builders`(`workspace_pk`, `guid`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_jobs_plan` ON `jobs`(`workspace_pk`, `asset_pk`, `kind`, `key`, `platform`) WHERE kind = 0 AND builder IS NULL;
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_jobs_build` ON `jobs`(`workspace_pk`, `asset_pk`, `kind`, `builder`, `key`, `platform`) WHERE kind = 1 AND builder IS NOT NULL;
--> statement-breakpoint
CREATE INDEX `idx_jobs_ready` ON `jobs`(`workspace_pk`, `kind`, `status`, `ready`, `job_id`);
--> statement-breakpoint
CREATE INDEX `idx_jobs_target` ON `jobs`(`workspace_pk`, `asset_pk`, `key`, `platform`, `status`);
--> statement-breakpoint
CREATE INDEX `idx_jobs_processing` ON `jobs`(`workspace_pk`, `platform`, `status`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_attempts_job_ordinal` ON `attempts`(`job_pk`, `ordinal`);
--> statement-breakpoint
CREATE INDEX `idx_attempts_lease` ON `attempts`(`status`, `expires`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_products_identity` ON `products`(`workspace_pk`, `platform`, `asset_pk`, `sub_id`);
--> statement-breakpoint
CREATE INDEX `idx_products_job` ON `products`(`job_pk`);
--> statement-breakpoint
CREATE INDEX `idx_product_edges_product` ON `product_edges`(`product_pk`);
--> statement-breakpoint
CREATE INDEX `idx_job_edges_job` ON `job_edges`(`job_pk`);
--> statement-breakpoint
CREATE INDEX `idx_job_edges_target` ON `job_edges`(`asset_pk`, `key`, `platform`);
--> statement-breakpoint
CREATE INDEX `idx_job_edges_authored_target` ON `job_edges`(`target`);
--> statement-breakpoint
CREATE UNIQUE INDEX `idx_source_edges_builder_asset_target` ON `source_edges`(`workspace_pk`, `builder`, `asset_pk`, `target`, `relation`);
--> statement-breakpoint
CREATE INDEX `idx_source_edges_target` ON `source_edges`(`workspace_pk`, `depends_pk`);
--> statement-breakpoint
CREATE INDEX `idx_source_edges_authored_target` ON `source_edges`(`workspace_pk`, `target`);
--> statement-breakpoint
CREATE VIEW `catalog` AS SELECT p.product_id AS product_pk, p.workspace_pk, p.platform, a.guid, p.sub_id, p.path, p.kind, p.format, p.version, p.aliases, p.registration, p.digest, p.bytes, e.path AS source, e.schema, p.job_pk, j.builder, j.key AS job_key FROM products AS p INNER JOIN assets AS a ON p.asset_pk = a.asset_id INNER JOIN jobs AS j ON p.job_pk = j.job_id INNER JOIN entries AS e ON e.workspace_pk = p.workspace_pk AND e.asset_pk = p.asset_pk WHERE e.diff IN (0, 1, 2) AND j.status IN (0, 1, 2) ORDER BY p.path, a.guid, p.sub_id;