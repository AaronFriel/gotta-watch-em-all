use std::{collections::HashMap, fmt::Write, ops::Deref, path::Path, str::FromStr, time::Duration};

use clap::{Parser, StructOpt};
use sysinfo::{set_open_files_limit, Pid, Process, ProcessExt, System, SystemExt};
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
#[derive(Parser, Debug, Clone)]
#[structopt(name = "gotta-watch-em-all", trailing_var_arg = true)]
pub struct ProgramArgs {
  /// Output file, - or absent for stderr.
  #[structopt(short, long)]
  out: Option<String>,

  #[structopt(flatten)]
  options: Options,

  /// Command to run
  #[structopt(required = true)]
  command: Vec<String>,
}

#[derive(StructOpt, Debug, Clone)]
struct Options {
  /// The minimum increase, in kilobytes, over the high water mark required
  /// to output stats.
  #[structopt(short = 'a', long, default_value = "1024", display_order = 10)]
  threshold_absolute: u64,

  /// The minimum increase, as a percentage, over the high water mark required
  /// to output stats.
  #[structopt(short = 'r', long, default_value = "0", display_order = 10)]
  threshold_relative: f64,

  /// How frequently, in milliseconds, to check memory stats.
  #[structopt(short = 'i', long, default_value = "250", display_order = 20)]
  check_interval: u64,

  /// The minimum number of intervals to wait between reporting memory stats
  /// without reaching a high water mark.
  #[structopt(short = 'n', long, display_order = 20)]
  report_every_nth: Option<u8>,

  /// Show free and used memory memory, like process free(1).
  #[structopt(short = 'f', long)]
  show_free: bool,

  /// Show command line for processes.
  #[structopt(short = 'c', long)]
  show_command: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args = ProgramArgs::parse();

  set_open_files_limit(0);

  let program = args.command.first().unwrap();

  let mut spawned_process = tokio::process::Command::new(program)
    .args(&args.command[1..])
    .kill_on_drop(true)
    .spawn()?;

  let token = CancellationToken::new();
  let child_token = token.child_token();

  let pid = spawned_process
    .id()
    .expect("Expected process to have a valid pid") as PidT;

  let output_file = get_file(args.out.as_deref()).await?;
  let handle = tokio::task::spawn(measure_memory(pid, child_token, output_file, args.options));

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
        .open(path)
        .await?,
    ),
  };

  Ok(file)
}

async fn measure_memory(
  pid: PidT,
  child_token: CancellationToken,
  output_file: Option<File>,
  threshold_options: Options,
) {
  measure_memory_internal(pid, child_token, output_file, threshold_options)
    .await
    .expect("Should not error on measuring memory");
}

async fn measure_memory_internal(
  pid: PidT,
  child_token: CancellationToken,
  mut output_file: Option<File>,
  options: Options,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut timer = time::interval(Duration::from_millis(options.check_interval));
  let mut i = 0;
  let mut sys = System::new_all();
  let pid = (pid as PidT).into();
  let mut high_water_mark_kib: u64 = 0;

  let mut buffer = String::new();

  loop {
    sys.refresh_processes();
    sys.refresh_memory();
    let processes = sys.processes();
    let process_children = get_process_children(processes);
    let process_memory = compute_memory_stats(pid, &process_children);

    if let Some(stats) = process_memory.get(&pid) {
      let aggregate_kib = stats.aggregate_kib;
      let met_threshold_absolute = aggregate_kib > high_water_mark_kib + options.threshold_absolute;
      let met_threshold_relative =
        aggregate_kib > ((high_water_mark_kib as f64) * (1.0 + options.threshold_relative)) as u64;
      let met_thresholds = met_threshold_absolute && met_threshold_relative;

      let print_anyway = match options.report_every_nth {
        Some(n) if n > 0 => i % n == 0,
        _ => false,
      };

      buffer.clear();

      let msg = if met_thresholds {
        "high water mark reached"
      } else {
        "presently at"
      };

      writeln!(
        buffer,
        "🌊 gotta-watch-em-all: {msg}: {} MiB used",
        aggregate_kib / 1024
      )
      .unwrap_or_else(|err| handle_write_err(&mut buffer, err));

      if aggregate_kib > high_water_mark_kib {
        high_water_mark_kib = aggregate_kib;
      }

      print_stats(
        &mut buffer,
        pid,
        &process_children,
        &process_memory,
        &options,
      )
      .await
      .unwrap_or_else(|err| handle_write_err(&mut buffer, err.deref()));

      if options.show_free {
        write_free_stats(&mut buffer, &sys);
      }

      if met_thresholds || print_anyway {
        write_output(&mut output_file, &mut buffer).await?;
      }
    };
    i += 1;

    select! {
        _ = child_token.cancelled() => {
            if !buffer.is_empty() {
              write_output(&mut output_file, &mut buffer).await?;
            }

            return Ok(());
        }
        _ = timer.tick() => {}
    }
  }
}

