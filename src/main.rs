use std::{collections::HashMap, fmt::Write, time::Duration, path::Path};

use structopt::{paw, StructOpt};
use sysinfo::{Pid, Process, ProcessExt, System, SystemExt};
use tokio::{
  fs::File,
  io::{stderr, AsyncWriteExt},
  select, time,
};
use tokio_util::sync::CancellationToken;


cfg_if::cfg_if! {
  if #[cfg(all(
      not(feature = "unknown-ci"),
      any(
          target_os = "freebsd",
          target_os = "linux",
          target_os = "android",
          target_os = "macos",
          target_os = "ios",
      )
  ))] {
      use libc::pid_t;

      type PidT = pid_t;
  } else {
      type PidT = usize;
  }
}


/// Run a process and monitor the memory usage of the process tree, logging to a
/// file or stdout. When a high water mark is reached, depending on options
/// provided, the process tree and memory usage will be written to output.
#[derive(StructOpt, Debug, Clone)]
#[structopt(name = "gotta-watch-em-all")]
pub struct ProgramArgs {
  /// Output file, - or absent for stderr.
  #[structopt(short, long)]
  out: Option<String>,

  #[structopt(flatten)]
  threshold_options: ThresholdOptions,

  /// Program to run
  #[structopt()]
  program: String,

  /// Program arguments
  #[structopt(raw(true))]
  args: Vec<String>,
}

#[derive(StructOpt, Debug, Clone)]
struct ThresholdOptions {
  /// The minimum increase, in kilobytes, over the high water mark required
  /// to output stats.
  #[structopt(short = "a", long, default_value = "1024")]
  threshold_absolute: u64,

  /// The minimum increase, as a percentage, over the high water mark required to
  /// output stats.
  #[structopt(short = "r", long, default_value = "0")]
  threshold_relative: f64,
}

#[paw::main]
#[tokio::main]
async fn main(args: ProgramArgs) -> Result<(), Box<dyn std::error::Error>> {
  let mut spawned_process = tokio::process::Command::new(args.program)
    .args(args.args)
    .kill_on_drop(true)
    .spawn()?;

  let token = CancellationToken::new();
  let child_token = token.child_token();

  let pid = spawned_process
    .id()
    .expect("Expected process to have a valid pid") as PidT;

  let output_file = get_file(args.out.as_deref()).await?;
  let handle = tokio::task::spawn(measure_memory(
    pid,
    child_token,
    output_file,
    args.threshold_options,
  ));

  spawned_process.wait().await?;
  token.cancel();

  let _ = handle.await?;

  Ok(())
}

async fn get_file(options: Option<&str>) -> Result<Option<File>, Box<dyn std::error::Error>> {
  let file = match options {
    None => None,
    Some("-") => None,
    Some(path) => Some(
      tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.clone())
        .await?,
    )
    .into(),
  };

  Ok(file)
}

async fn measure_memory(
  pid: PidT,
  child_token: CancellationToken,
  output_file: Option<File>,
  threshold_options: ThresholdOptions,
) {
  measure_memory_internal(pid, child_token, output_file, threshold_options)
    .await
    .expect("Should not error on measuring memory");
}

async fn measure_memory_internal(
  pid: PidT,
  child_token: CancellationToken,
  mut output_file: Option<File>,
  threshold_options: ThresholdOptions,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut timer = time::interval(Duration::from_millis(100));
  let mut sys = System::new_all();
  let pid = (pid as PidT).into();
  let mut high_water_mark_kib: u64 = 0;

  loop {
    sys.refresh_processes();
    let processes = sys.processes();

    let process_children = get_process_children(processes);
    let process_memory = compute_memory_stats(pid, &process_children);

    if let Some(stats) = process_memory.get(&pid) {
      let aggregate_kib = stats.aggregate_kib;
      let met_threshold_absolute =
        aggregate_kib > high_water_mark_kib + threshold_options.threshold_absolute;
      let met_threshold_relative = aggregate_kib
        > ((high_water_mark_kib as f64) * (1.0 + threshold_options.threshold_relative)) as u64;
      if met_threshold_absolute && met_threshold_relative {
        eprintln!(
          "🌊 gotta-watch-em-all: Reached a new high water mark of {} KiB, {} greater than before!",
          aggregate_kib,
          aggregate_kib - high_water_mark_kib
        );
        high_water_mark_kib = aggregate_kib;

        match print_stats(pid, &process_children, &process_memory, &mut output_file).await {
          Err(err) => eprintln!("Error: {:?}", err),
          _ => {}
        };
      }
    };

    select! {
        _ = child_token.cancelled() => {
            return Ok(());
        }
        _ = timer.tick() => {}
    }
  }
}

