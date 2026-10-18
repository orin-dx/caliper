//! Content-Length framing, ported from `crates/cli/src/commands/lsp.rs`'s tested
//! sync/server-role design to `tokio_util::codec`'s incremental, client-role shape.
//! Same parsing rules, same EOF-vs-malformed-header distinction; the size cap is
//! still checked before the body is buffered.
//!
//! Decoding goes through a [`Decoder`] (used via `FramedRead`) rather than a hand-
//! rolled read loop: `tokio_util::codec` is the ecosystem-standard way to turn a byte
//! stream into a `Stream` of frames, and it's what `Client`'s reader task and
//! `fake-tsc`'s request loop both build on, so they can't drift on wire format.
//! Writing is a plain async function, and only `Client`'s writer task calls it: one whole frame per queued channel
//! message, so a caller that gives up mid-request can never leave a partial frame on the pipe.

use bytes::BytesMut;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio_util::codec::Decoder;

use crate::error::{MalformedFrameReason, TscClientError};

/// Default cap on a single frame's declared `Content-Length`, matching
/// `crates/cli/src/commands/lsp.rs`'s `MAX_LSP_MESSAGE_BYTES`. `tsc`'s own frames are
/// server-controlled input with no upper bound of their own.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

/// Reject an oversized frame before its receive buffer is grown to hold it.
pub fn check_frame_len(len: usize, cap: usize) -> Result<(), TscClientError> {
    if len > cap {
        return Err(TscClientError::OversizedFrame { declared: len, cap });
    }
    Ok(())
}

/// Decodes Content-Length-framed messages. `decode` is called again as more bytes
/// arrive, so header and body parsing must tolerate being invoked before a full frame
/// — or even a full header — has arrived, returning `Ok(None)` to ask for more.
pub struct ContentLengthDecoder {
    cap: usize,
    /// `Some(len)` once a valid header has been consumed and we're waiting for `len`
    /// more bytes of body. `None` means still scanning for the header terminator.
    pending_body_len: Option<usize>,
}

impl ContentLengthDecoder {
    pub fn new(cap: usize) -> Self {
        Self { cap, pending_body_len: None }
    }

    /// Byte offset just past `\r\n\r\n`, the header block's blank-line terminator, if
    /// it's fully buffered yet. Headers are a handful of bytes, so a full rescan on
    /// every call is not a real cost.
    fn header_end(src: &[u8]) -> Option<usize> {
        src.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
    }

    /// Scans every line in a complete header block for `Content-Length` — like the
    /// sync original, an unrelated header before or after it doesn't confuse this.
    fn parse_content_length(header: &[u8]) -> Result<usize, MalformedFrameReason> {
        let text = std::str::from_utf8(header).map_err(|_| MalformedFrameReason::NoContentLengthHeader)?;
        let mut found = None;
        let mut saw_header_line = false;
        for line in text.split("\r\n") {
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                saw_header_line = true;
                match value.trim().parse() {
                    Ok(len) => found = Some(len),
                    Err(_) => return Err(MalformedFrameReason::UnparsableContentLength),
                }
            }
        }
        found.ok_or({
            if saw_header_line {
                MalformedFrameReason::UnparsableContentLength
            } else {
                MalformedFrameReason::NoContentLengthHeader
            }
        })
    }
}

