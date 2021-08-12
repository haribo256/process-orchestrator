use crate::event_pump::{Event, VoidResult};
use crate::errors::OrchestratorError;

use std::collections::HashMap;
use std::fs::File;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use log::{info, error};
use serde::{Serialize, Deserialize};
use nanoid::nanoid;
use chrono::{Utc, TimeZone};
use winapi::um::processthreadsapi::{TerminateProcess, OpenProcess, GetExitCodeProcess, GetProcessTimes, CreateProcessW, CreateProcessA, PROCESS_INFORMATION, STARTUPINFOA, GetCurrentProcess, GetCurrentProcessId};
use winapi::shared::ntdef::{HANDLE};
use winapi::um::winnt::{WT_EXECUTEONLYONCE, PVOID, BOOLEAN, SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_VM_READ, PROCESS_QUERY_INFORMATION, LPCSTR, DUPLICATE_SAME_ACCESS, PROCESS_DUP_HANDLE, FILE_APPEND_DATA, FILE_SHARE_WRITE, FILE_SHARE_READ, FILE_ATTRIBUTE_NORMAL, GENERIC_WRITE};
use winapi::um::winbase::{RegisterWaitForSingleObject, INFINITE, UnregisterWait, DETACHED_PROCESS, CREATE_NEW_CONSOLE, FORMAT_MESSAGE_FROM_HMODULE, FORMAT_MESSAGE_IGNORE_INSERTS, CREATE_NO_WINDOW, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE, STARTF_USESTDHANDLES, STD_INPUT_HANDLE};
use winapi::um::minwinbase::{STILL_ACTIVE, SYSTEMTIME, LPSECURITY_ATTRIBUTES, SECURITY_ATTRIBUTES};
use winapi::um::wincon::{AttachConsole, GenerateConsoleCtrlEvent, CTRL_C_EVENT, FreeConsole};
use winapi::um::consoleapi::SetConsoleCtrlHandler;
use winapi::shared::minwindef::{FILETIME, LPVOID, TRUE, FALSE};
use winapi::um::timezoneapi::FileTimeToSystemTime;
use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use std::time::Duration;
use std::ptr::null;
use std::ffi::{CString, CStr, c_void};
use std::os::raw::c_char;
use std::borrow::BorrowMut;
use winapi::um::errhandlingapi::GetLastError;
use std::io::{stdin, Stdin, Stdout};
use winapi::um::handleapi::{CloseHandle, DuplicateHandle};
use winapi::um::processenv::{SetStdHandle, GetStdHandle};
use winapi::um::fileapi::{CreateFileA, OPEN_ALWAYS, CREATE_ALWAYS};

pub struct StatefulProcess {
  pub id: String,
  pub config: StatefulProcessConfig,
  pub memory_usage_mbs: Option<f64>,
  pub duration_secs: Option<f64>,
  os_handler_context: Pin<Box<StatefulProcessOsHandlerContext>>,
  process_handle: Option<HANDLE>,
  pid: Option<u32>,
  log_file_handle: Option<HANDLE>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatefulProcessConfig {
  pub name: String,
  pub executable: String,
  pub arguments: Option<Vec<String>>,
  pub working_directory: Option<String>,
  pub log_file: Option<String>,
  pub stop_method: Option<StatefulProcessStopMethod>,
  pub environment_variables: Option<HashMap<String, String>>,
  pub recycle_on_memory_mbs: Option<f64>,
  pub recycle_on_duration_secs: Option<f64>,
}

#[serde(rename_all = "snake_case")]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum StatefulProcessStopMethod {
  CtrlC,
  Terminate
}

struct StatefulProcessOsHandlerContext {
  register_handle: Option<HANDLE>,
  process_id: String,
  sender: Sender<Event>,
}

impl StatefulProcess {
  pub fn new(config: StatefulProcessConfig, sender: Sender<Event>) -> Self {
    let process_id = StatefulProcess::create_process_id(config.name.as_str());

    let os_handler_context = Pin::new(Box::new(StatefulProcessOsHandlerContext {
      register_handle: None,
      process_id: process_id.clone(),
      sender,
    }));

    Self {
      id: process_id.clone(),
      config,
      os_handler_context,
      pid: None,
      process_handle: None,
      log_file_handle: None,
      memory_usage_mbs: None,
      duration_secs: None,
    }
  }

