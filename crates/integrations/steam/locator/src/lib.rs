use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamApp {
    pub app_id: u32,
    pub name: Option<String>,
    pub install_dir_name: String,
    pub install_dir: PathBuf,
    pub library_dir: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub app_id: Option<u32>,
    pub name: Option<String>,
    pub install_dir: Option<String>,
}

#[must_use]
pub fn find_steam_app(app_id: u32) -> Option<SteamApp> {
    let manifest_name = format!("appmanifest_{app_id}.acf");

    for library_dir in candidate_steam_library_dirs() {
        let manifest_path = library_dir.join("steamapps").join(&manifest_name);
        let Ok(raw) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        let manifest = parse_appmanifest_acf(&raw);

        if let Some(manifest_app_id) = manifest.app_id
            && manifest_app_id != app_id
        {
            continue;
        }

        let Some(install_dir_name) = manifest.install_dir else {
            continue;
        };
        let install_dir = library_dir
            .join("steamapps")
            .join("common")
            .join(&install_dir_name);

        if !install_dir.exists() {
            continue;
        }

        return Some(SteamApp {
            app_id,
            name: manifest.name,
            install_dir_name,
            install_dir,
            library_dir,
            manifest_path,
        });
    }

    None
}

#[must_use]
pub fn candidate_steam_library_dirs() -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    for steam_root in candidate_steam_roots() {
        push_unique_path(&mut libraries, steam_root.clone());
        for library in steam_library_dirs_from_root(&steam_root) {
            push_unique_path(&mut libraries, library);
        }
    }
    libraries
}

#[must_use]
pub fn candidate_libraryfolders_vdf_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for steam_root in candidate_steam_roots() {
        push_unique_path(
            &mut paths,
            steam_root.join("config").join("libraryfolders.vdf"),
        );
    }
    paths
}

pub fn candidate_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    for name in ["STEAM_DIR", "STEAM_PATH", "STEAMROOT", "SteamPath"] {
        if let Some(path) = env_path(name).and_then(steam_root_from_env_path) {
            push_unique_path(&mut roots, path);
        }
    }

    #[cfg(windows)]
    {
        for (key, value) in [
            (r"HKCU\Software\Valve\Steam", "SteamPath"),
            (r"HKCU\Software\Valve\Steam", "SteamExe"),
            (r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        ] {
            if let Some(path) = read_windows_registry_value(key, value)
                .map(PathBuf::from)
                .and_then(steam_root_from_env_path)
            {
                push_unique_path(&mut roots, path);
            }
        }

        if let Some(program_files_x86) = env_path("ProgramFiles(x86)") {
            push_unique_path(&mut roots, program_files_x86.join("Steam"));
        }
        if let Some(program_files) = env_path("ProgramFiles") {
            push_unique_path(&mut roots, program_files.join("Steam"));
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = env_path("HOME") {
            push_unique_path(&mut roots, home.join(".steam").join("steam"));
            push_unique_path(&mut roots, home.join(".local").join("share").join("Steam"));
        }
    }

    roots
}

#[must_use]
pub fn steam_library_dirs_from_root(steam_root: &Path) -> Vec<PathBuf> {
    let path = steam_root.join("config").join("libraryfolders.vdf");
    fs::read_to_string(path)
        .map(|raw| parse_libraryfolders_vdf(&raw))
        .unwrap_or_default()
}

#[must_use]
pub fn parse_libraryfolders_vdf(raw: &str) -> Vec<PathBuf> {
    let tokens = parse_vdf_quoted_strings(raw);
    let mut paths = Vec::new();

    for pair in tokens.windows(2) {
        if pair[0].eq_ignore_ascii_case("path") {
            push_unique_path(&mut paths, PathBuf::from(&pair[1]));
        }
    }

    paths
}

#[must_use]
pub fn parse_appmanifest_acf(raw: &str) -> AppManifest {
    let tokens = parse_vdf_quoted_strings(raw);
    let mut app_id = None;
    let mut name = None;
    let mut install_dir = None;

    for pair in tokens.windows(2) {
        match pair[0].as_str() {
            key if key.eq_ignore_ascii_case("appid") => {
                app_id = pair[1].parse::<u32>().ok();
            }
            key if key.eq_ignore_ascii_case("name") => {
                name = Some(pair[1].clone());
            }
            key if key.eq_ignore_ascii_case("installdir") => {
                install_dir = Some(pair[1].clone());
            }
            _ => {}
        }
    }

    AppManifest {
        app_id,
        name,
        install_dir,
    }
}

#[must_use]
pub fn parse_vdf_quoted_strings(raw: &str) -> Vec<String> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape = false;

    for ch in raw.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
                current.clear();
            }
            continue;
        }

        if escape {
            match ch {
                '\\' => current.push('\\'),
                '"' => current.push('"'),
                'n' => current.push('\n'),
                'r' => current.push('\r'),
                't' => current.push('\t'),
                other => current.push(other),
            }
            escape = false;
            continue;
        }

        match ch {
            '\\' => escape = true,
            '"' => {
                strings.push(std::mem::take(&mut current));
                in_string = false;
            }
            other => current.push(other),
        }
    }

    strings
}

fn env_path(name: &str) -> Option<PathBuf> {
    let value = env::var(name).ok()?;
    let trimmed = value.trim().trim_matches('"').trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn steam_root_from_env_path(path: PathBuf) -> Option<PathBuf> {
    if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("steam.exe"))
    {
        return path.parent().map(Path::to_path_buf);
    }
    Some(path)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    let key = path_key(&path);
    if paths.iter().any(|existing| path_key(existing) == key) {
        return;
    }
    paths.push(path);
}

fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

#[cfg(windows)]
fn read_windows_registry_value(key: &str, value: &str) -> Option<String> {
    let output = Command::new("reg")
        .args(["query", key, "/v", value])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        if !name.eq_ignore_ascii_case(value) {
            continue;
        }
        let _kind = parts.next()?;
        let data = parts.collect::<Vec<_>>().join(" ");
        if !data.is_empty() {
            return Some(data);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libraryfolders_paths() {
        let raw = r#"
            "libraryfolders"
            {
                "0"
                {
                    "path" "PrimaryLibrary"
                    "apps"
                    {
                        "730" "6524196981"
                    }
                }
                "1"
                {
                    "path" "SecondaryLibrary"
                    "apps"
                    {
                        "123456" "76416676920"
                    }
                }
            }
        "#;

        let paths = parse_libraryfolders_vdf(raw);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("PrimaryLibrary"));
        assert_eq!(paths[1], PathBuf::from("SecondaryLibrary"));
    }

    #[test]
    fn parses_appmanifest() {
        let raw = r#"
            "AppState"
            {
                "appid" "123456"
                "name" "Sample Game"
                "installdir" "SampleGame"
            }
        "#;

        let manifest = parse_appmanifest_acf(raw);
        assert_eq!(manifest.app_id, Some(123_456));
        assert_eq!(manifest.name.as_deref(), Some("Sample Game"));
        assert_eq!(manifest.install_dir.as_deref(), Some("SampleGame"));
    }
}
