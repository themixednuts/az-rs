//! Explicit Drizzle migration generation for the Azoth asset database.

use std::path::{Path, PathBuf};

use clap::Parser;
use drizzle_migrations::build::{Config, Output, run};
use drizzle_types::Dialect;

const SQLITE_REBUILD_DATA_PLAN: &str = "migration-plans/sqlite-rebuild-data-v1.json";

#[derive(Debug, Parser)]
#[command(about = "Generate one checked-in asset database migration")]
struct Args {
    /// Asset database crate directory containing `src/schema.rs` and `drizzle/`.
    #[arg(long, default_value_os_t = default_assetdb_crate_dir())]
    assetdb_crate: PathBuf,

    /// Replace the complete migration chain with one generated empty-database
    /// baseline. This never edits generated SQL and does not reset a live DB.
    #[arg(long)]
    baseline: bool,

    /// Required acknowledgement for --baseline.
    #[arg(long, requires = "baseline")]
    confirm_baseline: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.baseline {
        if args.confirm_baseline.as_deref() != Some("assetdb-wave-5") {
            return Err("--baseline requires --confirm-baseline assetdb-wave-5".into());
        }
        let output = replace_with_generated_baseline(&args.assetdb_crate)?;
        print_output(output);
        return Ok(());
    }

    print_output(run(&migration_config(&args.assetdb_crate))?);
    Ok(())
}

fn print_output(output: Output) {
    match output {
        Output::NoChanges => println!("asset database schema matches the migration chain"),
        Output::Generated {
            tag,
            path,
            statement_count,
        } => {
            println!(
                "generated migration {tag} ({statement_count} statements) at {}",
                path.display()
            );
        }
    }
}

fn default_assetdb_crate_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("asset-processor-host has an az crate parent")
        .join("assetdb")
}

fn migration_config(assetdb_crate: &Path) -> Config {
    Config::new(Dialect::SQLite)
        .file(assetdb_crate.join("src/schema.rs"))
        .out(assetdb_crate.join("drizzle"))
        .sqlite_rebuild_data_plan_file(assetdb_crate.join(SQLITE_REBUILD_DATA_PLAN))
}

fn replace_with_generated_baseline(
    assetdb_crate: &Path,
) -> Result<Output, Box<dyn std::error::Error>> {
    let requested = assetdb_crate.canonicalize()?;
    let expected = default_assetdb_crate_dir().canonicalize()?;
    if requested != expected {
        return Err(format!(
            "baseline target must be the canonical AssetDB crate `{}`; received `{}`",
            expected.display(),
            requested.display()
        )
        .into());
    }
    replace_with_generated_baseline_at_exact_target(&requested)
}

fn replace_with_generated_baseline_at_exact_target(
    assetdb_crate: &Path,
) -> Result<Output, Box<dyn std::error::Error>> {
    let migrations = assetdb_crate.join("drizzle");
    let parent = migrations
        .parent()
        .ok_or("asset database migration directory has no parent")?;
    if migrations.file_name().and_then(|name| name.to_str()) != Some("drizzle") {
        return Err("baseline target must be the assetdb drizzle directory".into());
    }

    let suffix = std::process::id();
    let staging = parent.join(format!(".drizzle-baseline-{suffix}"));
    let backup = parent.join(format!(".drizzle-backup-{suffix}"));
    if staging.exists() || backup.exists() {
        return Err("stale AssetDB baseline staging directory exists".into());
    }
    std::fs::create_dir(&staging)?;

    let config = Config::new(Dialect::SQLite)
        .file(assetdb_crate.join("src/schema.rs"))
        .out(&staging);
    let output = match run(&config) {
        Ok(Output::Generated {
            tag,
            path,
            statement_count,
        }) => {
            if let Err(error) = validate_fail_loud_generated_sql(&path) {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error);
            }
            Output::Generated {
                tag,
                path,
                statement_count,
            }
        }
        Ok(Output::NoChanges) => {
            std::fs::remove_dir_all(&staging)?;
            return Err("empty baseline generation produced no migration".into());
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    };

    install_generated_baseline(&migrations, &staging, &backup, |from, to| {
        std::fs::rename(from, to)
    })?;
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }

    Ok(match output {
        Output::Generated {
            tag,
            path,
            statement_count,
        } => Output::Generated {
            tag,
            path: migrations.join(path.file_name().ok_or("generated migration has no tag")?),
            statement_count,
        },
        Output::NoChanges => unreachable!(),
    })
}

