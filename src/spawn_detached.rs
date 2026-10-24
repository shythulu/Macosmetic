use std::{io, process};

// This code is from the open crate and retains its MIT license.
pub fn spawn_detached(command: &mut process::Command) -> io::Result<()> {
    command
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null());

    // On macOS, launch through posix_spawn in a new process group and reap the child on a
    // thread. The fork-twice path below calls fork() inside pre_exec, in a child of a
    // multithreaded process: macOS's fork handlers take locks there that another thread
    // may have held at the first fork, and the launch can then wait forever, freezing the
    // app. Without pre_exec, std uses posix_spawn, which runs no code in the child.
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt as _;

        let mut child = command.process_group(0).spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    unsafe {
        use std::os::unix::process::CommandExt as _;

        command
            .pre_exec(move || {
                match libc::fork() {
                    -1 => return Err(io::Error::last_os_error()),
                    0 => (),
                    _ => libc::_exit(0),
                }

                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }

                Ok(())
            })
            .spawn()?
            .wait()
            .map(|_| ())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// The command runs, and the call does not wait for it to finish.
    #[test]
    fn runs_the_command_without_waiting_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("ran");
        let mut command = process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!("sleep 1; touch '{}'", marker.display()));
        let start = Instant::now();
        spawn_detached(&mut command).unwrap();
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "spawn_detached waited for the command"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "the command never ran");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