const SPACE: &str = "";

#[derive(Default)]
pub struct ProcessEntry<'a> {
  process: Option<&'a Process>,
  children: Vec<Pid>,
}

async fn print_stats<'a>(
  pid: Pid,
  process_children: &HashMap<Pid, ProcessEntry<'a>>,
  process_memory: &HashMap<Pid, MemoryStats>,
  output: &mut Option<File>,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut buffer = String::new();

  let title = "process";
  let private_kib = "private ";
  let aggregate_kib = "total ";
  let depth = 0;
  writeln!(
    buffer,
    "🌊 {SPACE:<indent$}{title:<28}{SPACE:<width$}{private_kib:>9}KiB {aggregate_kib:>9}KiB",
    indent = depth * 2,
    width = 30 - depth * 2,
  )?;

  // We've reached a high water mark, print useful info. First we'll get all the
  // processes and their individual and aggregate memories:
  if let Some(entry) = process_children.get(&pid) {
    record_high_water_mark_entry(&mut buffer, entry, process_children, process_memory, 0)?;
  }
  writeln!(buffer)?;

  match output {
    Some(file) => file.write_all(buffer.as_bytes()).await?,
    None => stderr().write_all(buffer.as_bytes()).await?,
  }

  Ok(())
}

fn record_high_water_mark_entry(
  buffer: &mut String,
  entry: &ProcessEntry,
  process_children: &HashMap<Pid, ProcessEntry>,
  process_memory: &HashMap<Pid, MemoryStats>,
  depth: usize,
) -> Result<(), Box<dyn std::error::Error>> {
  let process = entry.process.unwrap();
  let process_exe = process.exe();
  let name = Path::new(process_exe).file_name().map(|x| x.to_str()).flatten().unwrap_or(process.name());
  let pid = process.pid();
  let title = format!("{name} ({pid})");
  let MemoryStats {
    private_kib,
    aggregate_kib,
  } = process_memory
    .get(&process.pid())
    .copied()
    .unwrap_or_default();

  writeln!(
    buffer,
    "🌊 {SPACE:<indent$}{title:<28}{SPACE:<width$}{private_kib:>9}KiB {aggregate_kib:>9}KiB",
    indent = depth * 2,
    width = 30 - depth * 2,
  )?;

  for child_pid in entry.children.iter() {
    if let Some(child_entry) = process_children.get(&child_pid) {
      record_high_water_mark_entry(
        buffer,
        child_entry,
        process_children,
        process_memory,
        depth + 1,
      )?;
    }
  }

  Ok(())
}

fn get_process_children(processes: &HashMap<Pid, Process>) -> HashMap<Pid, ProcessEntry> {
  let mut process_children = HashMap::<Pid, ProcessEntry>::new();
  for (pid, process) in processes {
    let entry = process_children.entry(*pid).or_default();
    entry.process = Some(process);

    if let Some(parent_pid) = process.parent() {
      let parent_entry = process_children.entry(parent_pid).or_default();
      parent_entry.children.push(*pid);
    }
  }

  process_children
}

#[derive(Copy, Clone, Default)]
pub struct MemoryStats {
  aggregate_kib: u64,
  private_kib: u64,
}

fn compute_memory_stats(
  pid: Pid,
  process_children: &HashMap<Pid, ProcessEntry>,
) -> HashMap<Pid, MemoryStats> {
  let mut process_memory = HashMap::new();
  process_children
    .get(&pid)
    .map(|entry| compute_aggregate_memory_kib(entry, &process_children, &mut process_memory));

  process_memory
}

fn compute_aggregate_memory_kib(
  entry: &ProcessEntry,
  process_children: &HashMap<Pid, ProcessEntry>,
  process_memory: &mut HashMap<Pid, MemoryStats>,
) -> u64 {
  let process = entry.process.unwrap();

  let private_kib = process.memory();
  let mut aggregate_kib: u64 = private_kib;

  for child_pid in entry.children.iter() {
    if let Some(child_entry) = process_children.get(&child_pid) {
      aggregate_kib += compute_aggregate_memory_kib(child_entry, process_children, process_memory);
    }
  }

  process_memory.insert(
    process.pid(),
    MemoryStats {
      aggregate_kib,
      private_kib,
    },
  );

  aggregate_kib
}
