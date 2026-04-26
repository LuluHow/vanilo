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
        Some("serve") => {
            if let Err(e) = builder::build() {
                eprintln!("error: {e}");
                process::exit(1);
            }
            let mut cfg = config::load();
            if let Some(port) = args.get(2).and_then(|s| s.parse().ok()) {
                cfg.port = port;
            }
            let build_lock = Arc::new(Mutex::new(()));
            let lock_clone = build_lock.clone();
            std::thread::spawn(move || {
                if let Err(e) = watcher::watch(lock_clone) {
                    eprintln!("watch error: {e}");
                }
            });
            if let Err(e) = server::serve(cfg, build_lock) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: vanilo <build|init|serve [port]>");
            process::exit(1);
        }
    }
}
