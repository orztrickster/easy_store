# easy_store

`easy_store` is an open-source Rust library for storing files in the flash memory of an ESP32-C3. The project is currently in testing, but most features should already be usable.

Language:
- [English](README.md)
- [繁體中文](README.zh-TW.md)

## Usage

Add the dependency in `Cargo.toml`:

```
[dependencies]
easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master" }
```

Create a partition table `partitions.csv` in the project directory:

```
#     Name,       Type,       SubType,       Offset,       Size,       Flags
       nvs,       data,           nvs,       0x9000,     0x4000
   otadata,       data,           ota,       0xD000,     0x2000
  phy_init,       data,           phy,       0xF000,     0x1000
   factory,        app,       factory,      0x10000,   0x200000
easy_store,          2,          0x40,     0x210000,   0x100000
```
In the partition table above, the usage of each field is as follows:<br>
The [Name] field specifies the name of the partition used for storing data, which can be任意指定，這裡設定成easy_store。<br>
The [Type] field should be set to any value other than 0 or 1, here it is set to 2. <br>
The [SubType] field should be set to 0x40.<br>
The [Offset] field indicates the memory location where the data partition begins, set here as 0x210000, but you can change this value as needed.<br>
The [Size] field indicates the size of the data partition, set here as 0x100000, equivalent to 1MB, but you can change this value to increase or decrease the space.<br>
<br>
Note that using a partition table requires adding the --partition-table partitions.csv directive in .cargo/config.toml to activate the partition table. Here is an example of config.toml:
```
[target.riscv32imc-unknown-none-elf]
runner = "espflash flash --monitor --chip esp32c3 --partition-table partitions.csv"

[env]

[build]
rustflags = [
  # Required to obtain backtraces (e.g. when using the "esp-backtrace" crate.)
  # NOTE: May negatively impact performance of produced code
  "-C", "force-frame-pointers",
]

target = "riscv32imc-unknown-none-elf"

[unstable]
build-std = ["alloc", "core"]
```

Once all the above are set up, you can start using `easy_store`. To use `easy_store`, add the following line in your code:
```
#![no_std]
#![no_main]
use easy_store::store::Store;
```

To add a file with the path name `/data/system_data.txt`, you can use the following code:
```
let file_name = "/data/system_data.txt"; // File name (UTF-8)
let file_data = "Hello World!!!";      // File content (UTF-8)

let mut store = Store::new(0x210000, 0x100000);
store.delete_all_data();

store.write(file_name,file_data);
println!("File saved --> {:?}",file_name);
```

To read the file with the path name `/data/system_data.txt`, you can use the following code:
```
store.show_file_name_exist("/data/system_data.txt");

let file_data = store.read("/data/system_data.txt");
println!("File content -->\n{}", file_data);
```


To check the used storage capacity, you can use:
```
store.show_usage_cluster();
```

To delete the file with the path name `/data/system_data.txt`, you can use the following code:
```
store.delete("/data/system_data.txt");
```

To delete all files, you can use the following code:
```
store.delete_all_data();
```




































