# gotta-watch-em-all

Executes a process with given arguments and monitors, logs when memory usage grows to a new peak.

Example:

```
cargo run -- cargo -- build --release
   Compiling gotta-watch-em-all v0.1.0 (C:\Users\aaron\c\gotta-watch-em-all)
    Finished dev [unoptimized + debuginfo] target(s) in 1.27s
     Running `target\debug\gotta-watch-em-all.exe cargo -- build --release`
   Compiling gotta-watch-em-all v0.1.0 (C:\Users\aaron\c\gotta-watch-em-all)
🌊 gotta-watch-em-all: Reached a new high water mark of 28904 KiB, 28904 greater than before!
🌊 process                                                    private KiB    total KiB
🌊 cargo.exe (7776)                                               8884KiB     28904KiB
🌊   cargo.exe (16476)                                           18419KiB     20020KiB
🌊     rustc.exe (15672)                                          1601KiB      1601KiB

🌊 gotta-watch-em-all: Reached a new high water mark of 56540 KiB, 27636 greater than before!
🌊 process                                                    private KiB    total KiB
🌊 cargo.exe (7776)                                               8884KiB     56540KiB
🌊   cargo.exe (16476)                                           18477KiB     47656KiB
🌊     rustc.exe (15672)                                         29179KiB     29179KiB

🌊 gotta-watch-em-all: Reached a new high water mark of 93179 KiB, 36639 greater than before!
🌊 process                                                    private KiB    total KiB
🌊 cargo.exe (7776)                                               8884KiB     93179KiB
🌊   cargo.exe (16476)                                           18477KiB     84295KiB
🌊     rustc.exe (15672)                                         65818KiB     65818KiB
```

## Options

Can output to a separate file, rather than stderr, and there are options for tuning the threshold
for writing out the process tree.

```
gotta-watch-em-all 0.1.0
Run a process and monitor the memory usage of the process tree, logging to a file or stdout. When a high water mark is
reached, depending on options provided, the process tree and memory usage will be written to output

USAGE:
    gotta-watch-em-all.exe [OPTIONS] <program> [-- <args>...]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -o, --out <out>                                  Output file, - or absent for stderr
    -a, --threshold-absolute <threshold-absolute>
            The minimum increase, as a percentage, over the high water mark required to output stats [default: 1024]

    -r, --threshold-relative <threshold-relative>
            The minimum increase, in kilobytes, over the high water mark required to output stats [default: 0]


ARGS:
    <program>    Program to run
    <args>...    Program arguments
```
