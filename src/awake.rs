//! Keep the computer from going to sleep while a file transfer runs.
//!
//! Windows is asked directly (`SetThreadExecutionState`). Linux takes a
//! sleep inhibitor through `systemd-inhibit` when it is there; elsewhere
//! this does nothing.

/// Holds the computer awake while it lives.
pub struct Awake {
    #[cfg(target_os = "linux")]
    child: Option<std::process::Child>,
}

impl Awake {
    pub fn start() -> Self {
        #[cfg(windows)]
        {
            // ES_CONTINUOUS | ES_SYSTEM_REQUIRED: stay awake until changed.
            unsafe {
                SetThreadExecutionState(0x8000_0000 | 0x0000_0001);
            }
            Self {}
        }
        #[cfg(target_os = "linux")]
        {
            let child = std::process::Command::new("systemd-inhibit")
                .args([
                    "--what=sleep",
                    "--who=OpenIRC",
                    "--why=A file transfer is running",
                    "--mode=block",
                    "sleep",
                    "infinity",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            Self { child }
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Self {}
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            // ES_CONTINUOUS alone: sleep is allowed again.
            SetThreadExecutionState(0x8000_0000);
        }
        #[cfg(target_os = "linux")]
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetThreadExecutionState(flags: u32) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_and_dropping_is_safe() {
        let guard = Awake::start();
        drop(guard);
    }
}
