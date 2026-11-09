//! The files of the admin GUI, embedded in the binary by `build.rs` when the `ui` feature is on (ADR 0032).
//!
//! A request is answered from this fixed set by exact path and never from the filesystem, so a path has nothing to
//! traverse to. With the feature off the set is empty and [`page`] serves a placeholder that says how to build the
//! GUI. Only `humpyard ui` (through `main`) may use this module.

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

/// Whether this build carries the GUI's files.
pub const EMBEDDED: bool = cfg!(feature = "ui");

/// The page shown by a build without the GUI. It loads nothing: the Content-Security-Policy forbids inline script
/// and style, and there is none.
const PLACEHOLDER: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>humpyard admin</title></head><body><h1>The admin GUI is not in this build</h1>\
<p>This binary was built without the <code>ui</code> feature. Release binaries and the container image include \
the GUI. To build it yourself: <code>cd ui &amp;&amp; npm ci --ignore-scripts &amp;&amp; npm run build</code>, \
then <code>cargo build --release --features ui</code>.</p></body></html>";

/// One file to send.
#[derive(Debug, PartialEq, Eq)]
pub struct Asset {
    pub body: &'static [u8],
    pub content_type: &'static str,
    /// A file whose name carries a content hash never changes under that name.
    pub immutable: bool,
}

/// The file at `path` (relative to the GUI's root, no leading slash), if there is one.
#[must_use]
pub fn lookup(path: &str) -> Option<Asset> {
    FILES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(name, body)| Asset {
            body,
            content_type: content_type(name),
            // The build puts every hashed file under `assets/`; `index.html` and the rest are not hashed.
            immutable: name.starts_with("assets/"),
        })
}

/// The GUI's entry page, or the placeholder in a build without it.
#[must_use]
pub fn page() -> Asset {
    lookup("index.html").unwrap_or(Asset {
        body: PLACEHOLDER.as_bytes(),
        content_type: "text/html; charset=utf-8",
        immutable: false,
    })
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(
            content_type("assets/index-1.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type("assets/style-1.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type("favicon.svg"), "image/svg+xml");
        assert_eq!(content_type("assets/x.wasm"), "application/octet-stream");
        assert_eq!(content_type("noextension"), "application/octet-stream");
    }

    #[test]
    fn a_name_that_is_not_in_the_set_is_not_found_whatever_it_looks_like() {
        for name in [
            "",
            "../Cargo.toml",
            "/etc/passwd",
            "assets/",
            "nope.js",
            "index.html/",
        ] {
            assert!(lookup(name).is_none(), "{name:?}");
        }
    }

    #[cfg(not(feature = "ui"))]
    #[test]
    fn a_build_without_the_gui_serves_the_placeholder() {
        assert_eq!(EMBEDDED, cfg!(feature = "ui"));
        assert_eq!(FILES.len(), 0);
        let page = page();
        assert_eq!(page.content_type, "text/html; charset=utf-8");
        assert!(String::from_utf8_lossy(page.body).contains("not in this build"));
        assert!(!page.immutable);
    }

    #[cfg(feature = "ui")]
    #[test]
    fn a_build_with_the_gui_serves_its_index_and_marks_hashed_files_immutable() {
        assert_eq!(EMBEDDED, cfg!(feature = "ui"));
        assert_ne!(FILES.len(), 0);
        let page = page();
        assert!(String::from_utf8_lossy(page.body).contains("id=\"app\""));
        let hashed = FILES
            .iter()
            .find(|(name, _)| name.starts_with("assets/"))
            .unwrap();
        assert!(lookup(hashed.0).unwrap().immutable);
    }
}
