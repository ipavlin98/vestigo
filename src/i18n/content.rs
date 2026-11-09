use super::{Language, SPACE, group_digits};

const SECONDS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;
const ESTIMATE_ROUNDING: usize = 100;

impl Language {
    pub(crate) fn at_least(self, count: usize) -> String {
        let matches = self.matches(count);
        match matches.rsplit_once(SPACE) {
            Some((number, word)) => format!("{number}+{SPACE}{word}"),
            None => matches,
        }
    }

    pub(crate) fn duration(self, seconds: u64) -> String {
        let minutes = seconds.div_ceil(SECONDS_PER_MINUTE);
        let (second, minute, hour) = match self {
            Self::English => ("s", "min", "h"),
            Self::Russian => ("с", "мин", "ч"),
        };
        match (seconds < SECONDS_PER_MINUTE, minutes.div_euclid(MINUTES_PER_HOUR)) {
            (true, _) => format!("{seconds}{SPACE}{second}"),
            (false, 0) => format!("{minutes}{SPACE}{minute}"),
            (false, hours) => format!("{hours}{SPACE}{hour} {}{SPACE}{minute}", minutes % MINUTES_PER_HOUR),
        }
    }

    fn files(self, count: usize) -> String {
        self.count(count, ["file", "files"], ["файл", "файла", "файлов"])
    }

    pub(crate) fn content_progress(self, percent: usize) -> String {
        match self {
            Self::English => format!("reading file contents: {percent}{SPACE}%"),
            Self::Russian => format!("индексация содержимого: {percent}{SPACE}%"),
        }
    }

    pub(crate) fn content_estimate(self, files: usize, text: u64, cache: u64, seconds: (u64, u64)) -> String {
        let files = self.files(files / ESTIMATE_ROUNDING * ESTIMATE_ROUNDING);
        let (text, cache) = (self.size(text), self.size(cache));
        let (fastest, slowest) = (self.duration(seconds.0), self.duration(seconds.1));
        match self {
            Self::English => format!(
                "About {files} ({text} of text) will be read. The cache will take about {cache}, the first indexing \
                 runs in the background for {fastest} to {slowest} and keeps the processor and the disk busy"
            ),
            Self::Russian => format!(
                "Будет прочитано ≈ {files} (≈ {text} текста). Кэш займёт ≈ {cache}, первая индексация в фоне — \
                 от {fastest} до {slowest}, процессор и диск будут заняты"
            ),
        }
    }

    pub(crate) fn content_building(self, done: usize, total: usize, left: Option<u64>) -> String {
        let (done, total) = (group_digits(done), self.files(total));
        let left = left.map(|seconds| self.duration(seconds));
        match (self, left) {
            (Self::English, Some(left)) => format!("Indexing: {done} of {total}, about {left} left"),
            (Self::English, None) => format!("Indexing: {done} of {total}"),
            (Self::Russian, Some(left)) => format!("Индексация: {done} из {total}, осталось ≈ {left}"),
            (Self::Russian, None) => format!("Индексация: {done} из {total}"),
        }
    }

    pub(crate) fn content_ready(self, files: usize, cache: u64, built: u64) -> String {
        let (files, cache) = (self.files(files), self.size(cache));
        let mut parts = match self {
            Self::English => vec![files, format!("{cache} on disk")],
            Self::Russian => vec![files, format!("{cache} на диске")],
        };
        if built > 0 {
            let built = self.duration(built);
            parts.push(match self {
                Self::English => format!("built in {built}"),
                Self::Russian => format!("построено за {built}"),
            });
        }
        parts.join(" · ")
    }

    pub(crate) fn content_unread(self, reasons: &[(usize, &str)]) -> String {
        if reasons.is_empty() {
            return String::new();
        }
        let reasons: Vec<String> =
            reasons.iter().map(|&(count, reason)| format!("{reason} — {}", group_digits(count))).collect();
        let reasons = reasons.join(", ");
        match self {
            Self::English => format!("Not read: {reasons}"),
            Self::Russian => format!("Не прочитано: {reasons}"),
        }
    }

    pub(crate) fn content_cache_kept(self, cache: u64) -> String {
        let cache = self.size(cache);
        match self {
            Self::English => format!("{cache}. After deleting it the index is built again"),
            Self::Russian => format!("{cache}. После удаления индекс будет построен заново"),
        }
    }

    pub(crate) fn content_cache_idle(self, cache: u64) -> String {
        let cache = self.size(cache);
        match self {
            Self::English => {
                format!(
                    "Content search is off, the cache takes {cache} and is no longer updated. Delete it if it's not needed"
                )
            }
            Self::Russian => format!(
                "Поиск по содержимому выключен, кэш занимает {cache} и больше не обновляется. Удалите его, если он не нужен"
            ),
        }
    }
}
