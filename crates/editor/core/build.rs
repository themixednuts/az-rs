//! Generate editor-local Drizzle migrations at build time.

use drizzle_migrations::build::{Config, Output, run};
use drizzle_types::Dialect;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::new(Dialect::SQLite)
        .file("src/project_manager_preferences/schema.rs")
        .out("./drizzle");

    cfg.watch();

    match run(&cfg)? {
        Output::NoChanges => {}
        Output::Generated {
            tag,
            path,
            statement_count,
        } => {
            println!(
                "cargo:warning=az-editor: generated project manager preference migration {tag} ({statement_count} stmts) at {}",
                path.display(),
            );
        }
    }

    Ok(())
}
