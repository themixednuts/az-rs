use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, ffi::OsString};

static NEXT_CASE_ID: AtomicUsize = AtomicUsize::new(0);

struct LuaTools {
    lua: OsString,
    luac: OsString,
}

struct CasePaths {
    source: PathBuf,
    bytecode: PathBuf,
    decompiled: PathBuf,
}

impl CasePaths {
    fn new(name: &str) -> Self {
        let id = NEXT_CASE_ID.fetch_add(1, Ordering::Relaxed);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_millis();
        let stem = format!(
            "az_lua_{name}_{}_{}",
            std::process::id(),
            millis + id as u128
        );
        let dir = std::env::temp_dir();
        Self {
            source: dir.join(format!("{stem}.lua")),
            bytecode: dir.join(format!("{stem}.luac")),
            decompiled: dir.join(format!("{stem}_decompiled.lua")),
        }
    }

    fn cleanup(&self) {
        let _ = fs::remove_file(&self.source);
        let _ = fs::remove_file(&self.bytecode);
        let _ = fs::remove_file(&self.decompiled);
    }
}

impl Drop for CasePaths {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[allow(dead_code)]
pub fn run_equivalence(name: &str, source: &str) -> Option<String> {
    run_equivalence_with_args(name, source, &[])
}

#[allow(dead_code)]
pub fn run_stripped_equivalence(name: &str, source: &str) -> Option<String> {
    run_equivalence_inner(name, source, &[], true)
}

#[allow(dead_code)]
pub fn run_equivalence_with_args(name: &str, source: &str, args: &[&str]) -> Option<String> {
    run_equivalence_inner(name, source, args, false)
}

#[allow(dead_code)]
pub fn compile_source_bytes(name: &str, source: &str, strip_debug: bool) -> Option<Vec<u8>> {
    let tools = lua_tools()?;
    let paths = CasePaths::new(name);

    fs::write(&paths.source, source).expect("write Lua source");
    compile_lua(&tools.luac, &paths.source, &paths.bytecode, strip_debug);
    let bytecode = fs::read(&paths.bytecode).expect("read compiled bytecode");

    paths.cleanup();
    Some(bytecode)
}

#[allow(dead_code)]
pub fn compile_file_bytes(name: &str, source: &Path, strip_debug: bool) -> Option<Vec<u8>> {
    let tools = lua_tools()?;
    let paths = CasePaths::new(name);

    compile_lua(&tools.luac, source, &paths.bytecode, strip_debug);
    let bytecode = fs::read(&paths.bytecode).expect("read compiled bytecode");

    paths.cleanup();
    Some(bytecode)
}

#[allow(dead_code)]
pub fn run_bytecode_equivalence(name: &str, bytecode: &[u8], args: &[&str]) -> Option<String> {
    let tools = lua_tools()?;
    let paths = CasePaths::new(name);

    fs::write(&paths.bytecode, bytecode).expect("write original bytecode");
    let original_stdout = run_lua(&tools.lua, &paths.bytecode, args, "original Lua bytecode");
    let decompiled = az_lua_bytecode::decompile(bytecode)
        .unwrap_or_else(|error| panic!("{name} failed to decompile bytecode: {error}"));
    full_moon::parse(&decompiled).expect("decompiled source reparses with full_moon");

    fs::write(&paths.decompiled, &decompiled).expect("write decompiled Lua source");
    let decompiled_stdout = run_lua(&tools.lua, &paths.decompiled, args, "decompiled Lua source");
    assert_eq!(
        original_stdout,
        decompiled_stdout,
        "{name} stdout differed\noriginal:\n{}\ndecompiled source:\n{}\ndecompiled stdout:\n{}",
        String::from_utf8_lossy(&original_stdout),
        decompiled,
        String::from_utf8_lossy(&decompiled_stdout)
    );

    paths.cleanup();
    Some(decompiled)
}

fn run_equivalence_inner(
    name: &str,
    source: &str,
    args: &[&str],
    strip_debug: bool,
) -> Option<String> {
    let tools = lua_tools()?;
    let paths = CasePaths::new(name);

    fs::write(&paths.source, source).expect("write original Lua source");
    compile_lua(&tools.luac, &paths.source, &paths.bytecode, strip_debug);

    let original_stdout = run_lua(&tools.lua, &paths.source, args, "original Lua source");
    let bytecode = fs::read(&paths.bytecode).expect("read compiled bytecode");
    let decompiled = az_lua_bytecode::decompile(&bytecode)
        .unwrap_or_else(|error| panic!("{name} failed to decompile bytecode: {error}"));
    full_moon::parse(&decompiled).expect("decompiled source reparses with full_moon");

    fs::write(&paths.decompiled, &decompiled).expect("write decompiled Lua source");
    let decompiled_stdout = run_lua(&tools.lua, &paths.decompiled, args, "decompiled Lua source");
    assert_eq!(
        original_stdout,
        decompiled_stdout,
        "{name} stdout differed\noriginal:\n{}\ndecompiled source:\n{}\ndecompiled stdout:\n{}",
        String::from_utf8_lossy(&original_stdout),
        decompiled,
        String::from_utf8_lossy(&decompiled_stdout)
    );

    paths.cleanup();
    Some(decompiled)
}

fn lua_tools() -> Option<LuaTools> {
    let lua = lua_command()?;
    let luac = luac_command()?;
    Some(LuaTools { lua, luac })
}

#[allow(dead_code)]
pub fn lua_command() -> Option<OsString> {
    resolve_lua_tool("AZ_LUA", &["lua5.1", "lua"])
}

#[allow(dead_code)]
pub fn luac_command() -> Option<OsString> {
    resolve_lua_tool("AZ_LUAC", &["luac5.1", "luac"])
}

#[allow(dead_code)]
pub fn bundled_corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus")
}

