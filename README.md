# easy_store

easy_store is an open-source Rust library that stores files in SPI NOR flash. It was born on the `esp32` family, and since `0.4.0` the core is generic: it runs on **any** backend that implements the [`embedded_storage::Storage`](https://docs.rs/embedded-storage) trait (ESP32, STM32, nRF, RP2040, or even a RAM mock on your PC).

`esp32` and `esp32c3` are verified on real hardware (see the examples in `tests`). `esp32c2`, `esp32c6`, `esp32h2`, `esp32s2` and `esp32s3` are supported through the same code path but have not been physically verified.

Language:
- [English](README.md)
- [繁體中文](README.zh-TW.md)

# Quick Test

To quickly test whether easy_store works on an ESP device, change the directory to the corresponding folder, for example `\easy_store\tests\esp32`, and execute `cargo run`.

To run the test suite on your PC (no hardware needed):

```
cargo test --target x86_64-unknown-linux-gnu   # or your host target
```

# Usage Instructions (ESP32 family)

(1a) If you are referencing directly from GitHub, for the `ESP32` model, please add the following to `Cargo.toml`:

```
[dependencies]

easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32"] }

[profile.dev.package.esp-storage]

opt-level = 3

```
If you are using the `ESP32C3` model, please add the following to `Cargo.toml`:

```
[dependencies]

easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32c3"] }

[profile.dev.package.esp-storage]

opt-level = 3

```
(1b) If referenced from crates.io, for the `ESP32` model, please add the following to `Cargo.toml`:

```
[dependencies]

easy_store = { version = "0.4.0", features = ["esp32"] }

[profile.dev.package.esp-storage]

opt-level = 3

```
If the model is `ESP32C3`, please add the following to `Cargo.toml`:

```
[dependencies]

easy_store = { version = "0.4.0", features = ["esp32c3"] }

[profile.dev.package.esp-storage]

opt-level = 3

```
(2) Next, add a partition table `partitions.csv` in the directory:

```
#     Name,       Type,       SubType,       Offset,       Size,       Flags
       nvs,       data,           nvs,       0x9000,     0x4000
   otadata,       data,           ota,       0xD000,     0x2000
  phy_init,       data,           phy,       0xF000,     0x1000
     ota_0,        app,         ota_0,      0x10000,   0x150000
     ota_1,        app,         ota_1,     0x160000,   0x150000
easy_store,       data,        spiffs,     0x3A0000,    0x50000

```
In the partition table above, please refer to the following instructions for the usage of each field:

The `[Name]` field represents the name of the partition to be used to store data. The name can be specified arbitrarily; here it is set to easy_store.<br>

The `[Type]` field should be set to `data`.<br>

The `[SubType]` field should be set to `spiffs`.<br>

The `[Offset]` field indicates the starting memory location of the partition used to store data. Here it's set to `0x3A0000`, but you can change this value arbitrarily as long as it does not overlap with the preceding partitions and stays 4096-aligned.<br>
The `[Size]` field indicates the size of the partition used to store data. Here it's set to `0x50000`, which is equivalent to setting the partition size to 320KB. However, you can change this value arbitrarily to increase or decrease the space.<br>
Note that to use the partition table, you need to add the command `--partition-table partitions.csv` to `.cargo/config.toml` to enable the partition table. An example of `config.toml` is as follows:

```
[target.riscv32imc-unknown-none-elf]

runner = "espflash flash --monitor --chip esp32c3 --partition-table partitions.csv"
```

(3) After adding all the above, you can start using it. To reference `easy_store`, add the following:

```
#![no_std]

#![no_main]

use easy_store::store::Store;

```

On ESP devices, create the store with `Store::new_esp`, passing the `FLASH` peripheral obtained from `esp_hal::init`. All operations return `Result`, so failures (missing file, corrupted data, no space...) are explicit values instead of silent empty strings:

```
let peripherals = esp_hal::init(esp_hal::Config::default());

let mut store = Store::new_esp(peripherals.FLASH, 0x3A0000, 0x50000);

let file_name = "/data/system_record_file.txt"; // file name (UTF-8)
let file_data = "Hello World!!!";               // file content (UTF-8)

store.write(file_name, file_data).unwrap();
println!("Archived --> {:?}", file_name);

let file_data = store.read(file_name).unwrap();
println!("Read file content -->\n{}", file_data);
```

If the file content is not text but arbitrary binary data (e.g. images, audio, compressed files), use `write_bytes` and `read_bytes` instead. They take `&[u8]` and return `Vec<u8>`:

```
let file_name = "/img/logo.png";
let file_data: &[u8] = include_bytes!("logo.png"); // arbitrary binary data

store.write_bytes(file_name, file_data).unwrap();

let file_data = store.read_bytes("/img/logo.png").unwrap(); // Vec<u8>
```
`write` and `read` are thin wrappers around the byte-oriented versions (doing UTF-8 encode/decode). The underlying storage format makes no assumption about the content.

# API Overview

| Function | Description |
|---|---|
| `write(name, &str)` / `write_bytes(name, &[u8])` | Store a file, overwriting any existing one with the same name |
| `read(name)` / `read_bytes(name)` | Read a whole file (content verified against its CRC32) |
| `read_range(name, offset, &mut buf)` | Read part of a file without loading it all into RAM |
| `append(name, &str)` / `append_bytes(name, &[u8])` | Append to a file, creating it if missing |
| `rename(from, to)` | Rename a file, replacing `to` if it exists |
| `delete(name)` | Delete a file |
| `delete_dir(path)` | Delete every file under a path, returns the count |
| `delete_all_data()` | Delete every file |
| `exists(name)` | Exact-match existence check |
| `file_size(name)` | File size in bytes (reads only the header) |
| `files()` | List all file names |
| `read_dir(path)` | List direct children of a path (like `std::fs::read_dir`) |
| `capacity()` / `used_space()` / `free_space()` | Space accounting in bytes |
| `show_all_data_name()` `show_read_dir()` `show_file_name_exist()` `show_usage_cluster()` `show_usage_capacity()` | `println!` helpers (ESP feature only) |

Every fallible operation returns `Result<_, StoreError>`:

| `StoreError` | Meaning |
|---|---|
| `NotFound` | No file with that name |
| `Corrupted` | CRC mismatch or broken cluster chain (e.g. torn write) |
| `NoSpace` | Not enough free clusters |
| `NameTooLong` | File name longer than `MAX_NAME_LEN` (4060 bytes) |
| `NotUtf8` | `read()` used on non-text content (use `read_bytes()`) |
| `Storage` | The underlying flash driver reported an error |

# Enumerating files

To enumerate or look up files programmatically, use `files()`, `read_dir(path)` and `exists(file_name)`:

```
let all = store.files();
// all files in storage, e.g.
// ["/data/foo.txt", "/data/sub/a.txt", "/user/cfg.json"]

let entries = store.read_dir("/data");
// direct children of "/data" only; subdirectories are marked with a trailing '/'
// ["/data/foo.txt", "/data/sub/"]

if store.exists("/data/foo.txt") {
    // ...
}
```

`read_dir` mirrors `std::fs::read_dir` semantics on top of the flat key-value layout:

- The trailing `/` of the prefix is normalised, so `read_dir("/data")` and `read_dir("/data/")` are equivalent.
- Only direct children are returned. `/data/sub/inner.txt` is reported as the synthetic directory entry `/data/sub/` (note the trailing `/`); to descend further, call `read_dir("/data/sub")`.
- Sibling prefixes are not confused — `read_dir("/data")` does not include `/data2/...`.
- Use `path.ends_with('/')` on each entry to tell a synthetic directory from a real file.

`exists` is a strict equality check on the full file path — `exists("/data")` returns `false` even when files like `/data/foo.txt` exist.

# Reading large files piece by piece

`read_bytes` loads the whole file into RAM, which may be impossible for files close to the partition size. `read_range` copies an arbitrary window instead, following the cluster chain and skipping clusters outside the window:

```
let mut buf = [0u8; 512];
let n = store.read_range("/media/big.bin", 40960, &mut buf).unwrap();
// n bytes copied; 0 when offset is at/past the end of file
```

Unlike `read_bytes`, `read_range` does not verify the whole-file CRC (that would require reading everything).

# Appending, renaming, deleting

```
store.append("/log/boot.log", "boot ok\n").unwrap();  // created on first use

store.rename("/cfg/current.json", "/cfg/backup.json").unwrap();

store.delete("/cfg/backup.json").unwrap();

let removed = store.delete_dir("/log").unwrap();  // number of files removed

store.delete_all_data().unwrap();
```

Note: `append` and `rename` pass the existing content through RAM (read + rewrite).

# Space accounting

```
let total = store.capacity();     // managed bytes (multiple of 4096)
let used  = store.used_space();   // bytes taken by used clusters
let free  = store.free_space();   // bytes still available

store.show_usage_cluster();       // +/- map of clusters (ESP only)
store.show_usage_capacity();      // printed totals (ESP only)
```

# Using easy_store on non-ESP chips

The core is `Store<S: embedded_storage::Storage>` and has no default features. Bring any driver that implements `ReadStorage` + `Storage` (most HAL flash drivers do, e.g. via `embedded-storage` adapters) and construct the generic way:

```
[dependencies]
easy_store = "0.4.0"          # no features
```

```rust
use easy_store::Store;

let mut store = Store::new(my_flash_driver, PARTITION_ADDR, PARTITION_SIZE);
store.write("/cfg.txt", "hello").unwrap();
```

`flash_addr` must be 4096-aligned and `flash_size` at least 4096 (rounded down to a multiple of 4096). The same trick powers the host-side test suite in `tests/host.rs`, which runs the full store against a RAM-backed mock — handy as a reference implementation.

# Reliability behavior

- File names, headers and content are all protected by CRC32; damaged entries are reported as `Corrupted` instead of returning garbage.
- Writes are atomic at the file level: the header cluster is written last, and an overwrite deletes the old copy only after the new one is fully written. A power loss can therefore cost you at most the file being written — never the previous version.
- Every `Store::new` scans the partition and self-heals: leftovers of interrupted writes are reclaimed, and if a power loss left two copies of the same name, the newer generation wins and the older one is removed.
- Because the old copy is kept until the new one is complete, overwriting a file needs free space for both copies at the same time.
- Write cursor rotation spreads erases across the whole partition to even out flash wear.