//! Markdown helpers (image syntax + local `file://` URIs).

use std::path::Path;

/// Parse a whole-line Markdown image `![alt](url)`.
pub fn parse_image(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let url = rest.strip_suffix(')')?;
    Some((alt, url))
}

/// Resolve a local image reference to a `file://` URI, or `None` for remote
/// URLs / unknown extensions. Relative paths resolve against `base` (the
/// document's directory).
pub fn image_uri(url: &str, base: Option<&Path>) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return None;
    }
    let path = Path::new(url);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base?.join(path)
    };
    let ext = abs.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    ) {
        return None;
    }
    // Normalize to a file URL: forward slashes, and a leading slash before a
    // Windows drive so `C:\dir\x.png` becomes `file:///C:/dir/x.png`.
    let mut path = abs.display().to_string().replace('\\', "/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    Some(format!("file://{path}"))
}

/// The implicit URI scheme for a Markdown preview of a file in `dir`: a
/// relative image path appended to it becomes that file's `file://` URL.
pub fn dir_uri_scheme(dir: &Path) -> String {
    let mut path = dir.display().to_string().replace('\\', "/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    if !path.ends_with('/') {
        path.push('/');
    }
    format!("file://{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_images_resolve_against_the_document_directory() {
        let scheme = dir_uri_scheme(Path::new("/srv/docs"));
        assert_eq!(scheme, "file:///srv/docs/");
        assert_eq!(format!("{scheme}img/a.png"), "file:///srv/docs/img/a.png");
        assert_eq!(dir_uri_scheme(Path::new("/")), "file:///");
        assert_eq!(dir_uri_scheme(Path::new("C:\\d\\e")), "file:///C:/d/e/");
    }

    #[test]
    fn parses_image_syntax() {
        assert_eq!(parse_image("![alt](img.png)"), Some(("alt", "img.png")));
        assert!(parse_image("text ![a](b)").is_none());
        assert!(parse_image("plain text").is_none());
    }

    #[test]
    fn resolves_local_image() {
        let doc = Path::new("/doc");
        let uri = image_uri("img.png", Some(doc)).unwrap();
        assert!(uri.starts_with("file:///"), "{uri}");
        assert!(uri.ends_with("img.png"), "{uri}");
        assert!(!uri.contains('\\'), "{uri}");
        assert!(image_uri("https://x/y.png", None).is_none());
        assert!(image_uri("script.rs", Some(doc)).is_none());
        // Relative path with no base cannot be resolved.
        assert!(image_uri("img.png", None).is_none());
    }
}
