//! Multipart byteranges streaming for HTTP 206 responses.
//!
//! Handles streaming of multi-range responses according to RFC 7233.

use godot::classes::FileAccess;
use godot::classes::file_access::ModeFlags;
use godot::prelude::*;

use super::range::ByteRange;

pub(crate) const MULTIPART_BOUNDARY: &str = "godot_cef_multipart_boundary";

#[derive(Clone, Debug)]
pub(crate) struct MultipartStreamState {
    pub ranges: Vec<ByteRange>,
    pub current_range_index: usize,
    pub current_range_offset: u64,
    pub phase: MultipartPhase,
    pub phase_offset: usize,
    pub total_size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MultipartPhase {
    Header,
    Data,
    TrailingCrlf,
    FinalBoundary,
    Complete,
}

impl MultipartStreamState {
    pub fn new(ranges: Vec<ByteRange>, mime_type: &str, file_size: u64) -> Self {
        let total_size = calculate_multipart_size(&ranges, mime_type, file_size);
        Self {
            ranges,
            current_range_index: 0,
            current_range_offset: 0,
            phase: MultipartPhase::Header,
            phase_offset: 0,
            total_size,
        }
    }

    pub fn build_current_header(&self, mime_type: &str, file_size: u64) -> String {
        if self.current_range_index >= self.ranges.len() {
            return String::new();
        }
        let range = &self.ranges[self.current_range_index];
        format!(
            "--{}\r\nContent-Type: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
            MULTIPART_BOUNDARY, mime_type, range.start, range.end, file_size
        )
    }

    pub fn final_boundary() -> Vec<u8> {
        format!("--{}--\r\n", MULTIPART_BOUNDARY).into_bytes()
    }
}

pub(crate) fn calculate_multipart_size(
    ranges: &[ByteRange],
    mime_type: &str,
    file_size: u64,
) -> u64 {
    let mut total: u64 = 0;

    for range in ranges {
        let header = format!(
            "--{}\r\nContent-Type: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
            MULTIPART_BOUNDARY, mime_type, range.start, range.end, file_size
        );
        total = total.saturating_add(header.len() as u64);
        total = total.saturating_add(range.end - range.start + 1);
        total = total.saturating_add(2); // CRLF
    }

    total = total.saturating_add(2 + MULTIPART_BOUNDARY.len() as u64 + 2 + 2); // "--" + boundary + "--" + "\r\n"

    total
}

pub(crate) fn read_multipart_streaming(
    stream: &mut MultipartStreamState,
    file_path: &str,
    mime_type: &str,
    file_size: u64,
    open_file: &mut Option<Gd<FileAccess>>,
    data_out: *mut u8,
    bytes_to_read: usize,
) -> usize {
    let mut written = 0usize;
    let mut out_ptr = data_out;

    while written < bytes_to_read {
        match stream.phase {
            MultipartPhase::Complete => break,

            MultipartPhase::Header => {
                let header = stream.build_current_header(mime_type, file_size);
                let header_bytes = header.as_bytes();
                let remaining_header = header_bytes.len().saturating_sub(stream.phase_offset);

                if remaining_header == 0 {
                    // Header fully sent, move to data phase
                    stream.phase = MultipartPhase::Data;
                    stream.phase_offset = 0;
                    continue;
                }

                let to_copy = (bytes_to_read - written).min(remaining_header);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        header_bytes.as_ptr().add(stream.phase_offset),
                        out_ptr,
                        to_copy,
                    );
                    out_ptr = out_ptr.add(to_copy);
                }
                written += to_copy;
                stream.phase_offset += to_copy;
            }

            MultipartPhase::Data => {
                if stream.current_range_index >= stream.ranges.len() {
                    stream.phase = MultipartPhase::FinalBoundary;
                    stream.phase_offset = 0;
                    continue;
                }

                let range = &stream.ranges[stream.current_range_index];
                let range_size = range.end - range.start + 1;
                let remaining_in_range = range_size.saturating_sub(stream.current_range_offset);

                if remaining_in_range == 0 {
                    // Range data fully sent, move to trailing CRLF
                    stream.phase = MultipartPhase::TrailingCrlf;
                    stream.phase_offset = 0;
                    continue;
                }

                if open_file.is_none() {
                    let gstring_path = GString::from(file_path);
                    *open_file = FileAccess::open(&gstring_path, ModeFlags::READ);
                }

                if let Some(file) = open_file.as_mut() {
                    file.seek(range.start + stream.current_range_offset);
                    let to_read = (bytes_to_read - written).min(remaining_in_range as usize);
                    let buffer = file.get_buffer(to_read as i64);
                    let actual_read = buffer.len();

                    if actual_read > 0 {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                buffer.as_slice().as_ptr(),
                                out_ptr,
                                actual_read,
                            );
                            out_ptr = out_ptr.add(actual_read);
                        }
                        written += actual_read;
                        stream.current_range_offset += actual_read as u64;
                    } else {
                        // EOF or error - abort the multipart response to avoid malformed output
                        stream.phase = MultipartPhase::Complete;
                        stream.phase_offset = 0;
                        *open_file = None; // Close file on error
                    }
                } else {
                    // File open failed - abort the multipart response to avoid malformed output
                    stream.phase = MultipartPhase::Complete;
                    stream.phase_offset = 0;
                }
            }

            MultipartPhase::TrailingCrlf => {
                const CRLF: &[u8] = b"\r\n";
                let remaining_crlf = CRLF.len().saturating_sub(stream.phase_offset);

                if remaining_crlf == 0 {
                    // CRLF fully sent, move to next range
                    stream.current_range_index += 1;
                    stream.current_range_offset = 0;

                    if stream.current_range_index >= stream.ranges.len() {
                        stream.phase = MultipartPhase::FinalBoundary;
                    } else {
                        stream.phase = MultipartPhase::Header;
                    }
                    stream.phase_offset = 0;
                    continue;
                }

                let to_copy = (bytes_to_read - written).min(remaining_crlf);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        CRLF.as_ptr().add(stream.phase_offset),
                        out_ptr,
                        to_copy,
                    );
                    out_ptr = out_ptr.add(to_copy);
                }
                written += to_copy;
                stream.phase_offset += to_copy;
            }

            MultipartPhase::FinalBoundary => {
                let final_boundary = MultipartStreamState::final_boundary();
                let remaining_boundary = final_boundary.len().saturating_sub(stream.phase_offset);

                if remaining_boundary == 0 {
                    stream.phase = MultipartPhase::Complete;
                    *open_file = None; // Close file when stream completes
                    continue;
                }

                let to_copy = (bytes_to_read - written).min(remaining_boundary);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        final_boundary.as_ptr().add(stream.phase_offset),
                        out_ptr,
                        to_copy,
                    );
                    out_ptr = out_ptr.add(to_copy);
                }
                written += to_copy;
                stream.phase_offset += to_copy;
            }
        }
    }

    written
}