  pub fn request_stop(&mut self) {
    info!("Process [{}]: Requesting stop", &self.id);

    if self.process_handle.is_none() || self.pid.is_none() {
      return;
    }

    if let Some(StatefulProcessStopMethod::CtrlC) = self.config.stop_method.clone() {
      self.send_ctrl_c().unwrap()
    }
    else {
      self.terminate().unwrap()
    }
  }

  #[cfg(windows)]
  pub fn start_instance(&mut self) -> VoidResult {
    let config = &self.config;

    let mut command_line = CString::new(config.executable.as_str())?;

    if let Some(arguments) = config.arguments.clone() {
      command_line = CString::new(format!("{} {}", &config.executable, arguments.iter().map(|x| format!("\"{}\"", x)).collect::<Vec<String>>().join(" ")))?;
    }

    let mut environment_cstring: *mut c_char = 0 as *mut c_char;
    if let Some(environment_variables) = config.environment_variables.clone() {
      let mut environment_string = String::new();

      for environment_variable in environment_variables {
        let pair = format!("{}={}\0", environment_variable.0, environment_variable.1);
        environment_string.push_str(pair.as_str());
      }

      environment_string.push_str("\0");
      unsafe {
        environment_cstring = environment_string.as_mut_ptr() as *mut c_char;
      }
    }

    let mut working_directory_cstring= 0 as *mut c_char;
    if let Some(work) = &config.working_directory {
      working_directory_cstring = CString::new(work.as_str())?.into_raw();
    }

    // let arguments = config.arguments
    // let command_line_cstring = CString::from(command_line);
    // let environment_cstring = Some(CString::new(environment)?);
    // let working_directory_cstring = Some(CString::new(&config.working_directory)?);
    // let cstr_none: *mut c_void;

    unsafe {
      let mut process_information = std::mem::zeroed::<PROCESS_INFORMATION>();
      let mut startup_information = std::mem::zeroed::<STARTUPINFOA>();
      startup_information.cb = std::mem::size_of::<STARTUPINFOA>() as u32;

      if let Some(log_file) = &config.log_file {
        let mut security_attributes: SECURITY_ATTRIBUTES = std::mem::zeroed();
        security_attributes.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        security_attributes.bInheritHandle = TRUE;

        let log_file_cstring = CString::new(log_file.as_str())?.into_raw();

        let log_file_handle = CreateFileA(
          log_file_cstring as LPCSTR,
          FILE_APPEND_DATA,
          FILE_SHARE_WRITE | FILE_SHARE_READ,
          &mut security_attributes,
          OPEN_ALWAYS,
          FILE_ATTRIBUTE_NORMAL,
          0 as HANDLE);

        startup_information.dwFlags = STARTF_USESTDHANDLES;
        startup_information.hStdOutput = log_file_handle;
        startup_information.hStdError = log_file_handle;

        self.log_file_handle = Some(log_file_handle);
      }
      // else {
      //   // let current_process_handle = OpenProcess(
      //   //   PROCESS_DUP_HANDLE,
      //   //   TRUE,
      //   //   GetCurrentProcessId());
      //   let stdin_handle = GetStdHandle(STD_INPUT_HANDLE);
      //   let stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE);
      //   let stderr_handle = GetStdHandle(STD_ERROR_HANDLE);
      //
      //   // let mut stdout_handle_dup = 0 as HANDLE;
      //   // let mut stderr_handle_dup = 0 as HANDLE;
      //
      //   // DuplicateHandle(
      //   //   current_process_handle,
      //   //   stdout_handle,
      //   //   current_process_handle,
      //   //   &mut stdout_handle_dup,
      //   //   0,
      //   //   TRUE,
      //   //   DUPLICATE_SAME_ACCESS);
      //   //
      //   // DuplicateHandle(
      //   //   current_process_handle,
      //   //   stderr_handle,
      //   //   current_process_handle,
      //   //   &mut stderr_handle_dup,
      //   //   0,
      //   //   TRUE,
      //   //   DUPLICATE_SAME_ACCESS);
      //
      //   startup_information.dwFlags = STARTF_USESTDHANDLES;
      //   startup_information.hStdInput = stdin_handle;
      //   startup_information.hStdOutput = stdout_handle;
      //   startup_information.hStdError = stderr_handle;
      // }

      if CreateProcessA(
        0 as LPCSTR,
        command_line.into_raw(),
        0 as LPSECURITY_ATTRIBUTES,
        0 as LPSECURITY_ATTRIBUTES,
        TRUE,
        CREATE_NO_WINDOW,
        environment_cstring as LPVOID,
        working_directory_cstring as LPCSTR,
        &mut startup_information,
        &mut process_information) == 0 {

        return Err(Box::new(std::io::Error::last_os_error()));
      }

      self.pid = Some(process_information.dwProcessId);
      self.process_handle = Some(process_information.hProcess);

      let os_handler_context_ptr = self.os_handler_context.as_mut().get_mut() as *mut StatefulProcessOsHandlerContext;
      let mut register_handle = 0 as HANDLE;

      if RegisterWaitForSingleObject(
        &mut register_handle,
        self.process_handle.unwrap(),
        Some(wait_or_timer_callback),
        os_handler_context_ptr as HANDLE,
        INFINITE,
        WT_EXECUTEONLYONCE) == 0 {
        return Err(Box::new(OrchestratorError::ProcessNotificationRegistrationFailed()));
      }

      self.os_handler_context.register_handle = Some(register_handle);

      Ok(())
    }
  }