impl Decoder for ContentLengthDecoder {
    type Item = Vec<u8>;
    type Error = TscClientError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Vec<u8>>, TscClientError> {
        let body_len = match self.pending_body_len {
            Some(len) => len,
            None => {
                let Some(header_end) = Self::header_end(src) else {
                    return Ok(None); // header not fully arrived yet
                };
                let header = src.split_to(header_end);
                let len =
                    Self::parse_content_length(&header).map_err(|reason| TscClientError::MalformedFrame { reason })?;
                check_frame_len(len, self.cap)?;
                self.pending_body_len = Some(len);
                len
            }
        };

        if src.len() < body_len {
            return Ok(None); // body not fully arrived yet
        }

        let body = src.split_to(body_len);
        self.pending_body_len = None;
        Ok(Some(body.to_vec()))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Vec<u8>>, TscClientError> {
        match self.decode(src)? {
            Some(frame) => Ok(Some(frame)),
            None if self.pending_body_len.is_some() => {
                // A header committed us to a body length, but the stream ended before
                // delivering that many bytes.
                Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::TruncatedBody })
            }
            // Stream ended before a full header block (blank-line terminator) ever
            // arrived — zero bytes, or some header bytes with no terminator yet.
            // Both are a normal, expected way for the child to close the connection.
            None => Ok(None),
        }
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, body: &[u8]) -> std::io::Result<()> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(input: &[u8], cap: usize) -> Result<Vec<Vec<u8>>, TscClientError> {
        let mut decoder = ContentLengthDecoder::new(cap);
        let mut buf = BytesMut::from(input);
        let mut frames = Vec::new();
        while let Some(frame) = decoder.decode(&mut buf)? {
            frames.push(frame);
        }
        // Mirrors what `FramedRead` does once the underlying reader hits EOF.
        if let Some(frame) = decoder.decode_eof(&mut buf)? {
            frames.push(frame);
        }
        Ok(frames)
    }

    #[test]
    fn oversized_content_length_is_rejected() {
        let result = check_frame_len(DEFAULT_MAX_FRAME_BYTES + 1, DEFAULT_MAX_FRAME_BYTES);
        assert!(result.is_err(), "expected a Content-Length past the cap to be rejected, got {result:?}");
    }

    #[test]
    fn content_length_at_cap_is_accepted() {
        let result = check_frame_len(DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_FRAME_BYTES);
        assert!(result.is_ok(), "expected a Content-Length exactly at the cap to be accepted, got {result:?}");
    }

    #[test]
    fn an_oversized_declared_length_is_rejected_by_the_decoder() {
        let header = format!("Content-Length: {}\r\n\r\n", DEFAULT_MAX_FRAME_BYTES + 1);
        let result = decode_all(header.as_bytes(), DEFAULT_MAX_FRAME_BYTES);
        assert!(
            matches!(result, Err(TscClientError::OversizedFrame { .. })),
            "expected OversizedFrame, got {result:?}"
        );
    }

    #[test]
    fn true_stream_eof_is_distinct_from_a_malformed_header_block() {
        let result = decode_all(b"", DEFAULT_MAX_FRAME_BYTES);
        assert_eq!(result.unwrap(), Vec::<Vec<u8>>::new(), "expected clean EOF with no frames");
    }

    #[test]
    fn headerless_frame_is_reported_as_malformed_not_eof() {
        let result = decode_all(b"X-Some-Other-Header: value\r\n\r\n", DEFAULT_MAX_FRAME_BYTES);
        assert!(
            matches!(
                result,
                Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::NoContentLengthHeader })
            ),
            "expected NoContentLengthHeader, got {result:?}"
        );
    }

    #[test]
    fn unparsable_content_length_is_reported_as_malformed() {
        let result = decode_all(b"Content-Length: not-a-number\r\n\r\n", DEFAULT_MAX_FRAME_BYTES);
        assert!(
            matches!(
                result,
                Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::UnparsableContentLength })
            ),
            "expected UnparsableContentLength, got {result:?}"
        );
    }

    #[test]
    fn connection_closed_mid_header_before_blank_line_is_treated_as_eof() {
        let result = decode_all(b"Content-Length: 10\r\n", DEFAULT_MAX_FRAME_BYTES);
        assert_eq!(result.unwrap(), Vec::<Vec<u8>>::new(), "expected clean EOF, not an error");
    }

    #[test]
    fn truncated_body_is_reported_as_malformed() {
        let result = decode_all(b"Content-Length: 20\r\n\r\ntoo short", DEFAULT_MAX_FRAME_BYTES);
        assert!(
            matches!(result, Err(TscClientError::MalformedFrame { reason: MalformedFrameReason::TruncatedBody })),
            "expected TruncatedBody, got {result:?}"
        );
    }

    #[test]
    fn a_well_formed_frame_round_trips() {
        let buf = framed(&[br#"{"id":1,"result":null}"#]);
        let frames = decode_all(&buf, DEFAULT_MAX_FRAME_BYTES).unwrap();
        assert_eq!(frames, vec![br#"{"id":1,"result":null}"#.to_vec()]);
    }

    #[test]
    fn two_frames_arriving_in_one_buffer_both_decode() {
        let buf = framed(&[b"first", b"second"]);
        let frames = decode_all(&buf, DEFAULT_MAX_FRAME_BYTES).unwrap();
        assert_eq!(frames, vec![b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn a_frame_split_across_multiple_decode_calls_still_assembles() {
        let full = framed(&[b"hello world"]);

        let mut decoder = ContentLengthDecoder::new(DEFAULT_MAX_FRAME_BYTES);
        let mut buf = BytesMut::new();
        // Feed one byte at a time — decode() must return Ok(None) until the whole
        // frame (header AND body) has arrived, never erroring on a partial header.
        let mut result = None;
        for &byte in &full {
            buf.extend_from_slice(&[byte]);
            result = decoder.decode(&mut buf).unwrap();
            if result.is_some() {
                break;
            }
        }
        assert_eq!(result, Some(b"hello world".to_vec()));
    }

    /// Encodes `bodies` with the real `write_frame`, so decode tests exercise the same
    /// bytes the writer produces. Sync wrapper keeps these tests off `#[tokio::test]`.
    fn framed(bodies: &[&[u8]]) -> Vec<u8> {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let mut buf = Vec::new();
        for body in bodies {
            runtime.block_on(write_frame(&mut buf, body)).unwrap();
        }
        buf
    }
}
