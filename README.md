# process-orchestrator

[![Build](https://github.com/haribo256/process-orchestrator/actions/workflows/build.yml/badge.svg)](https://github.com/haribo256/process-orchestrator/actions/workflows/build.yml)

Light-weight runtime, that runs executables, restarts them when they fail, and recycles them upon memory-usage conditions. Can be hosted as a windows service

# Configure each process

The process-orchestrator will search the `conf` folder for YAML files, and load each one to configure a process it is to run and keep-alive. 

## Inputs

| Name                  | Type          | Description                                                                 |
|-----------------------|---------------|-----------------------------------------------------------------------------|
| `name`                  | string        | Name of the configuration                                                   |
| `executable`            | string        | Path to the executable to run                                               |
| `arguments`             | string array  | Arguments to pass on the command line to the executable to running it       |
| `working_directory`     | string        | Path to the current working directory the executable should be run under    |
| `log_file`              | string        | The log file where the STDOUT / STDERR is written to. If this is omitted, it will be output on the process-orchestrator's STDOUT / STDERR. | 
| `environment_variables` | string map    | Key/value pairs that are passed to the executable as environment variables  |

## Example

```yaml
name: node_script
executable: "program.exe"
arguments:
  - "-e"
  - "program_argument"
working_directory: "program_dir/bin"
log_file: "program_dir/log/output.log"
environment_variables:
  DB_SERVER: myserver.local
  DB_DATABASE: mydb
```

# Roadmap

- Currently only works on Windows (will be looking to expand for linux)
- Multiply process to run a number of replicas
- API to have existing processes request new processes
