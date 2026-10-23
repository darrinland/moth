//! A real kill test for the library: a separate process writes flames, is killed with SIGKILL
//! in the middle of its writes, and the library must reopen intact with every flame the writer
//! reported as stored, and no partial rows.
//!
//! The writer is this same test binary, run again with an environment variable set. In the
//! normal test run the writer test does nothing.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use moth_core::library::{Library, now};

const WRITER_ENV: &str = "MOTH_KILL_TEST_DIR";
/// Each flame's XML is padded to this size, so a write takes long enough to be interrupted.
const PAD: usize = 64 * 1024;
/// How many committed writes the parent waits for before it kills the writer.
const KILL_AFTER: usize = 300;

fn flame_xml(n: usize) -> String {
    format!(
        "<flame name=\"n{n}\"><!--{}--></flame>",
        "x".repeat(PAD + n % 97)
    )
}

/// The child process. Writes until it is killed; prints one line per committed flame.
#[test]
fn writer_child() {
    let Some(dir) = std::env::var_os(WRITER_ENV) else {
        return;
    };
    let mut lib = Library::open(std::path::Path::new(&dir)).expect("open");
    for n in 0.. {
        lib.record(&flame_xml(n), &format!("n{n}"), None, now())
            .expect("record");
        println!("committed {n}");
    }
}

#[test]
fn a_killed_writer_leaves_an_intact_library() {
    if std::env::var_os(WRITER_ENV).is_some() {
        return; // This process is the writer, not the test.
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args(["--exact", "writer_child", "--nocapture", "--test-threads=1"])
        .env(WRITER_ENV, dir.path())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn writer");

    // Wait until the writer has reported KILL_AFTER committed flames, then kill it mid-write.
    let reader = BufReader::new(child.stdout.take().unwrap());
    let mut reported = 0;
    for line in reader.lines() {
        let line = line.unwrap();
        if line.starts_with("committed ") {
            reported += 1;
            if reported == KILL_AFTER {
                break;
            }
        }
    }
    child.kill().expect("kill writer");
    child.wait().unwrap();
    assert!(reported >= KILL_AFTER, "writer stopped early");

    let lib = Library::open(dir.path()).expect("reopen after the kill");
    assert!(lib.is_intact().unwrap(), "integrity check failed");

    // Every flame the writer reported must be there, whole. The writer may have committed
    // more before it was killed, so the count can be higher.
    let count = lib.len().unwrap() as usize;
    assert!(count >= reported, "{count} stored, {reported} reported");
    for n in 0..reported {
        let stored = lib
            .get(n as i64 + 1)
            .unwrap()
            .unwrap_or_else(|| panic!("flame {n} reported but missing"));
        assert_eq!(stored.xml, flame_xml(n), "flame {n} is damaged");
    }
}
