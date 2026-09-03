//! Crash-resilient ownership for synchronously awaited command process trees.

use std::io;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output, Stdio};

/// A fresh containment scope for one synchronously awaited command tree.
///
/// The platform containment boundary is established before the requested
/// program can create descendants. Attachment or activation failures are
/// fail-closed: the suspended/unexecuted root is killed and reaped before the
/// error is returned.
#[derive(Debug)]
pub struct OwnedSynchronousCommandTree {
    platform: PlatformCommandTreeOwner,
}

impl OwnedSynchronousCommandTree {
    /// Create a fresh containment scope for one command tree.
    ///
    /// # Errors
    ///
    /// Returns the platform error when the containment primitive cannot be
    /// created: a Windows job object, or a Unix lease pipe.
    pub fn new() -> io::Result<Self> {
        Self::new_with_window_visibility(false)
    }

    /// Create a scope whose child has no independent console window on Windows.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn new_hidden() -> io::Result<Self> {
        Self::new_with_window_visibility(true)
    }

    fn new_with_window_visibility(hidden: bool) -> io::Result<Self> {
        Ok(Self {
            platform: PlatformCommandTreeOwner::new(hidden)?,
        })
    }

    /// Spawn `command` inside this scope, contained before it can execute.
    ///
    /// # Errors
    ///
    /// Returns the spawn error, or the attachment error when the process
    /// cannot be joined to the scope. Attachment failures are fail-closed: the
    /// still-suspended root is killed and reaped before the error is returned.
    pub fn spawn(self, command: &mut Command) -> io::Result<OwnedSynchronousChild> {
        self.platform.configure_command(command);
        let mut child = command.spawn()?;
        if let Err(source) = self.platform.activate_child(&child) {
            PlatformCommandTreeOwner::abort_unactivated_child(&mut child);
            return Err(source);
        }

        Ok(OwnedSynchronousChild {
            child,
            owner: Some(self.platform),
            exit_status: None,
        })
    }
}

/// A child process whose descendants remain owned for the child's full scope.
///
/// Waiting for the root process also tears down descendants that outlive it.
/// Dropping a live child performs the same best-effort whole-tree cleanup.
#[derive(Debug)]
pub struct OwnedSynchronousChild {
    child: Child,
    owner: Option<PlatformCommandTreeOwner>,
    exit_status: Option<ExitStatus>,
}

impl OwnedSynchronousChild {
    #[must_use]
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub const fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub const fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    /// Poll for the root's exit without blocking, tearing down the tree once
    /// it has exited.
    ///
    /// # Errors
    ///
    /// Returns the wait error, or the teardown error when the root has exited
    /// but its surviving descendants cannot be terminated.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.exit_status {
            return Ok(Some(status));
        }
        let Some(status) = self.child.try_wait()? else {
            return Ok(None);
        };
        self.finish_tree(status)?;
        Ok(Some(status))
    }

    /// Block until the root exits, then tear down any surviving descendants.
    ///
    /// # Errors
    ///
    /// Returns the wait error, or the teardown error when descendants that
    /// outlived the root cannot be terminated.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        }
        let status = self.child.wait()?;
        self.finish_tree(status)?;
        Ok(status)
    }

    /// Drain both pipes on reader threads, then wait as [`Self::wait`] does.
    ///
    /// # Errors
    ///
    /// Returns the wait or teardown error, or [`io::ErrorKind::Other`] when a
    /// reader thread panicked before it finished draining its stream.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        drop(self.child.stdin.take());
        let stdout = self.child.stdout.take();
        let stderr = self.child.stderr.take();
        let stdout_reader = stdout.map(|mut stdout| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                io::Read::read_to_end(&mut stdout, &mut bytes).map(|_| bytes)
            })
        });
        let stderr_reader = stderr.map(|mut stderr| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                io::Read::read_to_end(&mut stderr, &mut bytes).map(|_| bytes)
            })
        });
        let status = self.wait()?;
        let stdout = join_output_reader(stdout_reader, "stdout")?;
        let stderr = join_output_reader(stderr_reader, "stderr")?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// Terminate the whole tree and reap the root.
    ///
    /// # Errors
    ///
    /// Returns the termination error, including
    /// [`io::ErrorKind::TimedOut`] when the tree does not exit within the
    /// teardown deadline, or the error from reaping the root.
    pub fn terminate_and_wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        }
        let Some(owner) = self.owner.take() else {
            let status = self.child.wait()?;
            self.exit_status = Some(status);
            return Ok(status);
        };
        let status = owner.terminate_tree_and_wait(&mut self.child)?;
        self.exit_status = Some(status);
        Ok(status)
    }

    fn finish_tree(&mut self, status: ExitStatus) -> io::Result<()> {
        let Some(owner) = self.owner.take() else {
            self.exit_status = Some(status);
            return Ok(());
        };
        owner.terminate_remaining_tree(self.child.id())?;
        self.exit_status = Some(status);
        Ok(())
    }
}

