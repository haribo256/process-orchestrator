mod errors;
mod windows_service_host;
mod stateful_process;
mod event_pump;
mod config;

use crate::errors::OrchestratorError;
use crate::windows_service_host::{start_windows_service};

use log::LevelFilter;
use structopt::StructOpt;
use simplelog::{CombinedLogger, TermLogger, Config, TerminalMode, ColorChoice, WriteLogger};
use std::fs::File;

#[cfg(windows)]
fn main() -> windows_service::Result<()> {
  let cli_options = CliOptions::from_args();

  set_current_directory_as_executable_directory();
  set_executable_logging_file(cli_options.verbose);

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
  // #[structopt(short = "c", long = "config-directory")]
  // pub config_directory: Option<PathBuf>,

  #[structopt(long = "verbose")]
  pub verbose: bool,
}

fn set_current_directory_as_executable_directory() {
  let mut path = std::env::current_exe().unwrap();
  path.pop();
  std::env::set_current_dir(path).unwrap();
}

fn set_executable_logging_file(verbose: bool) {
  let executable_path = std::env::current_exe().unwrap();
  let executable_name = executable_path.file_name().unwrap().to_str().unwrap();
  let log_file_name = format!("{}.log", executable_name);

  let mut level_filter = LevelFilter::Info;
  if verbose {
    level_filter = LevelFilter::Trace;
  }

  CombinedLogger::init(
    vec![
      TermLogger::new(level_filter, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
      WriteLogger::new(level_filter, Config::default(), File::create(log_file_name).unwrap()),
    ]
  ).unwrap();
}