  #[cfg(not(windows))]
  pub fn start_instance(&mut self) -> VoidResult {
    let config = &self.config;
    let mut command = Command::new(config.executable.as_str());

    if let Some(working_directory) = &config.working_directory {
      command.current_dir(working_directory);
    }

    if let Some(args) = &config.arguments {
      command.args(args);
    }

    if let Some(environment_variables) = &config.environment_variables {
      command.envs(environment_variables);
    }

    if let Some(log_file) = &config.log_file {
      let outputs = File::create(log_file)?;
      let errors = outputs.try_clone()?;
      command.stdout(Stdio::from(outputs));
      command.stderr(Stdio::from(errors));
    }

    let child = command.spawn()?;
    self.pid = Some(child.id());
    self.child = Some(child);

    let os_handler_context_ptr = self.os_handler_context.as_mut().get_mut() as *mut StatefulProcessOsHandlerContext;
    let mut register_handle = 0 as HANDLE;

    unsafe {
      let process_handle = OpenProcess(SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, self.pid.unwrap());
      self.process_handle = Some(process_handle);
    }

    unsafe {
      let register_success = RegisterWaitForSingleObject(
        &mut register_handle,
        self.process_handle.unwrap(),
        Some(wait_or_timer_callback),
        os_handler_context_ptr as HANDLE,
        INFINITE,
        WT_EXECUTEONLYONCE);

      if register_success == 0 {
        return Err(Box::new(OrchestratorError::ProcessNotificationRegistrationFailed()));
      }

      self.os_handler_context.register_handle = Some(register_handle);
    }

    Ok(())
  }

  pub fn is_running(&self) -> bool {
    if let Some(process_handle) = self.process_handle {
      let mut exit_code = 0u32;

      unsafe {
        GetExitCodeProcess(process_handle, &mut exit_code);
      }

      if exit_code == STILL_ACTIVE {
        return true
      }
    }

    false
  }

  pub fn send_ctrl_c(&self) -> VoidResult {
    if self.pid.is_none() {
      return Ok(());
    }

    info!("Process [{}]: Sending CTRL-C to process", &self.id);

    let pid = self.pid.unwrap();

    let current_pid = std::process::id();

    unsafe {
      FreeConsole();
      AttachConsole(pid);
      SetConsoleCtrlHandler(None, 1);
      GenerateConsoleCtrlEvent(CTRL_C_EVENT, pid);
      // AttachConsole(current_pid);
      std::thread::sleep(Duration::from_millis(500));
      SetConsoleCtrlHandler(None, 0);
      FreeConsole();
      AttachConsole(u32::max_value());
    }

    Ok(())
  }

  pub fn terminate(&mut self) -> VoidResult {
    if self.process_handle.is_none() {
      return Ok(());
    }

    let process_handle = self.process_handle.unwrap();

    info!("Process [{}]: Terminating process", &self.id);

    unsafe {
      TerminateProcess(process_handle, 0);
      CloseHandle(process_handle);
      self.process_handle = None;
    }

    Ok(())
  }

  pub fn on_stopped(&mut self) -> VoidResult {
    if let Some(log_file_handle) = self.log_file_handle {
      unsafe {
        CloseHandle(log_file_handle);
        self.log_file_handle = None;
      }
    }

    Ok(())
  }

