use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;
const MONTH: u64 = 30 * DAY;
const OLDEST_TOTAL: f32 = 2000.0;

const PICKED: f32 = 4.0;
const QUALITY: f32 = 3.0;
const VISITED: f32 = 2.0;
const RECENT: f32 = 1.0;
const SHALLOW: f32 = 0.5;

pub struct Block {
    pub path: Vec<u8>,
    pub quality: f32,
    pub depth: usize,
    pub modified: Option<SystemTime>,
    pub line: Option<u64>,
    pub text: Vec<u8>,
}

#[derive(Default)]
pub struct Score {
    pub picked: f32,
    pub name: f32,
    pub visited: f32,
    pub recent: f32,
    pub shallow: f32,
}

impl Score {
    pub fn total(&self) -> f32 {
        self.picked + self.name + self.visited + self.recent + self.shallow
    }

    pub fn row(&self) -> String {
        format!(
            "{:6.2}  {:6.2}  {:5.2}  {:7.2}  {:6.2}  {:7.2}  ",
            self.total(),
            self.picked,
            self.name,
            self.visited,
            self.recent,
            self.shallow
        )
    }

    pub fn heading() -> String {
        format!(
            "score = picked (max {PICKED}) + name (max {QUALITY}) + visited (max {VISITED}) + recent (max {RECENT}) + shallow (max {SHALLOW})\n{:>6}  {:>6}  {:>5}  {:>7}  {:>6}  {:>7}  path\n",
            "score", "picked", "name", "visited", "recent", "shallow"
        )
    }
}

#[derive(Default)]
struct Scores {
    known: HashMap<Vec<u8>, f32>,
    top: f32,
}

impl Scores {
    fn add(&mut self, path: Vec<u8>, score: f32) {
        self.top = self.top.max(score);
        self.known.insert(path, score);
    }

    fn near(&self, absolute: &[u8]) -> f32 {
        if self.top <= 0.0 {
            return 0.0;
        }
        let mut best: f32 = 0.0;
        let mut end = absolute.len();
        let mut distance = 0.0;
        loop {
            if let Some(score) = self.known.get(&absolute[..end]) {
                best = best.max(score / (1.0 + distance));
            }
            match absolute[..end].iter().rposition(|byte| *byte == b'/') {
                Some(slash) if slash > 0 => end = slash,
                _ => break,
            }
            distance += 1.0;
        }
        (1.0 + best).ln() / (1.0 + self.top).ln()
    }
}

pub struct Habits {
    visited: Scores,
    picked: Scores,
    here: Vec<u8>,
    now: SystemTime,
}

impl Habits {
    pub fn load() -> Habits {
        let now = SystemTime::now();
        let mut picked = Scores::default();
        for pick in read_picks() {
            picked.add(pick.path, frecency(pick.rank, pick.last, seconds(now)));
        }
        Habits {
            visited: zoxide(),
            picked,
            here: here(),
            now,
        }
    }

    #[cfg(test)]
    fn empty(now: SystemTime) -> Habits {
        Habits {
            visited: Scores::default(),
            picked: Scores::default(),
            here: b"/work".to_vec(),
            now,
        }
    }

    fn score(&self, block: &Block) -> Score {
        let absolute = absolute(&self.here, &block.path);
        let age = block
            .modified
            .and_then(|modified| self.now.duration_since(modified).ok())
            .map(|age| age.as_secs());
        let recent = match age {
            Some(age) if age < HOUR => 1.0,
            Some(age) if age < DAY => 0.75,
            Some(age) if age < WEEK => 0.5,
            Some(age) if age < MONTH => 0.25,
            _ => 0.0,
        };
        Score {
            picked: PICKED * self.picked.near(&absolute),
            name: QUALITY * block.quality,
            visited: VISITED * self.visited.near(&absolute),
            recent: RECENT * recent,
            shallow: SHALLOW / (1.0 + block.depth as f32),
        }
    }

    pub fn known(&self) -> impl Iterator<Item = &[u8]> {
        self.visited
            .known
            .keys()
            .chain(self.picked.known.keys())
            .map(Vec::as_slice)
    }

