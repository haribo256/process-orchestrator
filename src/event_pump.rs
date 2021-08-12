use crate::config::load_stateful_process_configs;
use crate::stateful_process::{StatefulProcessConfig, StatefulProcess};

use std::sync::mpsc::{Receiver, Sender, channel};
use std::error::Error;
use std::time::Duration;
use log::{info, error, trace};

pub type VoidResult = Result<(), Box<dyn Error>>;

pub struct EventPump {
  pub sender: Sender<Event>,
  receiver: Receiver<Event>,
  configs: Vec<StatefulProcessConfig>,
  processes: Vec<StatefulProcess>,
  is_stop_requested: bool,
  is_stopped: bool,
}

#[derive(Debug)]
pub enum Event {
  OrchestratorStarting(),
  OrchestratorTick(),
  OrchestratorRequestStop(),
  OrchestratorStopping(),
  ProcessConfigLoaded(StatefulProcessConfig),
  ProcessRequestStart(String),
  ProcessRequestPoll(String),
  ProcessRequestStop(String),
  ProcessStopped(String),
}

impl EventPump {
  pub fn new() -> Self {
    let (sender, receiver) = channel::<Event>();
    sender.send(Event::OrchestratorStarting()).unwrap();

    Self {
      sender,
      receiver,
      configs: Vec::<StatefulProcessConfig>::new(),
      processes: Vec::<StatefulProcess>::new(),
      is_stop_requested: false,
      is_stopped: false,
    }
  }

  pub fn run(&mut self) {
    loop {
      if self.is_stopped {
        break;
      }

      let result = self.receiver.recv();

      if result.is_err() {
        break;
      }

      if let Ok(message) = result {
        let message_string = format!("{:?}", &message);
        trace!("EventPump: {}", message_string);

        let message_result = self.process_message(message);

        if let Err(error) = message_result {
          error!("Error processing message [{}]: {:?}", message_string, error)
        }
      }
    }
  }

  #[allow(unreachable_patterns)]
  fn process_message(&mut self, message: Event) -> VoidResult {
    match message {
      Event::OrchestratorStarting() => self.on_orchestrator_starting(),
      Event::OrchestratorRequestStop() => self.on_orchestrator_request_stop(),
      Event::OrchestratorStopping() => self.on_orchestrator_stopping(),
      Event::OrchestratorTick() => self.on_orchestrator_tick(),
      Event::ProcessConfigLoaded(config) => self.on_process_config_loaded(config),
      Event::ProcessRequestStart(name) => self.on_process_start(name),
      Event::ProcessRequestPoll(process_id) => self.on_request_process_poll(process_id),
      Event::ProcessRequestStop(process_id) => self.on_request_process_stop(process_id),
      Event::ProcessStopped(process_id) => self.on_process_stopped(process_id),
      _ => panic!("Message not recognized [{:?}]", message),
    }
  }

  fn on_orchestrator_tick(&mut self) -> VoidResult {
    for process in &mut self.processes {
      process.poll()?;
    }

    for process in &mut self.processes {
      if process.is_recycle_required() {
        self.sender.send(Event::ProcessRequestStop(process.id.clone())).unwrap();
      }
    }

    Ok(())
  }

  fn on_orchestrator_starting(&mut self) -> VoidResult {
    let ctrlc_sender = self.sender.clone();
    ctrlc::set_handler(move || {
      ctrlc_sender.send(Event::OrchestratorRequestStop()).unwrap();
    })?;
    trace!("EventPump: Registered CTRL-C handler");

    let stateful_process_configs = load_stateful_process_configs()?;
    info!("EventPump: Loaded {} config files", stateful_process_configs.len());

    self.configs = stateful_process_configs.clone();

    for stateful_process_config in stateful_process_configs {
      self.sender.send(Event::ProcessConfigLoaded(stateful_process_config)).unwrap();
    }

    let timer_sender = self.sender.clone();
    std::thread::spawn(move || {
      loop {
        timer_sender.send(Event::OrchestratorTick()).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
      }
    });

    Ok(())
  }

  fn on_process_config_loaded(&mut self, config: StatefulProcessConfig) -> VoidResult {
    self.sender.send(Event::ProcessRequestStart(config.name)).unwrap();

    Ok(())
  }

  fn on_orchestrator_request_stop(&mut self) -> VoidResult {
    self.is_stop_requested = true;

    if self.processes.len() == 0 {
      self.sender.send(Event::OrchestratorStopping()).unwrap();
      return Ok(())
    }

    for process in &self.processes {
      self.sender.send(Event::ProcessRequestStop(process.id.clone())).unwrap();
    }

    Ok(())
  }

  fn on_process_start(&mut self, process_name: String) -> VoidResult {
    if let Some(config) = self.configs.iter().find(|x| x.name == process_name) {
      let mut process = StatefulProcess::new(config.clone(), self.sender.clone());

      process.start_instance()?;
      info!("Process [{}]: Started", &process.config.name);

      self.processes.push(process);
    }

    Ok(())
  }

  fn on_request_process_stop(&mut self, process_id: String) -> VoidResult {
    let mut process_option = self.find_process_by_process_id(process_id.clone());

    if let Some(process) = process_option {
      process.request_stop();
    }

    Ok(())
  }

  fn on_process_stopped(&mut self, process_id: String) -> VoidResult {
    let process_option = self.find_process_by_process_id(process_id.clone());
    if process_option.is_none() {
      return Ok(())
    }

    let process = process_option.unwrap();
    let process_name = process.config.name.clone();

    process.on_stopped();

    let index_option = self.processes.iter().position(|p| p.id == process_id);
    if let Some(index) = index_option {
      self.processes.remove(index);
    }

    if self.is_stop_requested {
      if self.processes.len() == 0 {
        self.sender.send(Event::OrchestratorStopping()).unwrap();
      }

      return Ok(())
    }
    else {
      self.sender.send(Event::ProcessRequestStart(process_name)).unwrap();
    }

    Ok(())
  }

  fn on_request_process_poll(&mut self, process_id: String) -> VoidResult {
    let process_option: Option<&mut StatefulProcess> = self.processes.iter_mut().find(|p| p.id == process_id);

    if let Some(process) = process_option {
      process.poll()?;

      if !process.is_running() {
        self.sender.send(Event::ProcessStopped(process_id.clone())).unwrap();
        return Ok(())
      }

      if process.is_recycle_required() {
        self.sender.send(Event::ProcessRequestStop(process_id.clone())).unwrap();
        return Ok(())
      }
    }

    Ok(())
  }

  fn on_orchestrator_stopping(&mut self) -> VoidResult {
    self.is_stopped = true;

    Ok(())
  }

  fn find_process_by_process_id(&mut self, process_id: String) -> Option<&mut StatefulProcess> {
    let item = self.processes.iter_mut().find(|p| p.id == process_id);
    return item;
  }

  // fn get_process_by_process_id_as_mut(&mut self, process_id: String) -> Option<&mut StatefulProcess> {
  //   let mut item = self.processes.iter().find(|p| p.id == process_id);
  //   return item;
  // }

  // fn get_config_by_name(&self, name: String) -> Option<&StatefulProcessConfig> {
  //   let item = self.configs.iter().find(|p| p.name == name);
  //   return item;
  // }
}