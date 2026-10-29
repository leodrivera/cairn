//! Downloads: ETags, byte ranges, and a stream of the file.

/// `"<size>-<mtime seconds>-<mtime nanoseconds>"`, quotes included; the same value text saves check.
pub fn etag(st: &rustix::fs::Stat) -> String {
    format!("\"{}-{}-{}\"", st.st_size, st.st_mtime, st.st_mtime_nsec)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    Full,
    /// Inclusive.
    Part {
        start: u64,
        end: u64,
    },
}

/// One range only; a header Cairn does not handle (other units, several ranges) means the whole file.
/// `Err` is "416 Range Not Satisfiable".
#[allow(clippy::result_unit_err)]
pub fn parse_range(header: Option<&str>, size: u64) -> Result<ByteRange, ()> {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return Ok(ByteRange::Full);
    };
    if spec.contains(',') {
        return Ok(ByteRange::Full);
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 || size == 0 {
            return Err(());
        }
        (size.saturating_sub(n), size - 1)
    } else {
        let start: u64 = a.parse().map_err(|_| ())?;
        let end = if b.is_empty() {
            size.saturating_sub(1)
        } else {
            b.parse::<u64>()
                .map_err(|_| ())?
                .min(size.saturating_sub(1))
        };
        (start, end)
    };
    if size == 0 || start >= size || start > end {
        return Err(());
    }
    Ok(ByteRange::Part { start, end })
}

/// For `filename*=UTF-8''...` (RFC 5987): everything but unreserved bytes is percent-encoded.
pub fn encode_filename(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 1000), Ok(ByteRange::Full));
        assert_eq!(
            parse_range(Some("bytes=0-"), 1000),
            Ok(ByteRange::Part { start: 0, end: 999 })
        );
        assert_eq!(
            parse_range(Some("bytes=-500"), 1000),
            Ok(ByteRange::Part {
                start: 500,
                end: 999
            })
        );
        assert_eq!(
            parse_range(Some("bytes=10-20"), 1000),
            Ok(ByteRange::Part { start: 10, end: 20 })
        );
        assert_eq!(
            parse_range(Some("bytes=900-2000"), 1000),
            Ok(ByteRange::Part {
                start: 900,
                end: 999
            })
        );
        assert_eq!(parse_range(Some("bytes=1000-"), 1000), Err(()));
        assert_eq!(parse_range(Some("bytes=5-1"), 1000), Err(()));
        assert_eq!(
            parse_range(Some("bytes=0-1,5-6"), 1000),
            Ok(ByteRange::Full)
        );
        assert_eq!(parse_range(Some("bytes=-0"), 1000), Err(()));
        assert_eq!(parse_range(Some("items=0-1"), 1000), Ok(ByteRange::Full));
        assert_eq!(parse_range(Some("bytes=0-"), 0), Err(()));
    }
}
