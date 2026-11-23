use std::fmt::Write;

use crate::flags::{Arity, FLAGS, Flag, Id};

pub const DEFAULT_NAME: &str = "s";
const KINDS: &str = "file dir link";
const SHELLS: &str = "fish zsh bash man";
const GROUPS: &str = "deps cache build lock min media all";

fn choices(flag: &Flag) -> Option<&'static str> {
    match flag.id {
        Id::Type => Some(KINDS),
        Id::Init => Some(SHELLS),
        Id::Ignore | Id::Unignore => Some(GROUPS),
        _ => None,
    }
}

pub fn valid(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|other| other.is_ascii_alphanumeric() || other == '_')
}

fn fish(name: &str) -> String {
    let jump = format!(
        "function {name} --description 'Jump to the best matching folder'
    if test (count $argv) -eq 0
        cd ~
        return
    end
    set -l target (command syk -t dir -1 -X $argv 2>/dev/null)
    or set target (command syk -t dir -1 $argv ~)
    or return 1
    cd -- $target
end

"
    );
    let mut out = if name.is_empty() { String::new() } else { jump };
    for flag in &FLAGS {
        let summary = flag.summary.replace('\\', "\\\\").replace('\'', "\\'");
        let value = match (flag.arity, choices(flag)) {
            (Arity::Switch | Arity::Optional(_), _) => String::new(),
            (Arity::Required(_), Some(choices)) => format!(" -x -a '{choices}'"),
            (Arity::Required(_), None) if flag.id == Id::Exec => " -r".to_string(),
            (Arity::Required(_), None) => " -x".to_string(),
        };
        let _ = writeln!(
            out,
            "complete -c syk -s {} -l {}{value} -d '{summary}'",
            flag.short, flag.long
        );
    }
    out
}

fn jump(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    format!(
        "{name}() {{
    if [ \"$#\" -eq 0 ]; then
        cd ~
        return
    fi
    local target
    target=$(command syk -t dir -1 -X \"$@\" 2>/dev/null) ||
        target=$(command syk -t dir -1 \"$@\" ~) ||
        return 1
    cd -- \"$target\"
}}

"
    )
}

fn zsh(name: &str) -> String {
    let mut out = match name.is_empty() {
        true => "#compdef syk\n".to_string(),
        false => jump(name),
    };
    out.push_str("_syk() {\n    _arguments -s -S \\\n");
    for flag in &FLAGS {
        let summary = flag
            .summary
            .replace('\\', "\\\\")
            .replace('\'', "'\\''")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(':', "\\:");
        let value = match (flag.arity, choices(flag)) {
            (Arity::Switch, _) => String::new(),
            (Arity::Optional(what), _) => format!("::{what}:"),
            (Arity::Required(what), Some(choices)) => format!(":{what}:({choices})"),
            (Arity::Required(what), None) => format!(":{what}:"),
        };
        let _ = writeln!(
            out,
            "        '(-{short} --{long})'{{-{short},--{long}}}'[{summary}]{value}' \\",
            short = flag.short,
            long = flag.long
        );
    }
    out.push_str("        '*:path:_files'\n}\n");
    out.push_str(match name.is_empty() {
        true => "_syk \"$@\"\n",
        false => "(( $+functions[compdef] )) && compdef _syk syk\n",
    });
    out
}

fn bash(name: &str) -> String {
    let longs: Vec<String> = FLAGS
        .iter()
        .map(|flag| format!("--{}", flag.long))
        .collect();
    format!(
        "{}_syk() {{
    local current=${{COMP_WORDS[COMP_CWORD]}}
    COMPREPLY=()
    if [[ $current == -* ]]; then
        COMPREPLY=($(compgen -W \"{}\" -- \"$current\"))
    fi
}}
complete -o default -F _syk syk
",
        jump(name),
        longs.join(" ")
    )
}

pub fn script(value: &str) -> Result<String, String> {
    let (shell, name) = value.split_once(':').unwrap_or((value, DEFAULT_NAME));
    if shell.eq_ignore_ascii_case("man") {
        return Ok(crate::manual::page());
    }
    if !name.is_empty() && !valid(name) {
        return Err(format!(
            "{name:?} cannot be the name of a shell function. Use letters, digits and underscores, for example: syk --init {shell}:j"
        ));
    }
    match shell.to_ascii_lowercase().as_str() {
        "fish" => Ok(fish(name)),
        "zsh" => Ok(zsh(name)),
        "bash" | "sh" => Ok(bash(name)),
        _ => Err(format!(
            "there is no setup script for {shell:?}. Use fish, zsh, bash or man."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shell_gets_every_flag_and_the_jump_function() {
        for shell in ["fish", "zsh", "bash"] {
            let script = script(shell).expect("a script");
            assert!(script.contains("syk -t dir -1"), "{shell} cannot jump");
            for flag in &FLAGS {
                assert!(script.contains(flag.long), "{shell} lacks --{}", flag.long);
            }
        }
    }

    #[test]
    fn group_flags_complete_every_group() {
        for preset in &crate::group::PRESETS {
            assert!(
                GROUPS.split(' ').any(|name| name == preset.name),
                "{} is not completed",
                preset.name
            );
        }
        assert!(GROUPS.ends_with(crate::group::EVERY));
    }

    #[test]
    fn the_jump_function_can_be_renamed() {
        assert!(script("fish").expect("a script").starts_with("function s "));
        assert!(
            script("fish:j")
                .expect("a script")
                .starts_with("function j ")
        );
        assert!(
            script("zsh:jump_to")
                .expect("a script")
                .starts_with("jump_to() {")
        );
        assert!(script("fish:my-name").is_err());
        for shell in ["fish:", "zsh:", "bash:"] {
            let completions = script(shell).expect("a script");
            assert!(!completions.contains("syk -t dir -1"), "{shell}");
            assert!(completions.contains("name"), "{shell}");
        }
        assert!(
            script("zsh:")
                .expect("a script")
                .starts_with("#compdef syk")
        );
        assert!(script("man").expect("a page").starts_with(".TH SYK 1"));
        assert!(script("powershell").is_err());
    }
}
