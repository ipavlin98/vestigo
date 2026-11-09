use super::docs::Outcome;
use super::worker::{parse_request, write_frame};
use crate::search::fold;
use anyhow::{Context, Result};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};
use parking_lot::Mutex;
use quick_xml::events::Event;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

pub(super) const TEXT_MAX_FILE: u64 = 16 << 20;
pub(super) const DOCUMENT_MAX_FILE: u64 = 256 << 20;
const TEXT_MAX_EXTRACTED: usize = 16 << 20;
const SNIFF_BYTES: usize = 8 * 1024;
const OPEN_DOCUMENT_TEXT: &str = "content.xml";
const STACK_BYTES: usize = 8 << 20;
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: [u8; 2] = [0xFF, 0xFE];
const UTF16_BE_BOM: [u8; 2] = [0xFE, 0xFF];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Pdf,
    Office,
    OpenDocument,
    Text,
}

impl Format {
    pub(super) fn of(lower_name: &str) -> Option<Self> {
        let dot = memchr::memrchr(b'.', lower_name.as_bytes())?;
        match lower_name.as_bytes().get(dot + 1..)? {
            b"pdf" => Some(Self::Pdf),
            b"docx" | b"docm" | b"doc" | b"xlsx" | b"xlsm" | b"xls" | b"pptx" | b"ppt" => Some(Self::Office),
            b"odt" | b"ods" | b"odp" => Some(Self::OpenDocument),
            b"txt" | b"md" | b"csv" | b"tsv" | b"log" | b"json" | b"xml" | b"html" | b"htm" | b"ini" | b"cfg"
            | b"toml" | b"yaml" | b"yml" | b"tex" | b"srt" | b"sql" | b"rs" | b"py" | b"js" | b"ts" | b"tsx" | b"c"
            | b"cpp" | b"h" | b"hpp" | b"cs" | b"java" | b"go" | b"kt" | b"ps1" | b"bat" | b"cmd" => Some(Self::Text),
            _ => None,
        }
    }

    pub(super) fn max_file(self) -> u64 {
        match self {
            Self::Text => TEXT_MAX_FILE,
            Self::Pdf | Self::Office | Self::OpenDocument => DOCUMENT_MAX_FILE,
        }
    }

    pub(super) fn is_document(self) -> bool {
        match self {
            Self::Text => false,
            Self::Pdf | Self::Office | Self::OpenDocument => true,
        }
    }
}

fn encoding_of(code_page: u32) -> Option<&'static Encoding> {
    match code_page {
        874 => Some(encoding_rs::WINDOWS_874),
        932 => Some(encoding_rs::SHIFT_JIS),
        936 => Some(encoding_rs::GBK),
        949 => Some(encoding_rs::EUC_KR),
        950 => Some(encoding_rs::BIG5),
        1250 => Some(encoding_rs::WINDOWS_1250),
        1251 => Some(encoding_rs::WINDOWS_1251),
        1252 => Some(encoding_rs::WINDOWS_1252),
        1253 => Some(encoding_rs::WINDOWS_1253),
        1254 => Some(encoding_rs::WINDOWS_1254),
        1255 => Some(encoding_rs::WINDOWS_1255),
        1256 => Some(encoding_rs::WINDOWS_1256),
        1257 => Some(encoding_rs::WINDOWS_1257),
        1258 => Some(encoding_rs::WINDOWS_1258),
        _ => None,
    }
}

fn decode(bytes: &[u8], code_page: u32) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    if bytes.starts_with(&UTF16_LE_BOM) {
        return Some(UTF_16LE.decode(bytes).0.into_owned());
    }
    if bytes.starts_with(&UTF16_BE_BOM) {
        return Some(UTF_16BE.decode(bytes).0.into_owned());
    }
    if memchr::memchr(0, bytes.get(..SNIFF_BYTES).unwrap_or(bytes)).is_some() {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        Err(_) => Some(encoding_of(code_page).unwrap_or(encoding_rs::WINDOWS_1252).decode(bytes).0.into_owned()),
    }
}

