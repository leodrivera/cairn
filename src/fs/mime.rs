//! A MIME type from the file name's extension only: Cairn never looks inside files to guess.

pub fn guess(name: &str) -> &'static str {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return "application/octet-stream";
    };
    if stem.is_empty() {
        return "application/octet-stream"; // a dotfile like .bashrc has no extension
    }
    match ext.to_ascii_lowercase().as_str() {
        "txt" | "log" | "ini" | "conf" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "ts" | "rs" | "py" | "sh" | "go" | "c" | "h" | "java" | "rb" | "toml" | "yaml" | "yml" => {
            "text/plain"
        }
        "xml" => "application/xml",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_by_extension() {
        assert_eq!(guess("notes.TXT"), "text/plain");
        assert_eq!(guess("photo.JPG"), "image/jpeg");
        assert_eq!(guess("a.svg"), "image/svg+xml");
        assert_eq!(guess("song.flac"), "audio/flac");
        assert_eq!(guess("data.json"), "application/json");
        assert_eq!(guess("archive.tar.gz"), "application/gzip");
        assert_eq!(guess("README"), "application/octet-stream");
        assert_eq!(guess(".bashrc"), "application/octet-stream");
    }
}
