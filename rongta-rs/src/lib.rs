//! Portable Rongta R22 protocol, image processing, and printer sessions.
//!
//! The caller supplies a transport implementing Tokio's `AsyncRead` and
//! `AsyncWrite`, plus the handshake nonce. Async sessions require a Tokio
//! runtime with time enabled. Portable Bluetooth discovery traits and printer
//! detection live in [`bluetooth`]; platform backends, transport setup, randomness,
//! and persisted CLI settings belong to `rongta-cli`.
//!
//! Framing, handshake state, media geometry, and image conversion can also be
//! used directly without running an async session.

pub mod bluetooth;
pub mod client;
pub mod handshake;
pub mod image_data;
pub mod image_processing;
pub mod media;
pub mod monochrome;
pub mod printing;
pub mod protocol;
pub mod raster;