impl Drop for OwnedSynchronousChild {
    fn drop(&mut self) {
        if self.owner.is_some() {
            let _ = self.terminate_and_wait();
        }
    }
}

/// Run a command with piped output inside a crash-resilient process scope.
///
/// # Errors
///
/// Returns any error from creating the scope, spawning the command, or
/// draining and waiting for it. See [`OwnedSynchronousCommandTree::new`],
/// [`OwnedSynchronousCommandTree::spawn`], and
/// [`OwnedSynchronousChild::wait_with_output`].
pub fn owned_command_output(command: &mut Command) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    OwnedSynchronousCommandTree::new()?
        .spawn(command)?
        .wait_with_output()
}

fn join_output_reader(
    reader: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>,
    stream: &'static str,
) -> io::Result<Vec<u8>> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    reader.join().map_err(|_| {
        io::Error::other(format!(
            "owned command {stream} reader panicked before completion"
        ))
    })?
}

#[cfg(windows)]
#[derive(Debug)]
struct PlatformCommandTreeOwner {
    job: std::os::windows::io::OwnedHandle,
    hidden: bool,
}

#[cfg(windows)]
impl PlatformCommandTreeOwner {
    fn new(hidden: bool) -> io::Result<Self> {
        use std::mem::size_of;
        use std::os::windows::io::FromRawHandle;
        use windows::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // SAFETY: the returned owned kernel handle is immediately wrapped and
        // all structure sizes/types match the Windows API contract.
        let job = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other)?;
        let job = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(job.0) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` remains live for the call and is passed with its
        // exact Windows structure size.
        unsafe {
            SetInformationJobObject(
                Self::raw_job(&job),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast::<core::ffi::c_void>(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .map_err(io::Error::other)?,
            )
        }
        .map_err(io::Error::other)?;

        Ok(Self { job, hidden })
    }

    fn raw_job(job: &std::os::windows::io::OwnedHandle) -> windows::Win32::Foundation::HANDLE {
        use std::os::windows::io::AsRawHandle;

        windows::Win32::Foundation::HANDLE(job.as_raw_handle())
    }

    fn configure_command(&self, command: &mut Command) {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};

        // The root cannot execute or create descendants before job assignment.
        let flags = CREATE_SUSPENDED.0 | if self.hidden { CREATE_NO_WINDOW.0 } else { 0 };
        command.creation_flags(flags);
    }

    fn activate_child(&self, child: &Child) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;

        let process = HANDLE(child.as_raw_handle());
        // SAFETY: `process` is the live suspended child and `job` is owned by
        // this scope for the child's entire lifetime.
        unsafe { AssignProcessToJobObject(Self::raw_job(&self.job), process) }
            .map_err(io::Error::other)?;
        resume_suspended_process(child.id())
    }

    fn abort_unactivated_child(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn terminate_tree_and_wait(self, child: &mut Child) -> io::Result<ExitStatus> {
        let tree_result = self.terminate_remaining_tree(child.id());
        let child_result = child.wait();
        tree_result?;
        child_result
    }

    fn terminate_remaining_tree(self, _root_pid: u32) -> io::Result<()> {
        use windows::Win32::Foundation::{WAIT_FAILED, WAIT_TIMEOUT};
        use windows::Win32::System::JobObjects::TerminateJobObject;
        use windows::Win32::System::Threading::WaitForSingleObject;

        // SAFETY: the owned job remains valid until this method returns.
        unsafe { TerminateJobObject(Self::raw_job(&self.job), 1) }.map_err(io::Error::other)?;
        let wait = unsafe { WaitForSingleObject(Self::raw_job(&self.job), 5_000) };
        if wait == WAIT_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for owned command process tree to terminate",
            ));
        }
        if wait == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    // CREATE_SUSPENDED guarantees the root has only its primary thread and has
    // not executed user code, making the documented ToolHelp snapshot stable.
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(io::Error::other)?;
    let snapshot = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(snapshot.0) };
    let snapshot_handle = PlatformCommandTreeOwner::raw_job(&snapshot);
    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(size_of::<THREADENTRY32>()).map_err(io::Error::other)?,
        ..THREADENTRY32::default()
    };
    unsafe { Thread32First(snapshot_handle, &raw mut entry) }.map_err(io::Error::other)?;
    let mut resumed = 0usize;
    loop {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) }
                .map_err(io::Error::other)?;
            let thread = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(thread.0) };
            let previous_suspend_count =
                unsafe { ResumeThread(PlatformCommandTreeOwner::raw_job(&thread)) };
            if previous_suspend_count == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            resumed += 1;
        }
        if unsafe { Thread32Next(snapshot_handle, &raw mut entry) }.is_err() {
            break;
        }
    }
    if resumed != 1 {
        return Err(io::Error::other(format!(
            "expected one suspended primary thread for process {process_id}, found {resumed}"
        )));
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Debug)]
struct PlatformCommandTreeOwner {
    lease_write: std::os::fd::OwnedFd,
    lease_read: std::os::fd::OwnedFd,
}

#[cfg(unix)]
impl PlatformCommandTreeOwner {
    fn new(_hidden: bool) -> io::Result<Self> {
        use std::os::fd::FromRawFd;

        let mut fds = [-1; 2];
        // SAFETY: `fds` points to storage for the two descriptors returned by
        // pipe2. Each successful descriptor is immediately wrapped exactly once.
        if create_cloexec_pipe(&mut fds) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            lease_read: unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[0]) },
            lease_write: unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[1]) },
        })
    }

    fn configure_command(&self, command: &mut Command) {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;

        let lease_read = self.lease_read.as_raw_fd();
        let lease_write = self.lease_write.as_raw_fd();
        // SAFETY: after `fork`, the closure only invokes async-signal-safe libc
        // operations. The watcher never returns into Rust or runs destructors;
        // the command branch returns normally so std can perform `exec`.
        unsafe {
            command.pre_exec(move || {
                let command_pid = libc::getpid();
                if libc::setpgid(0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                let watcher_pid = libc::fork();
                if watcher_pid < 0 {
                    return Err(io::Error::last_os_error());
                }
                if watcher_pid == 0 {
                    libc::close(lease_write);
                    close_watcher_descriptors_except(lease_read);
                    libc::raise(libc::SIGSTOP);
                    run_owner_lease_watcher(lease_read, command_pid);
                }
                let mut watcher_status = 0;
                if libc::waitpid(watcher_pid, &mut watcher_status, libc::WUNTRACED) != watcher_pid {
                    let source = io::Error::last_os_error();
                    abort_watcher(watcher_pid);
                    return Err(source);
                }
                if !libc::WIFSTOPPED(watcher_status) {
                    abort_watcher(watcher_pid);
                    return Err(io::Error::from_raw_os_error(libc::ECHILD));
                }
                if libc::setpgid(watcher_pid, watcher_pid) != 0 {
                    let source = io::Error::last_os_error();
                    abort_watcher(watcher_pid);
                    return Err(source);
                }
                if libc::kill(watcher_pid, libc::SIGCONT) != 0 {
                    let source = io::Error::last_os_error();
                    abort_watcher(watcher_pid);
                    return Err(source);
                }
                Ok(())
            });
        }
    }

    fn activate_child(&self, _child: &Child) -> io::Result<()> {
        Ok(())
    }

    fn abort_unactivated_child(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn terminate_tree_and_wait(self, child: &mut Child) -> io::Result<ExitStatus> {
        terminate_unix_process_group_and_wait(child)?;
        child.wait()
    }

    fn terminate_remaining_tree(self, root_pid: u32) -> io::Result<()> {
        let _ = signal_unix_process_group(root_pid, libc::SIGKILL)?;
        Ok(())
    }
}

#[cfg(unix)]
const OWNER_LEASE_WATCH_BLOCKING_TIMEOUT: libc::c_int = -1;

#[cfg(unix)]
unsafe fn run_owner_lease_watcher(lease_read: libc::c_int, command_pid: libc::pid_t) -> ! {
    let mut poll_fd = libc::pollfd {
        fd: lease_read,
        events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
        revents: 0,
    };
    loop {
        let poll_result =
            unsafe { libc::poll(&mut poll_fd, 1, OWNER_LEASE_WATCH_BLOCKING_TIMEOUT) };
        if poll_result > 0 && poll_fd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            unsafe { terminate_watched_group_and_exit(command_pid, 0) };
        }
        if poll_result < 0 {
            // With one valid descriptor and no deadline, EINTR is the only
            // expected syscall failure. Retrying every error is deliberately
            // fail-closed: an unexpected kernel failure keeps the watcher alive
            // instead of letting the command tree escape.
            continue;
        }
    }
}

#[cfg(unix)]
unsafe fn abort_watcher(watcher_pid: libc::pid_t) {
    unsafe {
        libc::kill(watcher_pid, libc::SIGKILL);
        libc::waitpid(watcher_pid, core::ptr::null_mut(), 0);
    }
}

#[cfg(unix)]
unsafe fn close_watcher_descriptors_except(lease_read: libc::c_int) {
    #[cfg(target_os = "linux")]
    {
        let before_ok = lease_read == 0
            || unsafe { libc::syscall(libc::SYS_close_range, 0u32, lease_read as u32 - 1, 0u32) }
                == 0;
        let after_ok = lease_read == libc::c_int::MAX
            || unsafe {
                libc::syscall(libc::SYS_close_range, lease_read as u32 + 1, u32::MAX, 0u32)
            } == 0;
        if before_ok && after_ok {
            return;
        }
    }

    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let upper = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
        limit.rlim_cur.min(libc::c_int::MAX as libc::rlim_t) as libc::c_int
    } else {
        1_024
    };
    for fd in 0..upper {
        if fd != lease_read {
            unsafe { libc::close(fd) };
        }
    }
}

#[cfg(unix)]
unsafe fn terminate_watched_group_and_exit(command_pid: libc::pid_t, code: libc::c_int) -> ! {
    #[cfg(target_os = "linux")]
    let exit_fd = unsafe { libc::syscall(libc::SYS_pidfd_open, command_pid, 0) as libc::c_int };
    unsafe { libc::kill(-command_pid, libc::SIGTERM) };
    #[cfg(target_os = "linux")]
    let exited = if exit_fd >= 0 {
        let mut event = libc::pollfd {
            fd: exit_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut event, 1, 250) };
        unsafe { libc::close(exit_fd) };
        result > 0
    } else {
        false
    };
    #[cfg(not(target_os = "linux"))]
    let exited = false;
    if !exited {
        unsafe { libc::kill(-command_pid, libc::SIGKILL) };
    }
    unsafe { libc::_exit(code) }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn create_cloexec_pipe(fds: &mut [libc::c_int; 2]) -> libc::c_int {
    // SAFETY: `fds` has exactly the two writable descriptor slots required by
    // pipe2; successful descriptors are wrapped by the caller.
    unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn create_cloexec_pipe(fds: &mut [libc::c_int; 2]) -> libc::c_int {
    // SAFETY: the pipe descriptors are local and not exposed before both
    // close-on-exec flags are installed.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return -1;
    }
    for fd in *fds {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            unsafe {
                libc::close(fds[0]);
                libc::close(fds[1]);
            }
            return -1;
        }
    }
    0
}

#[cfg(unix)]
const SIGTERM_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(unix)]
fn signal_unix_process_group(pid: u32, signal: libc::c_int) -> io::Result<bool> {
    let process_group = -(pid as libc::pid_t);
    // SAFETY: a negative pid targets only the process group created for this
    // owned command; ESRCH means the group is already gone.
    if unsafe { libc::kill(process_group, signal) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn terminate_unix_process_group_and_wait(child: &Child) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let exit = bind_linux_process_exit(child.id())?;
    if !signal_unix_process_group(child.id(), libc::SIGTERM)? {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if let Some(exit) = exit {
        if exit.wait_until(std::time::Instant::now() + SIGTERM_GRACE)? {
            return Ok(());
        }
    }
    let _ = signal_unix_process_group(child.id(), libc::SIGKILL)?;
    Ok(())
}

#[cfg(target_os = "linux")]
struct LinuxProcessExit(std::os::fd::OwnedFd);

#[cfg(target_os = "linux")]
impl LinuxProcessExit {
    fn wait_until(&self, deadline: std::time::Instant) -> io::Result<bool> {
        use std::os::fd::AsRawFd;

        let mut event = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as libc::c_int;
            let result = unsafe { libc::poll(&mut event, 1, timeout_ms.max(1)) };
            if result > 0 {
                return Ok(true);
            }
            if result == 0 {
                return Ok(false);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn bind_linux_process_exit(pid: u32) -> io::Result<Option<LinuxProcessExit>> {
    use std::os::fd::FromRawFd;

    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as libc::c_int;
    if fd >= 0 {
        return Ok(Some(LinuxProcessExit(unsafe {
            std::os::fd::OwnedFd::from_raw_fd(fd)
        })));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(None)
    } else {
        Err(error)
    }
}

#[cfg(not(any(windows, unix)))]
#[derive(Debug)]
struct PlatformCommandTreeOwner;

#[cfg(not(any(windows, unix)))]
impl PlatformCommandTreeOwner {
    fn new(_hidden: bool) -> io::Result<Self> {
        Ok(Self)
    }

    fn configure_command(&self, _command: &mut Command) {}

    fn activate_child(&self, _child: &Child) -> io::Result<()> {
        Ok(())
    }

    fn abort_unactivated_child(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn terminate_tree_and_wait(self, child: &mut Child) -> io::Result<ExitStatus> {
        child.kill()?;
        child.wait()
    }

    fn terminate_remaining_tree(self, _root_pid: u32) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARD_OWNER_PID_FILE: &str = "AZ_WORK_HARD_OWNER_PID_FILE";

    #[cfg(unix)]
    #[test]
    fn owner_lease_watcher_has_no_polling_deadline() {
        assert_eq!(OWNER_LEASE_WATCH_BLOCKING_TIMEOUT, -1);
    }

    #[test]
    fn completed_command_wait_is_idempotent() {
        let mut command = successful_command();
        let owner = OwnedSynchronousCommandTree::new().unwrap();
        let mut child = owner.spawn(&mut command).unwrap();

        let first = child.wait().unwrap();
        let second = child.wait().unwrap();

        assert!(first.success());
        assert_eq!(first.code(), second.code());
    }

    #[test]
    fn owned_output_captures_both_streams() {
        let mut command = output_command();
        let output = owned_command_output(&mut command).unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "out");
        assert_eq!(String::from_utf8(output.stderr).unwrap().trim(), "err");
    }

    #[test]
    fn hard_owner_death_reaps_descendant() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pid_file = std::env::temp_dir().join(format!(
            "az-work-hard-owner-{}-{nonce}.pid",
            std::process::id()
        ));
        let mut owner = Command::new(std::env::current_exe().unwrap());
        owner
            .args([
                "--exact",
                "process::tests::hard_owner_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HARD_OWNER_PID_FILE, &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut owner = owner.spawn().unwrap();

        let descendant_pid = (0..200)
            .find_map(|_| {
                let pid = std::fs::read_to_string(&pid_file)
                    .ok()
                    .and_then(|pid| pid.trim().parse::<u32>().ok());
                if pid.is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                pid
            })
            .expect("helper must publish its descendant pid");
        assert!(process_is_alive(descendant_pid));

        owner.kill().unwrap();
        owner.wait().unwrap();
        let reaped = (0..200).any(|_| {
            if process_is_alive(descendant_pid) {
                std::thread::sleep(std::time::Duration::from_millis(25));
                false
            } else {
                true
            }
        });
        let _ = std::fs::remove_file(&pid_file);
        assert!(
            reaped,
            "descendant {descendant_pid} survived hard owner termination"
        );
    }

    #[test]
    #[ignore = "launched as a subprocess by hard_owner_death_reaps_descendant"]
    fn hard_owner_helper() {
        let Some(pid_file) = std::env::var_os(HARD_OWNER_PID_FILE) else {
            return;
        };
        let mut command = descendant_command(std::path::Path::new(&pid_file));
        OwnedSynchronousCommandTree::new()
            .unwrap()
            .spawn(&mut command)
            .unwrap()
            .wait()
            .unwrap();
    }

    #[cfg(windows)]
    fn successful_command() -> Command {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "exit", "0"]);
        command
    }

    #[cfg(windows)]
    fn output_command() -> Command {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/C", "echo out & echo err 1>&2"]);
        command
    }

    #[cfg(windows)]
    fn descendant_command(pid_file: &std::path::Path) -> Command {
        let pid_file = pid_file.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$PID | Set-Content -NoNewline -LiteralPath '{pid_file}'; Start-Sleep -Seconds 30"
        );
        let mut command = Command::new("cmd.exe");
        command.args([
            "/D",
            "/C",
            "powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]);
        command
    }

    #[cfg(windows)]
    fn process_is_alive(pid: u32) -> bool {
        use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        let mut exit_code = 0;
        let alive = unsafe { GetExitCodeProcess(handle, &raw mut exit_code).is_ok() }
            && exit_code == STILL_ACTIVE.0 as u32;
        let _ = unsafe { CloseHandle(handle) };
        alive
    }

    #[cfg(unix)]
    fn successful_command() -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);
        command
    }

    #[cfg(unix)]
    fn output_command() -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", "printf out; printf err >&2"]);
        command
    }

    #[cfg(unix)]
    fn descendant_command(pid_file: &std::path::Path) -> Command {
        let pid_file = pid_file.to_string_lossy().replace('\'', "'\\''");
        let mut command = Command::new("sh");
        command.args(["-c", &format!("sleep 30 & echo $! > '{pid_file}'; wait")]);
        command
    }

    #[cfg(unix)]
    fn process_is_alive(pid: u32) -> bool {
        // SAFETY: signal zero performs an existence/permission check only.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(not(any(windows, unix)))]
    fn successful_command() -> Command {
        Command::new("true")
    }

    #[cfg(not(any(windows, unix)))]
    fn output_command() -> Command {
        Command::new("true")
    }

    #[cfg(not(any(windows, unix)))]
    fn descendant_command(_pid_file: &std::path::Path) -> Command {
        Command::new("true")
    }

    #[cfg(not(any(windows, unix)))]
    fn process_is_alive(_pid: u32) -> bool {
        false
    }
}