fn pdf(path: &str) -> Result<(Outcome, String), Outcome> {
    let encrypted = |error: pdf_oxide::Error| match error {
        pdf_oxide::Error::EncryptedPdf => Outcome::Encrypted,
        _ => Outcome::Failed,
    };
    let document = pdf_oxide::PdfDocument::open(path).map_err(encrypted)?;
    let pages = document.page_count().map_err(encrypted)?;
    let mut text = String::new();
    for page in 0..pages {
        if text.len() >= TEXT_MAX_EXTRACTED {
            break;
        }
        match document.extract_text(page) {
            Ok(page) => {
                text.push_str(&page);
                text.push('\n');
            }
            Err(pdf_oxide::Error::EncryptedPdf) => return Err(Outcome::Encrypted),
            Err(_) => {}
        }
    }
    Ok((Outcome::Text, text))
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

fn xml_text(xml: &[u8], out: &mut String) {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Text(text)) => out.push_str(&text.xml10_content()),
            Ok(Event::GeneralRef(reference)) => match reference.resolve_char_ref() {
                Ok(Some(ch)) => out.push(ch),
                Ok(None) | Err(_) => out.extend(entity(&reference.xml10_content())),
            },
            Ok(Event::End(end)) if matches!(end.local_name().into_inner(), "p" | "h" | "table-cell") => {
                out.push(' ');
            }
            Ok(Event::Empty(empty)) if matches!(empty.local_name().into_inner(), "tab" | "s" | "line-break") => {
                out.push(' ');
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
        buffer.clear();
    }
}

fn open_document(path: &str) -> Result<String> {
    let file = std::fs::File::open(path).context("opening the document")?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).context("reading the archive")?;
    let mut xml = Vec::new();
    archive.by_name(OPEN_DOCUMENT_TEXT).context("finding the text")?.read_to_end(&mut xml)?;
    let mut text = String::new();
    xml_text(&xml, &mut text);
    Ok(text)
}

fn plain(path: &str, code_page: u32) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    let file = std::fs::File::open(path).context("opening the file")?;
    file.take(TEXT_MAX_FILE).read_to_end(&mut bytes).context("reading the file")?;
    Ok(decode(&bytes, code_page))
}

fn raw_text(path: &str, code_page: u32) -> Result<(Outcome, String), Outcome> {
    if std::fs::File::open(path).is_err_and(|error| error.kind() == ErrorKind::PermissionDenied) {
        return Err(Outcome::Denied);
    }
    let lower = fold::lowered(path);
    let name = Path::new(&lower).file_name().and_then(|name| name.to_str()).unwrap_or_default();
    match Format::of(name) {
        Some(Format::Pdf) => pdf(path),
        Some(Format::Office) => match office_oxide::extract_text(path) {
            Ok(text) => Ok((Outcome::Text, text)),
            Err(_) => Err(Outcome::Failed),
        },
        Some(Format::OpenDocument) => match open_document(path) {
            Ok(text) => Ok((Outcome::Text, text)),
            Err(_) => Err(Outcome::Failed),
        },
        Some(Format::Text) => match plain(path, code_page) {
            Ok(Some(text)) => Ok((Outcome::Text, text)),
            Ok(None) => Err(Outcome::Unsupported),
            Err(_) => Err(Outcome::Failed),
        },
        None => Err(Outcome::Unsupported),
    }
}

fn limit(text: &mut String) {
    if text.len() > TEXT_MAX_EXTRACTED {
        let mut end = TEXT_MAX_EXTRACTED;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

pub(super) fn normalized(raw: &str, out: &mut String) -> Outcome {
    fold::collapse(raw, out);
    limit(out);
    if out.is_empty() { Outcome::Empty } else { Outcome::Text }
}

pub(super) fn extract(path: &str, code_page: u32, out: &mut String) -> Outcome {
    out.clear();
    match catch_unwind(AssertUnwindSafe(|| raw_text(path, code_page))) {
        Ok(Ok((_, raw))) => normalized(&raw, out),
        Ok(Err(outcome)) => outcome,
        Err(_) => Outcome::Failed,
    }
}

fn answer<I: BufRead, O: Write>(input: &Mutex<I>, output: &Mutex<O>, code_page: u32) {
    let mut line = String::new();
    let mut text = String::new();
    loop {
        line.clear();
        if !matches!(input.lock().read_line(&mut line), Ok(read) if read > 0) {
            return;
        }
        let Some((id, path)) = parse_request(&line) else { continue };
        let outcome = extract(path, code_page, &mut text);
        if write_frame(&mut *output.lock(), id, outcome, text.as_bytes()).is_err() {
            return;
        }
    }
}

pub(crate) fn serve_on<I: BufRead + Send, O: Write + Send>(input: I, output: O, code_page: u32) {
    let threads = std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).max(1));
    let (input, output) = (Mutex::new(input), Mutex::new(output));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let spawned = std::thread::Builder::new()
                .stack_size(STACK_BYTES)
                .spawn_scoped(scope, || answer(&input, &output, code_page));
            if spawned.is_err() {
                break;
            }
        }
    });
}

