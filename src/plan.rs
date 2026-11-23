use crate::pattern::Pattern;
use crate::query::{Plan, Query, Targets, Terms};

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum Compiled {
    Union {
        pattern: Option<Pattern>,
        targets: Targets,
    },
    Narrow {
        dir: Option<Pattern>,
        name: Option<Pattern>,
        content: Option<Pattern>,
    },
}

impl Compiled {
    pub fn content(&self) -> Option<&Pattern> {
        match self {
            Compiled::Union { pattern, targets } => pattern.as_ref().filter(|_| targets.content),
            Compiled::Narrow { content, .. } => content.as_ref(),
        }
    }
}

pub fn compile(query: &Query) -> Result<Compiled, String> {
    let build = |terms: &Option<Terms>| -> Result<Option<Pattern>, String> {
        terms
            .as_deref()
            .map(|terms| Pattern::compile(terms, &query.matching))
            .transpose()
    };
    Ok(match &query.plan {
        Plan::Union { pattern, targets } => Compiled::Union {
            pattern: build(pattern)?,
            targets: *targets,
        },
        Plan::Narrow { dir, name, content } => Compiled::Narrow {
            dir: build(dir)?,
            name: build(name)?,
            content: build(content)?,
        },
    })
}
