use crate::event_pump::{Event, VoidResult};
use crate::errors::OrchestratorError;

use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use log::{info, error};
use nanoid::nanoid;
use serde::{Serialize, Deserialize};
use winapi::um::processthreadsapi::{TerminateProcess, OpenProcess, GetExitCodeProcess, GetProcessTimes};
use winapi::shared::ntdef::{HANDLE};
use winapi::um::winnt::{WT_EXECUTEONLYONCE, PVOID, BOOLEAN, SYNCHRONIZE, PROCESS_TERMINATE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ, PROCESS_QUERY_INFORMATION};
use winapi::um::winbase::{RegisterWaitForSingleObject, INFINITE, UnregisterWait};
use winapi::um::minwinbase::{STILL_ACTIVE, SYSTEMTIME};
use winapi::um::wincon::{AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, CTRL_C_EVENT, PHANDLER_ROUTINE};
use winapi::um::consoleapi::SetConsoleCtrlHandler;
use std::ptr::null;
use winapi::_core::ptr::null_mut;
use winapi::_core::mem::size_of;
use winapi::shared::minwindef::{LPFILETIME, FILETIME};
use winapi::um::timezoneapi::FileTimeToSystemTime;
use chrono::{Utc, TimeZone};
use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};

pub struct StatefulProcess {
  pub id: String,
  pub config: StatefulProcessConfig,
  child: Option<Child>,
  os_handler_context: Pin<Box<StatefulProcessOsHandlerContext>>,
  process_handle: Option<HANDLE>,
  pid: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatefulProcessConfig {
  pub name: String,
  pub executable: String,
  pub arguments: Option<Vec<String>>,
  pub working_directory: Option<String>,
  pub log_file: Option<String>,
  pub stop_method: Option<StatefulProcessStopMethod>,
  pub environment_variables: Option<HashMap<String, String>>
}

#[serde(rename_all = "snake_case")]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum StatefulProcessStopMethod {
  SendCtrlC,
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
      child: None,
      os_handler_context,
      process_handle: None,
      pid: None,
    }
  }

  pub fn request_stop(&self) {
    info!("Process [{}]: Requesting stop", &self.id);

    if self.process_handle.is_none() || self.pid.is_none() {
      return;
    }

    if let Some(StatefulProcessStopMethod::SendCtrlC) = self.config.stop_method.clone() {
      self.send_ctrl_c().unwrap()
    }
    else {
      self.terminate().unwrap()
    }
  }

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

    unsafe {
      AttachConsole(pid);
      SetConsoleCtrlHandler(None, 1);
      GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
    }

    Ok(())
  }

  pub fn terminate(&self) -> VoidResult {
    if self.process_handle.is_none() {
      return Ok(());
    }

    let process_handle = self.process_handle.unwrap();

    info!("Process [{}]: Terminating process", &self.id);

    unsafe {
      TerminateProcess(process_handle, 0);
    }

    Ok(())
  }

  pub fn poll(&mut self) -> VoidResult {
    let duration = self.get_duration_in_seconds();
    if let Some(duration_secs) = duration {
      info!("Process [{}]: Uptime {}", self.id, duration_secs);
    }
;
    let memory_usage = self.get_memory_usage();
    if let Some(memory_usage_mbs) = memory_usage {
      info!("Process [{}]: Memory {}", self.id, memory_usage_mbs);
    }

    Ok(())
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
  let os_handler_context = Box::from_raw(lp_parameter as *mut StatefulProcessOsHandlerContext);

  // Send the event to ensure the process has been terminated.
  os_handler_context.sender.send(Event::ProcessRequestPoll(os_handler_context.process_id.clone())).unwrap();

  // We unregister the handler to free the resource, and prevent further updates.
  if let Some(register_handle) = os_handler_context.register_handle {
    UnregisterWait(register_handle);
  }

  // Because the box is a mutable owned reference, this function will deallocate it.
  // This is a problem, as another de-allocation happens later (a double de-allocation).
  // This is why we prevent de-allocation here.
  Box::leak(os_handler_context);
}

