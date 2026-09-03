//! Locate the proprietary Oodle SDK library selected by the `oodle` feature.
//!
//! Consumers point at their licensed SDK with `OODLE_LIB_DIR`.

use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=OODLE_LIB_DIR");

    if env::var_os("CARGO_FEATURE_OODLE").is_none() {
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // Rad names each Oodle Data library after the platform it was built for, so
    // both the file to look for and the name to link are per target triple.
    let (link_name, library_names): (&str, &[&str]) = match (
        target_os.as_str(),
        target_arch.as_str(),
    ) {
        ("windows", _) => ("oo2core_win64", &["oo2core_win64.lib"]),
        ("linux", "aarch64") => (
            "oo2corelinuxarm64",
            &["liboo2corelinuxarm64.a", "liboo2corelinuxarm64.so"],
        ),
        ("linux", _) => (
            "oo2corelinux64",
            &["liboo2corelinux64.a", "liboo2corelinux64.so"],
        ),
        ("macos", _) => ("oo2coremac64", &["liboo2coremac64.a"]),
        _ => {
            println!(
                "cargo:warning=the default Oodle provider has no library convention for target OS \
                 `{target_os}`; disable default features or set up a target-specific provider"
            );
            return;
        }
    };

    let Some(library_dir) = env::var_os("OODLE_LIB_DIR").map(PathBuf::from) else {
        panic!("the `oodle` feature requires OODLE_LIB_DIR");
    };
    if library_names
        .iter()
        .all(|name| !library_dir.join(name).is_file())
    {
        panic!(
            "the `oodle` feature is enabled, but none of {} was found in OODLE_LIB_DIR ({})",
            library_names.join(", "),
            library_dir.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    // `oodle-sys` picks its link name from the *host* cfg, which is silent on
    // macOS and wrong when cross-compiling. Name the target's library here; a
    // repeated `-l` for the same library is harmless.
    println!("cargo:rustc-link-lib={link_name}");
    if target_os == "linux" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