fn write_free_stats(buffer: &mut String, sys: &System) {
  // All units in MiB
  let mtot = sys.total_memory() / 1024;
  let muse = sys.used_memory() / 1024;
  let mfre = sys.free_memory() / 1024;
  let mavl = sys.available_memory() / 1024;

  let stot = sys.total_swap() / 1024;
  let suse = sys.used_swap() / 1024;
  let sfre = sys.free_swap() / 1024;

  writeln!(
    buffer,
    "\
🌊                 total        used        free   available
🌊   Mem:   {mtot:>10}Mi{muse:>10}Mi{mfre:>10}Mi{mavl:>10}Mi
🌊   Swap:  {stot:>10}Mi{suse:>10}Mi{sfre:>10}Mi",
  )
  .unwrap_or_else(|err| handle_write_err(buffer, err));
}

async fn write_output(
  output_file: &mut Option<File>,
  buffer: &mut String,
) -> Result<(), Box<dyn std::error::Error>> {
  match *output_file {
    Some(ref mut file) => file.write_all(buffer.as_bytes()).await?,
    None => stderr().write_all(buffer.as_bytes()).await?,
  }

  Ok(())
}

fn handle_write_err(buffer: &mut String, err: impl std::error::Error) {
  writeln!(buffer, "🌊 gotta-watch-em-all: error {}", err).expect("Error writing to buffer")
}

const SPACE: &str = "";

#[derive(Default)]
pub struct ProcessEntry<'a> {
  process: Option<&'a Process>,
  children: Vec<Pid>,
}

async fn print_stats<'a>(
  buffer: &mut String,
  pid: Pid,
  process_children: &HashMap<Pid, ProcessEntry<'a>>,
  process_memory: &HashMap<Pid, MemoryStats>,
  options: &Options,
) -> Result<(), Box<dyn std::error::Error>> {
  let title = "process";
  let private_mib = "private ";
  let aggregate_mib = "total ";
  let depth = 0;
  writeln!(
    buffer,
    "🌊 {SPACE:<indent$}{title:<50}{SPACE:<width$}{private_mib:>9}MiB {aggregate_mib:>9}MiB",
    indent = depth * 2,
    width = 30 - (depth * 2),
  )?;

  // We've reached a high water mark, print useful info. First we'll get all the
  // processes and their individual and aggregate memories:
  if let Some(entry) = process_children.get(&pid) {
    record_high_water_mark_entry(buffer, entry, process_children, process_memory, 0, options)?;
  }
  writeln!(buffer)?;

  Ok(())
}

fn record_high_water_mark_entry(
  buffer: &mut String,
  entry: &ProcessEntry,
  process_children: &HashMap<Pid, ProcessEntry>,
  process_memory: &HashMap<Pid, MemoryStats>,
  depth: usize,
  options: &Options,
) -> Result<(), Box<dyn std::error::Error>> {
  let process = entry.process.unwrap();
  let process_exe = process.exe();
  let name = Path::new(process_exe)
    .file_name()
    .and_then(|x| x.to_str())
    .unwrap_or_else(|| process.name());
  let pid = process.pid();
  let title = format!("{name} ({pid})");
  let MemoryStats {
    private_mib,
    aggregate_kib,
  } = process_memory
    .get(&process.pid())
    .copied()
    .unwrap_or_default();
  let aggregate_mib = aggregate_kib / 1024;

  writeln!(
    buffer,
    "🌊 {SPACE:<indent$}{title:<50}{SPACE:<width$}{private_mib:>9}MiB {aggregate_mib:>9}MiB",
    indent = depth * 2,
    width = 30 - depth * 2,
  )?;

  if options.show_command {
    let mut line_buffer = String::from_str("  ")?;
    let cmd = process.cmd();

    let mut clear_buffer =
      |line_buffer: &mut String, min_length: usize| -> Result<(), Box<dyn std::error::Error>> {
        if line_buffer.len() > min_length {
          writeln!(
            buffer,
            "🌊 {SPACE:<indent$}{line_buffer}",
            indent = depth * 2,
          )?;
          line_buffer.clear();
          write!(line_buffer, "    ")?;
        }

        Ok(())
      };

    for arg in cmd {
      clear_buffer(&mut line_buffer, 100)?;
      write!(line_buffer, " {arg}")?;
    }
    clear_buffer(&mut line_buffer, 0)?;
  }

  for child_pid in entry.children.iter() {
    if let Some(child_entry) = process_children.get(child_pid) {
      record_high_water_mark_entry(
        buffer,
        child_entry,
        process_children,
        process_memory,
        depth + 1,
        options,
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
  private_mib: u64,
}

fn compute_memory_stats(
  pid: Pid,
  process_children: &HashMap<Pid, ProcessEntry>,
) -> HashMap<Pid, MemoryStats> {
  let mut process_memory = HashMap::new();
  process_children
    .get(&pid)
    .map(|entry| compute_aggregate_memory_kib(entry, process_children, &mut process_memory));

  process_memory
}

fn compute_aggregate_memory_kib(
  entry: &ProcessEntry,
  process_children: &HashMap<Pid, ProcessEntry>,
  process_memory: &mut HashMap<Pid, MemoryStats>,
) -> u64 {
  let process = entry.process.unwrap();

  let memory = process.memory();
  let private_mib = memory / 1024;
  let mut aggregate_kib: u64 = memory;

  for child_pid in entry.children.iter() {
    if let Some(child_entry) = process_children.get(child_pid) {
      aggregate_kib += compute_aggregate_memory_kib(child_entry, process_children, process_memory);
    }
  }

  process_memory.insert(
    process.pid(),
    MemoryStats {
      aggregate_kib,
      private_mib,
    },
  );

  aggregate_kib
}
