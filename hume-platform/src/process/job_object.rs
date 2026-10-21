//! Windows Job Objects: a `Job` holds a child and every process it starts,
//! so a kill reaches the child's own children the way a process-group kill
//! does on Unix. Each job is created kill-on-close: closing its last handle,
//! which a force-exit's `std::process::exit` also does, ends every process
//! in it.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt as _;
use std::process::{Child, Command};

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};
use windows_sys::core::BOOL;

pub(crate) struct Job(OwnedHandle);

impl Job {
    /// Spawns `command` in a new job. The child starts suspended and runs
    /// only once it is in the job, so no process it starts escapes the job.
    /// On any failure after the spawn the child is killed and reaped.
    pub(crate) fn spawn(command: &mut Command) -> io::Result<(Child, Job)> {
        let mut child = command.creation_flags(CREATE_SUSPENDED).spawn()?;
        match Self::adopt(&child) {
            Ok(job) => Ok((child, job)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    /// Ends every process in the job.
    pub(crate) fn terminate(&self) -> io::Result<()> {
        // SAFETY: the job handle is open for as long as `self` lives.
        succeeded(unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) })
    }

    /// A new kill-on-close job holding `child`, which is then resumed.
    fn adopt(child: &Child) -> io::Result<Job> {
        // SAFETY: both arguments may be null: default security, no name.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a handle this function just opened, owned by
        // nothing else.
        let job = Job(unsafe { OwnedHandle::from_raw_handle(raw) });

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is the structure `JobObjectExtendedLimitInformation`
        // names, the length is its size, and it outlives the call.
        succeeded(unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of_u32::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
            )
        })?;
        // SAFETY: both handles are open for the call.
        succeeded(unsafe {
            AssignProcessToJobObject(job.0.as_raw_handle(), child.as_raw_handle())
        })?;
        resume_threads(child.id())?;
        Ok(job)
    }
}

/// Resumes every thread of process `pid`, created suspended with its main
/// thread as its only one. `Err` when it has none.
fn resume_threads(pid: u32) -> io::Result<()> {
    // SAFETY: no pointer arguments.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a handle this function just opened, owned by nothing
    // else.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = THREADENTRY32 {
        dwSize: size_of_u32::<THREADENTRY32>(),
        ..THREADENTRY32::default()
    };
    let mut resumed = 0;
    // SAFETY: `entry` is a `THREADENTRY32` with `dwSize` set, as both
    // enumeration calls require, and the snapshot handle is open.
    let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            resume_thread(entry.th32ThreadID)?;
            resumed += 1;
        }
        // SAFETY: as for `Thread32First` above.
        more = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other(format!(
            "process {pid} has no thread to resume"
        )));
    }
    Ok(())
}

fn resume_thread(tid: u32) -> io::Result<()> {
    // SAFETY: no pointer arguments.
    let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, tid) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a handle this function just opened, owned by nothing
    // else.
    let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: the thread handle is open and has suspend/resume access.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `Ok` for a Win32 call that returned a nonzero `BOOL`, the last OS error
/// otherwise.
fn succeeded(result: BOOL) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// `size_of::<T>()` as the `u32` a Win32 size field takes.
fn size_of_u32<T>() -> u32 {
    u32::try_from(size_of::<T>()).expect("a Win32 structure's size fits in u32")
}
