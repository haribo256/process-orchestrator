use std::path::PathBuf;
use std::error::Error;
use std::fmt::{Formatter, Display};

#[derive(Debug)]
pub enum OrchestratorError {
  ConfigLoadFailed(PathBuf, Box<dyn Error>),
  ServiceStartFailed(windows_service::Error),
  ServiceControllerNotPresent(),
  ProcessNotificationRegistrationFailed(),
}

impl Display for OrchestratorError {
  fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      OrchestratorError::ConfigLoadFailed(file_path, err) => write!(formatter, "Could not load config file [{}]: {}", file_path.to_str().unwrap(), err),
      OrchestratorError::ServiceStartFailed(err) => write!(formatter, "Windows service failed to start: {:?}", err),
      OrchestratorError::ServiceControllerNotPresent() => write!(formatter, "Windows service controller not present"),
      OrchestratorError::ProcessNotificationRegistrationFailed() => write!(formatter, "Registration of the process notification handler has failed"),
    }
  }
}

impl Error for OrchestratorError {
}