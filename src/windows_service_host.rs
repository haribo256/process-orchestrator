use crate::errors::OrchestratorError;

use std::sync::mpsc::channel;
use std::error::Error;
use std::ffi::OsString;
use std::time::Duration;
use log::{error, info};
use windows_service::define_windows_service;
use windows_service::service_dispatcher;
use windows_service::service_control_handler;
use windows_service::service_control_handler::ServiceControlHandlerResult;
use windows_service::service::{ServiceExitCode, ServiceControlAccept, ServiceState, ServiceType, ServiceStatus, ServiceControl};
use crate::event_pump::{EventPump, Event};

#[cfg(windows)]
define_windows_service!(ffi_service_main, service_main_outer);

#[cfg(windows)]
pub fn start_windows_service() -> Result<(), OrchestratorError> {
  let service_name = get_service_name();
  let start_result = service_dispatcher::start(service_name, ffi_service_main);

  if let Err(start_error) = start_result {
    if let windows_service::Error::Winapi(io_error) = start_error {
      let start_os_error = std::io::Error::from(io_error);
      if let Some(start_error_code) = start_os_error.raw_os_error() {
        if start_error_code == 1063 {
          return Err(OrchestratorError::ServiceControllerNotPresent())
        }
      }
    }
    else {
      return Err(OrchestratorError::ServiceStartFailed(start_error))
    }
  }

  Ok(())
}

#[cfg(windows)]
fn service_main_outer(arguments: Vec<OsString>) {
  if let Err(service_error) = service_main_inner(arguments) {
    error!("{:?}", service_error);
  }
}

#[cfg(windows)]
fn service_main_inner(_arguments: Vec<OsString>) -> Result<(), Box<dyn Error>> {
  let mut event_pump = EventPump::new();
  let (stopped_event_sender, stopped_event_receiver) = channel();
  let request_stop_sender = event_pump.sender.clone();

  let event_handler = move |control_event| -> ServiceControlHandlerResult {
    match control_event {
      ServiceControl::Stop => {
        info!("Windows service: Stop received");
        request_stop_sender.send(Event::OrchestratorRequestStop()).unwrap();
        stopped_event_receiver.recv().unwrap();
        ServiceControlHandlerResult::NoError
      }
      ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
      _ => ServiceControlHandlerResult::NotImplemented,
    }
  };

  info!("Windows service: Starting");
  let status_sender = service_control_handler::register("process-orchestrator", event_handler)?;

  info!("Windows service: Started");
  status_sender.set_service_status(ServiceStatus {
    service_type: ServiceType::OWN_PROCESS,
    current_state: ServiceState::Running,
    controls_accepted: ServiceControlAccept::STOP,
    exit_code: ServiceExitCode::Win32(0),
    checkpoint: 0,
    wait_hint: Duration::default(),
    process_id: None,
  })?;

  info!("Windows service: Running");
  event_pump.run();

  info!("Windows service: Stopped");
  status_sender.set_service_status(ServiceStatus {
    service_type: ServiceType::OWN_PROCESS,
    current_state: ServiceState::Stopped,
    controls_accepted: ServiceControlAccept::empty(),
    exit_code: ServiceExitCode::Win32(0),
    checkpoint: 0,
    wait_hint: Duration::default(),
    process_id: None,
  })?;

  stopped_event_sender.send(())?;

  Ok(())
}

pub fn get_service_name() -> String {
  let executable_path = std::env::current_exe().unwrap();
  let executable_name = executable_path.file_stem().unwrap().to_str().unwrap();
  return String::from(executable_name);
}