//! threadmail: static-site comments backed by a mailing list, not a
//! database. See the README for the full architecture; in short, every
//! comment is a real RFC 5322 message, threading comes from
//! `In-Reply-To`/`References`, and a thread is resolved fresh from the
//! mailbox on every request — the mailing list is the only system of
//! record, nothing is persisted on this side.

pub mod compose;
pub mod config;
pub mod imap_source;
pub mod mail;
pub mod render;
pub mod smtp_sink;
pub mod source;
pub mod thread;
pub mod web;
