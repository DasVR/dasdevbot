//! The bundled demo daemon's lifetime and start state (#36 M2, M3).
//!
//! M2: a failed spawn is kept, not discarded. Until the bundled daemon is
//! running, and after it exits, every loopback call is refused with an error
//! the page shows in its banner. The shell never falls back to whatever else
//! answers on 127.0.0.1:8787.
//!
//! M3: on Windows the child goes into a Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` right after spawn. The job handle lives
//! as long as the app process, so when the app exits, crashes or is killed,
//! Windows closes the handle and kills the daemon. If the child can't be put
//! in the job, it is killed and the start counts as failed.
//!
//! This module is compiled in every build so its logic is tested on Linux CI.
//! Only the `demo-daemon` feature installs a supervisor.

#![cfg_attr(not(feature = "demo-daemon"), allow(dead_code))]

use std::process::Child;
use std::sync::{Mutex, OnceLock};

/// Shown when a demo build has no running daemon. The page shows it as the banner.
pub(crate) const NOT_STARTED: &str = "The bundled daemon has not started.";

pub(crate) enum ChildState {
    Running {
        child: Child,
        #[cfg(windows)]
        _job: job::Job,
    },
    Failed(String),
}

pub(crate) struct Supervisor(Mutex<ChildState>);

impl Supervisor {
    /// Keep the spawn result. A spawned child is put in a kill-on-close job
    /// first (Windows); if that fails, the child is killed and the start fails.
    pub(crate) fn from_spawn(spawned: Result<Child, String>) -> Self {
        let state = match spawned {
            Err(err) => ChildState::Failed(start_failed(&err)),
            Ok(child) => contain(child),
        };
        Supervisor(Mutex::new(state))
    }

    /// Ok only while the bundled daemon is still running. A child that has
    /// exited turns the state into `Failed` for good: it is never restarted
    /// behind the user's back, and nothing else on the port is trusted.
    pub(crate) fn check(&self) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| NOT_STARTED.to_string())?;
        let exited = match &mut *state {
            ChildState::Failed(message) => return Err(message.clone()),
            ChildState::Running { child, .. } => match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => format!("The bundled daemon stopped ({status})."),
                Err(err) => format!("The bundled daemon can't be checked: {err}."),
            },
        };
        *state = ChildState::Failed(exited.clone());
        Err(exited)
    }

    /// Clean exit: kill and reap. The job (Windows) covers every other exit.
    pub(crate) fn shutdown(&self) {
        if let Ok(mut state) = self.0.lock() {
            if let ChildState::Running { child, .. } = &mut *state {
                let _ = child.kill();
                let _ = child.wait();
            }
            *state = ChildState::Failed("The bundled daemon was shut down.".into());
        }
    }
}

fn start_failed(err: &str) -> String {
    format!("The bundled daemon did not start: {err}. dasdevbot won't connect to anything else on 127.0.0.1:8787.")
}

#[cfg(windows)]
fn contain(mut child: Child) -> ChildState {
    match job::Job::kill_on_close().and_then(|job| job.assign(&child).map(|()| job)) {
        Ok(job) => ChildState::Running { child, _job: job },
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            ChildState::Failed(start_failed(&format!("job object: {err}")))
        }
    }
}

#[cfg(not(windows))]
fn contain(child: Child) -> ChildState {
    ChildState::Running { child }
}

static SUPERVISOR: OnceLock<Supervisor> = OnceLock::new();

/// Called once from setup in the demo build, before any window loads.
pub(crate) fn install(supervisor: Supervisor) -> &'static Supervisor {
    SUPERVISOR.get_or_init(|| supervisor)
}

pub(crate) fn installed() -> Option<&'static Supervisor> {
    SUPERVISOR.get()
}

/// The gate every loopback call passes first.
pub(crate) fn ready() -> Result<(), String> {
    ready_with(SUPERVISOR.get(), cfg!(feature = "demo-daemon"))
}

/// `bundled`: this build ships its own daemon, so no supervisor means no call.
/// A dev build without the feature talks to the daemon the developer started,
/// and still needs that daemon's proof (daemon_http).
pub(crate) fn ready_with(supervisor: Option<&Supervisor>, bundled: bool) -> Result<(), String> {
    match supervisor {
        Some(supervisor) => supervisor.check(),
        None if bundled => Err(NOT_STARTED.into()),
        None => Ok(()),
    }
}

#[cfg(windows)]
pub(crate) mod job {
    //! A Job Object whose last handle closing kills every process in it.

    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub(crate) struct Job(HANDLE);