    pub fn absolute(&self, path: &[u8]) -> Vec<u8> {
        absolute(&self.here, path)
    }

    pub fn relative<'a>(&self, path: &'a [u8]) -> &'a [u8] {
        match path
            .strip_prefix(self.here.as_slice())
            .and_then(|rest| rest.strip_prefix(b"/"))
        {
            Some(rest) if !rest.is_empty() => rest,
            _ => path,
        }
    }

    pub fn order(&self, blocks: Vec<Block>) -> Vec<(Score, Block)> {
        let mut scored: Vec<(Score, Block)> = blocks
            .into_iter()
            .map(|block| (self.score(&block), block))
            .collect();
        scored.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .total()
                .total_cmp(&left_score.total())
                .then_with(|| left.path.len().cmp(&right.path.len()))
                .then_with(|| left.path.cmp(&right.path))
        });
        scored
    }

    pub fn remember(&self, path: &[u8]) {
        let picked = absolute(&self.here, path);
        let now = seconds(self.now);
        let mut picks = read_picks();
        match picks.iter_mut().find(|pick| pick.path == picked) {
            Some(pick) => {
                pick.rank += 1.0;
                pick.last = now;
            }
            None => picks.push(Pick {
                rank: 1.0,
                last: now,
                path: picked,
            }),
        }
        if picks.iter().map(|pick| pick.rank).sum::<f32>() > OLDEST_TOTAL {
            for pick in &mut picks {
                pick.rank *= 0.9;
            }
            picks.retain(|pick| pick.rank >= 1.0);
        }
        write_picks(&picks);
    }
}

pub fn quality(name: &[u8], hit: Option<(usize, usize)>) -> f32 {
    let Some((start, end)) = hit else {
        return 0.0;
    };
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte >= 0x80;
    let ends_word = end >= name.len() || !word(name[end]);
    if start == 0 && end >= name.len() {
        1.0
    } else if start == 0 && name[end] == b'.' {
        0.9
    } else if start == 0 {
        0.6
    } else if !word(name[start - 1])
        || (name[start - 1].is_ascii_lowercase() && name[start].is_ascii_uppercase())
    {
        if ends_word { 0.5 } else { 0.4 }
    } else {
        0.2
    }
}

struct Pick {
    rank: f32,
    last: u64,
    path: Vec<u8>,
}

fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => "just now".to_string(),
        60..HOUR => format!("{} min ago", seconds / 60),
        HOUR..DAY => format!("{} h ago", seconds / HOUR),
        _ => format!("{} d ago", seconds / DAY),
    }
}

pub fn report() -> String {
    let now = seconds(SystemTime::now());
    let mut picks = read_picks();
    picks.sort_by(|left, right| {
        frecency(right.rank, right.last, now)
            .total_cmp(&frecency(left.rank, left.last, now))
            .then_with(|| left.path.cmp(&right.path))
    });
    let file = picks_file()
        .map(|file| file.display().to_string())
        .unwrap_or_else(|| "no data folder found".to_string());
    let mut text = String::new();
    if picks.is_empty() {
        text.push_str(
            "Nothing is remembered yet. A path chosen with -1 or -O is remembered from then on.\n",
        );
    } else {
        text.push_str(&format!(
            "{:>6}  {:>5}  {:<11}  path\n",
            "score", "picks", "last picked"
        ));
        for pick in &picks {
            text.push_str(&format!(
                "{:6.2}  {:5.1}  {:<11}  {}\n",
                frecency(pick.rank, pick.last, now),
                pick.rank,
                ago(now.saturating_sub(pick.last)),
                String::from_utf8_lossy(&pick.path)
            ));
        }
    }
    let folders = zoxide().known.len();
    text.push_str(&format!("\nkept in  {file}\n"));
    text.push_str(&match folders {
        0 => "zoxide   no folders known, or zoxide is not installed\n".to_string(),
        folders => format!("zoxide   {folders} folders known\n"),
    });
    text
}

