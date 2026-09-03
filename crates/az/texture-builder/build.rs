use std::env;

fn main() {
    let target = env::var("TARGET").expect("Cargo sets TARGET for build scripts");
    if target.contains("linux") || target.contains("freebsd") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    } else if target.contains("apple") {
        println!("cargo:rustc-link-lib=dylib=c++");
    }
}
