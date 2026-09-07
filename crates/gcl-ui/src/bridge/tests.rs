//! Unit tests for the job thread. No Slint instance and no display needed.

use super::{error_chain, spawn_job};

#[test]
fn spawn_job_delivers_ok() {
    let rx = spawn_job(|| Ok(21 * 2));
    let value = rx.recv().expect("result").expect("the job succeeded");
    assert_eq!(value, 42);
}

#[test]
fn spawn_job_delivers_err() {
    let rx = spawn_job(|| Err::<(), _>(gcl_core::Error::Io(std::io::Error::other("disk on fire"))));
    let err = rx.recv().expect("result").expect_err("the job failed");
    assert!(err.to_string().contains("disk on fire"));
}

#[test]
fn error_chain_joins_sources() {
    #[derive(Debug)]
    struct Outer(std::io::Error);
    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("could not read config")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let err = Outer(std::io::Error::other("permission denied"));
    assert_eq!(
        error_chain(&err),
        "could not read config: permission denied"
    );
}
