use crate::stateful_process::StatefulProcessConfig;
use crate::errors::OrchestratorError;

use std::error::Error;
use std::path::PathBuf;

pub fn load_stateful_process_configs() -> Result<Vec<StatefulProcessConfig>, Box<dyn Error>> {
  let config_directory = std::env::current_dir()?;

  let mut results = Vec::<StatefulProcessConfig>::new();

  let config_directory_entries = std::fs::read_dir(&config_directory)?;

  for config_directory_entry in config_directory_entries {
    let config_file = config_directory_entry?;
    let config_file_name = config_file.file_name().into_string().unwrap();
    let config_file_path = config_file.path();

    if !config_file.metadata()?.is_file() {
      continue;
    }

    if !config_file_name.ends_with(".yml") {
      continue;
    }

    let config_file_document_result = load_config_file(&config_file_path);
    if let Err(load_config_file_error) = config_file_document_result {
      return Err(Box::new(OrchestratorError::ConfigLoadFailed(config_file_path, load_config_file_error)))
    }

    let config_file_document = config_file_document_result.unwrap();
    results.push(config_file_document);
  }

  Ok(results)
}

pub fn load_config_file(config_file_path: &PathBuf) -> Result<StatefulProcessConfig, Box<dyn Error>> {
  let config_file_contents = std::fs::read_to_string(config_file_path)?;
  let config_file_document = serde_yaml::from_str::<StatefulProcessConfig>(config_file_contents.as_str())?;
  Ok(config_file_document)
}