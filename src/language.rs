const LANGUAGES: [(&str, &[&str]); 13] = [
    ("c++", &["cpp", "cc", "cxx", "h", "hpp", "hh", "hxx"]),
    ("config", &["json", "toml", "yaml", "yml", "ini"]),
    ("csharp", &["cs"]),
    ("golang", &["go"]),
    ("javascript", &["js", "jsx", "mjs", "cjs"]),
    ("kotlin", &["kt", "kts"]),
    ("markdown", &["md", "mdx"]),
    ("python", &["py", "pyi"]),
    ("ruby", &["rb", "erb", "rake"]),
    ("rust", &["rs"]),
    ("shell", &["sh", "bash", "zsh", "fish"]),
    ("typescript", &["ts", "tsx", "mts", "cts"]),
    (
        "web",
        &[
            "html", "htm", "css", "scss", "sass", "less", "js", "jsx", "mjs", "ts", "tsx", "vue",
            "svelte", "astro",
        ],
    ),
];

pub fn extensions(name: &str) -> &'static [&'static str] {
    LANGUAGES
        .iter()
        .find(|(language, _)| language.eq_ignore_ascii_case(name))
        .map_or(&[], |(_, extensions)| extensions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::{Id, by_id};

    #[test]
    fn a_language_has_extensions_and_an_extension_does_not_become_a_language() {
        assert_eq!(extensions("RUST"), ["rs"]);
        assert!(extensions("typescript").contains(&"tsx"));
        for plain in ["ts", "js", "py", "go", "cpp", "md"] {
            assert!(extensions(plain).is_empty(), "{plain}");
        }
    }

    #[test]
    fn the_help_page_names_every_language() {
        let detail = by_id(Id::Ext).detail;
        for (language, _) in &LANGUAGES {
            assert!(detail.contains(language), "{language}");
        }
    }
}
