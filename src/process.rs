use std::{
    ffi::OsString,
    mem,
    ops::Deref,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, IntoRawHandle},
        prelude::*,
    },
    path::{Path, PathBuf},
};

use winapi::{
    shared::{
        basetsd::{ULONG64, DWORD_PTR},
        minwindef::{DWORD, HMODULE, MAX_PATH},
    },
    um::{
        handleapi::INVALID_HANDLE_VALUE,
        processthreadsapi::{
            GetCurrentProcess,
            GetExitCodeProcess,
            GetProcessId,
            GetThreadId,
            GetThreadIdealProcessorEx,
            OpenProcess,
            OpenThread,
            SetThreadIdealProcessor,
        },
        realtimeapiset::QueryThreadCycleTime,
        tlhelp32::{
            CreateToolhelp32Snapshot,
            MODULEENTRY32W,
            Module32NextW,
            PROCESSENTRY32,
            Process32Next,
            TH32CS_SNAPMODULE,
            TH32CS_SNAPPROCESS,
            TH32CS_SNAPTHREAD,
            THREADENTRY32,
            Thread32Next,
        },
        winbase::{GetProcessAffinityMask, QueryFullProcessImageNameW, SetThreadAffinityMask},
        winnt::{self, PROCESSOR_NUMBER, PROCESS_ALL_ACCESS, THREAD_ALL_ACCESS, WCHAR},
    },
};

use Error;
use Handle;
use WinResult;

#[derive(Debug)]
pub struct Process {
    handle: Handle,
}

impl Process {
    /// Creates a process handle from a PID.
    pub fn from_id(id: u32) -> WinResult<Process> {
        unsafe {
            let handle = OpenProcess(PROCESS_ALL_ACCESS, 0, id);
            if handle.is_null() {
                Err(Error::last())
            } else {
                Ok(Process {
                    handle: Handle::new(handle),
                })
            }
        }
    }

    /// Creates a process handle from a name.
    pub fn from_name(name: &str) -> WinResult<Process> {
        Process::all()?
            .find(|p| p.name().map(|n| n == name).unwrap_or(false))
            .ok_or(Error::NoProcess(name.to_string()))
    }

    /// Creates a process handle from a handle.
    pub fn from_handle(handle: Handle) -> Process {
        Process { handle }
    }

    /// Returns a handle to the current process.
    pub fn current() -> Process {
        unsafe { Process::from_handle(Handle::from_raw_handle(GetCurrentProcess())) }
    }

