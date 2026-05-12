# easy_store

easy_store是用於在`esp32`及`esp32c3`中的flash中儲存檔案的Rust開源程式，請參考`tests`中的範例。<br>
`esp32c2`、`esp32c6`、`esp32h2`、`esp32s2`及`esp32s3`並沒有實際驗證過，但是可以使用`esp-generate`建立檔案，並參考`esp32`及`esp32c3`中的用法。

Language:
- [English](README.md)
- [繁體中文](README.zh-TW.md)
# 快速測試
如果要快速測試easy_store在ESP的裝置上是否可以使用，請直接將目錄指定到對應的資料夾，例如`\easy_store\tests\esp32`，並執行`cargo run`。
# 使用教學
(1a)如果是直接從GitHub引用的，在`ESP32`的型號請在`Cargo.toml`中新增
```
[dependencies]
easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
如果是`ESP32C3`的型號請在`Cargo.toml`中新增
```
[dependencies]
easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32c3"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
(1b)如果是從crates.io引用的，在`ESP32`的型號請在`Cargo.toml`中新增
```
[dependencies]
easy_store = { version = "0.3.0", features = ["esp32"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
如果是`ESP32C3`的型號請在`Cargo.toml`中新增
```
[dependencies]
easy_store = { version = "0.3.0", features = ["esp32c3"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
(2)接著於目錄下新增分區表`partitions.csv`
```
#     Name,       Type,       SubType,       Offset,       Size,       Flags
       nvs,       data,           nvs,       0x9000,     0x4000
   otadata,       data,           ota,       0xD000,     0x2000
  phy_init,       data,           phy,       0xF000,     0x1000
     ota_0,        app,         ota_0,      0x10000,   0x150000
     ota_1,        app,         ota_1,     0x160000,   0x150000
easy_store,       data,        spiffs,     0x3A0000,    0x50000
```
在上述的分區表中，各個欄位的用法請參閱以下說明<br>
`[Name]`的欄位中表示要用於存放資料的分區名稱，名稱可以任意指定，這裡設定成easy_store。<br>
`[Type]`的欄位請設定為`data`。<br>
`[SubType]`的欄位請設定為`spiffs`。<br>
`[Offset]`的欄位表示用於存放資料的分區開始的記憶體位置，這裡設定成`0x3A0000`，但你可以任意更改這個值，只要不與前面的分區重疊即可。<br>
`[Size]`的欄位表示用於存放資料的分區大小，這裡設定成`0x50000`，相當於將分區的大小設定成320KB，但你可以任意更改這個值使空間增加或減少。<br>
<br>
要注意，使用分區表需要在`.cargo/config.toml`中新增`--partition-table partitions.csv`的指令才會啟用分區表，`config.toml`可以參考的範例例如:
```
[target.riscv32imc-unknown-none-elf]
runner = "espflash flash --monitor --chip esp32c3 --partition-table partitions.csv"
```

(3)以上都新增好後就可以開始使用了，要引用`easy_store`時新增
```
#![no_std]
#![no_main]
use easy_store::store::Store;
```

在後續中使用假設要新增路徑名稱為`/data/系統紀錄檔.txt`的檔案，可以使用下列方式。

`Store::new` 的第一個參數現在需要傳入 `FLASH` peripheral，所以要先從 `esp_hal::init` 取得 `peripherals` 再把 `peripherals.FLASH` 傳進去：
```
let peripherals = esp_hal::init(esp_hal::Config::default());

let file_name = "/data/系統紀錄檔.txt"; // 檔案名稱（UTF-8）
let file_data = "Hello World!!!";      // 檔案資料（UTF-8）

let mut store = Store::new(peripherals.FLASH, 0x3A0000, 0x50000);
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

如果要儲存的不是文字而是任意二進位資料（例如圖片、音訊、壓縮檔等），請改用 `write_bytes` 與 `read_bytes`，它們的參數及回傳值是 `&[u8]` 與 `Vec<u8>`：
```
let file_name = "/img/logo.png";
let file_data: &[u8] = include_bytes!("logo.png"); // 任意二進位資料

store.write_bytes(file_name, file_data);

let file_data = store.read_bytes("/img/logo.png"); // 回傳 Vec<u8>
```
`write` 與 `read` 其實是上述兩個函式的薄封裝（多做了 UTF-8 編解碼），底層儲存格式對內容沒有任何限制。


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





































