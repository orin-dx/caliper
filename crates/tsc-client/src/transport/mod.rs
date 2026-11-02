//! The wire format between this crate and `tsc --api`: `Content-Length`-framed messages. The fake server in the tests
//! uses the same encoder and decoder, so the client and the fake cannot drift apart on it.

mod framing;

pub use framing::{check_frame_len, write_frame, ContentLengthDecoder, DEFAULT_MAX_FRAME_BYTES};