#[allow(dead_code)]
pub fn external_corpus_root() -> Option<PathBuf> {
    let root = env::var_os("AZ_LUA_CORPUS_ROOT").map(PathBuf::from)?;
    if root.is_dir() {
        Some(root)
    } else {
        eprintln!(
            "AZ_LUA_CORPUS_ROOT does not name a directory: {}",
            root.display()
        );
        None
    }
}

#[allow(dead_code)]
pub fn corpus_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let bundled = bundled_corpus_root();
    if bundled.is_dir() {
        roots.push(bundled);
    }
    if let Some(external) = external_corpus_root() {
        roots.push(external);
    }
    roots
}

#[allow(dead_code)]
pub fn external_fixture(relative: impl AsRef<Path>) -> Option<PathBuf> {
    let relative = relative.as_ref();
    let path = external_corpus_root()?.join(relative);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "skipping missing external Lua fixture {} (set AZ_LUA_CORPUS_ROOT to the extracted Lua root)",
            path.display()
        );
        None
    }
}

fn resolve_lua_tool(env_name: &str, candidates: &[&str]) -> Option<OsString> {
    if let Some(configured) = env::var_os(env_name) {
        if command_is_lua_51(&configured) {
            return Some(configured);
        }
        eprintln!(
            "skipping Lua 5.1 runtime equivalence tests; {env_name} does not name a Lua 5.1 executable"
        );
        return None;
    }

    let tool = candidates
        .iter()
        .map(OsString::from)
        .find(command_is_lua_51);
    if tool.is_none() {
        eprintln!(
            "skipping Lua 5.1 runtime equivalence tests; set {env_name} or install one of: {}",
            candidates.join(", ")
        );
    }
    tool
}

fn command_is_lua_51(command: &OsString) -> bool {
    let Ok(output) = Command::new(command)
        .arg("-v")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    else {
        return false;
    };
    output.status.success()
        && [output.stdout, output.stderr]
            .concat()
            .windows(b"Lua 5.1".len())
            .any(|window| window == b"Lua 5.1")
}

fn run_lua(lua: &OsString, source: &Path, args: &[&str], context: &str) -> Vec<u8> {
    let mut child = Command::new(lua)
        .arg(source)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to run {context}: {err}"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .unwrap_or_else(|err| panic!("failed to collect {context}: {err}"));
                return assert_success(output, context);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{context} timed out after 10s: {}", source.display());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(err) => panic!("failed to wait for {context}: {err}"),
        }
    }
}

fn compile_lua(luac: &OsString, source: &Path, bytecode: &Path, strip_debug: bool) {
    let mut command = Command::new(luac);
    if strip_debug {
        command.arg("-s");
    }
    let output = command
        .arg("-o")
        .arg(bytecode)
        .arg(source)
        .output()
        .unwrap_or_else(|err| panic!("failed to run luac: {err}"));
    let _ = assert_success(output, "luac compile");
}

fn assert_success(output: Output, context: &str) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{context} failed with status {}\nstderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    output.stdout
}
