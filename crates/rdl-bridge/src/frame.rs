//! Length-prefixed frames, as in native messaging: a little-endian `u32`
//! byte count, then the message.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest message either side accepts. A page's cookies and a few
/// hundred links fit easily; anything bigger is a mistake or an attack.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// Read one frame. `None` means the other side closed the connection
/// cleanly, between frames. An empty or oversized frame is `InvalidData`.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let first = reader.read(&mut len).await?;
    if first == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut len[first..]).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message of {len} bytes (allowed: 1 to {MAX_MESSAGE})"),
        ));
    }
    let mut message = vec![0u8; len];
    reader.read_exact(&mut message).await?;
    Ok(Some(message))
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, message: &[u8]) -> io::Result<()> {
    if message.is_empty() || message.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("message of {} bytes is not allowed", message.len()),
        ));
    }
    writer
        .write_all(&(message.len() as u32).to_le_bytes())
        .await?;
    writer.write_all(message).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            write_frame(&mut a, b"{\"type\":\"ping\"}").await.unwrap();
            write_frame(&mut a, &[b'x'; 200]).await.unwrap();
        });
        assert_eq!(
            read_frame(&mut b).await.unwrap().as_deref(),
            Some(&b"{\"type\":\"ping\"}"[..])
        );
        assert_eq!(read_frame(&mut b).await.unwrap().unwrap().len(), 200);
        writer.await.unwrap();
        // The writer is gone: a clean end.
        assert_eq!(read_frame(&mut b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn bad_lengths_are_refused() {
        for len in [0u32, MAX_MESSAGE as u32 + 1] {
            let bytes = len.to_le_bytes();
            let err = read_frame(&mut &bytes[..]).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        }
        // Cut off in the middle of a message.
        let mut bytes = 10u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(b"abc");
        let err = read_frame(&mut &bytes[..]).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);

        let mut sink = Vec::new();
        assert!(write_frame(&mut sink, b"").await.is_err());
    }
}
