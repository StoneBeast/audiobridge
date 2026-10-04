//! # audiobridge-core
//!
//! AudioBridge 的平台无关核心库：线协议（handshake/帧）、抖动缓冲、
//! 有界块队列与 PCM 辅助工具。
//!
//! 设计约束：
//! - 仅使用 `std`，不依赖任何音频/平台 API，可在 Windows / Android / 测试环境一致运行；
//! - 线协议字节布局在 [`hello`] 与 [`framing`] 中实现，配套的一致性测试向量
//!   位于仓库根目录 `protocol/conformance-vectors.json`（Rust 与 Kotlin 两侧共用）。

pub mod codec;
pub mod error;
pub mod framing;
pub mod hello;
pub mod jitter;
pub mod pcm;
pub mod queue;
pub mod session;

pub use codec::Codec;
pub use error::{AckStatus, Error, Result};
pub use hello::{Hello, HelloAck, PROTOCOL_VERSION};
pub use jitter::{JitterBuffer, JitterStats};
pub use queue::ChunkQueue;
