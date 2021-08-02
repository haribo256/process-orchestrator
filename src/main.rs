mod errors;
mod windows_service_host;
mod stateful_process;
mod event_pump;
mod config;

use crate::errors::OrchestratorError;
use crate::windows_service_host::{start_windows_service};

use log::LevelFilter;
use std::path::{PathBuf};
use structopt::StructOpt;
use simplelog::{CombinedLogger, TermLogger, Config, TerminalMode, ColorChoice, WriteLogger};
use std::fs::File;

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
  // let cli_options = CliOptions::from_args();

  set_current_directory_as_executable_directory();
  set_executable_logging_file();

  let start_result = start_windows_service();

  if let Err(OrchestratorError::ServiceControllerNotPresent()) = start_result {
    let mut event_pump = event_pump::EventPump::new();
    event_pump.run();
  }

  Ok(())
}

#[cfg(not(windows))]
fn main() {
  panic!("Only works on windows")
}

#[derive(StructOpt)]
#[structopt(
  name = "process-orchestrator",
  about = "Keeps processes up and running using desired-state-configuration")]
struct CliOptions {
  #[structopt(short = "c", long = "config-directory")]
  pub config_directory: Option<PathBuf>,
}

fn set_current_directory_as_executable_directory() {
  let mut path = std::env::current_exe().unwrap();
  path.pop();
  std::env::set_current_dir(path).unwrap();
}

fn set_executable_logging_file() {
  let executable_path = std::env::current_exe().unwrap();
  let executable_name = executable_path.file_name().unwrap().to_str().unwrap();
  let log_file_name = format!("{}.log", executable_name);

  CombinedLogger::init(
    vec![
      TermLogger::new(LevelFilter::Info, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
      WriteLogger::new(LevelFilter::Info, Config::default(), File::create(log_file_name).unwrap()),
    ]
  ).unwrap();
}