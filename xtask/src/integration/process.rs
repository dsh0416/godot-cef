//! Bounded process supervision for Godot and its CEF subprocesses.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, PipeReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const LOG_LIMIT: usize = 8 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Outcome {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub process_error: Option<String>,
    pub output_overflow: bool,
    pub duration_ms: u64,
    #[serde(skip)]
    pub log: String,
}

impl Outcome {
    fn error(&mut self, message: impl std::fmt::Display) {
        match &mut self.process_error {
            Some(previous) => {
                previous.push_str("; ");
                previous.push_str(&message.to_string());
            }
            None => self.process_error = Some(message.to_string()),
        }
    }
}

/// Run a command with merged output and a deadline, then terminate remaining
/// descendants even after a successful parent exit. Launch/supervision errors
/// are reported in the outcome; errors writing the evidence file are returned.
pub(super) fn run(
    command: &mut Command,
    log_path: &Path,
    timeout: Duration,
) -> io::Result<Outcome> {
    // Fail before launching if evidence cannot be written.
    let mut log_file = File::create(log_path)?;
    let started = Instant::now();
    let mut outcome = Outcome::default();
    let mut output = VecDeque::new();
    if let Err(error) = supervise(command, timeout, started, &mut outcome, &mut output) {
        outcome.error(error);
    }
    outcome.log = String::from_utf8_lossy(output.make_contiguous()).into_owned();
    // Invalid UTF-8 can expand during replacement. Bound the decoded log too.
    if outcome.log.len() > LOG_LIMIT {
        let mut start = outcome.log.len() - LOG_LIMIT;
        while !outcome.log.is_char_boundary(start) {
            start += 1;
        }
        outcome.log.drain(..start);
        outcome.output_overflow = true;
    }
    log_file.write_all(outcome.log.as_bytes())?;
    outcome.duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    Ok(outcome)
}

