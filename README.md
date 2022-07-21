# gotta-watch-em-all

Executes a process with given arguments and monitors, logs when memory usage grows to a new peak.

Example running with the `-c` option to print full commands:

```
$ gotta-watch-em-all -c -- cargo build --release
   Compiling gotta-watch-em-all v0.1.2 (/home/friel/c/gotta-watch-em-all)
    Finished dev [unoptimized + debuginfo] target(s) in 1.94s
     Running `target/debug/gotta-watch-em-all -c -- cargo build --release`
🌊 gotta-watch-em-all: high water mark reached: 12 MiB
🌊 process                                                                          private MiB    total MiB
🌊 cargo (102478)                                                                         12MiB        12MiB
🌊    cargo build --release

🌊 gotta-watch-em-all: high water mark reached: 15 MiB
🌊 process                                                                          private MiB    total MiB
🌊 cargo (102478)                                                                         15MiB        15MiB
🌊    cargo build --release

   Compiling gotta-watch-em-all v0.1.2 (/home/friel/c/gotta-watch-em-all)
🌊 gotta-watch-em-all: high water mark reached: 125 MiB-watch-em-all(bin)
🌊 process                                                                          private MiB    total MiB
🌊 cargo (102478)                                                                         22MiB       125MiB
🌊    cargo build --release
🌊   rustc (102490)                                                                      103MiB       103MiB
🌊      /home/friel/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rustc --crate-name gotta_watch_em_all
🌊        --edition=2021 src/main.rs --error-format=json --json=diagnostic-rendered-ansi,artifacts,future-incompat
...
```

## Options

Can output to a separate file, rather than stderr, and there are options for tuning the threshold
for writing out the process tree.

```
gotta-watch-em-all
Run a process and monitor the memory usage of the process tree, logging to a file or stdout. When a
high water mark is reached, depending on options provided, the process tree and memory usage will be
written to output

USAGE:
    gotta-watch-em-all [OPTIONS] <COMMAND>...

ARGS:
    <COMMAND>...    Command to run

OPTIONS:
    -a, --threshold-absolute <THRESHOLD_ABSOLUTE>
            The minimum increase, in kilobytes, over the high water mark required to output stats
            [default: 1024]

    -r, --threshold-relative <THRESHOLD_RELATIVE>
            The minimum increase, as a percentage, over the high water mark required to output stats
            [default: 0]

    -i, --check-interval <CHECK_INTERVAL>
            How frequently, in milliseconds, to check memory stats [default: 250]

    -n, --report-every-nth <REPORT_EVERY_NTH>
            The minimum number of intervals to wait between reporting memory stats without reaching
            a high water mark

    -c, --show-command
            Toggles showing the command line for processes

    -f, --show-free
            Show free and used memory memory, like process free(1)

    -h, --help
            Print help information

    -o, --out <OUT>
            Output file, - or absent for stderr
```
