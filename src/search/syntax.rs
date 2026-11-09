use std::ops::RangeInclusive;

const DAY: i64 = 24 * 60 * 60;
const WEEK_DAYS: i64 = 7;
const MONTH_DAYS: i64 = 30;
const YEAR_DAYS: i64 = 365;
const KIBI: f64 = 1024.0;
const UNITS: [(&str, i32); 5] = [("tb", 4), ("gb", 3), ("mb", 2), ("kb", 1), ("b", 0)];
const DEFAULT_UNIT: i32 = 2;
const RANGE: &str = "..";
pub(super) const FULL: &str = "full";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Clock {
    pub(crate) now: i64,
    pub(crate) offset: i64,
}

impl Clock {
    fn today(self) -> i64 {
        (self.now + self.offset).div_euclid(DAY)
    }

    fn day_start(self, day: i64) -> i64 {
        day * DAY - self.offset
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Token {
    pub(super) text: String,
    pub(super) negated: bool,
    pub(super) quoted: bool,
}

pub(super) fn tokens(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let negated = ch == '-' && chars.clone().nth(1).is_some_and(|next| !next.is_whitespace());
        if negated {
            chars.next();
        }
        if chars.peek() == Some(&'"') {
            chars.next();
            let text: String = chars.by_ref().take_while(|&ch| ch != '"').collect();
            tokens.push(Token { text, negated, quoted: true });
            continue;
        }
        let mut text = String::new();
        while let Some(ch) = chars.next_if(|ch| !ch.is_whitespace()) {
            text.push(ch);
            if matches!(chars.peek(), Some('"' | '-')) && operator(&text, FULL) == Some("") {
                break;
            }
        }
        tokens.push(Token { text, negated, quoted: false });
    }
    tokens
}

pub(super) fn operator<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let rest = text.get(name.len()..)?;
    if !text.get(..name.len())?.eq_ignore_ascii_case(name) {
        return None;
    }
    rest.strip_prefix(':').or_else(|| rest.strip_prefix('：'))
}

pub(super) fn extensions(value: &str) -> Vec<String> {
    value
        .split([';', ','])
        .map(|extension| extension.trim().trim_start_matches("*.").trim_start_matches('.').to_lowercase())
        .filter(|extension| !extension.is_empty() && !extension.contains(['\\', '/', '*']))
        .map(|extension| format!(".{extension}"))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Period {
    Today,
    Week,
    Month,
    Year,
    Older,
}

impl Period {
    pub(crate) const ALL: [Self; 5] = [Self::Today, Self::Week, Self::Month, Self::Year, Self::Older];

    pub(crate) fn range(self, clock: Clock) -> RangeInclusive<i64> {
        let since = |days: i64| clock.now - days * DAY..=i64::MAX;
        match self {
            Self::Today => clock.day_start(clock.today())..=i64::MAX,
            Self::Week => since(WEEK_DAYS),
            Self::Month => since(MONTH_DAYS),
            Self::Year => since(YEAR_DAYS),
            Self::Older => i64::MIN..=clock.now - YEAR_DAYS * DAY - 1,
        }
    }
}

pub(super) fn date(value: &str, clock: Clock) -> Option<RangeInclusive<i64>> {
    let open = i64::MIN..=i64::MAX;
    if let Some((from, to)) = value.split_once(RANGE) {
        let (from, to) = (period(from, clock)?, period(to, clock)?);
        return Some(*from.start()..=*to.end());
    }
    if let Some(rest) = value.strip_prefix(">=") {
        return Some(*period(rest, clock)?.start()..=*open.end());
    }
    if let Some(rest) = value.strip_prefix("<=") {
        return Some(*open.start()..=*period(rest, clock)?.end());
    }
    if let Some(rest) = value.strip_prefix('>') {
        return Some(period(rest, clock)?.end().checked_add(1)?..=*open.end());
    }
    if let Some(rest) = value.strip_prefix('<') {
        return Some(*open.start()..=period(rest, clock)?.start().checked_sub(1)?);
    }
    period(value, clock)
}

fn period(value: &str, clock: Clock) -> Option<RangeInclusive<i64>> {
    let today = clock.today();
    match value.trim().to_lowercase().as_str() {
        "today" => return Some(Period::Today.range(clock)),
        "yesterday" => return Some(clock.day_start(today - 1)..=clock.day_start(today) - 1),
        "week" => return Some(Period::Week.range(clock)),
        "month" => return Some(Period::Month.range(clock)),
        "year" => return Some(Period::Year.range(clock)),
        _ => {}
    }
    let parts: Vec<&str> = value.trim().split('-').collect();
    let number = |part: Option<&&str>| part.and_then(|part| part.parse::<i64>().ok());
    let year = number(parts.first()).filter(|year| (1601..=9999).contains(year))?;
    let (first, last) = match parts.len() {
        1 => (days_from_civil(year, 1, 1), days_from_civil(year + 1, 1, 1) - 1),
        2 => {
            let month = number(parts.get(1)).filter(|month| (1..=12).contains(month))?;
            let next = if month == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, month + 1, 1) };
            (days_from_civil(year, month, 1), next - 1)
        }
        3 => {
            let month = number(parts.get(1)).filter(|month| (1..=12).contains(month))?;
            let day = number(parts.get(2)).filter(|day| (1..=days_in_month(year, month)).contains(day))?;
            let day = days_from_civil(year, month, day);
            (day, day)
        }
        _ => return None,
    };
    Some(clock.day_start(first)..=clock.day_start(last + 1) - 1)
}

