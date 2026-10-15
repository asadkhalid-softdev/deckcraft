//! Read explicitly referenced files on the transport worker without replacing the live document.
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

const MAX_CHUNK: usize = 32_768;
const MAX_DOCUMENT: u64 = 32 * 1024 * 1024;

/// Only the user's own chat text grants a path; document/tool content never grants file access.
pub(super) fn authorize<'a>(path: &str, messages: impl Iterator<Item = &'a str>, document: Option<&Path>) -> Result<Value, String> {
    if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
        return Err("Provide a valid file path referenced in your chat message".into());
    }
    if !messages.into_iter().any(|message| mentions_path(message, path)) {
        return Err("The user must reference this file path in a chat message before it can be read".into());
    }
    let supplied = PathBuf::from(path);
    let resolved = if supplied.is_absolute() {
        supplied
    } else {
        document
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(|_| "Could not resolve the file path")?
            .join(supplied)
    };
    let resolved = resolved.to_str().ok_or("File path is not valid Unicode")?;
    deckcraft_engine::file_access::check_read(resolved)?;
    Ok(json!({"path": resolved}))
}

fn mentions_path(message: &str, path: &str) -> bool {
    #[cfg(windows)]
    let (message, path) = (message.to_lowercase(), path.to_lowercase());
    #[cfg(not(windows))]
    let (message, path) = (message.to_owned(), path.to_owned());
    message.match_indices(&path).any(|(start, _)| {
        let before = message.get(..start).and_then(|s| s.chars().next_back());
        let after = message.get(start.saturating_add(path.len())..).and_then(|s| s.chars().next());
        let boundary = |c: char| c.is_whitespace() || "\"'`<>[](){},;!?".contains(c);
        before.is_none_or(boundary) && after.is_none_or(boundary)
    })
}