    /// Returns a reference to the inner handle.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Enumerates all running processes.
    pub fn all() -> WinResult<impl Iterator<Item = Process>> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                Err(Error::last())
            } else {
                Ok(ProcessIter {
                    snapshot: Handle::new(snap),
                }.filter_map(Result::ok))
            }
        }
    }

    /// Returns the process's id.
    pub fn id(&self) -> u32 {
        unsafe { GetProcessId(self.handle.as_raw_handle()) }
    }

    /// Returns true if the process is running.
    pub fn running(&self) -> bool {
        unsafe {
            let mut status = 0;
            GetExitCodeProcess(self.handle.as_raw_handle(), &mut status);
            status == 259
        }
    }

    /// Returns the path of the executable of the process.
    pub fn path(&self) -> WinResult<PathBuf> {
        unsafe {
            let mut size = MAX_PATH as u32;
            let mut buffer: [WCHAR; MAX_PATH] = mem::zeroed();
            let ret = QueryFullProcessImageNameW(
                self.handle.as_raw_handle(),
                0,
                buffer.as_mut_ptr(),
                &mut size,
            );
            if ret == 0 {
                Err(Error::last())
            } else {
                Ok(OsString::from_wide(&buffer[0..size as usize]).into())
            }
        }
    }

    /// Returns the unqualified name of the executable of the process.
    pub fn name(&self) -> WinResult<String> {
        Ok(self
            .path()?
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned())
    }

    /// Returns the affinity mask of the process.
    pub fn affinity_mask(&self) -> WinResult<usize> {
        unsafe {
            let mut process_mask: DWORD_PTR = 0;
            let mut system_mask: DWORD_PTR = 0;
            let ret = GetProcessAffinityMask(
                self.handle.as_raw_handle(),
                &mut process_mask,
                &mut system_mask,
            );
            if ret == 0 {
                Err(Error::last())
            } else {
                Ok(process_mask as usize)
            }
        }
    }

    /// Returns an iterator over the threads of the process.
    pub fn threads<'a>(&'a self) -> WinResult<impl Iterator<Item = Thread> + 'a> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == INVALID_HANDLE_VALUE {
                Err(Error::last())
            } else {
                Ok(ThreadIter {
                    process: &self,
                    snapshot: Handle::new(snap),
                }.filter_map(Result::ok))
            }
        }
    }

    /// Returns an iterator over the ids of threads of the process.
    pub fn thread_ids<'a>(&'a self) -> WinResult<impl Iterator<Item = u32> + 'a> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == INVALID_HANDLE_VALUE {
                Err(Error::last())
            } else {
                Ok(ThreadIdIter {
                    process: &self,
                    snapshot: Handle::new(snap),
                })
            }
        }
    }

    /// Returns an iterator over the modules of the process.
    pub fn module_entries<'a>(&'a self) -> WinResult<impl Iterator<Item = ModuleEntry> + 'a> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, 0);
            if snap == INVALID_HANDLE_VALUE {
                Err(Error::last())
            } else {
                Ok(ModuleEntryIter {
                    process: &self,
                    snapshot: Handle::new(snap),
                })
            }
        }
    }
}

impl AsRawHandle for Process {
    fn as_raw_handle(&self) -> winnt::HANDLE {
        self.handle.as_raw_handle()
    }
}

impl Deref for Process {
    type Target = winnt::HANDLE;

    fn deref(&self) -> &winnt::HANDLE {
        &*self.handle
    }
}

impl FromRawHandle for Process {
    unsafe fn from_raw_handle(handle: winnt::HANDLE) -> Process {
        Process {
            handle: Handle::new(handle),
        }
    }
}

impl IntoRawHandle for Process {
    fn into_raw_handle(self) -> winnt::HANDLE {
        self.handle.into_raw_handle()
    }
}

#[derive(Debug)]
struct ProcessIter {
    snapshot: Handle,
}

impl Iterator for ProcessIter {
    type Item = WinResult<Process>;

    fn next(&mut self) -> Option<WinResult<Process>> {
        unsafe {
            let mut entry: PROCESSENTRY32 = mem::zeroed();
            entry.dwSize = mem::size_of::<PROCESSENTRY32>() as DWORD;
            let ret = Process32Next(self.snapshot.as_raw_handle(), &mut entry);
            //            if ret == 0 || Error::last().code() == 18 {
            if ret == 0 {
                None
            } else {
                Some(Process::from_id(entry.th32ProcessID))
            }
        }
    }
}

#[derive(Debug)]
pub struct Thread {
    handle: Handle,
}

