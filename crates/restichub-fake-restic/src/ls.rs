//! The `ls` command: a snapshot message and a stream of node messages (AC-RS-02).
//!
//! The variable `FAKE_RESTIC_LS_COUNT` sets the number of nodes in the listed directory, so a test
//! can stream a million entries. The nodes have the names `file-0000000`, `file-0000001` and so on.

use std::env;
use std::io::{self, BufWriter, Write};

use serde::Serialize;

use crate::cli::Invocation;

const DEFAULT_COUNT: u64 = 3;
const SNAPSHOT_ID: &str = "b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c5b1d2e4f6089a3b7c1d5e9f2a406b8";
const TREE_ID: &str = "5c1d2e4f6089a3b7c1d5e9f2a406b8b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c";
const TIME: &str = "2026-01-01T00:00:00Z";

#[derive(Serialize)]
struct SnapshotMessage {
    time: &'static str,
    tree: &'static str,
    paths: Vec<&'static str>,
    hostname: &'static str,
    username: &'static str,
    id: &'static str,
    short_id: &'static str,
    message_type: &'static str,
    struct_type: &'static str,
}

#[derive(Serialize)]
struct NodeMessage<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    node_type: &'static str,
    path: &'a str,
    uid: u32,
    gid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    mode: u32,
    permissions: &'static str,
    mtime: &'static str,
    message_type: &'static str,
    struct_type: &'static str,
}

fn count() -> u64 {
    env::var("FAKE_RESTIC_LS_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_COUNT)
}

fn write_line<W: Write, T: Serialize>(out: &mut W, value: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *out, value).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

/// Writes the listing of the directory in the second argument (the root by default).
pub fn emit<W: Write>(out: W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    let directory = invocation
        .arguments
        .get(1)
        .map_or("/", String::as_str)
        .trim_end_matches('/');
    let mut out = BufWriter::with_capacity(64 * 1024, out);
    write_line(
        &mut out,
        &SnapshotMessage {
            time: TIME,
            tree: TREE_ID,
            paths: vec!["/data"],
            hostname: "test-host",
            username: "backup",
            id: SNAPSHOT_ID,
            short_id: &SNAPSHOT_ID[..8],
            message_type: "snapshot",
            struct_type: "snapshot",
        },
    )?;
    // restic prints the listed directory first, then its entries.
    if !directory.is_empty() {
        let name = directory.rsplit('/').next().unwrap_or(directory);
        write_line(
            &mut out,
            &NodeMessage {
                name,
                node_type: "dir",
                path: directory,
                uid: 1000,
                gid: 1000,
                size: None,
                mode: 0o20_000_000_755,
                permissions: "drwxr-xr-x",
                mtime: TIME,
                message_type: "node",
                struct_type: "node",
            },
        )?;
    }
    for index in 0..count() {
        let name = format!("file-{index:07}");
        let path = format!("{directory}/{name}");
        write_line(
            &mut out,
            &NodeMessage {
                name: &name,
                node_type: "file",
                path: &path,
                uid: 1000,
                gid: 1000,
                size: Some(index),
                mode: 0o644,
                permissions: "-rw-r--r--",
                mtime: TIME,
                message_type: "node",
                struct_type: "node",
            },
        )?;
    }
    out.flush()
}
