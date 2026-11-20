use std::path::PathBuf;

use crate::query::Defaults;

fn set(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn path() -> Option<PathBuf> {
    if let Some(chosen) = std::env::var_os("SYK_CONFIG") {
        return (!chosen.is_empty()).then(|| PathBuf::from(chosen));
    }
    let folder = set("XDG_CONFIG_HOME")
        .filter(|folder| folder.is_absolute())
        .or_else(|| set("HOME").map(|home| home.join(".config")))?;
    Some(folder.join("syk").join("config"))
}

pub fn split(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for line in text.lines() {
        let mut word = String::new();
        let mut open = false;
        let mut quote: Option<char> = None;
        let mut characters = line.chars();
        while let Some(character) = characters.next() {
            match (quote, character) {
                (Some(end), _) if character == end => quote = None,
                (Some(_), _) => word.push(character),
                (None, '#') if !open => break,
                (None, '\'' | '"') => {
                    quote = Some(character);
                    open = true;
                }
                (None, '\\') => {
                    word.extend(characters.next());
                    open = true;
                }
                (None, _) if character.is_whitespace() => {
                    if open {
                        words.push(std::mem::take(&mut word));
                    }
                    open = false;
                }
                (None, _) => {
                    word.push(character);
                    open = true;
                }
            }
        }
        if open {
            words.push(word);
        }
    }
    words
}

pub fn load() -> Option<Defaults> {
    let path = path()?;
    let args = split(&std::fs::read_to_string(&path).ok()?);
    (!args.is_empty()).then_some(Defaults { path, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_split_like_a_shell_would() {
        let text = "# defaults\n-E node_modules,dist --stats\n\n  -f 'wc -l'  # count\n--exclude \"my folder\" a\\ b\n";
        assert_eq!(
            split(text),
            [
                "-E",
                "node_modules,dist",
                "--stats",
                "-f",
                "wc -l",
                "--exclude",
                "my folder",
                "a b"
            ]
        );
        assert!(split("# nothing\n\n").is_empty());
        assert_eq!(split("-E ''"), ["-E", ""]);
    }
}
