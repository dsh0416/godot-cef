//! HTTP Range header parsing utilities.
//!
//! Supports single ranges ("bytes=start-end", "bytes=start-", "bytes=-suffix_length")
//! and multi-range requests ("bytes=0-100,200-300").

/// Limit to prevent DoS via excessive multipart response generation
pub(crate) const MAX_MULTI_RANGES: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ParsedRanges {
    Single(ByteRange),
    Multi(Vec<ByteRange>),
}

pub(crate) fn parse_single_range(range_spec: &str, file_size: u64) -> Option<ByteRange> {
    // Empty file has no valid byte ranges
    if file_size == 0 {
        return None;
    }

    let parts: Vec<&str> = range_spec.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start_str = parts[0].trim();
    let end_str = parts[1].trim();

    if !start_str.is_empty() {
        // "start-" or "start-end"
        match start_str.parse::<u64>() {
            Ok(start) => {
                if start >= file_size {
                    return None;
                }
                let end = if end_str.is_empty() {
                    file_size - 1
                } else {
                    end_str.parse::<u64>().ok()?.min(file_size - 1)
                };
                if start <= end {
                    Some(ByteRange { start, end })
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    } else if !end_str.is_empty() {
        // "-suffix_length"
        match end_str.parse::<u64>() {
            Ok(suffix_len) if suffix_len > 0 => {
                let start = file_size.saturating_sub(suffix_len);
                Some(ByteRange {
                    start,
                    end: file_size - 1,
                })
            }
            _ => None,
        }
    } else {
        None
    }
}

pub(crate) fn parse_range_header(range_str: &str, file_size: u64) -> Option<ParsedRanges> {
    if range_str.is_empty() || !range_str.starts_with("bytes=") {
        return None;
    }

    let range_part = &range_str[6..];

    if range_part.contains(',') {
        // Multi-range request
        let ranges: Vec<ByteRange> = range_part
            .split(',')
            .filter_map(|spec| parse_single_range(spec.trim(), file_size))
            .collect();

        if ranges.is_empty() {
            None
        } else if ranges.len() == 1 {
            Some(ParsedRanges::Single(ranges.into_iter().next().unwrap()))
        } else if ranges.len() > MAX_MULTI_RANGES {
            None
        } else {
            Some(ParsedRanges::Multi(ranges))
        }
    } else {
        // Single range request
        parse_single_range(range_part, file_size).map(ParsedRanges::Single)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FILE_SIZE: u64 = 1000;

    // Helper to create a single range result
    fn single(start: u64, end: u64) -> Option<ParsedRanges> {
        Some(ParsedRanges::Single(ByteRange { start, end }))
    }

    // Helper to create a multi range result
    fn multi(ranges: Vec<(u64, u64)>) -> Option<ParsedRanges> {
        Some(ParsedRanges::Multi(
            ranges
                .into_iter()
                .map(|(start, end)| ByteRange { start, end })
                .collect(),
        ))
    }

    #[test]
    fn test_range_header_empty() {
        assert_eq!(parse_range_header("", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_no_bytes_prefix() {
        // Invalid: missing "bytes=" prefix
        assert_eq!(parse_range_header("0-100", TEST_FILE_SIZE), None);
        assert_eq!(parse_range_header("range=0-100", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_single_range_start_end() {
        // bytes=start-end
        assert_eq!(
            parse_range_header("bytes=0-100", TEST_FILE_SIZE),
            single(0, 100)
        );
        assert_eq!(
            parse_range_header("bytes=100-200", TEST_FILE_SIZE),
            single(100, 200)
        );
        assert_eq!(
            parse_range_header("bytes=0-999", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=500-999", TEST_FILE_SIZE),
            single(500, 999)
        );
    }

    #[test]
    fn test_range_header_open_ended() {
        // bytes=start- (from start to end of file)
        assert_eq!(
            parse_range_header("bytes=0-", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=100-", TEST_FILE_SIZE),
            single(100, 999)
        );
        assert_eq!(
            parse_range_header("bytes=500-", TEST_FILE_SIZE),
            single(500, 999)
        );
        assert_eq!(
            parse_range_header("bytes=999-", TEST_FILE_SIZE),
            single(999, 999)
        );
    }

    #[test]
    fn test_range_header_suffix_length() {
        // bytes=-suffix_length (last N bytes)
        assert_eq!(
            parse_range_header("bytes=-100", TEST_FILE_SIZE),
            single(900, 999)
        );
        assert_eq!(
            parse_range_header("bytes=-500", TEST_FILE_SIZE),
            single(500, 999)
        );
        assert_eq!(
            parse_range_header("bytes=-1", TEST_FILE_SIZE),
            single(999, 999)
        );

        // Suffix length >= file size should return entire file
        assert_eq!(
            parse_range_header("bytes=-1000", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=-2000", TEST_FILE_SIZE),
            single(0, 999)
        );
    }

    #[test]
    fn test_range_header_suffix_zero() {
        // bytes=-0 is invalid (suffix length must be > 0)
        assert_eq!(parse_range_header("bytes=-0", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_multi_range() {
        // Multi-range requests should now be properly parsed
        assert_eq!(
            parse_range_header("bytes=0-100,200-300", TEST_FILE_SIZE),
            multi(vec![(0, 100), (200, 300)])
        );
        assert_eq!(
            parse_range_header("bytes=0-100,200-300,400-500", TEST_FILE_SIZE),
            multi(vec![(0, 100), (200, 300), (400, 500)])
        );
        assert_eq!(
            parse_range_header("bytes=0-50,100-150,200-250", TEST_FILE_SIZE),
            multi(vec![(0, 50), (100, 150), (200, 250)])
        );
    }

    #[test]
    fn test_range_header_multi_range_with_open_ended() {
        // Multi-range with open-ended ranges
        assert_eq!(
            parse_range_header("bytes=0-100,500-", TEST_FILE_SIZE),
            multi(vec![(0, 100), (500, 999)])
        );
        assert_eq!(
            parse_range_header("bytes=-100,0-50", TEST_FILE_SIZE),
            multi(vec![(900, 999), (0, 50)])
        );
    }

    #[test]
    fn test_range_header_multi_range_with_invalid_parts() {
        // Multi-range with some invalid parts - invalid parts are skipped
        // Only "0-100" is valid, "abc-def" is skipped, result is single range
        assert_eq!(
            parse_range_header("bytes=0-100,abc-def", TEST_FILE_SIZE),
            single(0, 100)
        );

        // All parts invalid
        assert_eq!(
            parse_range_header("bytes=abc-def,xyz-123", TEST_FILE_SIZE),
            None
        );
    }

    #[test]
    fn test_range_header_multi_range_empty_parts() {
        // Edge case: empty parts after comma (invalid parts filtered out)
        // "0-100" valid, empty string invalid, result is single
        assert_eq!(
            parse_range_header("bytes=0-100,", TEST_FILE_SIZE),
            single(0, 100)
        );

        // Leading comma - empty first part filtered out
        assert_eq!(
            parse_range_header("bytes=,0-100", TEST_FILE_SIZE),
            single(0, 100)
        );
    }

    #[test]
    fn test_range_header_multi_range_whitespace() {
        // Whitespace around ranges in multi-range
        assert_eq!(
            parse_range_header("bytes= 0-100 , 200-300 ", TEST_FILE_SIZE),
            multi(vec![(0, 100), (200, 300)])
        );
    }

    #[test]
    fn test_range_header_multi_range_limit() {
        // Exactly at the limit (MAX_MULTI_RANGES = 10) should work
        let at_limit = "bytes=0-10,20-30,40-50,60-70,80-90,100-110,120-130,140-150,160-170,180-190";
        assert_eq!(
            parse_range_header(at_limit, TEST_FILE_SIZE),
            multi(vec![
                (0, 10),
                (20, 30),
                (40, 50),
                (60, 70),
                (80, 90),
                (100, 110),
                (120, 130),
                (140, 150),
                (160, 170),
                (180, 190)
            ])
        );

        // Exceeding the limit should return None (falls back to full file response)
        let over_limit =
            "bytes=0-10,20-30,40-50,60-70,80-90,100-110,120-130,140-150,160-170,180-190,200-210";
        assert_eq!(parse_range_header(over_limit, TEST_FILE_SIZE), None);

        // Many more ranges should also return None
        let many_ranges = (0..100)
            .map(|i| format!("{}-{}", i * 10, i * 10 + 5))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            parse_range_header(&format!("bytes={}", many_ranges), TEST_FILE_SIZE),
            None
        );
    }

    #[test]
    fn test_range_header_whitespace() {
        // Whitespace around numbers should be trimmed
        assert_eq!(
            parse_range_header("bytes= 0 - 100 ", TEST_FILE_SIZE),
            single(0, 100)
        );
        assert_eq!(
            parse_range_header("bytes=  100  -  ", TEST_FILE_SIZE),
            single(100, 999)
        );
        assert_eq!(
            parse_range_header("bytes=  -  100  ", TEST_FILE_SIZE),
            single(900, 999)
        );
    }

    #[test]
    fn test_range_header_invalid_numbers() {
        // Invalid start number
        assert_eq!(parse_range_header("bytes=abc-100", TEST_FILE_SIZE), None);
        assert_eq!(parse_range_header("bytes=-1x-100", TEST_FILE_SIZE), None);

        // Invalid end number (but valid start - end clamped to file size - 1)
        assert_eq!(parse_range_header("bytes=0-abc", TEST_FILE_SIZE), None);

        // Invalid suffix
        assert_eq!(parse_range_header("bytes=-abc", TEST_FILE_SIZE), None);

        // Negative numbers (parsed as invalid)
        assert_eq!(parse_range_header("bytes=--100", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_malformed() {
        // Missing dash
        assert_eq!(parse_range_header("bytes=100", TEST_FILE_SIZE), None);

        // Multiple dashes
        assert_eq!(parse_range_header("bytes=0-100-200", TEST_FILE_SIZE), None);

        // Empty both sides
        assert_eq!(parse_range_header("bytes=-", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_range_clamping() {
        // End value beyond file size should be clamped
        assert_eq!(
            parse_range_header("bytes=0-5000", TEST_FILE_SIZE),
            single(0, 999)
        );
        assert_eq!(
            parse_range_header("bytes=500-2000", TEST_FILE_SIZE),
            single(500, 999)
        );
    }

    #[test]
    fn test_range_header_start_beyond_file() {
        // Start beyond file size is invalid
        assert_eq!(parse_range_header("bytes=1000-2000", TEST_FILE_SIZE), None);
        assert_eq!(parse_range_header("bytes=5000-", TEST_FILE_SIZE), None);
    }

    #[test]
    fn test_range_header_edge_cases() {
        // Very small file (1 byte)
        assert_eq!(parse_range_header("bytes=0-0", 1), single(0, 0));
        assert_eq!(parse_range_header("bytes=0-", 1), single(0, 0));
        assert_eq!(parse_range_header("bytes=-1", 1), single(0, 0));
        assert_eq!(parse_range_header("bytes=1-", 1), None); // start >= file_size

        // Very large numbers
        let large_file: u64 = 10_000_000_000;
        assert_eq!(
            parse_range_header("bytes=0-9999999999", large_file),
            single(0, 9999999999)
        );
        assert_eq!(
            parse_range_header("bytes=5000000000-", large_file),
            single(5000000000, 9999999999)
        );
    }

    #[test]
    fn test_range_header_zero_file_size() {
        // Zero file size - all ranges are invalid since there are no bytes
        assert_eq!(parse_range_header("bytes=0-0", 0), None);
        assert_eq!(parse_range_header("bytes=0-", 0), None);
        assert_eq!(parse_range_header("bytes=-1", 0), None);
        assert_eq!(parse_range_header("bytes=-100", 0), None);

        // Multi-range on empty file
        assert_eq!(parse_range_header("bytes=0-0,1-1", 0), None);
    }

    #[test]
    fn test_range_header_multi_range_many_ranges() {
        // Test with many ranges
        assert_eq!(
            parse_range_header("bytes=0-10,100-110,200-210,300-310,400-410", TEST_FILE_SIZE),
            multi(vec![
                (0, 10),
                (100, 110),
                (200, 210),
                (300, 310),
                (400, 410)
            ])
        );
    }

    #[test]
    fn test_range_header_overlapping_ranges() {
        // Overlapping ranges are allowed per HTTP spec (server can coalesce or serve as-is)
        assert_eq!(
            parse_range_header("bytes=0-100,50-150", TEST_FILE_SIZE),
            multi(vec![(0, 100), (50, 150)])
        );
    }

    #[test]
    fn test_single_range_helper() {
        // Test parse_single_range directly
        assert_eq!(
            parse_single_range("0-100", TEST_FILE_SIZE),
            Some(ByteRange { start: 0, end: 100 })
        );
        assert_eq!(
            parse_single_range("100-", TEST_FILE_SIZE),
            Some(ByteRange {
                start: 100,
                end: 999
            })
        );
        assert_eq!(
            parse_single_range("-100", TEST_FILE_SIZE),
            Some(ByteRange {
                start: 900,
                end: 999
            })
        );
        assert_eq!(parse_single_range("invalid", TEST_FILE_SIZE), None);
        assert_eq!(parse_single_range("", TEST_FILE_SIZE), None);
    }
}