pub(crate) fn serve(code_page: u32) {
    serve_on(BufReader::new(std::io::stdin()), std::io::stdout(), code_page);
}

#[cfg(test)]
mod tests {
    use super::*;

    const CP1251: u32 = 1251;

    fn decoded(bytes: &[u8]) -> Option<String> {
        decode(bytes, CP1251)
    }

    #[test]
    fn plain_text_is_decoded_by_its_mark_or_as_the_system_code_page() {
        assert_eq!(decoded("привет".as_bytes()).as_deref(), Some("привет"));
        assert_eq!(decoded(b"\xEF\xBB\xBFhello").as_deref(), Some("hello"));
        assert_eq!(decoded(&[0xFF, 0xFE, 0x3F, 0x04, 0x40, 0x04]).as_deref(), Some("пр"));
        assert_eq!(decoded(&[0xFE, 0xFF, 0x04, 0x3F, 0x04, 0x40]).as_deref(), Some("пр"));
        assert_eq!(decoded(&[0xEF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]).as_deref(), Some("привет"));
    }

    #[test]
    fn a_nul_near_the_start_means_a_binary_file() {
        assert_eq!(decoded(b"MZ\0\x01\x02"), None);
        let mut late = vec![b'a'; SNIFF_BYTES];
        late.push(0);
        assert!(decoded(&late).is_some());
    }

    #[test]
    fn extracted_text_collapses_whitespace_and_stops_at_the_limit() {
        let mut out = String::new();
        assert_eq!(normalized("  договор\r\n\r\n  аренды\t", &mut out), Outcome::Text);
        assert_eq!(out, "договор аренды");
        assert_eq!(normalized(" \n ", &mut out), Outcome::Empty);
        let long = "я".repeat(TEXT_MAX_EXTRACTED);
        normalized(&long, &mut out);
        assert_eq!(out.len(), TEXT_MAX_EXTRACTED);
    }

    #[test]
    fn open_documents_keep_words_apart_and_decode_entities() {
        let mut out = String::new();
        let xml =
            "<office:text><text:p>Иванов &amp; сын&#33;</text:p><text:p>вторая<text:s/>строка</text:p></office:text>";
        xml_text(xml.as_bytes(), &mut out);
        assert_eq!(out, "Иванов & сын! вторая строка ");
    }

    #[test]
    fn formats_follow_the_extension() {
        assert_eq!(Format::of("отчёт.pdf"), Some(Format::Pdf));
        assert_eq!(Format::of("а.б.docx"), Some(Format::Office));
        assert_eq!(Format::of("main.rs"), Some(Format::Text));
        assert_eq!(Format::of("photo.jpg"), None);
        assert_eq!(Format::of("makefile"), None);
    }

    #[test]
    fn a_text_file_is_read_through_the_whole_path() {
        let path = std::env::temp_dir().join(format!("vestigo-extract-{}.TXT", std::process::id()));
        std::fs::write(&path, "  первая\n\nвторая ").expect("scratch file");
        let mut out = String::new();
        let outcome = extract(&path.to_string_lossy(), CP1251, &mut out);
        let _ = std::fs::remove_file(&path);
        assert_eq!(outcome, Outcome::Text);
        assert_eq!(out, "первая вторая");
    }

    #[test]
    fn a_missing_file_fails_without_text() {
        let mut out = String::from("old");
        assert_eq!(extract("Z:\\нет\\такого.txt", CP1251, &mut out), Outcome::Failed);
        assert!(out.is_empty());
    }
}
