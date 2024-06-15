//! This build script copies the `memory.x` file from the crate root into
//! a directory where the linker can always find it at build time.
//! For many projects this is optional, as the linker always searches the
//! project root directory -- wherever `Cargo.toml` is. However, if you
//! are using a workspace or have a more complicated build setup, this
//! build script becomes required. Additionally, by requesting that
//! Cargo re-run the build script whenever `memory.x` is changed,
//! updating `memory.x` ensures a rebuild of the application with the
//! new memory settings.

use serde_json;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

fn main() {
    println!("in build.rs");
    memory_x();
    wifi_secrets().unwrap();
}

fn wifi_secrets() -> io::Result<()> {
    println!("in wifi_secrets");
    // Fetch the output directory from the OUT_DIR environment variable
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR environment variable not set");
    // Create a new file in the output directory
    let dest_path = Path::new(&out_dir).join("wifi_secrets.rs");
    let mut f = File::create(&dest_path).expect("Could not create wifi_secrets.rs file");

    // Read the wifi_config.json file
    println!("in wifi_secrets, before reading wifi_config.json");
    let config_contents =
        fs::read_to_string("wifi_config.json").expect("Could not read wifi_config.json file");
    let config: serde_json::Value =
        serde_json::from_str(&config_contents).expect("Could not parse wifi_config.json file");

    // read the ssid and password from the json file
    println!("in wifi_secrets, before reading ssid and password from json file");
    let ssid = config["ssid"]
        .as_str()
        .expect("ssid not found in wifi_config.json file");
    let password = config["password"]
        .as_str()
        .expect("password not found in wifi_config.json file");

    // Write the ssid and password to the output file
    println!("in wifi_secrets, before writing ssid and password to output file");
    writeln!(f, "pub const SSID: &str = {:?};", ssid)?;
    writeln!(f, "pub const PASSWORD: &str = {:?};", password)?;

    // return the result, which is an empty Ok() in this case if everything went well
    Ok(())
}

fn memory_x() {
    print!("in memory_x");
    // Put `memory.x` in our output directory and ensure it's
    // on the linker search path.
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    // By default, Cargo will re-run a build script whenever
    // any file in the project changes. By specifying `memory.x`
    // here, we ensure the build script is only re-run when
    // `memory.x` is changed.
    println!("cargo:rerun-if-changed=memory.x");

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tlink-rp.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
}