fn frecency(rank: f32, last: u64, now: u64) -> f32 {
    let age = now.saturating_sub(last);
    if age < HOUR {
        rank * 4.0
    } else if age < DAY {
        rank * 2.0
    } else if age < WEEK {
        rank * 0.5
    } else {
        rank * 0.25
    }
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn here() -> Vec<u8> {
    let same = |logical: &OsString| match (fs::metadata(logical), fs::metadata(".")) {
        (Ok(named), Ok(actual)) => named.dev() == actual.dev() && named.ino() == actual.ino(),
        _ => false,
    };
    match std::env::var_os("PWD") {
        Some(logical) if logical.as_bytes().starts_with(b"/") && same(&logical) => {
            logical.into_vec()
        }
        _ => std::env::current_dir()
            .map(|path| path.into_os_string().into_vec())
            .unwrap_or_else(|_| b"/".to_vec()),
    }
}

fn absolute(here: &[u8], path: &[u8]) -> Vec<u8> {
    let base: &[u8] = if path.starts_with(b"/") { b"" } else { here };
    let mut parts: Vec<&[u8]> = Vec::new();
    for part in base
        .split(|byte| *byte == b'/')
        .chain(path.split(|byte| *byte == b'/'))
    {
        match part {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let mut joined = Vec::with_capacity(here.len() + path.len() + 1);
    for part in parts {
        joined.push(b'/');
        joined.extend_from_slice(part);
    }
    if joined.is_empty() {
        joined.push(b'/');
    }
    joined
}

fn zoxide() -> Scores {
    let mut scores = Scores::default();
    let listed = Command::new("zoxide")
        .args(["query", "--list", "--score"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Ok(listed) = listed else {
        return scores;
    };
    for line in listed.stdout.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii_start();
        let Some(space) = line.iter().position(|byte| *byte == b' ') else {
            continue;
        };
        let score = std::str::from_utf8(&line[..space])
            .ok()
            .and_then(|text| text.parse::<f32>().ok());
        if let Some(score) = score {
            scores.add(line[space + 1..].to_vec(), score);
        }
    }
    scores
}

fn picks_file() -> Option<PathBuf> {
    let data = match std::env::var_os("XDG_DATA_HOME") {
        Some(data) if !data.is_empty() => PathBuf::from(data),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(data.join("syk").join("picks"))
}

fn read_picks() -> Vec<Pick> {
    let Some(text) = picks_file().and_then(|file| fs::read(file).ok()) else {
        return Vec::new();
    };
    text.split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let mut fields = line.splitn(3, |byte| *byte == b'\t');
            let rank = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
            let last = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
            let path = fields.next()?.to_vec();
            Some(Pick { rank, last, path })
        })
        .collect()
}

fn write_picks(picks: &[Pick]) {
    let Some(file) = picks_file() else {
        return;
    };
    let Some(folder) = file.parent() else {
        return;
    };
    let mut text = Vec::new();
    for pick in picks {
        let _ = write!(text, "{}\t{}\t", pick.rank, pick.last);
        text.extend_from_slice(&pick.path);
        text.push(b'\n');
    }
    let mut partial = OsString::from(file.as_os_str());
    partial.push(format!(".{}", std::process::id()));
    let saved = fs::create_dir_all(folder)
        .and_then(|_| fs::write(&partial, text))
        .and_then(|_| fs::rename(&partial, &file));
    if saved.is_err() {
        let _ = fs::remove_file(&partial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(path: &str, quality: f32, age: Option<u64>, now: SystemTime) -> Block {
        Block {
            path: path.as_bytes().to_vec(),
            quality,
            depth: path.split('/').count(),
            modified: age.map(|age| now - Duration::from_secs(age)),
            line: None,
            text: Vec::new(),
        }
    }

    fn order(habits: &Habits, blocks: Vec<Block>) -> Vec<String> {
        habits
            .order(blocks)
            .into_iter()
            .map(|(_, block)| String::from_utf8_lossy(&block.path).into_owned())
            .collect()
    }

    #[test]
    fn a_score_is_the_sum_of_its_parts() {
        let now = SystemTime::now();
        let mut habits = Habits::empty(now);
        habits.picked.add(b"/work/a/config.ts".to_vec(), 4.0);
        let score = habits.score(&block("a/config.ts", 0.9, Some(10), now));
        assert_eq!((score.picked, score.recent), (PICKED, RECENT));
        assert!((score.name - 2.7).abs() < 0.001);
        assert_eq!(score.visited, 0.0);
        assert!((score.total() - (PICKED + 2.7 + RECENT + SHALLOW / 3.0)).abs() < 0.001);
        assert!(
            score
                .row()
                .starts_with("  7.87    4.00   2.70     0.00    1.00     0.17  ")
        );
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(5 * 60), "5 min ago");
        assert_eq!(ago(3 * HOUR), "3 h ago");
        assert_eq!(ago(9 * DAY), "9 d ago");
    }

    #[test]
    fn a_better_name_match_scores_higher() {
        let hit = |name: &str, start, end| quality(name.as_bytes(), Some((start, end)));
        let exact = hit("config", 0, 6);
        let stem = hit("config.ts", 0, 6);
        let prefix = hit("configure.ts", 0, 6);
        let word = hit("app_config.ts", 4, 10);
        let camel = hit("appConfig.ts", 3, 9);
        let inside = hit("reconfigure.ts", 2, 8);
        assert!(exact > stem && stem > prefix && prefix > word && word > inside);
        assert_eq!(word, camel);
        assert_eq!(quality(b"config", None), 0.0);
    }

    #[test]
    fn paths_are_made_absolute_without_touching_the_disk() {
        assert_eq!(absolute(b"/work/app", b"src/a.ts"), b"/work/app/src/a.ts");
        assert_eq!(absolute(b"/work/app", b"../lib/b.ts"), b"/work/lib/b.ts");
        assert_eq!(absolute(b"/work/app", b"/etc/hosts"), b"/etc/hosts");
        assert_eq!(absolute(b"/work/app", b"src/"), b"/work/app/src");
        assert_eq!(absolute(b"/", b".."), b"/");
    }

    #[test]
    fn match_quality_then_recency_then_depth_decide() {
        let now = SystemTime::now();
        let habits = Habits::empty(now);
        let found = order(
            &habits,
            vec![
                block("deep/down/reconfigure.ts", 0.2, Some(10), now),
                block("old/config.ts", 0.9, None, now),
                block("new/config.ts", 0.9, Some(10), now),
                block("a/b/c/config.ts", 0.9, None, now),
            ],
        );
        assert_eq!(
            found,
            [
                "new/config.ts",
                "old/config.ts",
                "a/b/c/config.ts",
                "deep/down/reconfigure.ts"
            ]
        );
    }

    #[test]
    fn visited_folders_and_picked_paths_lift_what_is_below_them() {
        let now = SystemTime::now();
        let mut habits = Habits::empty(now);
        habits.visited.add(b"/work/used".to_vec(), 500.0);
        let visited = order(
            &habits,
            vec![
                block("other/config.ts", 0.9, None, now),
                block("used/src/config.ts", 0.9, None, now),
            ],
        );
        assert_eq!(visited[0], "used/src/config.ts");

        habits.picked.add(b"/work/other/config.ts".to_vec(), 4.0);
        let picked = order(
            &habits,
            vec![
                block("used/src/config.ts", 0.9, None, now),
                block("other/config.ts", 0.9, None, now),
            ],
        );
        assert_eq!(picked[0], "other/config.ts");
    }

    #[test]
    fn recent_picks_count_for_more() {
        let now = 100_000_000;
        assert_eq!(frecency(2.0, now - 60, now), 8.0);
        assert_eq!(frecency(2.0, now - 2 * HOUR, now), 4.0);
        assert_eq!(frecency(2.0, now - 2 * DAY, now), 1.0);
        assert_eq!(frecency(2.0, now - 2 * WEEK, now), 0.5);
    }
}
