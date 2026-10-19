use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use super::{Cancellation, Error};

const POLL: Duration = Duration::from_millis(5);

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn read_bounded(
    reader: impl Read,
    limit: usize,
    overflow: &AtomicBool,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        overflow.store(true, Ordering::Relaxed);
    }
    Ok(bytes)
}

pub(super) fn run(
    command: &mut Command,
    output_limit: usize,
    deadline: Instant,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, Error> {
    cancellation.check()?;
    if Instant::now() >= deadline {
        return Err(Error::Timeout);
    }
    let mut child = Running(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Error::Unavailable,
                _ => Error::Io(error),
            })?,
    );
    let stdout = child.0.stdout.take().expect("piped stdout");
    let stderr = child.0.stderr.take().expect("piped stderr");
    let overflow = AtomicBool::new(false);
    thread::scope(|scope| {
        let out = scope.spawn(|| read_bounded(stdout, output_limit, &overflow));
        let err = scope.spawn(|| read_bounded(stderr, 16 * 1024, &overflow));
        let result = loop {
            if let Err(error) = cancellation.check() {
                break Err(error);
            }
            if overflow.load(Ordering::Relaxed) {
                break Err(Error::Limit);
            }
            if Instant::now() >= deadline {
                break Err(Error::Timeout);
            }
            match child.0.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(Error::Failed)
                    };
                }
                Ok(None) => thread::sleep(POLL),
                Err(error) => break Err(Error::Io(error)),
            }
        };
        drop(child);
        let stdout = out.join().map_err(|_| Error::Failed)??;
        let _stderr = err.join().map_err(|_| Error::Failed)??;
        // The process may have exited before the readers signalled overflow.
        if overflow.load(Ordering::Relaxed) {
            return Err(Error::Limit);
        }
        result?;
        Ok(stdout)
    })
}
