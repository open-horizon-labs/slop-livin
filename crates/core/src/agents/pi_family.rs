//! Neutral JSONL session-header parsing shared by the Pi-family
//! adapters (`crate::agents::pi`, `crate::agents::oh_my_pi`).
//!
//! Oh My Pi is a fork of Pi, so the two tools' session files have
//! overlapping — but *not* identical — on-disk shapes: Oh My Pi's
//! documented format puts a fixed-width title slot ahead of the JSON
//! header line, Pi's own README documents the header at byte offset 0.
//!
//! Neither adapter may reach into the other to borrow that parsing
//! (`.oh/guardrails/agent-adapters-are-pluggable.md`: an adapter names
//! no other adapter, so a change to one tool's format can never silently
//! change another tool's identification). The shared *mechanics* live
//! here instead, and each adapter declares which layouts it is willing
//! to accept, in which order. A file matching none of them stays
//! `unknown-format` — never a guess, and never "whatever the sibling
//! tool does".

use std::fs;
use std::io::Read;
use std::path::Path;

/// How many bytes past any title slot a header read may consume. Header
/// reads are bounded by construction: an adapter never reads past its
/// session's first line, and never reads message bodies at all.
pub const HEADER_READ_BYTES: usize = 8192;

/// Oh My Pi's documented fixed-width title slot.
pub const TITLE_SLOT_BYTES: usize = 256;

/// One on-disk header layout an adapter is willing to accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderLayout {
    /// The JSON header is the file's first line, at byte offset 0.
    OffsetZero,
    /// The JSON header follows a fixed-width title slot.
    AfterTitleSlot,
}

impl HeaderLayout {
    pub fn describe(self) -> &'static str {
        match self {
            Self::OffsetZero => "a JSON header at byte offset 0",
            Self::AfterTitleSlot => "a JSON header after a 256-byte title slot",
        }
    }
}

/// What one bounded header read yielded. Contents beyond these declared
/// fields are never retained: nothing here returns message text.
#[derive(Debug, Default, Clone)]
pub struct SessionHeader {
    pub cwd: Option<String>,
    pub additional_directories: Vec<String>,
    /// Which of the caller's accepted layouts actually matched.
    pub layout: Option<HeaderLayout>,
}

impl SessionHeader {
    pub fn is_empty(&self) -> bool {
        self.cwd.is_none() && self.additional_directories.is_empty()
    }
}

fn parse_line(line: &str) -> Option<SessionHeader> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let cwd = value
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let additional_directories = value
        .get("additionalDirectories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if cwd.is_none() && additional_directories.is_empty() {
        return None;
    }
    Some(SessionHeader {
        cwd,
        additional_directories,
        layout: None,
    })
}

/// Reads `path`'s header once (one bounded read, never the whole file)
/// and tries each of `layouts` in the caller's own order. The adapter
/// decides which layouts its tool documents; this function only knows
/// how to look.
///
/// `counters` records the bytes actually read so the incremental
/// measurement tests can assert "zero header reads on an unchanged
/// home".
pub fn read_header(path: &Path, layouts: &[HeaderLayout]) -> SessionHeader {
    let Ok(mut f) = fs::File::open(path) else {
        return SessionHeader::default();
    };
    let mut buf = vec![0u8; TITLE_SLOT_BYTES + HEADER_READ_BYTES];
    let Ok(n) = f.read(&mut buf) else {
        return SessionHeader::default();
    };
    buf.truncate(n);
    crate::work_counters::record_header_bytes(n as u64);
    for layout in layouts {
        let slice: &[u8] = match layout {
            HeaderLayout::OffsetZero => &buf,
            HeaderLayout::AfterTitleSlot => {
                if n <= TITLE_SLOT_BYTES {
                    continue;
                }
                &buf[TITLE_SLOT_BYTES..]
            }
        };
        let text = String::from_utf8_lossy(slice);
        let Some(line) = text.lines().next() else {
            continue;
        };
        if let Some(mut header) = parse_line(line) {
            header.layout = Some(*layout);
            return header;
        }
    }
    SessionHeader::default()
}

/// The honest `Unresolved` reason when no accepted layout matched:
/// names every layout that was checked, so "we could not tell" never
/// reads like "there is nothing there".
pub fn no_layout_matched_reason(layouts: &[HeaderLayout]) -> String {
    let checked: Vec<&str> = layouts.iter().map(|l| l.describe()).collect();
    format!(
        "no cwd field found; checked {} (unknown-format for this session)",
        checked.join(" then ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_with(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        fs::write(&p, bytes).unwrap();
        (d, p)
    }

    #[test]
    fn offset_zero_layout_is_read_when_the_adapter_accepts_it() {
        let (_d, p) = tmp_with(br#"{"cwd":"/tmp/x"}"#);
        let h = read_header(&p, &[HeaderLayout::OffsetZero]);
        assert_eq!(h.cwd.as_deref(), Some("/tmp/x"));
        assert_eq!(h.layout, Some(HeaderLayout::OffsetZero));
    }

    #[test]
    fn an_adapter_that_only_accepts_the_title_slot_layout_does_not_read_offset_zero() {
        // The tempting shortcut this rejects: "both tools are similar,
        // so try everything". An adapter gets exactly the layouts its
        // own tool documents; anything else stays unknown-format.
        let (_d, p) = tmp_with(br#"{"cwd":"/tmp/x"}"#);
        let h = read_header(&p, &[HeaderLayout::AfterTitleSlot]);
        assert!(h.is_empty());
        assert_eq!(h.layout, None);
    }

    #[test]
    fn title_slot_layout_skips_the_slot() {
        let mut bytes = vec![b' '; TITLE_SLOT_BYTES];
        bytes.extend_from_slice(br#"{"cwd":"/tmp/y","additionalDirectories":["/tmp/z"]}"#);
        let (_d, p) = tmp_with(&bytes);
        let h = read_header(&p, &[HeaderLayout::AfterTitleSlot]);
        assert_eq!(h.cwd.as_deref(), Some("/tmp/y"));
        assert_eq!(h.additional_directories, vec!["/tmp/z".to_string()]);
    }

    #[test]
    fn nothing_parseable_names_every_layout_it_checked() {
        let (_d, p) = tmp_with(b"not json at all");
        let h = read_header(
            &p,
            &[HeaderLayout::OffsetZero, HeaderLayout::AfterTitleSlot],
        );
        assert!(h.is_empty());
        let reason =
            no_layout_matched_reason(&[HeaderLayout::OffsetZero, HeaderLayout::AfterTitleSlot]);
        assert!(reason.contains("byte offset 0"));
        assert!(reason.contains("title slot"));
        assert!(reason.contains("unknown-format"));
    }
}
