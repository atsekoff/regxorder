mod app;

use clap::Parser;

use crate::app::{Cli, run};

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