/// Skip bytes in a multipart streaming response without reading data.
///
/// Advances the stream state by the specified number of bytes, returning
/// the actual number of bytes skipped.
pub(crate) fn skip_multipart_streaming(
    stream: &mut MultipartStreamState,
    mime_type: &str,
    file_size: u64,
    bytes_to_skip: usize,
) -> usize {
    let mut skipped = 0usize;

    while skipped < bytes_to_skip {
        match stream.phase {
            MultipartPhase::Complete => break,

            MultipartPhase::Header => {
                let header = stream.build_current_header(mime_type, file_size);
                let header_len = header.len();
                let remaining_header = header_len.saturating_sub(stream.phase_offset);

                if remaining_header == 0 {
                    stream.phase = MultipartPhase::Data;
                    stream.phase_offset = 0;
                    continue;
                }

                let to_skip = (bytes_to_skip - skipped).min(remaining_header);
                skipped += to_skip;
                stream.phase_offset += to_skip;
            }

            MultipartPhase::Data => {
                if stream.current_range_index >= stream.ranges.len() {
                    stream.phase = MultipartPhase::FinalBoundary;
                    stream.phase_offset = 0;
                    continue;
                }

                let range = &stream.ranges[stream.current_range_index];
                let range_size = range.end - range.start + 1;
                let remaining_in_range = range_size.saturating_sub(stream.current_range_offset);

                if remaining_in_range == 0 {
                    stream.phase = MultipartPhase::TrailingCrlf;
                    stream.phase_offset = 0;
                    continue;
                }

                let to_skip = (bytes_to_skip - skipped).min(remaining_in_range as usize);
                skipped += to_skip;
                stream.current_range_offset += to_skip as u64;
            }

            MultipartPhase::TrailingCrlf => {
                const CRLF_LEN: usize = 2;
                let remaining_crlf = CRLF_LEN.saturating_sub(stream.phase_offset);

                if remaining_crlf == 0 {
                    stream.current_range_index += 1;
                    stream.current_range_offset = 0;

                    if stream.current_range_index >= stream.ranges.len() {
                        stream.phase = MultipartPhase::FinalBoundary;
                    } else {
                        stream.phase = MultipartPhase::Header;
                    }
                    stream.phase_offset = 0;
                    continue;
                }

                let to_skip = (bytes_to_skip - skipped).min(remaining_crlf);
                skipped += to_skip;
                stream.phase_offset += to_skip;
            }

            MultipartPhase::FinalBoundary => {
                let final_boundary = MultipartStreamState::final_boundary();
                let remaining_boundary = final_boundary.len().saturating_sub(stream.phase_offset);

                if remaining_boundary == 0 {
                    stream.phase = MultipartPhase::Complete;
                    continue;
                }

                let to_skip = (bytes_to_skip - skipped).min(remaining_boundary);
                skipped += to_skip;
                stream.phase_offset += to_skip;
            }
        }
    }

    skipped
}