pub(super) fn read(authorization: &Value, args: &Value, cancelled: &AtomicBool) -> Result<Value, String> {
    check_cancelled(cancelled)?;
    let path = authorization.get("path").and_then(Value::as_str).ok_or("File access was not authorized")?;
    deckcraft_engine::file_access::check_read(path)?;
    // Reject devices, pipes and directories before opening; recheck the actual opened handle too.
    let metadata = std::fs::metadata(path).map_err(|_| "Could not access the referenced file")?;
    if !metadata.is_file() {
        return Err("The referenced path must be a regular file".into());
    }
    let mut file = std::fs::File::open(path).map_err(|_| "Could not open the referenced file; check the path and operating-system permissions")?;
    let metadata = file.metadata().map_err(|_| "Could not inspect the referenced file")?;
    if !metadata.is_file() {
        return Err("The referenced path must be a regular file".into());
    }
    let offset = args.get("offset").map(|v| v.as_u64().ok_or("File offset must be a nonnegative integer")).transpose()?.unwrap_or(0);
    let limit = args
        .get("limit")
        .map(|v| v.as_u64().ok_or("File limit must be a nonnegative integer"))
        .transpose()?
        .unwrap_or(MAX_CHUNK as u64)
        .clamp(4, MAX_CHUNK as u64) as usize;
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("auto");
    if !["auto", "text", "base64"].contains(&mode) {
        return Err("File mode must be auto, text or base64".into());
    }
    let extension = Path::new(path).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if mode == "auto" && ["docx", "docm", "dotx", "dotm", "odt"].contains(&extension.as_str()) {
        if metadata.len() > MAX_DOCUMENT {
            return Err("Document exceeds the 32 MB extraction limit; use base64 mode for paged bytes".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_DOCUMENT.saturating_add(1)).read_to_end(&mut bytes).map_err(|_| "Could not read referenced document")?;
        if bytes.len() as u64 > MAX_DOCUMENT {
            return Err("Document grew beyond the extraction limit".into());
        }
        check_cancelled(cancelled)?;
        let text = document_text(&bytes, extension == "odt")?;
        return text_page(path, &text, offset, limit, cancelled);
    }
    if mode == "auto" && ["pptx", "pptm", "potx", "ppsx", "deckcraft", "slidecraft"].contains(&extension.as_str()) {
        if metadata.len() > MAX_DOCUMENT {
            return Err("Presentation exceeds the 32 MB extraction limit; use base64 mode for paged raw bytes.".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_DOCUMENT.saturating_add(1)).read_to_end(&mut bytes).map_err(|_| "Could not read the presentation")?;
        if bytes.len() as u64 > MAX_DOCUMENT {
            return Err("Presentation grew beyond the extraction limit".into());
        }
        check_cancelled(cancelled)?;
        let mut session = deckcraft_engine::Session::new();
        let result =
            session.execute("file.openBytes", &json!({"name":path,"data":deckcraft_engine::cmd::base64_encode(&bytes)})).map_err(|e| e.to_string());
        result?;
        let st = session.active().ok_or("Could not open referenced presentation")?;
        let mut text = String::new();
        for (i, slide) in st.doc.slides.iter().enumerate() {
            text.push_str(&format!("Slide {}: {}\n", i + 1, slide.title()));
            deckcraft_model::walk(&slide.shapes, &mut |s, _| {
                if let Some(t) = &s.text
                    && text.len() < 2_000_000
                {
                    text.push_str(&t.text());
                    text.push('\n');
                }
            });
            text.push_str(&slide.notes_text());
            text.push('\n');
            if text.len() > 2_000_000 {
                return Err("Extracted text exceeds the 2 MB limit; use base64 mode for raw bytes".into());
            }
        }
        check_cancelled(cancelled)?;
        let start = usize::try_from(offset).ok().filter(|n| *n <= text.len() && text.is_char_boundary(*n)).ok_or("Invalid extracted-text offset")?;
        let mut end = start.saturating_add(limit).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        return Ok(
            json!({"path":path,"encoding":"text","offsetUnit":"UTF-8 bytes of extracted text","text":text.get(start..end).unwrap_or_default(),"nextOffset":end,"hasMore":end<text.len(),"total":text.len()}),
        );
    }
    if offset > metadata.len() {
        return Err("File offset is beyond the end of the file".into());
    }
    let mut sample = vec![0u8; 4096];
    let sampled = file.read(&mut sample).map_err(|_| "Could not read the referenced file")?;
    sample.truncate(sampled);
    let utf16 = sample.starts_with(&[0xff, 0xfe]) || sample.starts_with(&[0xfe, 0xff]);
    let utf8 = !sample.contains(&0)
        && match std::str::from_utf8(&sample) {
            Ok(_) => true,
            Err(e) => e.error_len().is_none(),
        };
    file.seek(SeekFrom::Start(offset)).map_err(|_| "Could not seek in the referenced file")?;
    let mut bytes = Vec::new();
    file.take(limit as u64).read_to_end(&mut bytes).map_err(|_| "Could not read the referenced file")?;
    let binary = mode == "base64" || (mode == "auto" && !utf8 && !utf16);
    let (encoding, content) = if binary {
        ("base64", json!({"data": STANDARD.encode(&bytes), "note": "Raw file bytes, not extracted text"}))
    } else if utf16 {
        if offset % 2 != 0 {
            return Err("UTF-16 file offsets must be even".into());
        }
        bytes.truncate(bytes.len() - bytes.len() % 2);
        let little = sample.starts_with(&[0xff, 0xfe]);
        if offset.saturating_add(bytes.len() as u64) < metadata.len() && bytes.len() >= 2 {
            let tail = bytes.get(bytes.len().saturating_sub(2)..).unwrap_or_default();
            let unit = tail.first().zip(tail.get(1)).map(|(a, b)| if little { u16::from_le_bytes([*a, *b]) } else { u16::from_be_bytes([*a, *b]) });
            if unit.is_some_and(|unit| (0xd800..=0xdbff).contains(&unit)) {
                bytes.truncate(bytes.len().saturating_sub(2));
            }
        }
        let units = bytes.as_chunks::<2>().0.iter().filter_map(|pair| {
            Some(if little { u16::from_le_bytes([*pair.first()?, *pair.get(1)?]) } else { u16::from_be_bytes([*pair.first()?, *pair.get(1)?]) })
        });
        let text: String = char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
        ("text", json!({"text": text.trim_start_matches('\u{feff}')}))
    } else {
        if let Err(error) = std::str::from_utf8(&bytes)
            && error.error_len().is_none()
            && error.valid_up_to() > 0
        {
            bytes.truncate(error.valid_up_to());
        }
        ("text", json!({"text": String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}')}))
    };
    check_cancelled(cancelled)?;
    let next = offset.saturating_add(bytes.len() as u64);
    let mut result = json!({"path": path, "encoding": encoding, "offsetUnit": "file bytes", "offset": offset, "nextOffset": next, "total": metadata.len(), "hasMore": next < metadata.len()});
    if let (Some(result), Some(content)) = (result.as_object_mut(), content.as_object()) {
        result.extend(content.clone());
    }
    Ok(result)
}

fn text_page(path: &str, text: &str, offset: u64, limit: usize, cancelled: &AtomicBool) -> Result<Value, String> {
    check_cancelled(cancelled)?;
    let start = usize::try_from(offset).ok().filter(|n| *n <= text.len() && text.is_char_boundary(*n)).ok_or("Invalid extracted-text offset")?;
    let mut end = start.saturating_add(limit).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    Ok(
        json!({"path":path,"encoding":"text","offsetUnit":"UTF-8 bytes of extracted text","text":text.get(start..end).unwrap_or_default(),"nextOffset":end,"hasMore":end<text.len(),"total":text.len()}),
    )
}

fn document_text(bytes: &[u8], odt: bool) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "Document package is invalid")?;
    let entry = zip.by_name(if odt { "content.xml" } else { "word/document.xml" }).map_err(|_| "Document has no supported text part")?;
    if entry.size() > 8_000_000 {
        return Err("Document XML exceeds the 8 MB extraction limit".into());
    }
    let mut xml = Vec::new();
    entry.take(8_000_001).read_to_end(&mut xml).map_err(|_| "Could not decode document text")?;
    if xml.len() > 8_000_000 {
        return Err("Document XML exceeds the extraction limit".into());
    }
    let mut reader = quick_xml::Reader::from_reader(std::io::Cursor::new(xml));
    let mut buffer = Vec::new();
    let mut text = String::new();
    loop {
        use quick_xml::events::Event;
        match reader.read_event_into(&mut buffer).map_err(|_| "Document XML is malformed")? {
            Event::Text(t) => {
                let value = t.decode().map_err(|_| "Document text encoding is unsupported")?;
                text.push_str(&value);
            }
            Event::CData(t) => text.push_str(&t.decode().map_err(|_| "Document text encoding is unsupported")?),
            Event::GeneralRef(t) => {
                let name = t.decode().map_err(|_| "Invalid XML text reference")?;
                let c = match name.as_ref() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n => n
                        .strip_prefix("#x")
                        .and_then(|n| u32::from_str_radix(n, 16).ok())
                        .or_else(|| n.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()))
                        .and_then(char::from_u32),
                };
                if let Some(c) = c {
                    text.push(c);
                }
            }
            Event::End(t) if matches!(t.local_name().as_ref(), b"p" | b"h") => text.push('\n'),
            Event::Empty(t) if matches!(t.local_name().as_ref(), b"tab") => text.push('\t'),
            Event::Empty(t) if matches!(t.local_name().as_ref(), b"br" | b"line-break") => text.push('\n'),
            Event::Eof => break,
            _ => {}
        }
        if text.len() > 2_000_000 {
            return Err("Extracted text exceeds the 2 MB limit; use base64 mode".into());
        }
        buffer.clear();
    }
    Ok(text)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) { Err("File read cancelled".into()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn referenced_files_are_authorized_paged_and_never_confused_with_a_directory_grant() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes with spaces.txt");
        std::fs::write(&path, "Café 🦀 notes").unwrap();
        let supplied = path.to_str().unwrap();
        assert!(authorize(supplied, ["No file provided"].into_iter(), None).is_err());
        let message = format!("Read `{supplied}`");
        let allowed = authorize(supplied, [message.as_str()].into_iter(), None).unwrap();
        let cancelled = AtomicBool::new(false);
        let first = read(&allowed, &json!({"limit": 5}), &cancelled).unwrap();
        assert_eq!(first["text"], "Café");
        assert_eq!(first["nextOffset"], 5);
        let next = read(&allowed, &json!({"offset": first["nextOffset"]}), &cancelled).unwrap();
        assert_eq!(next["text"], " 🦀 notes");
        assert_eq!(next["hasMore"], false);
        assert!(read(&allowed, &json!({"offset": u64::MAX}), &cancelled).is_err());
        assert!(read(&allowed, &json!({"offset": -1}), &cancelled).is_err());
        assert!(read(&allowed, &json!({"mode": "shell"}), &cancelled).is_err());
        cancelled.store(true, Ordering::Relaxed);
        assert!(read(&allowed, &json!({}), &cancelled).is_err());
        let folder = temp.path().to_str().unwrap();
        assert!(authorize(folder, [message.as_str()].into_iter(), None).is_err());
        assert!(read(&json!({"path": folder}), &json!({}), &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn binary_utf16_and_rich_documents_are_read_without_a_live_session() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("arbitrary.data");
        std::fs::write(&binary, [0, 255, 17, 128, 9]).unwrap();
        let result = read(&json!({"path": binary}), &json!({}), &AtomicBool::new(false)).unwrap();
        assert_eq!(result["encoding"], "base64");
        assert_eq!(STANDARD.decode(result["data"].as_str().unwrap()).unwrap(), vec![0, 255, 17, 128, 9]);
        let utf16 = temp.path().join("utf16.txt");
        let bytes: Vec<u8> = [0xff, 0xfe].into_iter().chain("Café".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        std::fs::write(&utf16, bytes).unwrap();
        let result = read(&json!({"path": utf16}), &json!({}), &AtomicBool::new(false)).unwrap();
        assert_eq!(result["text"], "Café");
        let native = temp.path().join("example.deckcraft");
        let mut session = deckcraft_engine::Session::with_new();
        session.execute("slide.new", &json!({"title":"Hello presentation"})).unwrap();
        let bytes = deckcraft_engine::cmd::file::save_bytes(&session.active().unwrap().doc, "deckcraft").unwrap();
        std::fs::write(&native, bytes).unwrap();
        let result = read(&json!({"path":native}), &json!({}), &AtomicBool::new(false)).unwrap();
        assert!(result["text"].as_str().unwrap().contains("Hello presentation"));
        let supplied = "notes.txt";
        let message = "Read `notes.txt`";
        let authorization = authorize(supplied, [message].into_iter(), Some(&temp.path().join("document.docx"))).unwrap();
        assert_eq!(PathBuf::from(authorization["path"].as_str().unwrap()), temp.path().join(supplied));
    }
    #[test]
    fn word_and_odt_extract_entities_unicode_and_page_without_opening_live_deck() {
        use std::io::Write;
        for (name, entry, xml) in [
            ("source.docx", "word/document.xml", "<w:document xmlns:w='urn:test'><w:p><w:r><w:t>Café &amp; 🦀</w:t></w:r></w:p></w:document>"),
            ("source.odt", "content.xml", "<office xmlns:text='urn:test'><text:p>Café &amp; 🦀</text:p></office>"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(name);
            let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            archive.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
            archive.write_all(xml.as_bytes()).unwrap();
            archive.finish().unwrap();
            let result = read(&json!({"path":path}), &json!({}), &AtomicBool::new(false)).unwrap();
            assert_eq!(result["text"], "Café & 🦀\n");
            let first = read(&json!({"path":path}), &json!({"limit":5}), &AtomicBool::new(false)).unwrap();
            assert_eq!(first["text"], "Café");
            assert!(read(&json!({"path":path}), &json!({"offset":4}), &AtomicBool::new(false)).is_err());
        }
        assert!(document_text(b"invalid archive", false).is_err());
    }
}
