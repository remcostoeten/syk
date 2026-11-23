mod act;
mod banner;
mod build;
mod changed;
mod check;
mod config;
#[cfg(target_os = "linux")]
mod crawl;
mod explain;
mod filter;
mod flags;
mod group;
mod help;
mod index;
mod item;
mod kept;
mod language;
mod load;
mod manage;
mod manual;
mod need;
mod output;
mod pager;
mod parse;
mod pattern;
mod pick;
mod piped;
mod plan;
mod query;
mod rank;
mod replay;
mod report;
#[cfg(target_os = "linux")]
mod rule;
mod run;
mod scan;
mod search;
mod setup;
mod shell;
mod sign;
mod spell;
mod stop;
mod walk;
mod worker;

use std::ffi::OsString;
use std::process::ExitCode;

use banner::Look;
use pager::{Setup, Sheet};
use parse::Outcome;
use query::Defaults;

fn read(args: Vec<OsString>, defaults: Option<Defaults>) -> Result<Outcome, String> {
    match (parse::parse(args.clone())?, defaults) {
        (Outcome::Run(query), Some(defaults)) if !query.no_config => {
            parse::with_defaults(defaults, args)
        }
        (outcome, _) => Ok(outcome),
    }
}

fn view(above: &str, text: &str) {
    let look = Look::of(&std::io::stdout());
    let sheet = Sheet {
        text: format!("{}{text}", banner::above(above, look)),
        marks: Vec::new(),
    };
    let setup = Setup {
        styled: look.styled,
        open: None,
    };
    let shown = match look.banner {
        true => pager::offer(sheet, &setup),
        false => Err(sheet),
    };
    if let Err(sheet) = shown {
        print!("{}", sheet.text);
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let typed: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let mut views = parse::views(&typed);
    let defaults = match views.no_config {
        true => None,
        false => config::load(),
    };
    if let Some(defaults) = &defaults {
        let kept = parse::views(&defaults.args);
        views.no_pager |= kept.no_pager;
        views.no_banner |= kept.no_banner;
    }
    if views.no_pager {
        pager::turn_off();
    }
    if views.no_banner {
        banner::hide_logo();
    }
    match read(args, defaults) {
        Ok(Outcome::Run(query)) if query.output.explain => match explain::render(&query) {
            Ok(text) => {
                view(banner::EXPLAIN, &text);
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
        Ok(Outcome::Run(query)) if query.piped => piped::run(&query),
        Ok(Outcome::Run(query)) => run::run(*query),
        Ok(Outcome::Help(topics, terms)) => {
            help::print(&topics, &terms);
            ExitCode::SUCCESS
        }
        Ok(Outcome::Version) => {
            println!(concat!("syk ", env!("CARGO_PKG_VERSION")));
            ExitCode::SUCCESS
        }
        Ok(Outcome::Picks) => {
            view(banner::PICKS, &rank::report());
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
        Ok(Outcome::Indexes(action, paths)) => manage::run(action, &paths),
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
