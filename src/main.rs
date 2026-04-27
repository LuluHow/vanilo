mod builder;
mod component;
mod config;
mod content;
mod css;
mod functions;
mod lint;
mod parser;
mod server;
mod typescript;
mod watcher;

use std::env;
use std::process;
use std::sync::{Arc, Mutex};

fn uninstall() {
    use std::fs;
    use std::path::PathBuf;

    let bin = env::current_exe().unwrap_or_else(|_| PathBuf::from("vanilo"));

    // Remove the binary
    if bin.exists() {
        if let Err(e) = fs::remove_file(&bin) {
            eprintln!("could not remove {}: {e}", bin.display());
            process::exit(1);
        }
        println!("removed {}", bin.display());
    }

    // Remove cargo install metadata if installed via cargo
    if let Ok(home) = env::var("CARGO_HOME")
        .or_else(|_| env::var("HOME").map(|h| format!("{h}/.cargo")))
    {
        let crate_file = PathBuf::from(&home).join(".crates.toml");
        if crate_file.exists() {
            if let Ok(contents) = fs::read_to_string(&crate_file) {
                let cleaned: String = contents
                    .lines()
                    .filter(|line| !line.contains("vanilo"))
                    .map(|line| format!("{line}\n"))
                    .collect();
                let _ = fs::write(&crate_file, cleaned);
            }
        }
        let crate_file2 = PathBuf::from(&home).join(".crates2.json");
        if crate_file2.exists() {
            if let Ok(contents) = fs::read_to_string(&crate_file2) {
                if contents.contains("vanilo") {
                    // Remove vanilo entry from JSON — simple line-based removal
                    let cleaned: String = contents
                        .lines()
                        .filter(|line| !line.contains("vanilo"))
                        .map(|line| format!("{line}\n"))
                        .collect();
                    let _ = fs::write(&crate_file2, cleaned);
                }
            }
        }
    }

    println!("vanilo uninstalled");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("build") => {
            if let Err(e) = builder::build() {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Some("init") => {
            if let Err(e) = builder::init() {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Some("uninstall") => {
            uninstall();
        }
        Some("serve") => {
            let prod = args.iter().any(|a| a == "--prod");

            if prod {
                if !std::path::Path::new("dist").exists() {
                    eprintln!("error: dist/ not found — run `vanilo build` first");
                    process::exit(1);
                }
            } else {
                if let Err(e) = builder::build() {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }

            let mut cfg = config::load();
            for arg in &args[2..] {
                if let Ok(port) = arg.parse::<u16>() {
                    cfg.port = port;
                }
            }

            let build_lock = Arc::new(Mutex::new(()));

            if !prod {
                let lock_clone = build_lock.clone();
                std::thread::spawn(move || {
                    if let Err(e) = watcher::watch(lock_clone) {
                        eprintln!("watch error: {e}");
                    }
                });
            }

            if let Err(e) = server::serve(cfg, build_lock) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Some(cmd) => {
            eprintln!("error: unknown command '{cmd}'");
            eprintln!("commands: build, init, serve [--prod] [port], uninstall");
            process::exit(1);
        }
        None => {
            eprintln!("error: no command provided");
            eprintln!("commands: build, init, serve [--prod] [port], uninstall");
            process::exit(1);
        }
    }
}
