//! Frames on a Spaces stream: u32 length (of type + body), u8 type, body. Max 1 MiB.
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::limits::MAX_FRAME_BYTES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame { pub kind: u8, pub body: Vec<u8> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError { TooBig, Empty, Io }

pub async fn write_frame<W: AsyncWrite + Unpin + ?Sized>(w: &mut W, kind: u8, body: &[u8]) -> Result<(), FrameError> {
    let len = body.len() + 1;
    if len > MAX_FRAME_BYTES { return Err(FrameError::TooBig); }
    let mut buf = Vec::with_capacity(4 + len);
    buf.extend_from_slice(&(len as u32).to_be_bytes());
    buf.push(kind);
    buf.extend_from_slice(body);
    w.write_all(&buf).await.map_err(|_| FrameError::Io)?;
    w.flush().await.map_err(|_| FrameError::Io)
}

pub async fn read_frame<R: AsyncRead + Unpin + ?Sized>(r: &mut R) -> Result<Frame, FrameError> {
    read_frame_max(r, MAX_FRAME_BYTES).await
}

/// Like `read_frame`, with a smaller cap (the first frame of an unknown peer). Checked before allocating.
pub async fn read_frame_max<R: AsyncRead + Unpin + ?Sized>(r: &mut R, max: usize) -> Result<Frame, FrameError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(|_| FrameError::Io)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 { return Err(FrameError::Empty); }
    if len > max.min(MAX_FRAME_BYTES) { return Err(FrameError::TooBig); } // checked BEFORE allocating
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.map_err(|_| FrameError::Io)?;
    Ok(Frame { kind: buf[0], body: buf[1..].to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_frame(&mut a, 7, b"hi").await.unwrap();
        assert_eq!(read_frame(&mut b).await.unwrap(), Frame { kind: 7, body: b"hi".to_vec() });
    }
    #[tokio::test]
    async fn oversized_length_is_refused_without_allocating() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&(u32::MAX).to_be_bytes()).await.unwrap();
        assert_eq!(read_frame(&mut b).await, Err(FrameError::TooBig));
    }
    #[tokio::test]
    async fn a_smaller_cap_refuses_before_reading_the_body() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&(16 * 1024 + 2u32).to_be_bytes()).await.unwrap();
        assert_eq!(read_frame_max(&mut b, 16 * 1024).await, Err(FrameError::TooBig));
        let (mut a, mut b) = tokio::io::duplex(64);
        write_frame(&mut a, 7, b"hi").await.unwrap();
        assert_eq!(read_frame_max(&mut b, 16 * 1024).await.unwrap().body, b"hi".to_vec());
    }
    #[tokio::test]
    async fn zero_length_is_refused() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&0u32.to_be_bytes()).await.unwrap();
        assert_eq!(read_frame(&mut b).await, Err(FrameError::Empty));
    }
}