impl Thread {
    /// Creates a thread handle from a thread ID.
    pub fn from_id(id: u32) -> WinResult<Thread> {
        unsafe {
            let handle = OpenThread(THREAD_ALL_ACCESS, 0, id);
            if handle.is_null() {
                Err(Error::last())
            } else {
                Ok(Thread {
                    handle: Handle::new(handle),
                })
            }
        }
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Return's the thread's ID.
    pub fn id(&self) -> u32 {
        unsafe { GetThreadId(self.handle.as_raw_handle()) }
    }

    /// Returns the thread's cycle time.
    pub fn cycle_time(&self) -> WinResult<u64> {
        unsafe {
            let mut cycles: ULONG64 = 0;
            let ret = QueryThreadCycleTime(self.handle.as_raw_handle(), &mut cycles);
            if ret == 0 {
                Err(Error::last())
            } else {
                Ok(cycles as u64)
            }
        }
    }

    /// Returns the preferred processor for the thread.
    pub fn ideal_processor(&self) -> WinResult<u32> {
        unsafe {
            let mut ideal: PROCESSOR_NUMBER = mem::zeroed();
            let ret = GetThreadIdealProcessorEx(self.handle.as_raw_handle(), &mut ideal);
            if ret == 0 {
                Err(Error::last())
            } else {
                Ok(ideal.Number as u32)
            }
        }
    }

    /// Sets the preferred processor for the thread.
    /// On success, returns the previous idea processor.
    pub fn set_ideal_processor(&mut self, processor: u32) -> WinResult<u32> {
        unsafe {
            let ret = SetThreadIdealProcessor(self.handle.as_raw_handle(), processor as DWORD);
            if ret == DWORD::max_value() {
                Err(Error::last())
            } else {
                Ok(ret)
            }
        }
    }

    /// Returns the thread's current affinity mask.
    pub fn affinity_mask(&self) -> WinResult<usize> {
        unsafe {
            let ret = SetThreadAffinityMask(self.handle.as_raw_handle(), DWORD_PTR::max_value());
            if ret == 0 {
                Err(Error::last())
            } else {
                let ret = SetThreadAffinityMask(self.handle.as_raw_handle(), ret);
                if ret == 0 {
                    Err(Error::last())
                } else {
                    Ok(ret)
                }
            }
        }
    }

    /// Sets the affinity mask of the thread. On success, returns the previous affinity mask.
    ///
    /// A thread affinity mask is a bit vector in which each bit represents a logical processor
    /// that a thread is allowed to run on. A thread affinity mask must be a subset of the process
    /// affinity mask for the containing process of a thread. A thread can only run on the
    /// processors its process can run on. Therefore, the thread affinity mask cannot specify a
    /// 1 bit for a processor when the process affinity mask specifies a 0 bit for that processor.
    ///
    /// Setting an affinity mask for a process or thread can result in threads receiving less
    /// processor time, as the system is restricted from running the threads on certain processors.
    /// In most cases, it is better to let the system select an available processor.
    ///
    /// If the new thread affinity mask does not specify the processor that is currently running
    /// the thread, the thread is rescheduled on one of the allowable processors.
    pub fn set_affinity_mask(&mut self, mask: usize) -> WinResult<usize> {
        unsafe {
            let ret = SetThreadAffinityMask(self.handle.as_raw_handle(), mask as DWORD_PTR);
            if ret == 0 {
                Err(Error::last())
            } else {
                Ok(ret)
            }
        }
    }

    /// Sets the affinity of the thread to the single given processor.
    ///
    /// If the processor index equals or exceeds the width of usize, the mask is not changed.
    /// On success, or if unchanged, returns the previous affinity mask.
    pub fn set_affinity(&mut self, processor: u8) -> WinResult<usize> {
        let processor = processor as usize;
        if processor >= mem::size_of::<usize>() * 8 {
            self.affinity_mask()
        } else {
            self.set_affinity_mask(1 << processor)
        }
    }
}

impl AsRawHandle for Thread {
    fn as_raw_handle(&self) -> winnt::HANDLE {
        self.handle.as_raw_handle()
    }
}

impl Deref for Thread {
    type Target = winnt::HANDLE;

    fn deref(&self) -> &winnt::HANDLE {
        &*self.handle
    }
}

impl FromRawHandle for Thread {
    unsafe fn from_raw_handle(handle: winnt::HANDLE) -> Thread {
        Thread {
            handle: Handle::new(handle),
        }
    }
}

impl IntoRawHandle for Thread {
    fn into_raw_handle(self) -> winnt::HANDLE {
        self.handle.into_raw_handle()
    }
}

#[derive(Debug)]
struct ThreadIter<'a> {
    process: &'a Process,
    snapshot: Handle,
}

impl<'a> Iterator for ThreadIter<'a> {
    type Item = WinResult<Thread>;

