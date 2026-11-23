use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Instant;

use grep_matcher::Matcher;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};

use crate::output::{Budget, Entry, Layout, Lines, Style, write_count, write_entry};
use crate::plan::compile;
use crate::query::Query;
use crate::report::totals;
use crate::search::{View, view};
use crate::worker::Tally;

pub const LABEL: &str = "<stdin>";

struct Flowing<'a, M> {
    lines: Lines<'a, M>,
    closed: bool,
}

impl<M> Flowing<'_, M> {
    fn flush(&mut self) -> bool {
        if !self.lines.out.is_empty() {
            let mut output = io::stdout().lock();
            self.closed |= output
                .write_all(self.lines.out)
                .and_then(|()| output.flush())
                .is_err();
            self.lines.out.clear();
        }
        !self.closed
    }
}

impl<M: Matcher> Sink for Flowing<'_, M> {
    type Error = io::Error;

    fn matched(&mut self, searcher: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let going = self.lines.matched(searcher, found)?;
        Ok(self.flush() && going)
    }

    fn context(
        &mut self,
        searcher: &Searcher,
        context: &SinkContext<'_>,
    ) -> Result<bool, io::Error> {
        let going = self.lines.context(searcher, context)?;
        Ok(self.flush() && going)
    }

    fn context_break(&mut self, searcher: &Searcher) -> Result<bool, io::Error> {
        self.lines.context_break(searcher)
    }

    fn binary_data(&mut self, searcher: &Searcher, offset: u64) -> Result<bool, io::Error> {
        self.lines.binary_data(searcher, offset)
    }
}

struct Scanned {
    hits: u64,
    headed: bool,
    binary: bool,
    failure: Option<io::Error>,
}

fn scan<M: Matcher>(
    matcher: &M,
    context: usize,
    entry: &Entry<'_>,
    shown: View,
    budget: &Budget,
) -> Scanned {
    let mut out = Vec::new();
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(0))
        .line_number(true)
        .before_context(context)
        .after_context(context)
        .build();
    let mut sink = Flowing {
        lines: Lines {
            out: &mut out,
            entry,
            matcher,
            layout: shown.layout,
            style: style(shown),
            budget,
            headed: false,
            hits: 0,
            first: None,
            binary: false,
        },
        closed: false,
    };
    let result = searcher.search_reader(matcher, io::stdin().lock(), &mut sink);
    Scanned {
        hits: sink.lines.hits,
        headed: sink.lines.headed,
        binary: sink.lines.binary,
        failure: result.err(),
    }
}

fn style(shown: View) -> Style<'static> {
    Style {
        color: shown.color,
        json: shown.json,
        end: shown.end,
        base: None,
    }
}

fn close(entry: &Entry<'_>, shown: View, budget: &Budget, scanned: &Scanned) {
    let mut out = Vec::new();
    match shown.layout {
        Layout::List if scanned.hits > 0 && budget.take() => {
            write_entry(&mut out, entry, style(shown));
        }
        Layout::Count if scanned.hits > 0 && budget.take() => {
            write_count(&mut out, entry, scanned.hits, style(shown));
        }
        Layout::Grouped if scanned.headed => out.push(b'\n'),
        _ => {}
    }
    let _ = io::stdout().lock().write_all(&out);
}

pub fn run(query: &Query) -> ExitCode {
    let started = Instant::now();
    if io::stdin().is_terminal() {
        eprintln!("syk: - reads what is piped in, and nothing is. Example: cat log | syk error -");
        return ExitCode::from(2);
    }
    let plan = match compile(query) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("syk: {error}");
            return ExitCode::from(2);
        }
    };
    let Some(pattern) = plan.content() else {
        return ExitCode::from(2);
    };
    let shown = view(query);
    let context = match shown.layout {
        Layout::List | Layout::Count => 0,
        _ => query.output.context,
    };
    let entry = Entry {
        path: LABEL.as_bytes(),
        name_at: 0,
        hit: None,
        folder: false,
    };
    let budget = Budget::new(query.output.max);
    let scanned = match pattern.single() {
        true => scan(pattern.lines(), context, &entry, shown, &budget),
        false => scan(pattern, context, &entry, shown, &budget),
    };
    close(&entry, shown, &budget, &scanned);
    if scanned.binary {
        eprintln!("syk: standard input holds binary data, so the search stopped there.");
    }
    if query.output.stats {
        let tally = Tally {
            entries: 1,
            read: 1,
            files: u64::from(scanned.hits > 0),
            lines: scanned.hits,
            ..Tally::default()
        };
        let text = totals(tally, shown.layout, started.elapsed(), None);
        eprintln!("syk: {text}");
    }
    match (&scanned.failure, scanned.hits > 0) {
        (Some(error), _) => {
            eprintln!("syk: {LABEL}: {error}");
            ExitCode::from(2)
        }
        (None, true) => ExitCode::SUCCESS,
        (None, false) => ExitCode::from(1),
    }
}