fn install_generated_baseline(
    migrations: &Path,
    staging: &Path,
    backup: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let replaced_existing_chain = migrations.exists();
    if replaced_existing_chain {
        rename(migrations, backup)?;
    }
    if let Err(install) = rename(staging, migrations) {
        if !replaced_existing_chain {
            return Err(format!(
                "failed to install generated AssetDB baseline from `{}` to `{}`: {install}",
                staging.display(),
                migrations.display(),
            )
            .into());
        }
        return match rename(backup, migrations) {
            Ok(()) => Err(format!(
                "failed to install generated AssetDB baseline from `{}` to `{}`: {install}; restored the previous chain",
                staging.display(),
                migrations.display(),
            )
            .into()),
            Err(rollback) => Err(format!(
                "failed to install generated AssetDB baseline from `{}` to `{}`: {install}; rollback also failed: {rollback}; previous chain remains recoverable at `{}`",
                staging.display(),
                migrations.display(),
                backup.display(),
            )
            .into()),
        };
    }
    Ok(())
}

fn validate_fail_loud_generated_sql(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let sql = std::fs::read_to_string(path.join("migration.sql"))?;
    let uppercase = sql.to_ascii_uppercase();
    if uppercase.contains("IF NOT EXISTS") || uppercase.contains("IF EXISTS") {
        return Err("generated baseline contains conditional DDL".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_command_refuses_to_generate_without_its_checked_in_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assetdb = dir.path();
        std::fs::create_dir(assetdb.join("src")).expect("create source directory");
        std::fs::write(
            assetdb.join("src/schema.rs"),
            "#[SQLiteTable] pub struct Records { pub id: i64 }",
        )
        .expect("write schema");

        let error = run(&migration_config(assetdb)).expect_err("missing plan must fail");
        assert!(matches!(
            error,
            drizzle_migrations::build::BuildError::ReadSqliteRebuildDataPlan { .. }
        ));
    }

    #[test]
    fn baseline_replaces_the_chain_with_generated_fail_loud_ddl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assetdb = dir.path();
        std::fs::create_dir_all(assetdb.join("src")).expect("create source directory");
        std::fs::create_dir_all(assetdb.join("drizzle/20000101000000_old"))
            .expect("create old migration");
        std::fs::write(
            assetdb.join("drizzle/20000101000000_old/migration.sql"),
            "CREATE TABLE old(id INTEGER PRIMARY KEY);",
        )
        .expect("write old migration");
        std::fs::write(
            assetdb.join("src/schema.rs"),
            "#[SQLiteTable] pub struct Records { #[column(primary)] pub id: i64 }",
        )
        .expect("write schema");

        let Output::Generated { path, .. } =
            replace_with_generated_baseline_at_exact_target(assetdb).expect("replace baseline")
        else {
            panic!("baseline must generate a migration");
        };
        assert!(!assetdb.join("drizzle/20000101000000_old").exists());
        assert!(path.join("snapshot.json").is_file());
        validate_fail_loud_generated_sql(&path).expect("fail-loud DDL");
        let entries = std::fs::read_dir(assetdb.join("drizzle"))
            .expect("read baseline")
            .count();
        assert_eq!(entries, 1);
    }

    #[test]
    fn baseline_cli_rejects_a_different_assetdb_shaped_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assetdb = dir.path().join("assetdb");
        std::fs::create_dir_all(assetdb.join("src")).expect("create source directory");
        std::fs::create_dir(assetdb.join("drizzle")).expect("create migration directory");

        let error = replace_with_generated_baseline(&assetdb)
            .expect_err("baseline must be restricted to the canonical repository crate");

        assert!(error.to_string().contains("canonical AssetDB crate"));
    }

    #[test]
    fn baseline_install_reports_install_and_rollback_failures_with_backup_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let migrations = dir.path().join("drizzle");
        let staging = dir.path().join(".drizzle-baseline-test");
        let backup = dir.path().join(".drizzle-backup-test");
        std::fs::create_dir(&migrations).expect("old migration chain");
        std::fs::create_dir(&staging).expect("generated staging chain");
        let mut calls = 0;

        let error = install_generated_baseline(&migrations, &staging, &backup, |from, to| {
            calls += 1;
            match calls {
                1 => std::fs::rename(from, to),
                2 => Err(std::io::Error::other("injected install failure")),
                3 => Err(std::io::Error::other("injected rollback failure")),
                _ => unreachable!(),
            }
        })
        .expect_err("double rename failure must be reported");

        let message = error.to_string();
        assert!(message.contains("injected install failure"), "{message}");
        assert!(message.contains("injected rollback failure"), "{message}");
        assert!(message.contains(&backup.display().to_string()), "{message}");
        assert!(backup.is_dir(), "the previous chain stays recoverable");
        assert!(staging.is_dir(), "the generated chain stays recoverable");
    }
}
