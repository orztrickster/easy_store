# easy_store

easy_store 是將檔案儲存在 SPI NOR flash 中的 Rust 開源程式庫。

`esp32` 及 `esp32c3` 已在實體硬體上驗證（請參考 `tests` 中的範例）。`esp32c2`、`esp32c6`、`esp32h2`、`esp32s2` 及 `esp32s3` 走相同的程式路徑，但沒有實際驗證過。

Language:
- [English](README.md)
- [繁體中文](README.zh-TW.md)

# 快速測試

如果要快速測試 easy_store 在 ESP 的裝置上是否可以使用，請直接將目錄指定到對應的資料夾，例如 `\easy_store\tests\esp32`，並執行 `cargo run`。

如果要在電腦上執行測試（不需要硬體）：

```
cargo test --target x86_64-pc-windows-msvc   # 或你的主機 target
```

# 使用教學（ESP32 系列）

(1a) 如果是直接從 GitHub 引用的，在 `ESP32` 的型號請在 `Cargo.toml` 中新增
```
[dependencies]
easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
如果是 `ESP32C3` 的型號請在 `Cargo.toml` 中新增
```
[dependencies]
easy_store = { git = "https://github.com/orztrickster/easy_store", branch = "master", features = ["esp32c3"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
(1b) 如果是從 crates.io 引用的，在 `ESP32` 的型號請在 `Cargo.toml` 中新增
```
[dependencies]
easy_store = { version = "0.4.0", features = ["esp32"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
如果是 `ESP32C3` 的型號請在 `Cargo.toml` 中新增
```
[dependencies]
easy_store = { version = "0.4.0", features = ["esp32c3"] }
[profile.dev.package.esp-storage]
opt-level = 3
```
(2) 接著於目錄下新增分區表 `partitions.csv`
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
`[Name]` 的欄位中表示要用於存放資料的分區名稱，名稱可以任意指定，這裡設定成 easy_store。<br>
`[Type]` 的欄位請設定為 `data`。<br>
`[SubType]` 的欄位請設定為 `spiffs`。<br>
`[Offset]` 的欄位表示用於存放資料的分區開始的記憶體位置，這裡設定成 `0x3A0000`，但你可以任意更改這個值，只要不與前面的分區重疊、並保持對齊 4096 即可。<br>
`[Size]` 的欄位表示用於存放資料的分區大小，這裡設定成 `0x50000`，相當於將分區的大小設定成 320KB，但你可以任意更改這個值使空間增加或減少。<br>
<br>
要注意，使用分區表需要在 `.cargo/config.toml` 中新增 `--partition-table partitions.csv` 的指令才會啟用分區表，`config.toml` 可以參考的範例例如:
```
[target.riscv32imc-unknown-none-elf]
runner = "espflash flash --monitor --chip esp32c3 --partition-table partitions.csv"
```

(3) 以上都新增好後就可以開始使用了，要引用 `easy_store` 時新增
```
#![no_std]
#![no_main]
use easy_store::store::Store;
```

在 ESP 裝置上使用 `Store::new_esp` 建立 store，傳入從 `esp_hal::init` 取得的 `FLASH` peripheral。所有操作都回傳 `Result`，失敗（檔案不存在、資料損毀、空間不足等）會是明確的錯誤值，而不是無聲的空字串：

```
let peripherals = esp_hal::init(esp_hal::Config::default());

let mut store = Store::new_esp(peripherals.FLASH, 0x3A0000, 0x50000);

let file_name = "/data/系統紀錄檔.txt"; // 檔案名稱（UTF-8）
let file_data = "Hello World!!!";      // 檔案資料（UTF-8）

store.write(file_name, file_data).unwrap();
println!("已存檔 --> {:?}", file_name);

let file_data = store.read(file_name).unwrap();
println!("讀取檔案內容 -->\n{}", file_data);
```

如果要儲存的不是文字而是任意二進位資料（例如圖片、音訊、壓縮檔等），請改用 `write_bytes` 與 `read_bytes`，它們的參數及回傳值是 `&[u8]` 與 `Vec<u8>`：
```
let file_name = "/img/logo.png";
let file_data: &[u8] = include_bytes!("logo.png"); // 任意二進位資料

store.write_bytes(file_name, file_data).unwrap();

let file_data = store.read_bytes("/img/logo.png").unwrap(); // Vec<u8>
```
`write` 與 `read` 是上述兩個函式的薄封裝（多做了 UTF-8 編解碼），底層儲存格式對內容沒有任何限制。

# API 總覽

| 函式 | 說明 |
|---|---|
| `write(name, &str)` / `write_bytes(name, &[u8])` | 儲存檔案，同名檔案會被覆寫 |
| `read(name)` / `read_bytes(name)` | 讀取整個檔案（內容會用 CRC32 驗證） |
| `read_range(name, offset, &mut buf)` | 讀取檔案的一部分，不必整份載入 RAM |
| `append(name, &str)` / `append_bytes(name, &[u8])` | 追加內容，檔案不存在會自動建立 |
| `rename(from, to)` | 更名，`to` 已存在時會被取代 |
| `delete(name)` | 刪除檔案 |
| `delete_dir(path)` | 刪除路徑下的全部檔案，回傳刪除數量 |
| `delete_all_data()` | 刪除全部檔案 |
| `exists(name)` | 完整路徑的精確比對 |
| `file_size(name)` | 檔案大小（只讀 header，不讀內容） |
| `files()` | 列出全部檔案名稱 |
| `read_dir(path)` | 列出路徑的直屬子項目（語意同 `std::fs::read_dir`） |
| `capacity()` / `used_space()` / `free_space()` | 以 bytes 回傳的容量統計 |
| `show_all_data_name()` `show_read_dir()` `show_file_name_exist()` `show_usage_cluster()` `show_usage_capacity()` | `println!` 顯示用（僅 ESP feature） |

所有可能失敗的操作都回傳 `Result<_, StoreError>`：

| `StoreError` | 意義 |
|---|---|
| `NotFound` | 沒有這個名稱的檔案 |
| `Corrupted` | CRC 驗證失敗或 cluster 鏈損毀（例如寫入中斷電的殘檔） |
| `NoSpace` | 可用空間不足 |
| `NameTooLong` | 檔名超過 `MAX_NAME_LEN`（4060 bytes） |
| `NotUtf8` | 用 `read()` 讀非文字內容（請改用 `read_bytes()`） |
| `Storage` | 底層 flash 驅動回報錯誤 |

# 列出與查詢檔案

要在程式裡列出或檢查檔案，使用 `files()`、`read_dir(path)`、`exists(file_name)`：

```
let all = store.files();
// 全部檔案，例如：
// ["/data/foo.txt", "/data/sub/a.txt", "/user/cfg.json"]

let entries = store.read_dir("/data");
// 只列「/data」的直屬子項目；子目錄結尾用 '/' 標示
// ["/data/foo.txt", "/data/sub/"]

if store.exists("/data/foo.txt") {
    // ...
}
```

`read_dir` 對齊 `std::fs::read_dir` 的語意（底層是扁平的 key-value）：

- 結尾 `/` 會自動正規化，`read_dir("/data")` 與 `read_dir("/data/")` 等價。
- 只回直屬子項目。`/data/sub/inner.txt` 不會直接出現，而是以 `/data/sub/`（結尾 `/`）這個合成目錄出現；要再進去看就呼叫 `read_dir("/data/sub")`。
- 同前綴但不同目錄不會混淆 —— `read_dir("/data")` 不會把 `/data2/...` 列進來。
- 對每個回傳項目可用 `path.ends_with('/')` 判斷是合成目錄還是真實檔案。

`exists` 是完整路徑的精確比對 —— 就算 `/data/foo.txt` 存在，`exists("/data")` 仍會回 `false`（因為沒有「`/data`」這個檔案）。

# 分段讀取大檔案

`read_bytes` 會把整個檔案載入 RAM，接近分區大小的檔案可能載不動。`read_range` 改為複製任意區段，沿著 cluster 鏈前進、跳過範圍外的 cluster：

```
let mut buf = [0u8; 512];
let n = store.read_range("/media/big.bin", 40960, &mut buf).unwrap();
// 複製了 n bytes；offset 在檔案結尾之後會回 0
```

與 `read_bytes` 不同，`read_range` 不做全檔 CRC 驗證（那需要讀完整份檔案）。

# 追加、更名、刪除

```
store.append("/log/boot.log", "boot ok\n").unwrap();  // 第一次使用會自動建檔

store.rename("/cfg/current.json", "/cfg/backup.json").unwrap();

store.delete("/cfg/backup.json").unwrap();

let removed = store.delete_dir("/log").unwrap();  // 回傳刪除的檔案數

store.delete_all_data().unwrap();
```

注意：`append` 與 `rename` 的既有內容會經過 RAM（讀出後重寫）。

# 容量統計

```
let total = store.capacity();     // 管理的總空間（4096 的倍數）
let used  = store.used_space();   // 已使用 cluster 佔的空間
let free  = store.free_space();   // 尚可使用的空間

store.show_usage_cluster();       // 各 cluster 的 +/- 圖（僅 ESP）
store.show_usage_capacity();      // 印出總容量與使用量（僅 ESP）
```

# 在非 ESP 晶片上使用

核心是 `Store<S: embedded_storage::Storage>`，預設不啟用任何 feature。只要你的 flash 驅動實作了 `ReadStorage` + `Storage`（多數 HAL 都有對應的 adapter），就能用泛型建構子：

```
[dependencies]
easy_store = "0.4.0"          # 不加 feature
```

```rust
use easy_store::Store;

let mut store = Store::new(my_flash_driver, PARTITION_ADDR, PARTITION_SIZE);
store.write("/cfg.txt", "hello").unwrap();
```

`flash_addr` 必須對齊 4096，`flash_size` 至少 4096（多餘的尾數會捨去）。`tests/host.rs` 的主機端測試就是用同樣方式對 RAM 模擬 flash 跑完整測試，可以當作參考實作。

# 可靠性行為

- 檔名、header、內容都有 CRC32 保護；損毀的資料會回報 `Corrupted`，不會回傳垃圾內容。
- 寫入以檔案為單位是原子的：header 所在的首塊最後才寫入，覆寫時舊檔要等新檔完整落地後才刪除。斷電最多損失「正在寫的那份」，永遠不會弄丟前一版。
- 每次 `Store::new` 都會掃描分區並自我修復：寫到一半的殘留會被回收；若斷電讓同名檔案留下兩份，會保留世代較新的那份、清掉舊的。
- 因為舊檔要保留到新檔寫完，覆寫檔案時需要新舊兩份同時放得下的空間。
- 寫入位置會輪流分散到整個分區，平均 flash 的抹寫磨損。