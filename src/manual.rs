use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use crate::flags::{Arity, FLAGS, Flag, Group};

fn escaped(text: &str) -> String {
    let body = text.replace('\\', "\\e").replace('-', "\\-");
    match body.starts_with(['.', '\'']) {
        true => format!("\\&{body}"),
        false => body,
    }
}

fn paragraphs(out: &mut String, text: &str) {
    for (at, part) in text.split("\n\n").enumerate() {
        if at > 0 {
            out.push_str(".IP\n");
        }
        for line in part.lines() {
            let _ = writeln!(out, "{}", escaped(line));
        }
    }
}

fn entry(out: &mut String, flag: &Flag) {
    let value = match flag.arity {
        Arity::Switch => String::new(),
        Arity::Required(what) => format!(" \\fI{}\\fR", escaped(what)),
        Arity::Optional(what) => format!(" [\\fI{}\\fR]", escaped(what)),
    };
    let _ = writeln!(
        out,
        ".TP\n\\fB\\-{}\\fR, \\fB\\-\\-{}\\fR{value}",
        flag.short,
        escaped(flag.long)
    );
    paragraphs(out, flag.detail);
    for (command, about) in flag.examples {
        let _ = writeln!(
            out,
            ".IP\n\\fB{}\\fR\n.br\n{}",
            escaped(command),
            escaped(about)
        );
    }
}

pub fn page() -> String {
    let mut out = format!(
        ".TH SYK 1 \"\" \"syk {}\" \"User Commands\"\n",
        env!("CARGO_PKG_VERSION")
    );
    out.push_str(concat!(
        ".SH NAME\n",
        "syk \\- search folder names, file names and file contents in one pass\n",
        ".SH SYNOPSIS\n",
        ".B syk\n",
        "[\\fIFLAGS\\fR] [\\fIPATTERN\\fR] [\\fIPATH\\fR ...]\n",
        ".br\n",
        "\\fIcommand\\fR | \\fBsyk\\fR \\fIPATTERN\\fR \\fB\\-\\fR\n",
        ".SH DESCRIPTION\n",
        "syk reads each word by its shape. A word that exists on disk or ends in a slash is a ",
        "path, a word such as .ts is an extension, +1mb and \\-7d are filters, and anything else ",
        "is the pattern. A single dash searches what is piped in.\n",
    ));
    for group in Group::ALL {
        let _ = writeln!(out, ".SH {}", group.title());
        for flag in FLAGS.iter().filter(|flag| flag.group == group) {
            entry(&mut out, flag);
        }
    }
    out.push_str(concat!(
        ".SH EXIT STATUS\n",
        ".TP\n0\nSomething was found.\n",
        ".TP\n1\nNothing was found.\n",
        ".TP\n2\nAn argument was wrong, or a file or folder could not be read.\n",
        ".TP\n130\nThe search was stopped with Ctrl\\-C.\n",
        ".SH FILES\n",
        ".TP\n\\fI~/.cache/syk/index\\fR\nThe stored indexes.\n",
        ".SH SEE ALSO\n",
        "\\fBrg\\fR(1), \\fBfd\\fR(1), \\fBgrep\\fR(1), \\fBfind\\fR(1)\n",
    ));
    out
}

fn place(program: &Path) -> Option<PathBuf> {
    let prefix = program.parent()?.parent()?;
    Some(prefix.join("share/man/man1/syk.1"))
}

pub fn install() -> Result<String, String> {
    let program = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| format!("could not find the syk program: {error}"))?;
    let path =
        place(&program).ok_or_else(|| format!("{} has no folder above it.", program.display()))?;
    let text = page();
    if fs::read_to_string(&path).is_ok_and(|present| present == text) {
        return Ok(format!("Already installed: {}", path.display()));
    }
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&path, text));
    written.map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(format!(
        "Installed {}. Open it with: man syk",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_goes_next_to_the_program() {
        assert_eq!(
            place(Path::new("/home/me/.cargo/bin/syk")),
            Some(PathBuf::from("/home/me/.cargo/share/man/man1/syk.1"))
        );
        assert_eq!(place(Path::new("/syk")), None);
    }

    #[test]
    fn the_page_names_every_flag_and_no_line_starts_a_stray_request() {
        let page = page();
        for flag in &FLAGS {
            let wanted = format!("\\fB\\-\\-{}\\fR", escaped(flag.long));
            assert!(page.contains(&wanted), "--{} is missing", flag.long);
        }
        for line in page.lines() {
            assert!(!line.starts_with('\''), "{line}");
            if let Some(rest) = line.strip_prefix('.') {
                let request = rest.split(' ').next().unwrap_or("");
                assert!(
                    ["TH", "SH", "TP", "IP", "B", "br"].contains(&request),
                    "{line}"
                );
            }
        }
    }
}