pub(super) fn size(value: &str) -> Option<RangeInclusive<u64>> {
    if let Some((from, to)) = value.split_once(RANGE) {
        return Some(bytes(from)?..=bytes(to)?);
    }
    if let Some(rest) = value.strip_prefix(">=") {
        return Some(bytes(rest)?..=u64::MAX);
    }
    if let Some(rest) = value.strip_prefix("<=") {
        return Some(0..=bytes(rest)?);
    }
    if let Some(rest) = value.strip_prefix('>') {
        return Some(bytes(rest)?.checked_add(1)?..=u64::MAX);
    }
    if let Some(rest) = value.strip_prefix('<') {
        return Some(0..=bytes(rest)?.checked_sub(1)?);
    }
    Some(bytes(value)?..=u64::MAX)
}

fn bytes(value: &str) -> Option<u64> {
    let value = value.trim().to_lowercase();
    let (number, power) = UNITS
        .iter()
        .find_map(|(unit, power)| value.strip_suffix(unit).map(|number| (number, *power)))
        .unwrap_or((value.as_str(), DEFAULT_UNIT));
    let number: f64 = number.trim().replace(',', ".").parse().ok().filter(|number: &f64| number.is_finite())?;
    if number < 0.0 {
        return None;
    }
    let bytes = (number * KIBI.powi(power)).round();
    (bytes <= u64::MAX as f64).then_some(bytes as u64)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let next = if month == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, month + 1, 1) };
    next - days_from_civil(year, month, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTC: Clock = Clock { now: 1_784_116_800, offset: 0 };

    fn day(year: i64, month: i64, day: i64) -> i64 {
        days_from_civil(year, month, day) * DAY
    }

    #[test]
    fn quotes_keep_spaces_and_minus_marks_exclusions() {
        let found = tokens(r#"report -draft -"old version" "q3 plan"  - my-file "open"#);
        let texts: Vec<(&str, bool, bool)> =
            found.iter().map(|token| (token.text.as_str(), token.negated, token.quoted)).collect();
        assert_eq!(
            texts,
            [
                ("report", false, false),
                ("draft", true, false),
                ("old version", true, true),
                ("q3 plan", false, true),
                ("-", false, false),
                ("my-file", false, false),
                ("open", false, true),
            ]
        );
    }

    #[test]
    fn the_full_operator_lets_a_quote_or_a_minus_start_its_value() {
        let found = tokens(r#"full:"договор аренды" FULL:-старый full:слово"#);
        let texts: Vec<(&str, bool, bool)> =
            found.iter().map(|token| (token.text.as_str(), token.negated, token.quoted)).collect();
        assert_eq!(
            texts,
            [
                ("full:", false, false),
                ("договор аренды", false, true),
                ("FULL:", false, false),
                ("старый", true, false),
                ("full:слово", false, false),
            ]
        );
    }

    #[test]
    fn extensions_accept_lists_dots_and_stars() {
        assert_eq!(extensions("pdf;.DOCX,*.tar.gz"), [".pdf", ".docx", ".tar.gz"]);
        assert!(extensions(";;").is_empty());
    }

    #[test]
    fn days_count_from_the_unix_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
    }

    #[test]
    fn calendar_periods_cover_whole_local_days() {
        assert_eq!(date("2026-07", UTC), Some(day(2026, 7, 1)..=day(2026, 8, 1) - 1));
        assert_eq!(date("2026", UTC), Some(day(2026, 1, 1)..=day(2027, 1, 1) - 1));
        assert_eq!(date("2026-12-31", UTC), Some(day(2026, 12, 31)..=day(2027, 1, 1) - 1));
        let moscow = Clock { now: UTC.now, offset: 3 * 60 * 60 };
        assert_eq!(date("2026-07-15", moscow), Some(day(2026, 7, 15) - 10_800..=day(2026, 7, 16) - 10_801));
        assert_eq!(date("2026-02-30", UTC), None);
        assert_eq!(date("2026-13", UTC), None);
    }

    #[test]
    fn relative_dates_count_back_from_now() {
        let today = day(2026, 7, 15);
        assert_eq!(UTC.now, today + 12 * 60 * 60);
        assert_eq!(date("today", UTC), Some(today..=i64::MAX));
        assert_eq!(date("Yesterday", UTC), Some(today - DAY..=today - 1));
        assert_eq!(date("week", UTC), Some(UTC.now - 7 * DAY..=i64::MAX));
    }

    #[test]
    fn older_starts_where_the_past_year_ends() {
        let year = Period::Year.range(UTC);
        let older = Period::Older.range(UTC);
        assert_eq!(*year.start(), UTC.now - 365 * DAY);
        assert_eq!(*older.end(), UTC.now - 365 * DAY - 1);
        assert_eq!(*older.start(), i64::MIN);
    }

    #[test]
    fn date_comparisons_and_ranges_use_period_edges() {
        let june_end = day(2026, 7, 1) - 1;
        assert_eq!(date(">2026-06", UTC), Some(june_end + 1..=i64::MAX));
        assert_eq!(date("<=2026-06", UTC), Some(i64::MIN..=june_end));
        assert_eq!(date("<2026-06", UTC), Some(i64::MIN..=day(2026, 6, 1) - 1));
        assert_eq!(date("2026-01..2026-06", UTC), Some(day(2026, 1, 1)..=june_end));
        assert_eq!(date("soon", UTC), None);
    }

    #[test]
    fn sizes_use_binary_units_and_megabytes_by_default() {
        assert_eq!(size(">10mb"), Some(10 * 1024 * 1024 + 1..=u64::MAX));
        assert_eq!(size("<=100KB"), Some(0..=100 * 1024));
        assert_eq!(size("10mb..1gb"), Some(10 << 20..=1 << 30));
        assert_eq!(size("1gb"), Some(1 << 30..=u64::MAX));
        assert_eq!(size("1.5gb"), Some(1_610_612_736..=u64::MAX));
        assert_eq!(size("2"), Some(2 << 20..=u64::MAX));
        assert_eq!(size("<0"), None);
        assert_eq!(size("big"), None);
    }

    #[test]
    fn operators_ignore_case_and_take_full_width_colons() {
        assert_eq!(operator("EXT:pdf", "ext"), Some("pdf"));
        assert_eq!(operator("ext：pdf", "ext"), Some("pdf"));
        assert_eq!(operator("extra", "ext"), None);
        assert_eq!(operator("дата:", "ext"), None);
    }
}