    // The handle is only used through these methods and closed once in Drop.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// An unnamed job (no other process can open it by name) with
        /// KILL_ON_JOB_CLOSE set and read back.
        pub(crate) fn kill_on_close() -> Result<Job, String> {
            // SAFETY: null attributes and a null name create a fresh unnamed job.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(format!(
                    "CreateJobObjectW failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let job = Job(handle);
            // SAFETY: a zeroed JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a valid value.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            // SAFETY: the pointer and size describe `info`, which outlives the call.
            let set = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    size,
                )
            };
            if set == 0 {
                return Err(format!(
                    "SetInformationJobObject failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if !job.kills_on_close()? {
                return Err("KILL_ON_JOB_CLOSE did not stick".into());
            }
            Ok(job)
        }

        pub(crate) fn kills_on_close(&self) -> Result<bool, String> {
            // SAFETY: as above; the out pointer and size describe `info`.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            let ok = unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectExtendedLimitInformation,
                    &mut info as *mut _ as *mut core::ffi::c_void,
                    size,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!(
                    "QueryInformationJobObject failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE != 0)
        }

        pub(crate) fn assign(&self, child: &std::process::Child) -> Result<(), String> {
            // SAFETY: both handles are valid for the duration of the call.
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                return Err(format!(
                    "AssignProcessToJobObject failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn spawn(script: &str) -> Child {
        std::process::Command::new("sh")
            .args(["-c", script])
            .spawn()
            .unwrap()
    }

    #[test]
    fn a_failed_spawn_is_kept_and_every_call_is_refused() {
        let supervisor = Supervisor::from_spawn(Err("no such file".into()));
        let first = supervisor.check().unwrap_err();
        assert!(first.contains("did not start: no such file"), "{first}");
        assert!(first.contains("won't connect to anything else"), "{first}");
        assert_eq!(supervisor.check().unwrap_err(), first);
    }

    #[test]
    fn a_demo_build_without_a_supervisor_refuses_and_a_dev_build_passes() {
        assert_eq!(ready_with(None, true), Err(NOT_STARTED.to_string()));
        assert_eq!(ready_with(None, false), Ok(()));
        let failed = Supervisor::from_spawn(Err("x".into()));
        assert!(ready_with(Some(&failed), false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_running_child_passes_and_an_exited_child_fails_for_good() {
        let supervisor = Supervisor::from_spawn(Ok(spawn("sleep 30")));
        assert_eq!(supervisor.check(), Ok(()));
        supervisor.shutdown();
        assert!(supervisor.check().is_err());

        let quick = Supervisor::from_spawn(Ok(spawn("exit 3")));
        let start = std::time::Instant::now();
        let message = loop {
            match quick.check() {
                Ok(()) if start.elapsed() < std::time::Duration::from_secs(5) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Ok(()) => panic!("an exited child still passes"),
                Err(message) => break message,
            }
        };
        assert!(message.contains("stopped"), "{message}");
        assert_eq!(quick.check(), Err(message));
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_kills_and_reaps_the_child() {
        let child = spawn("sleep 30");
        let pid = child.id();
        let supervisor = Supervisor::from_spawn(Ok(child));
        supervisor.shutdown();
        // Reaped: the pid no longer names our child.
        let alive = std::path::Path::new(&format!("/proc/{pid}")).exists()
            && std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|stat| !stat.contains(") Z "))
                .unwrap_or(false);
        assert!(!alive);
    }

    /// The M3 source contract, checked on every target: the Windows path sets
    /// KILL_ON_JOB_CLOSE, reads it back, and kills the child when it can't be
    /// contained.
    #[test]
    fn the_windows_path_uses_a_kill_on_close_job_and_fails_closed() {
        // A Windows checkout may have CRLF line endings.
        let source = include_str!("daemon_child.rs").replace("\r\n", "\n");
        assert!(source.contains(
            "info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE"
        ));
        assert!(source.contains("if !job.kills_on_close()?"));
        assert!(source.contains("CreateJobObjectW(std::ptr::null(), std::ptr::null())"));
        let contain = source.split("#[cfg(windows)]\nfn contain").nth(1).unwrap();
        let contain = contain.split("#[cfg(not(windows))]").next().unwrap();
        assert!(contain.contains("child.kill()"));
        assert!(contain.contains("ChildState::Failed"));
    }

    #[cfg(windows)]
    #[test]
    fn a_job_kills_its_process_when_the_handle_closes() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let job = job::Job::kill_on_close().unwrap();
        assert!(job.kills_on_close().unwrap());
        job.assign(&child).unwrap();
        drop(job);
        let start = std::time::Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "child outlived the job"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