    fn next(&mut self) -> Option<WinResult<Thread>> {
        unsafe {
            loop {
                let mut entry: THREADENTRY32 = mem::zeroed();
                entry.dwSize = mem::size_of::<THREADENTRY32>() as DWORD;
                let ret = Thread32Next(self.snapshot.as_raw_handle(), &mut entry);
                if ret == 0 {
                    return None;
                } else {
                    if entry.th32OwnerProcessID == self.process.id() {
                        return Some(Thread::from_id(entry.th32ThreadID));
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
struct ThreadIdIter<'a> {
    process: &'a Process,
    snapshot: Handle,
}

impl<'a> Iterator for ThreadIdIter<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        unsafe {
            loop {
                let mut entry: THREADENTRY32 = mem::zeroed();
                entry.dwSize = mem::size_of::<THREADENTRY32>() as DWORD;
                let ret = Thread32Next(self.snapshot.as_raw_handle(), &mut entry);
                if ret == 0 {
                    return None;
                } else {
                    if entry.th32OwnerProcessID == self.process.id() {
                        return Some(entry.th32ThreadID);
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct ModuleEntry {
    pub id: u32,
    pub name: String,
    pub path: PathBuf,
    pub hmodule: HMODULE,
    pub process_id: u32,
    pub global_load_count: u32,
    pub proc_load_count: u32,
    pub mod_base_addr: *mut u8,
    pub mod_base_size: u32,
}

impl From<MODULEENTRY32W> for ModuleEntry {
    fn from(me: MODULEENTRY32W) -> ModuleEntry {
        let name_end = me
            .szModule
            .iter()
            .position(|&b| b == 0)
            .unwrap_or_else(|| me.szModule.len());
        let name = OsString::from_wide(&me.szModule[..name_end])
            .to_string_lossy()
            .into_owned();

        let path_end = me
            .szExePath
            .iter()
            .position(|&b| b == 0)
            .unwrap_or_else(|| me.szModule.len());
        let path = Path::new(&OsString::from_wide(&me.szExePath[..path_end])).to_owned();

        ModuleEntry {
            id: me.th32ModuleID,
            name,
            path,
            hmodule: me.hModule,
            process_id: me.th32ProcessID,
            global_load_count: me.GlblcntUsage,
            proc_load_count: me.ProccntUsage,
            mod_base_addr: me.modBaseAddr,
            mod_base_size: me.modBaseSize,
        }
    }
}

#[derive(Debug)]
struct ModuleEntryIter<'a> {
    process: &'a Process,
    snapshot: Handle,
}

impl<'a> Iterator for ModuleEntryIter<'a> {
    type Item = ModuleEntry;

    fn next(&mut self) -> Option<ModuleEntry> {
        unsafe {
            let mut entry: MODULEENTRY32W = mem::zeroed();
            entry.dwSize = mem::size_of::<MODULEENTRY32W>() as DWORD;
            let ret = Module32NextW(self.snapshot.as_raw_handle(), &mut entry);
            if ret == 0 {
                None
            } else {
                Some(entry.into())
            }
        }
    }
}

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn enumerates_processes() {
        let procs: Vec<_> = Process::all().unwrap().collect();
        assert_eq!(procs.is_empty(), false);
        println!("{:?}", procs);
    }

    #[test]
    fn accesses_process_names() {
        let names: Vec<_> = Process::all()
            .unwrap()
            .filter_map(|p| p.name().ok())
            .collect();
        assert_eq!(names.is_empty(), false);
        println!("{:?}", names);
    }

    #[test]
    fn enumerates_threads() {
        let process = Process::all().unwrap().next().unwrap();
        let threads: Vec<_> = process.threads().unwrap().collect();
        assert_eq!(threads.is_empty(), false);
        println!("{:?}", threads);
    }

    #[test]
    fn enumerates_module_entries() {
        let process = Process::all().unwrap().next().unwrap();
        let entries: Vec<_> = process.module_entries().unwrap().collect();
        assert_eq!(entries.is_empty(), false);
        println!("{:?}", entries);
    }
}
