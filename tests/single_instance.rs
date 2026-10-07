//! End-to-end tests for "one Zima per data folder". They start real copies of Zima in the tray
//! (`--hidden`, so no window opens) against throwaway data folders, and close them afterwards.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Time for a copy to start up and claim its folder.
const STARTUP: Duration = Duration::from_secs(3);

struct DataDir(PathBuf);

impl DataDir {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("zima-instance-test-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A copy of Zima this test started; closed when the test ends, pass or fail.
struct Zima(Child);

impl Zima {
    fn start(data: &DataDir, extra: &[&str]) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_zima"))
            .arg("--hidden")
            .args(extra)
            .env("ZIMA_DATA_DIR", &data.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start zima");
        Self(child)
    }

    /// Whether it's still running after waiting up to `within` for it to exit.
    fn still_running_after(&mut self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.0.try_wait().unwrap().is_some() {
                return false;
            }
            sleep(Duration::from_millis(100));
        }
        self.0.try_wait().unwrap().is_none()
    }

    fn exit_code(&mut self) -> Option<i32> {
        self.0.try_wait().unwrap().and_then(|status| status.code())
    }

    fn stop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Zima {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn second_copy_on_the_same_folder_exits() {
    let data = DataDir::new("same");
    let mut first = Zima::start(&data, &[]);
    assert!(first.still_running_after(STARTUP), "the first copy should keep running");

    let mut second = Zima::start(&data, &[]);
    assert!(!second.still_running_after(Duration::from_secs(10)), "the second copy should hand over and exit");
    assert_eq!(second.exit_code(), Some(0));
    assert!(first.still_running_after(Duration::ZERO), "the first copy must not be affected");
}

#[test]
fn copies_on_different_folders_both_run() {
    let (data_a, data_b) = (DataDir::new("a"), DataDir::new("b"));
    let mut first = Zima::start(&data_a, &[]);
    assert!(first.still_running_after(STARTUP));

    let mut second = Zima::start(&data_b, &[]);
    assert!(second.still_running_after(STARTUP), "a different data folder is a separate Zima");
    assert!(first.still_running_after(Duration::ZERO));
}

#[test]
fn restarted_copy_waits_for_the_old_one_then_takes_over() {
    let data = DataDir::new("restart");
    let mut old = Zima::start(&data, &[]);
    assert!(old.still_running_after(STARTUP));

    // What "Restart now" does: start the new copy while the old one is still quitting.
    let mut new = Zima::start(&data, &["--restarted"]);
    assert!(new.still_running_after(STARTUP), "a restarted copy waits instead of exiting");

    old.stop();
    assert!(new.still_running_after(STARTUP), "once the old copy is gone, the new one carries on");

    // And now the new copy owns the folder: another launch hands over to it.
    let mut another = Zima::start(&data, &[]);
    assert!(!another.still_running_after(Duration::from_secs(10)));
}

#[test]
fn folder_is_free_again_after_zima_quits() {
    let data = DataDir::new("free");
    let mut first = Zima::start(&data, &[]);
    assert!(first.still_running_after(STARTUP));
    first.stop();

    let mut next = Zima::start(&data, &[]);
    assert!(next.still_running_after(STARTUP), "a stale lock must not block the next start");
}
