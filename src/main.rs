mod act;
mod banner;
mod config;
mod explain;
mod filter;
mod flags;
mod help;
mod output;
mod parse;
mod pattern;
mod query;
mod rank;
mod setup;
mod shell;
mod spell;
mod walk;

use std::ffi::OsString;
use std::process::ExitCode;

use banner::Look;
use parse::Outcome;

fn read(args: Vec<OsString>) -> Result<Outcome, String> {
    match parse::parse(args.clone())? {
        Outcome::Run(query) if !query.no_config => match config::load() {
            Some(defaults) => parse::with_defaults(defaults, args),
            None => Ok(Outcome::Run(query)),
        },
        outcome => Ok(outcome),
    }
}

fn main() -> ExitCode {
    match read(std::env::args_os().skip(1).collect()) {
        Ok(Outcome::Run(query)) if query.output.explain => match explain::render(&query) {
            Ok(text) => {
                let look = Look::of(&std::io::stdout());
                print!("{}{text}", banner::above(banner::EXPLAIN, look));
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
        Ok(Outcome::Run(query)) => walk::run(*query),
        Ok(Outcome::Help(topics)) => {
            help::print(&topics);
            ExitCode::SUCCESS
        }
        Ok(Outcome::Version) => {
            println!(concat!("syk ", env!("CARGO_PKG_VERSION")));
            ExitCode::SUCCESS
        }
        Ok(Outcome::Picks) => {
            let look = Look::of(&std::io::stdout());
            print!("{}{}", banner::above(banner::PICKS, look), rank::report());
            ExitCode::SUCCESS
        }
        Ok(Outcome::Init(shell)) => match shell::script(&shell) {
            Ok(script) => {
                print!("{script}");
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
        Ok(Outcome::Setup(shell)) => setup::run(shell.as_deref()),
        Ok(Outcome::Usage) => {
            help::print_welcome();
            ExitCode::from(2)
        }
        Err(error) => fail(&error),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("syk: {message}");
    ExitCode::from(2)
}
