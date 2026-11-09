use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

const APP_TYPES: &str = "exe;lnk;appref-ms;url";
const DOCUMENT_TYPES: &str = "pdf;doc;docx;xls;xlsx;ppt;pptx;odt;ods;odp;rtf;txt;md;csv;epub;djvu";
const IMAGE_TYPES: &str = "jpg;jpeg;png;gif;bmp;webp;svg;tif;tiff;heic;ico;psd;raw;avif";
const AUDIO_TYPES: &str = "mp3;wav;flac;aac;ogg;m4a;wma;opus";
const VIDEO_TYPES: &str = "mp4;mkv;avi;mov;wmv;webm;flv;m4v;mpg;mpeg";
const ARCHIVE_TYPES: &str = "zip;rar;7z;tar;gz;bz2;xz;zst;iso;cab";
const SEPARATORS: [char; 3] = [';', ',', ' '];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Builtin {
    Folders,
    Files,
    Apps,
    Documents,
    Images,
    Audio,
    Video,
    Archives,
}

impl Builtin {
    pub(crate) const ALL: [Self; 8] = [
        Self::Folders,
        Self::Files,
        Self::Apps,
        Self::Documents,
        Self::Images,
        Self::Audio,
        Self::Video,
        Self::Archives,
    ];

    fn keyword(self) -> &'static str {
        match self {
            Self::Folders => "folder",
            Self::Files => "file",
            Self::Apps => "app",
            Self::Documents => "doc",
            Self::Images => "pic",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Archives => "zip",
        }
    }

    fn extensions(self) -> &'static str {
        match self {
            Self::Folders | Self::Files => "",
            Self::Apps => APP_TYPES,
            Self::Documents => DOCUMENT_TYPES,
            Self::Images => IMAGE_TYPES,
            Self::Audio => AUDIO_TYPES,
            Self::Video => VIDEO_TYPES,
            Self::Archives => ARCHIVE_TYPES,
        }
    }

    fn entries(self) -> Entries {
        match self {
            Self::Folders => Entries::Folders,
            Self::Files | Self::Apps | Self::Documents | Self::Images | Self::Audio | Self::Video | Self::Archives => {
                Entries::Files
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Entries {
    #[default]
    Any,
    Folders,
    Files,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rule {
    entries: Entries,
    extensions: Vec<String>,
    folder: Option<String>,
}

impl Rule {
    pub(crate) fn accepts(&self, lower: &str, dir: bool) -> bool {
        let entries = match self.entries {
            Entries::Any => true,
            Entries::Folders => dir,
            Entries::Files => !dir,
        };
        entries && (self.extensions.is_empty() || !dir && ends_with_any(lower, &self.extensions))
    }

    pub(crate) fn scope(&self) -> Self {
        Self { folder: self.folder.clone(), ..Self::default() }
    }

    pub(crate) fn folder(&self) -> Option<&str> {
        self.folder.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) builtin: Option<Builtin>,
    pub(crate) name: String,
    pub(crate) keyword: String,
    pub(crate) extensions: String,
    pub(crate) folder: String,
    pub(crate) enabled: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            builtin: None,
            name: String::new(),
            keyword: String::new(),
            extensions: String::new(),
            folder: String::new(),
            enabled: true,
        }
    }
}

impl Filter {
    pub(crate) fn builtin(builtin: Builtin) -> Self {
        Self {
            builtin: Some(builtin),
            keyword: builtin.keyword().to_owned(),
            extensions: builtin.extensions().to_owned(),
            ..Self::default()
        }
    }

    pub(crate) fn defaults() -> Vec<Self> {
        Builtin::ALL.into_iter().map(Self::builtin).collect()
    }

    pub(crate) fn complete(filters: &mut Vec<Self>) {
        for builtin in Builtin::ALL {
            if !filters.iter().any(|filter| filter.builtin == Some(builtin)) {
                filters.push(Self::builtin(builtin));
            }
        }
    }

    pub(crate) fn rule(&self) -> Rule {
        let extensions: Vec<String> = self
            .extensions
            .split(SEPARATORS)
            .map(|extension| extension.trim().trim_start_matches('.').to_lowercase())
            .filter(|extension| !extension.is_empty())
            .map(|extension| format!(".{extension}"))
            .collect();
        let entries = match self.builtin {
            Some(builtin) => builtin.entries(),
            None if extensions.is_empty() => Entries::Any,
            None => Entries::Files,
        };
        let folder = self.folder.trim().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
        Rule { entries, extensions, folder: (!folder.is_empty()).then_some(folder) }
    }

    pub(crate) fn keyword(&self) -> Option<String> {
        let keyword = self.keyword.trim().trim_end_matches([':', '：']).to_lowercase();
        (self.enabled && !keyword.is_empty()).then_some(keyword)
    }
}

pub(crate) struct Kinds {
    folders: u64,
    files: u64,
    suffixes: FxHashMap<String, u64>,
}

impl Kinds {
    pub(crate) const MAX: usize = u64::BITS as usize;

    pub(crate) fn new(rules: &[Rule]) -> Self {
        let mut kinds = Self { folders: 0, files: 0, suffixes: FxHashMap::default() };
        for (position, rule) in rules.iter().enumerate().take(Self::MAX) {
            let bit = 1 << position;
            match (rule.entries, rule.extensions.is_empty()) {
                (Entries::Any, true) => {
                    kinds.folders |= bit;
                    kinds.files |= bit;
                }
                (Entries::Folders, true) => kinds.folders |= bit,
                (Entries::Files, true) => kinds.files |= bit,
                (Entries::Folders, false) => {}
                (Entries::Any | Entries::Files, false) => {
                    for extension in &rule.extensions {
                        *kinds.suffixes.entry(extension.clone()).or_default() |= bit;
                    }
                }
            }
        }
        kinds
    }

    pub(crate) fn of(&self, lower: &str, dir: bool) -> u64 {
        if dir {
            return self.folders;
        }
        lower
            .match_indices('.')
            .filter(|&(dot, _)| dot > 0)
            .filter_map(|(dot, _)| lower.get(dot..).and_then(|suffix| self.suffixes.get(suffix)))
            .fold(self.files, |mask, kinds| mask | kinds)
    }
}

pub(super) fn ends_with_any(lower: &str, dotted: &[String]) -> bool {
    dotted.iter().any(|extension| lower.len() > extension.len() && lower.ends_with(extension.as_str()))
}
