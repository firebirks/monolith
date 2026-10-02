//! The byte stream a system Tor backend hands out.

use core::pin::Pin;
use core::task::{Context, Poll};
use std::io;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;

/// A stream through Tor: an outbound stream through the SOCKS endpoint,
/// or an inbound stream that Tor delivered to the local listener.
#[derive(Debug)]
pub enum TorStream {
    /// Over TCP.
    Tcp(TcpStream),
    /// Over a Unix socket (a SOCKS endpoint that is a socket).
    #[cfg(unix)]
    Unix(UnixStream),
}

impl AsyncRead for TorStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TorStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

impl TorStream {
    /// Reads what is available without waiting.
    pub(crate) fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.try_read(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.try_read(buf),
        }
    }

    /// Waits until the stream is readable.
    pub(crate) async fn readable(&self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.readable().await,
            #[cfg(unix)]
            Self::Unix(stream) => stream.readable().await,
        }
    }
}

/// Connects to a configured local endpoint. A TCP endpoint is a
/// `SocketAddr` and a socket endpoint a path: there is no name to resolve.
pub(crate) async fn connect(endpoint: &crate::Endpoint) -> io::Result<TorStream> {
    match endpoint {
        crate::Endpoint::Tcp(address) => TcpStream::connect(*address).await.map(TorStream::Tcp),
        #[cfg(unix)]
        crate::Endpoint::Unix(path) => UnixStream::connect(path).await.map(TorStream::Unix),
    }
}
