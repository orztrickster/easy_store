# easy_store

easy_store是用於在esp32c3中的flash中儲存檔案的Rust開源程式，目前還在測試中，但大多功能應已可使用。

Language:
- [English](README.md)
- [繁體中文](README.zh-TW.md)

# 使用教學
在`Cargo.toml`中新增
```
[dependencies]
easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master" }
```

於目錄下新增分區表`partitions.csv`
```
# Name,   Type, SubType, Offset,   Size,     Flags
nvs,      data, nvs,     0x9000,   0x4000
otadata,  data, ota,     0xD000,   0x2000
phy_init, data, phy,     0xF000,   0x1000
factory,  app,  factory, 0x10000,  0x200000
easy_store,2, 0x40,   0x210000, 0x100000
```
在上述的分區表中，各個欄位的用法請參閱以下說明<br>
[Name]的欄位中表示要用於存放資料的分區名稱，名稱可以任意指定，這裡設定成easy_store。<br>
[Type]的欄位中請設定除了0、1以外的任意值，這裡設定2。<br>
[SubType]的欄位請設定0x40。<br>
[Offset]的欄位表示用於存放資料的分區開始的記憶體位置，這裡設定成0x210000，但你可以任意更改這個值。<br>
[Size]的欄位表示用於存放資料的分區大小，這裡設定成0x100000，大小是1mb，但你可以任意更改這個值使空間增加或減少。<br>
<br>
要注意，使用分區表需要在`.cargo/config.toml`中新增`--partition-table partitions.csv`的指令才會啟用分區表，`config.toml`可以參考的範例例如:
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

以上都新增好後就可以開始使用了，要引用`easy_store`時新增
```
#![no_std]
#![no_main]
use easy_store::store::Store;
```

在後續中使用假設要新增路徑名稱為`/data/系統紀錄檔.txt`的檔案，可以使用下列方式
```
let file_name = "/data/系統紀錄檔.txt"; // 檔案名稱（UTF-8）
let file_data = "Hello World!!!";      // 檔案資料（UTF-8）

let mut store = Store::new(0x210000, 0x100000);
store.delete_all_data();

store.write(file_name,file_data);
println!("已存檔 --> {:?}",file_name);
```

在後續中使用假設要讀取路徑名稱為`/data/系統紀錄檔.txt`的檔案，可以使用下列方式
```
store.show_file_name_exist("/data/系統紀錄檔.txt");

let file_data = store.read("/data/系統紀錄檔.txt");
println!("讀取檔案內容 -->\n{}", file_data);
```


如果要讀取使用了多少容量可以使用
```
store.show_usage_cluster();
```

如果要刪除路徑名稱為`/data/系統紀錄檔.txt`的檔案，可以使用下列方式
```
store.delete("/data/系統紀錄檔.txt");
```

如果要刪除全部檔案，可以使用下列方式
```
store.delete_all_data();
```





































