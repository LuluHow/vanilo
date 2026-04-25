mod builder;
mod component;
mod config;
mod functions;
mod lint;
mod parser;
mod server;

use std::env;
use std::process;

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
            if let Err(e) = server::serve(cfg) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: simple <build|init|serve [port]>");
            process::exit(1);
        }
    }
}
