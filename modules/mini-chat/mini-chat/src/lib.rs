#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

// === PUBLIC API (from SDK) ===
pub use mini_chat_sdk::{Chat, ChatPatch, MiniChatClientV1, MiniChatError, NewChat};

// === MODULE DEFINITION ===
pub mod module;
pub use module::MiniChatModule;

// === INTERNAL MODULES ===
#[doc(hidden)]
pub mod api;
#[doc(hidden)]
pub mod config;
#[doc(hidden)]
pub mod domain;
#[doc(hidden)]
pub mod infra;
