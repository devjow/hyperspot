#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod client;
pub mod error;
pub mod models;

pub use client::MiniChatClientV1;
pub use error::MiniChatError;
pub use models::{Chat, ChatPatch, NewChat};