  pub fn poll(&mut self) -> VoidResult {
    let duration = self.get_duration_in_seconds();
    if let Some(duration_seconds) = duration {
      self.duration_secs = Some(duration_seconds);
      // info!("Process [{}]: Uptime {}", self.id, duration_seconds);
    }
;
    let memory_usage = self.get_memory_usage();
    if let Some(memory_usage_mbs) = memory_usage {
      self.memory_usage_mbs = Some(memory_usage_mbs);
      // info!("Process [{}]: Memory {}", self.id, memory_usage_mbs);
    }

    Ok(())
  }

  pub fn is_recycle_required(&self) -> bool {
    if let Some(limit_memory_mbs) = self.config.recycle_on_memory_mbs {
      if let Some(current_memory_mbs) = self.memory_usage_mbs {
        if current_memory_mbs > limit_memory_mbs {
          info!("Process [{}]: Memory {}MB has reached recycle threshold {}MB", &self.id, current_memory_mbs, limit_memory_mbs);
          return true
        }
      }
    }

    if let Some(limit_duration_secs) = self.config.recycle_on_duration_secs {
      if let Some(current_duration_secs) = self.duration_secs {
        if current_duration_secs > limit_duration_secs {
          info!("Process [{}]: Uptime of {} seconds has reached recycle threshold of {} seconds", &self.id, current_duration_secs, limit_duration_secs);
          return true
        }
      }
    }

    false
  }

  pub fn get_duration_in_seconds(&self) -> Option<f64> {
    if self.process_handle.is_none() {
      return None;
    }

    unsafe {
      let mut creation_time: FILETIME = std::mem::zeroed::<FILETIME>();
      let mut exit_time: FILETIME = std::mem::zeroed::<FILETIME>();
      let mut kernel_time: FILETIME = std::mem::zeroed::<FILETIME>();
      let mut user_time: FILETIME = std::mem::zeroed::<FILETIME>();

      GetProcessTimes(self.process_handle.unwrap(),
                      &mut creation_time,
                      &mut exit_time,
                      &mut kernel_time,
                      &mut user_time);

      let mut creation_system_time: SYSTEMTIME = std::mem::zeroed::<SYSTEMTIME>();
      FileTimeToSystemTime(&creation_time, &mut creation_system_time);

      let creation_datetime = Utc
        .ymd(creation_system_time.wYear as i32, creation_system_time.wMonth as u32, creation_system_time.wDay as u32)
        .and_hms(creation_system_time.wHour as u32, creation_system_time.wMinute as u32, creation_system_time.wSecond as u32);

      let current_datetime = Utc::now();

      let duration = current_datetime.signed_duration_since(creation_datetime);

      let duration_seconds = duration.num_milliseconds() as f64 / 1000f64;

      Some(duration_seconds)
    }
  }

  pub fn get_memory_usage(&self) -> Option<f64> {
    if self.process_handle.is_none() {
      return None;
    }

    unsafe {
      let mut process_memory_counters = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();

      if GetProcessMemoryInfo(
        self.process_handle.unwrap(),
        &mut process_memory_counters,
        std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32) == 0 {
        error!("Process [{}]: Failed to query memory information", self.id);
        return None;
      }

      let usage_megabytes = process_memory_counters.WorkingSetSize as f64 / 1024f64 / 1024f64;

      return Some(usage_megabytes)
    }
  }

  fn create_process_id(process_name: &str) -> String {
    let alphabet: [char; 16] = [
      '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', 'a', 'b', 'c', 'd', 'e', 'f'
    ];

    let id = nanoid!(5, &alphabet);

    format!("{}-{}", process_name, id)
  }
}

unsafe extern "system" fn wait_or_timer_callback(lp_parameter: PVOID, _timer_or_wait_fired: BOOLEAN) {
  // Get an owned mutable reference here from the pointer passed.
  let mut os_handler_context = Box::from_raw(lp_parameter as *mut StatefulProcessOsHandlerContext);

  // Send the event to ensure the process has been terminated.
  os_handler_context.sender.send(Event::ProcessRequestPoll(os_handler_context.process_id.clone())).unwrap();

  // We unregister the handler to free the resource, and prevent further updates.
  if let Some(register_handle) = os_handler_context.register_handle {
    UnregisterWait(register_handle);
    // CloseHandle(register_handle);
    os_handler_context.register_handle = None;
  }

  // Because the box is a mutable owned reference, this function will deallocate it.
  // This is a problem, as another de-allocation happens later (a double de-allocation).
  // This is why we prevent de-allocation here.
  Box::leak(os_handler_context);
}