fn supervise(
    command: &mut Command,
    timeout: Duration,
    started: Instant,
    outcome: &mut Outcome,
    output: &mut VecDeque<u8>,
) -> io::Result<()> {
    let (mut reader, writer) = io::pipe()?;
    platform::prepare_pipe(&reader)?;
    let tree = platform::Tree::prepare(command)?;
    command
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    let spawned = command.spawn();
    // Command retains its Stdio handles after spawn. Release both writer copies
    // now so the caller's Command cannot prevent EOF in our reader.
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut process = ManagedProcess {
        child: spawned?,
        tree,
        terminated: false,
    };
    let mut cleanup_started = None;
    if let Err(error) = process.tree.start(&process.child) {
        outcome.error(format!("Cannot supervise process tree: {error}"));
        process.terminate(outcome);
        cleanup_started = Some(Instant::now());
    }
    let mut status = None;
    let mut pipe_closed = false;
    let mut wait_failed = false;
    loop {
        let mut received_output = false;
        if !pipe_closed {
            match drain(&mut reader, output, &mut outcome.output_overflow) {
                Ok((closed, received)) => {
                    pipe_closed = closed;
                    received_output = received;
                }
                Err(error) => {
                    outcome.error(format!("Reading process output: {error}"));
                    pipe_closed = true;
                }
            }
        }
        if status.is_none() && !wait_failed {
            match process.child.try_wait() {
                Ok(value) => status = value,
                Err(error) => {
                    outcome.error(format!("Waiting for process: {error}"));
                    wait_failed = true;
                }
            }
        }
        if cleanup_started.is_none() {
            if status.is_none() && started.elapsed() >= timeout {
                outcome.timed_out = true;
            }
            if status.is_some() || outcome.timed_out || outcome.process_error.is_some() {
                // Parent exit is separate from pipe EOF: a surviving helper may
                // still own stdout. Kill its group/job before waiting for EOF.
                process.terminate(outcome);
                cleanup_started = Some(Instant::now());
            }
        }
        if status.is_some() && pipe_closed {
            break;
        }
        if cleanup_started.is_some_and(|time| time.elapsed() >= CLEANUP_TIMEOUT) {
            outcome.error("Process cleanup exceeded its deadline");
            // Never join a blocking reader: dropping this pipe also handles a
            // descendant that escaped its Unix group and retained stdout.
            break;
        }
        if !received_output {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
    if let Some(status) = status {
        outcome.exit_code = status.code();
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            outcome.signal = status.signal();
        }
    }
    Ok(())
}

/// Drain a bounded batch so a continuously noisy process cannot starve its
/// timeout or status check. Both streams share this pipe, preserving byte order.
fn drain(
    reader: &mut PipeReader,
    output: &mut VecDeque<u8>,
    overflow: &mut bool,
) -> io::Result<(bool, bool)> {
    let mut buffer = [0; 16 * 1024];
    let mut received = false;
    for _ in 0..16 {
        match platform::read_ready(reader, &mut buffer) {
            Ok(0) => return Ok((true, received)),
            Ok(length) => {
                received = true;
                let excess = (output.len() + length).saturating_sub(LOG_LIMIT);
                if excess > 0 {
                    output.drain(..excess);
                    *overflow = true;
                }
                output.extend(&buffer[..length]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok((false, received))
}

struct ManagedProcess {
    child: Child,
    tree: platform::Tree,
    terminated: bool,
}

impl ManagedProcess {
    fn terminate(&mut self, outcome: &mut Outcome) {
        if let Err(error) = self.tree.terminate() {
            outcome.error(format!("Terminating process tree: {error}"));
        }
        // Also covers a Windows child that failed assignment to its Job.
        let _ = self.child.kill();
        self.terminated = true;
    }
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self.tree.terminate();
            let _ = self.child.kill();
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    pub(super) struct Tree(Option<libc::pid_t>);

    impl Tree {
        pub(super) fn prepare(command: &mut Command) -> io::Result<Self> {
            command.process_group(0);
            Ok(Self(None))
        }

        pub(super) fn start(&mut self, child: &Child) -> io::Result<()> {
            self.0 = Some(child.id() as libc::pid_t);
            Ok(())
        }

        pub(super) fn terminate(&self) -> io::Result<()> {
            if let Some(group) = self.0 {
                // SAFETY: a negative PID addresses only the group created for
                // this child. No pointer arguments are involved.
                if unsafe { libc::kill(-group, libc::SIGKILL) } != 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error);
                    }
                }
            }
            Ok(())
        }
    }

    pub(super) fn prepare_pipe(reader: &PipeReader) -> io::Result<()> {
        // SAFETY: reader owns this live descriptor; fcntl changes its flags
        // without transferring ownership or accessing any pointers.
        unsafe {
            let flags = libc::fcntl(reader.as_raw_fd(), libc::F_GETFL);
            if flags == -1
                || libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) == -1
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub(super) fn read_ready(reader: &mut PipeReader, buffer: &mut [u8]) -> io::Result<usize> {
        reader.read(buffer)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::io::Read;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    pub(super) struct Tree(OwnedHandle);

    impl Tree {
        pub(super) fn prepare(command: &mut Command) -> io::Result<Self> {
            // Suspend before any child code runs: assigning an already running
            // launcher to a Job races with it spawning its own descendants.
            command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
            // SAFETY: null pointers request an unnamed, non-inherited Job.
            let raw = unsafe { CreateJobObjectW(null(), null()) };
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: CreateJobObjectW returned a unique valid handle.
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            // SAFETY: all-zero limit fields are valid; only LimitFlags is set.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: pointer and size describe the initialized limits value.
            if unsafe {
                SetInformationJobObject(
                    handle.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(handle))
        }

        pub(super) fn start(&mut self, child: &Child) -> io::Result<()> {
            // SAFETY: both handles remain owned and valid for this call.
            if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) }
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            resume_initial_thread(child.id())
        }

        pub(super) fn terminate(&self) -> io::Result<()> {
            // SAFETY: the owned Job handle remains valid for this call.
            if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    fn resume_initial_thread(pid: u32) -> io::Result<()> {
        // Command retains the process handle but closes the primary thread
        // handle. The suspended child has exactly one thread to find and resume.
        // SAFETY: this snapshot API takes no pointer arguments.
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful snapshot creation transfers one owned handle.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: THREADENTRY32 supports zero initialization plus dwSize.
        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        // SAFETY: the live snapshot and output structure have correct types.
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        while found {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: the thread ID comes from the snapshot of our child.
                let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: OpenThread returned a unique valid handle.
                let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
                // SAFETY: thread remains live and has suspend/resume rights.
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            // SAFETY: same valid snapshot and output structure as above.
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        Err(io::Error::other(
            "Cannot find suspended child's initial thread",
        ))
    }

    pub(super) fn prepare_pipe(_reader: &PipeReader) -> io::Result<()> {
        Ok(())
    }

    pub(super) fn read_ready(reader: &mut PipeReader, buffer: &mut [u8]) -> io::Result<usize> {
        let mut available = 0;
        // SAFETY: only the available-byte count is requested; reader owns the
        // handle and this supervisor is its sole reader, so the following read
        // cannot lose those bytes to another consumer and unexpectedly block.
        if unsafe {
            PeekNamedPipe(
                reader.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                return Ok(0);
            }
            return Err(error);
        }
        if available == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let length = buffer.len().min(available as usize);
        reader.read(&mut buffer[..length])
    }
}

#[cfg(test)]
// Test assertions panic deliberately; Result propagates fixture I/O failures.
#[allow(clippy::panic_in_result_fn)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    const FIXTURE: &str = "integration::process::tests::child_fixture";
    const MODE: &str = "GDCEF_PROCESS_TEST_MODE";
    const DIRECTORY: &str = "GDCEF_PROCESS_TEST_DIRECTORY";
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "gdcef-process-test-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn command(&self, mode: &str) -> io::Result<Command> {
            let mut command = fixture_command(mode)?;
            command.env(DIRECTORY, &self.0);
            Ok(command)
        }

        fn run(&self, mode: &str, timeout: Duration) -> io::Result<Outcome> {
            run(
                &mut self.command(mode)?,
                &self.0.join("output.log"),
                timeout,
            )
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_command(mode: &str) -> io::Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--exact", FIXTURE, "--ignored", "--nocapture"])
            .env(MODE, mode);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        Ok(command)
    }

    #[test]
    fn captures_both_streams_and_exit_code() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let outcome = directory.run("success", Duration::from_secs(10))?;
        assert_eq!(outcome.exit_code, Some(0), "{outcome:?}");
        assert!(!outcome.timed_out);
        assert!(outcome.process_error.is_none(), "{outcome:?}");
        assert!(outcome.log.contains("stdout marker\nstderr marker"));
        assert_eq!(
            std::fs::read_to_string(directory.0.join("output.log"))?,
            outcome.log
        );
        let outcome = directory.run("failure", Duration::from_secs(10))?;
        assert_eq!(outcome.exit_code, Some(23), "{outcome:?}");
        assert!(outcome.process_error.is_none());
        let assessment = super::super::report::assess(
            &outcome,
            Some(super::super::report::Case {
                class: "CefTexture",
                case: "js_ipc",
            }),
        );
        assert!(
            assessment.report.is_some(),
            "fixture must report success before exiting"
        );
        assert!(
            !assessment.passed,
            "a success report must not hide an abnormal exit"
        );
        Ok(())
    }

    #[test]
    fn reports_start_failure_and_writes_evidence() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let log = directory.0.join("output.log");
        let outcome = run(
            &mut Command::new(directory.0.join("nonexistent-program")),
            &log,
            Duration::from_secs(1),
        )?;
        assert!(outcome.process_error.is_some());
        assert!(outcome.exit_code.is_none());
        assert!(!outcome.timed_out);
        assert!(log.exists());
        Ok(())
    }

    #[test]
    fn bounds_output_and_keeps_tail() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let outcome = directory.run("overflow", Duration::from_secs(15))?;
        assert_eq!(outcome.exit_code, Some(0));
        assert!(
            outcome.process_error.is_none(),
            "{:?}",
            outcome.process_error
        );
        assert!(outcome.output_overflow);
        assert_eq!(outcome.log.len(), LOG_LIMIT);
        assert!(outcome.log.ends_with("tail marker\n"));
        Ok(())
    }

    #[test]
    fn noisy_output_cannot_starve_deadline() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let started = Instant::now();
        let outcome = directory.run("noisy", Duration::from_secs(1))?;
        assert!(outcome.timed_out);
        assert!(
            outcome.process_error.is_none(),
            "{:?}",
            outcome.process_error
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(outcome.log.len() <= LOG_LIMIT);
        Ok(())
    }

    #[test]
    fn timeout_terminates_descendants() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let started = Instant::now();
        let outcome = directory.run("tree", Duration::from_secs(3))?;
        assert!(outcome.timed_out, "{outcome:?}");
        assert!(outcome.process_error.is_none(), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(7));
        assert_descendant_stopped(&directory.0.join("branch.pid"))?;
        assert_descendant_stopped(&directory.0.join("leaf.pid"))?;
        Ok(())
    }

    #[test]
    fn successful_parent_cannot_leave_a_pipe_holding_orphan() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let started = Instant::now();
        let outcome = directory.run("orphan", Duration::from_secs(10))?;
        assert_eq!(outcome.exit_code, Some(0), "{outcome:?}");
        assert!(!outcome.timed_out);
        assert!(outcome.process_error.is_none(), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_descendant_stopped(&directory.0.join("leaf.pid"))?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn preserves_termination_signal() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let outcome = directory.run("signal", Duration::from_secs(10))?;
        assert_eq!(outcome.exit_code, None);
        assert_eq!(outcome.signal, Some(libc::SIGTERM));
        assert!(outcome.process_error.is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn escaped_pipe_writer_cannot_block_cleanup_forever() -> io::Result<()> {
        let directory = TestDirectory::new()?;
        let started = Instant::now();
        let outcome = directory.run("escaped", Duration::from_secs(10))?;
        let pid: i32 = std::fs::read_to_string(directory.0.join("leaf.pid"))?
            .parse()
            .map_err(io::Error::other)?;
        // The fixture deliberately escaped the supervisor's process group.
        // Clean it up before assertions, including when the assertions fail.
        // SAFETY: the PID was recorded by this test's isolated descendant.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        assert_eq!(outcome.exit_code, Some(0));
        assert!(!outcome.timed_out);
        assert!(
            outcome
                .process_error
                .as_deref()
                .is_some_and(|error| { error.contains("Process cleanup exceeded its deadline") })
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        Ok(())
    }

    fn assert_descendant_stopped(path: &Path) -> io::Result<()> {
        let pid: u32 = std::fs::read_to_string(path)?
            .parse()
            .map_err(io::Error::other)?;
        let started = Instant::now();
        while process_running(pid)? && started.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(POLL_INTERVAL);
        }
        assert!(!process_running(pid)?, "descendant {pid} is still running");
        Ok(())
    }

    #[cfg(unix)]
    fn process_running(pid: u32) -> io::Result<bool> {
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            && stat
                .rsplit_once(") ")
                .is_some_and(|(_, rest)| rest.starts_with('Z'))
        {
            // An orphan awaiting init's waitpid has stopped executing.
            return Ok(false);
        }
        // SAFETY: signal zero only probes existence; there are no pointers.
        if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        Err(error)
    }

    #[cfg(windows)]
    fn process_running(pid: u32) -> io::Result<bool> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, WAIT_FAILED, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        // SAFETY: request only synchronization access to this child PID.
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if raw.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(false)
            } else {
                Err(error)
            };
        }
        // SAFETY: OpenProcess returned a unique valid handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: live handle, zero timeout; no pointer arguments.
        match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
            WAIT_FAILED => Err(io::Error::last_os_error()),
            WAIT_TIMEOUT => Ok(true),
            _ => Ok(false),
        }
    }

    fn wait_for_file(path: &Path) -> io::Result<()> {
        let started = Instant::now();
        while !path.exists() {
            if started.elapsed() > Duration::from_secs(10) {
                return Err(io::Error::other("fixture child did not become ready"));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(())
    }

    fn publish_pid(path: &Path) -> io::Result<()> {
        // Existence is the fixture's readiness signal. Publish only after the
        // PID is fully written, so killing a ready child cannot leave an empty
        // file between create and write.
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, std::process::id().to_string())?;
        std::fs::rename(temporary, path)
    }

    #[test]
    #[ignore = "subprocess fixture, invoked explicitly by process supervision tests"]
    fn child_fixture() -> io::Result<()> {
        let Ok(mode) = std::env::var(MODE) else {
            return Ok(());
        };
        let directory = PathBuf::from(std::env::var_os(DIRECTORY).unwrap_or_default());
        match mode.as_str() {
            "success" | "failure" => {
                io::stdout().write_all(b"stdout marker\n")?;
                io::stderr().write_all(b"stderr marker\n")?;
                if mode == "failure" {
                    io::stdout().write_all(b"GDCEF_ITEST_RESULT {\"class\":\"CefTexture\",\"case\":\"js_ipc\",\"passed\":true,\"checks\":3,\"failures\":[]}\n")?;
                }
                std::process::exit(if mode == "success" { 0 } else { 23 });
            }
            "overflow" => {
                io::stdout().write_all(&vec![b'x'; LOG_LIMIT + 64 * 1024])?;
                io::stderr().write_all(b"tail marker\n")?;
                std::process::exit(0);
            }
            "noisy" => {
                let started = Instant::now();
                while started.elapsed() < Duration::from_secs(20) {
                    io::stdout().write_all(&[b'x'; 16 * 1024])?;
                }
            }
            "tree" | "branch" | "orphan" | "escaped" => {
                let next = if mode == "tree" { "branch" } else { "leaf" };
                if mode == "branch" {
                    publish_pid(&directory.join("branch.pid"))?;
                }
                // Intentionally retain inherited output and do not wait. The
                // supervisor must clean up the complete descendant tree.
                let mut child_command = fixture_command(next)?;
                #[cfg(unix)]
                if mode == "escaped" {
                    use std::os::unix::process::CommandExt;
                    child_command.process_group(0);
                }
                let _child = child_command.spawn()?;
                wait_for_file(&directory.join("leaf.pid"))?;
                if mode == "orphan" || mode == "escaped" {
                    std::process::exit(0);
                }
                std::thread::sleep(Duration::from_secs(20));
            }
            "leaf" => {
                publish_pid(&directory.join("leaf.pid"))?;
                std::thread::sleep(Duration::from_secs(20));
            }
            #[cfg(unix)]
            "signal" => {
                // SAFETY: this isolated child deliberately terminates itself.
                unsafe { libc::raise(libc::SIGTERM) };
            }
            _ => return Err(io::Error::other("unknown fixture mode")),
        }
        Ok(())
    }
}
