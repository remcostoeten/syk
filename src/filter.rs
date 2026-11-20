use crate::query::{AgeFilter, SizeFilter};

pub enum Parsed {
    Size(SizeFilter),
    Age(AgeFilter),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    Plus,
    Minus,
    Absent,
}

enum Unit {
    Bytes(u64),
    Seconds(u64),
    Absent,
}

struct Amount {
    sign: Sign,
    number: f64,
    unit: Unit,
}

pub fn is_token(text: &str) -> bool {
    let mut chars = text.chars();
    !matches!(text, "-0" | "-1")
        && matches!(chars.next(), Some('+' | '-'))
        && chars.next().is_some_and(|c| c.is_ascii_digit())
}

pub fn parse_token(text: &str) -> Result<Parsed, String> {
    let amount = split(text)?;
    match amount.unit {
        Unit::Bytes(scale) => Ok(Parsed::Size(size(text, &amount, scale))),
        Unit::Seconds(scale) => Ok(Parsed::Age(age(text, &amount, scale))),
        Unit::Absent => Err(format!(
            "{text} has no unit. Write {text}kb for a size or {text}d for an age. Run syk -h size or syk -h modified for the units."
        )),
    }
}

pub fn parse_size(text: &str) -> Result<SizeFilter, String> {
    let amount = split(text)?;
    match amount.unit {
        Unit::Bytes(scale) => Ok(size(text, &amount, scale)),
        Unit::Absent => Ok(size(text, &amount, 1)),
        Unit::Seconds(_) => Err(format!(
            "{text} is an age, not a size. Use it with -M, or write a size such as +10kb."
        )),
    }
}

pub fn parse_age(text: &str) -> Result<AgeFilter, String> {
    let amount = split(text)?;
    match amount.unit {
        Unit::Seconds(scale) => Ok(age(text, &amount, scale)),
        Unit::Bytes(_) => Err(format!(
            "{text} is a size, not an age. Use it with -S, or write an age such as -2d."
        )),
        Unit::Absent => Err(format!(
            "{text} has no unit. Write an age such as -2d, +6h or +1y."
        )),
    }
}

fn size(text: &str, amount: &Amount, scale: u64) -> SizeFilter {
    SizeFilter {
        at_least: amount.sign != Sign::Minus,
        bytes: (amount.number * scale as f64) as u64,
        text: text.to_string(),
    }
}

fn age(text: &str, amount: &Amount, scale: u64) -> AgeFilter {
    AgeFilter {
        older: amount.sign == Sign::Plus,
        seconds: (amount.number * scale as f64) as u64,
        text: text.to_string(),
    }
}

fn split(text: &str) -> Result<Amount, String> {
    let (sign, rest) = match text.chars().next() {
        Some('+' | '>') => (Sign::Plus, &text[1..]),
        Some('-' | '<') => (Sign::Minus, &text[1..]),
        _ => (Sign::Absent, text),
    };
    let rest = rest.strip_prefix('=').unwrap_or(rest);
    let digits = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    let number: f64 = rest[..digits]
        .parse()
        .map_err(|_| format!("{text} is not a size or an age. Examples: +10kb, -1mb, +7d, -2h."))?;
    let unit = unit(&rest[digits..].to_ascii_lowercase())
        .ok_or_else(|| format!("{text} has an unknown unit. Sizes use b, kb, mb, gb, tb. Ages use s, min, h, d, w, mo, y."))?;
    Ok(Amount { sign, number, unit })
}

fn unit(text: &str) -> Option<Unit> {
    const KB: u64 = 1024;
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    Some(match text {
        "" => Unit::Absent,
        "b" | "byte" | "bytes" => Unit::Bytes(1),
        "k" | "kb" | "kib" => Unit::Bytes(KB),
        "m" | "mb" | "mib" => Unit::Bytes(KB * KB),
        "g" | "gb" | "gib" => Unit::Bytes(KB * KB * KB),
        "t" | "tb" | "tib" => Unit::Bytes(KB * KB * KB * KB),
        "s" | "sec" | "secs" | "second" | "seconds" => Unit::Seconds(1),
        "min" | "mins" | "minute" | "minutes" => Unit::Seconds(MINUTE),
        "h" | "hr" | "hrs" | "hour" | "hours" => Unit::Seconds(HOUR),
        "d" | "day" | "days" => Unit::Seconds(DAY),
        "w" | "wk" | "wks" | "week" | "weeks" => Unit::Seconds(7 * DAY),
        "mo" | "month" | "months" => Unit::Seconds(30 * DAY),
        "y" | "yr" | "yrs" | "year" | "years" => Unit::Seconds(365 * DAY),
        _ => return None,
    })
}
