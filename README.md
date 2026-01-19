# easy_store

easy_store is an open-source Rust program for storing files in the flash memory of `esp32`, `esp32c3`, and `esp32c6`. It is currently in testing, but most functions should already be usable.

Language:

- [English](README.md)

- [Traditional Chinese](README.zh-TW.md)

# Quick Test
To quickly test whether easy_store works on an ESP device, simply specify the directory to the corresponding folder, for example, `\easy_store\tests\esp32`, and execute `cargo run`.

# Usage Instructions

(1) If you are referencing directly from GitHub, for the `ESP32` model, please add the following to `Cargo.toml`:

```
[dependencies]

easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32"] }

[profile.dev.package.esp-storage]

opt-level = 3

```
For the `ESP32C3` model, please add the following to `Cargo.toml`:

```
[dependencies]

easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32c3"] }

[profile.dev.package.esp-storage]

opt-level = 3

```
(2) Next, add a partition table `partitions.csv` to the directory:

```
# Name, Type, SubType, Offset, Size, Flags

nvs, data, nvs, 0x9000, 0x4000

otadata, data, ota, 0xD000, 0x2000

phy_init, data, phy, 0xF000, 0x1000

factory, app, factory, 0x10000, 0x200000

easy_store, 2, 0x40, 0x210000, 0x100000

``` 
In the partition table above, please refer to the following instructions for the usage of each field:

The `[Name]` field represents the name of the partition to be used to store data. The name can be arbitrarily specified; here it is set to easy_store.

The `[Type]` field should be set to any value other than 0 or 1; here it is set to 2.

The `[SubType]` field should be set to 0x40. The `[Offset]` field indicates the starting memory location of the partition used to store data. Here it's set to `0x210000`, but you can change this value arbitrarily.
The `[Size]` field indicates the size of the partition used to store data. Here it's set to `0x100000`, which is equivalent to setting the partition size to 1MB. However, you can change this value arbitrarily to increase or decrease the space. <br>
<br>Note that using partition tables requires adding the command `--partition-table partitions.csv` to `.cargo/config.toml` to enable partition tables. An example of `config.toml` is as follows:

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

In subsequent use, assuming you want to add a file with the path name `/data/system_record.txt`, you can use the following method:

```
let file_name = "/data/system_record.txt"; // File name (UTF-8)

let file_data = "Hello World!!!"; // File data (UTF-8)

let mut store = Store::new(0x210000, 0x100000);

store.delete_all_data();

store.write(file_name,file_data);

println!("Archived --> {:?}",file_name);

```

In subsequent uses, assuming you want to read the file named `/data/system_log_file.txt`, you can use the following methods:

``` 
store.show_file_name_exist("/data/system_log_file.txt");

let file_data = store.read("/data/system_log_file.txt");

println!("Reading file content -->\n{}", file_data);

```

To read the usage, you can use:

``` 
store.show_usage_cluster();
```

To delete the file named `/data/system_log_file.txt`, you can use the following methods:

```
store.delete("/data/system_log_file.txt");
```

To delete all files, you can use the following methods:

``` 
store.delete_all_data();
```




























