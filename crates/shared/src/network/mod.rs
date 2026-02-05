// Network module - Async TCP networking
// Rust equivalent of AsyncSocket.hpp and AsyncListener.hpp using Tokio

// The C++ code uses Boost ASIO for async I/O.
// In Rust, we use Tokio which provides the same async I/O model.
// The actual socket handling is done in the realmd crate since
// it's protocol-specific.

/// Re-export tokio networking types for convenience
pub use tokio::net::{TcpListener, TcpStream};
pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
