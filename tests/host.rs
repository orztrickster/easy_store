// 主機端測試：用 RAM 模擬 flash，驗證儲存格式的全部行為
// 執行：cargo test --target <主機 target>（根目錄 .cargo/config.toml 預設 target 是 riscv）
use easy_store::{Store, StoreError, MAX_NAME_LEN};
use embedded_storage::{ReadStorage, Storage};

const CLUSTER: usize = 4096;
const FLASH_SIZE: u32 = 0x50000; // 80 clusters，與 tests/esp32 的分區一致

struct RamFlash {
    mem: Vec<u8>,
}

impl RamFlash {
    fn new(size: usize) -> Self {
        Self {
            mem: vec![0xFF; size],
        }
    }
}

impl ReadStorage for RamFlash {
    type Error = ();
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), ()> {
        let o = offset as usize;
        if o + bytes.len() > self.mem.len() {
            return Err(());
        }
        bytes.copy_from_slice(&self.mem[o..o + bytes.len()]);
        Ok(())
    }
    fn capacity(&self) -> usize {
        self.mem.len()
    }
}

impl Storage for RamFlash {
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ()> {
        let o = offset as usize;
        if o + bytes.len() > self.mem.len() {
            return Err(());
        }
        self.mem[o..o + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
}

fn new_store() -> Store<RamFlash> {
    Store::new(RamFlash::new(FLASH_SIZE as usize), 0, FLASH_SIZE)
}

// 模擬重新開機（會重跑 heal）
fn reopen(store: Store<RamFlash>) -> Store<RamFlash> {
    Store::new(store.into_storage(), 0, FLASH_SIZE)
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

// 找出所有以 MAGIC 開頭的 cluster index
fn magic_clusters(store: &mut Store<RamFlash>) -> Vec<usize> {
    let magic = [0x01, 0x31, 0x2A, 0xAB];
    let mem = &store.storage_mut().mem;
    (0..mem.len() / CLUSTER)
        .filter(|i| mem[i * CLUSTER..i * CLUSTER + 4] == magic)
        .collect()
}

#[test]
fn roundtrip_small() {
    let mut s = new_store();
    s.write("/data/a.txt", "Hello World!!!").unwrap();
    assert_eq!(s.read("/data/a.txt").unwrap(), "Hello World!!!");
    assert_eq!(s.read_bytes("/data/a.txt").unwrap(), b"Hello World!!!");
}

#[test]
fn roundtrip_multi_cluster() {
    let mut s = new_store();
    let data = pattern(12000);
    s.write_bytes("/data/big.bin", &data).unwrap();
    assert_eq!(s.read_bytes("/data/big.bin").unwrap(), data);
}

#[test]
fn roundtrip_empty_file() {
    let mut s = new_store();
    s.write_bytes("/empty", &[]).unwrap();
    assert!(s.exists("/empty"));
    assert_eq!(s.read_bytes("/empty").unwrap(), Vec::<u8>::new());
    assert_eq!(s.file_size("/empty").unwrap(), 0);
}

// 掃過所有 cluster 邊界附近的大小（舊版 0.3.x 在部分大小會 panic）
#[test]
fn boundary_size_sweep() {
    let name = "/data/x.bin"; // 11 bytes → 首塊資料容量 4060 - 11 = 4049
    let cap0 = 4060 - name.len();
    let mut sizes: Vec<usize> = Vec::new();
    for base in [cap0, cap0 + 4080, cap0 + 2 * 4080] {
        for delta in -6i64..=6 {
            let v = base as i64 + delta;
            if v >= 0 {
                sizes.push(v as usize);
            }
        }
    }
    for d in sizes {
        let mut s = new_store();
        let data = pattern(d);
        s.write_bytes(name, &data).unwrap();
        let r = s.read_bytes(name).unwrap();
        assert_eq!(r, data, "資料長度 {} 的往返失敗", d);
        assert_eq!(s.file_size(name).unwrap(), d);
    }
}

#[test]
fn max_name_length() {
    // 檔名撐滿上限：首塊資料容量為 0，資料全部在接續塊
    let name: String = std::iter::repeat('n').take(MAX_NAME_LEN).collect();
    let data = pattern(5000);
    let mut s = new_store();
    s.write_bytes(&name, &data).unwrap();
    assert_eq!(s.read_bytes(&name).unwrap(), data);

    // 空資料也要能存
    s.write_bytes(&name, &[]).unwrap();
    assert_eq!(s.read_bytes(&name).unwrap(), Vec::<u8>::new());
}

#[test]
fn name_too_long_rejected() {
    let name: String = std::iter::repeat('n').take(MAX_NAME_LEN + 1).collect();
    let mut s = new_store();
    assert_eq!(s.write_bytes(&name, b"hi"), Err(StoreError::NameTooLong));
    assert!(s.files().is_empty());
}

#[test]
fn not_found_errors() {
    let mut s = new_store();
    assert_eq!(s.read("/none"), Err(StoreError::NotFound));
    assert_eq!(s.read_bytes("/none"), Err(StoreError::NotFound));
    assert_eq!(s.file_size("/none"), Err(StoreError::NotFound));
    assert_eq!(s.delete("/none"), Err(StoreError::NotFound));
    assert_eq!(s.read_range("/none", 0, &mut [0u8; 4]), Err(StoreError::NotFound));
    assert!(!s.exists("/none"));
}

#[test]
fn not_utf8_error() {
    let mut s = new_store();
    s.write_bytes("/bin", &[0xFF, 0xFE, 0x00, 0x9C]).unwrap();
    assert_eq!(s.read("/bin"), Err(StoreError::NotUtf8));
    assert_eq!(s.read_bytes("/bin").unwrap(), vec![0xFF, 0xFE, 0x00, 0x9C]);
}

#[test]
fn overwrite_replaces_content() {
    let mut s = new_store();
    let big = pattern(9000);
    s.write_bytes("/cfg", &big).unwrap();
    s.write("/cfg", "v2").unwrap();
    assert_eq!(s.read("/cfg").unwrap(), "v2");
    assert_eq!(s.files(), vec![String::from("/cfg")]);
    // 舊的 3 個 cluster 要被釋放，只剩 1 個
    assert_eq!(s.used_space(), CLUSTER as u32);
}

#[test]
fn exists_is_exact_match() {
    let mut s = new_store();
    s.write("/data/foo.txt", "x").unwrap();
    assert!(s.exists("/data/foo.txt"));
    assert!(!s.exists("/data"));
    assert!(!s.exists("/data/"));
}

#[test]
fn delete_frees_space() {
    let mut s = new_store();
    let data = pattern(10000);
    s.write_bytes("/a", &data).unwrap();
    assert!(s.used_space() > 0);
    s.delete("/a").unwrap();
    assert_eq!(s.used_space(), 0);
    assert_eq!(s.read_bytes("/a"), Err(StoreError::NotFound));
    // 刪掉後空間要能重複使用
    s.write_bytes("/a", &data).unwrap();
    assert_eq!(s.read_bytes("/a").unwrap(), data);
}

#[test]
fn delete_all_data() {
    let mut s = new_store();
    s.write("/a", "1").unwrap();
    s.write_bytes("/b", &pattern(9000)).unwrap();
    s.delete_all_data().unwrap();
    assert!(s.files().is_empty());
    assert_eq!(s.used_space(), 0);
}

#[test]
fn wear_leveling_cursor_rotates() {
    let mut s = new_store();
    s.write("/w", "1").unwrap();
    assert_eq!(magic_clusters(&mut s), vec![0]);
    s.delete("/w").unwrap();
    // 下一次寫入不應回頭用 cluster 0，而是接著用 cluster 1
    s.write("/w", "2").unwrap();
    assert_eq!(magic_clusters(&mut s), vec![1]);
}

#[test]
fn no_space_rejected_cleanly() {
    let mut s = new_store();
    // 81 個 cluster 的需求 > 80 個容量
    let too_big = 4060 - 2 + 80 * 4080; // name "/f" → cap0=4058，再 +80 塊都不夠
    assert_eq!(
        s.write_bytes("/f", &pattern(too_big)),
        Err(StoreError::NoSpace)
    );
    assert!(s.files().is_empty());
    assert_eq!(s.used_space(), 0);
}

#[test]
fn overwrite_needs_space_for_both_copies() {
    let mut s = new_store();
    // 佔掉 79 個 cluster：cap0("/big"=4) = 4056，+ 78 塊 = 4056 + 78*4080
    let d79 = 4056 + 78 * 4080;
    s.write_bytes("/big", &pattern(d79)).unwrap();
    s.write("/one", "x").unwrap(); // 第 80 個 cluster
    assert_eq!(s.free_space(), 0);
    // 空間已滿：再寫新檔要失敗
    assert_eq!(s.write("/more", "y"), Err(StoreError::NoSpace));
    // 覆寫 /one 需要新舊並存（需要 1 個 free cluster），也要失敗且舊檔完好
    assert_eq!(s.write("/one", "changed"), Err(StoreError::NoSpace));
    assert_eq!(s.read("/one").unwrap(), "x");
    // 刪掉大檔後就能寫了
    s.delete("/big").unwrap();
    s.write("/one", "changed").unwrap();
    assert_eq!(s.read("/one").unwrap(), "changed");
}

#[test]
fn read_range_windows() {
    let mut s = new_store();
    let name = "/r.bin"; // 6 bytes → cap0 = 4054
    let data = pattern(10000);
    s.write_bytes(name, &data).unwrap();

    // 檔案開頭
    let mut buf = [0u8; 100];
    assert_eq!(s.read_range(name, 0, &mut buf).unwrap(), 100);
    assert_eq!(&buf[..], &data[0..100]);

    // 跨越首塊與第二塊的邊界（cap0 = 4054）
    let mut buf = [0u8; 20];
    assert_eq!(s.read_range(name, 4044, &mut buf).unwrap(), 20);
    assert_eq!(&buf[..], &data[4044..4064]);

    // 檔案尾端：要的比剩下的多，只回剩下的
    let mut buf = [0u8; 100];
    assert_eq!(s.read_range(name, 9990, &mut buf).unwrap(), 10);
    assert_eq!(&buf[..10], &data[9990..10000]);

    // offset 在檔案結尾或超過 → 0
    assert_eq!(s.read_range(name, 10000, &mut buf).unwrap(), 0);
    assert_eq!(s.read_range(name, 99999, &mut buf).unwrap(), 0);

    // 空 buffer → 0
    assert_eq!(s.read_range(name, 0, &mut []).unwrap(), 0);

    // 整份用 read_range 讀回來要與 read_bytes 一致
    let mut all = vec![0u8; 10000];
    assert_eq!(s.read_range(name, 0, &mut all).unwrap(), 10000);
    assert_eq!(all, data);

    // 只讀最後一塊的內容（前面的 cluster 走快速跳過路徑）
    let mut tail = vec![0u8; 500];
    assert_eq!(s.read_range(name, 9000, &mut tail).unwrap(), 500);
    assert_eq!(&tail[..], &data[9000..9500]);
}

#[test]
fn append_cases() {
    let mut s = new_store();
    // 不存在 → 建新檔
    s.append("/log", "line1\n").unwrap();
    s.append("/log", "line2\n").unwrap();
    s.append("/log", "line3\n").unwrap();
    assert_eq!(s.read("/log").unwrap(), "line1\nline2\nline3\n");

    // 追加到跨 cluster
    let mut expect = pattern(4000);
    s.write_bytes("/grow", &expect).unwrap();
    let extra = pattern(5000);
    s.append_bytes("/grow", &extra).unwrap();
    expect.extend_from_slice(&extra);
    assert_eq!(s.read_bytes("/grow").unwrap(), expect);
    assert_eq!(s.file_size("/grow").unwrap(), 9000);
}

#[test]
fn rename_cases() {
    let mut s = new_store();
    s.write("/old", "content").unwrap();
    s.rename("/old", "/new").unwrap();
    assert_eq!(s.read("/new").unwrap(), "content");
    assert_eq!(s.read("/old"), Err(StoreError::NotFound));

    // 目標已存在 → 覆蓋
    s.write("/target", "will be replaced").unwrap();
    s.rename("/new", "/target").unwrap();
    assert_eq!(s.read("/target").unwrap(), "content");
    assert!(!s.exists("/new"));

    // 來源不存在 → NotFound
    assert_eq!(s.rename("/ghost", "/x"), Err(StoreError::NotFound));

    // 同名 → no-op
    s.rename("/target", "/target").unwrap();
    assert_eq!(s.read("/target").unwrap(), "content");
    assert_eq!(s.rename("/ghost", "/ghost"), Err(StoreError::NotFound));
}

#[test]
fn read_dir_semantics() {
    let mut s = new_store();
    s.write("/data/foo.txt", "1").unwrap();
    s.write("/data/sub/a.txt", "2").unwrap();
    s.write("/data/sub/b.txt", "3").unwrap();
    s.write("/data2/other.txt", "4").unwrap();
    s.write("/top", "5").unwrap();

    let mut entries = s.read_dir("/data");
    entries.sort();
    assert_eq!(entries, vec!["/data/foo.txt", "/data/sub/"]);

    // 結尾斜線正規化
    assert_eq!(s.read_dir("/data"), s.read_dir("/data/"));

    let mut root = s.read_dir("/");
    root.sort();
    assert_eq!(root, vec!["/data/", "/data2/", "/top"]);

    let mut sub = s.read_dir("/data/sub");
    sub.sort();
    assert_eq!(sub, vec!["/data/sub/a.txt", "/data/sub/b.txt"]);
}

#[test]
fn delete_dir_cases() {
    let mut s = new_store();
    s.write("/data/a", "1").unwrap();
    s.write("/data/sub/b", "2").unwrap();
    s.write("/data2/c", "3").unwrap();
    s.write("/top", "4").unwrap();

    assert_eq!(s.delete_dir("/data").unwrap(), 2);
    let mut left = s.files();
    left.sort();
    assert_eq!(left, vec!["/data2/c", "/top"]);

    // 沒有東西可刪 → 0
    assert_eq!(s.delete_dir("/none").unwrap(), 0);
}

#[test]
fn persistence_across_reopen() {
    let mut s = new_store();
    let data = pattern(9000);
    s.write_bytes("/keep.bin", &data).unwrap();
    s.write("/keep.txt", "still here").unwrap();
    let mut s = reopen(s);
    assert_eq!(s.read_bytes("/keep.bin").unwrap(), data);
    assert_eq!(s.read("/keep.txt").unwrap(), "still here");
}

#[test]
fn torn_write_reads_as_corrupted_not_panic() {
    // 模擬多 cluster 檔案寫到一半斷電：接續塊被清掉
    let mut s = new_store();
    let data = pattern(10000);
    s.write_bytes("/torn.bin", &data).unwrap();
    let chain = magic_clusters(&mut s);
    let first = chain[0];
    // 找出鏈上的第二塊（首塊尾端記錄的下一塊位址）
    let next_addr = {
        let mem = &s.storage_mut().mem;
        let off = first * CLUSTER + CLUSTER - 4;
        u32::from_be_bytes(mem[off..off + 4].try_into().unwrap()) as usize
    };
    // 把第二塊整塊清成 0xFF（模擬從未寫入）
    s.storage_mut().mem[next_addr..next_addr + CLUSTER].fill(0xFF);

    // 讀取必須優雅回報損毀，不能 panic
    assert_eq!(s.read_bytes("/torn.bin"), Err(StoreError::Corrupted));

    // 重新開機：heal 要把殘檔整個回收
    let mut s = reopen(s);
    assert_eq!(s.read_bytes("/torn.bin"), Err(StoreError::NotFound));
    assert_eq!(s.used_space(), 0);
}

#[test]
fn heal_collects_orphan_continuation() {
    // 模擬「接續塊已寫入、header 還沒寫就斷電」：只剩孤兒 CONT_B 塊
    let mut s = new_store();
    let cont_b = [0x01u8, 0x31, 0x2A, 0xAD];
    s.storage_mut().mem[3 * CLUSTER..3 * CLUSTER + 4].copy_from_slice(&cont_b);
    assert_eq!(s.used_space(), CLUSTER as u32); // 目前被視為已使用
    let mut s = reopen(s);
    assert_eq!(s.used_space(), 0); // heal 後回收
}

#[test]
fn interrupted_overwrite_keeps_newest_generation() {
    // 模擬覆寫在「新檔已寫完、舊檔還沒刪」時斷電 → 同名兩份
    let mut s = new_store();
    s.write("/cfg", "version-1").unwrap();
    let v1_cluster = magic_clusters(&mut s)[0];
    let snapshot = s.storage_mut().mem[v1_cluster * CLUSTER..(v1_cluster + 1) * CLUSTER].to_vec();

    s.write("/cfg", "version-2").unwrap();
    // 把 v1 的 cluster 原樣塞回去，重現斷電後的狀態
    s.storage_mut().mem[v1_cluster * CLUSTER..(v1_cluster + 1) * CLUSTER].copy_from_slice(&snapshot);
    assert_eq!(magic_clusters(&mut s).len(), 2);

    // 未重開機：read 要挑 generation 較新的 v2（不是掃到哪份算哪份）
    assert_eq!(s.read("/cfg").unwrap(), "version-2");
    // files() 不能出現重複名稱
    assert_eq!(s.files(), vec![String::from("/cfg")]);

    // 重開機：heal 清掉舊的一份
    let mut s = reopen(s);
    assert_eq!(magic_clusters(&mut s).len(), 1);
    assert_eq!(s.read("/cfg").unwrap(), "version-2");
}

#[test]
fn corrupted_data_detected_by_crc() {
    let mut s = new_store();
    s.write("/c.txt", "important data").unwrap();
    let first = magic_clusters(&mut s)[0];
    // 翻轉資料區的一個 byte（header [20..20+L) 後面是 name_crc、data_crc，資料從 28+L 開始）
    let data_off = first * CLUSTER + 28 + "/c.txt".len();
    s.storage_mut().mem[data_off] ^= 0x01;
    assert_eq!(s.read("/c.txt"), Err(StoreError::Corrupted));
}

#[test]
fn corrupted_header_makes_file_invisible_and_healed() {
    let mut s = new_store();
    s.write("/h.txt", "data").unwrap();
    let first = magic_clusters(&mut s)[0];
    // 破壞 name_len 欄位 → header CRC 對不上 → 檔案不可見（而不是 panic）
    s.storage_mut().mem[first * CLUSTER + 7] ^= 0xFF;
    assert_eq!(s.read("/h.txt"), Err(StoreError::NotFound));
    assert!(s.files().is_empty());
    // 重開機後該 cluster 被回收
    let mut s = reopen(s);
    assert_eq!(s.used_space(), 0);
}

#[test]
fn used_free_capacity_consistency() {
    let mut s = new_store();
    assert_eq!(s.capacity(), FLASH_SIZE);
    s.write_bytes("/a", &pattern(5000)).unwrap();
    s.write("/b", "x").unwrap();
    assert_eq!(s.used_space() + s.free_space(), s.capacity());
    assert_eq!(s.used_space(), 3 * CLUSTER as u32); // 2 + 1 clusters
    s.delete("/a").unwrap();
    assert_eq!(s.used_space(), CLUSTER as u32);
}

#[test]
fn unicode_names_and_content() {
    let mut s = new_store();
    s.write("/資料/系統紀錄檔.txt", "繁體中文內容，含表情符號 \u{1F980}").unwrap();
    assert!(s.exists("/資料/系統紀錄檔.txt"));
    assert_eq!(
        s.read("/資料/系統紀錄檔.txt").unwrap(),
        "繁體中文內容，含表情符號 \u{1F980}"
    );
    assert_eq!(s.read_dir("/資料"), vec!["/資料/系統紀錄檔.txt"]);
}

#[test]
fn many_files_fill_and_list() {
    let mut s = new_store();
    for i in 0..40 {
        s.write(&format!("/n/{i:02}.txt"), &format!("file {i}")).unwrap();
    }
    assert_eq!(s.files().len(), 40);
    for i in 0..40 {
        assert_eq!(s.read(&format!("/n/{i:02}.txt")).unwrap(), format!("file {i}"));
    }
    assert_eq!(s.delete_dir("/n").unwrap(), 40);
    assert_eq!(s.used_space(), 0);
}

// ============================ v1 → v2 自動遷移 ============================
// 用 0.2.3/0.3.1 的寫入邏輯直接在 RAM 上鋪出 v1 格式資料，
// 驗證 Store::new 的自動遷移在各種情境下都不遺失檔案。

mod v1_writer {
    // 依 v1 佈局產生資料（與 0.2.3/0.3.1 的 save_cluster 一致）
    pub const CLUSTER: usize = 4096;
    const MAGIC: u32 = 0x01311AAB;
    const CONT_A: u32 = 0x01311AAC;
    const CONT_B: u32 = 0x01311AAD;

    fn crc32(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = if (crc & 1) != 0 { 0xEDB8_8320 } else { 0 };
                crc = (crc >> 1) ^ mask;
            }
        }
        !crc
    }

    pub fn payload(name: &str, data: &[u8]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&MAGIC.to_be_bytes());
        p.extend_from_slice(&(name.len() as u32).to_be_bytes());
        p.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut len_block = [0u8; 8];
        len_block[0..4].copy_from_slice(&(name.len() as u32).to_be_bytes());
        len_block[4..8].copy_from_slice(&(data.len() as u32).to_be_bytes());
        p.extend_from_slice(&crc32(&len_block).to_be_bytes());
        p.extend_from_slice(name.as_bytes());
        p.extend_from_slice(&crc32(name.as_bytes()).to_be_bytes());
        p.extend_from_slice(data);
        p.extend_from_slice(&crc32(data).to_be_bytes());
        p
    }

    // 單 cluster 檔案（v1 有隨機 0xFF 前導填充，front_pad 模擬它）
    pub fn write_single(mem: &mut [u8], cluster: usize, name: &str, data: &[u8], front_pad: usize) {
        let p = payload(name, data);
        assert!(front_pad + p.len() <= CLUSTER - 8, "v1 單塊 payload 超出");
        let base = cluster * CLUSTER;
        mem[base..base + CLUSTER].fill(0xFF);
        mem[base + front_pad..base + front_pad + p.len()].copy_from_slice(&p);
    }

    // 多 cluster 檔案：首塊裝 4088、之後每塊裝 4080，鏈用絕對位址串起來
    pub fn write_multi(mem: &mut [u8], clusters: &[usize], name: &str, data: &[u8]) {
        let p = payload(name, data);
        let mut off = 0usize;
        for (k, &c) in clusters.iter().enumerate() {
            let base = c * CLUSTER;
            mem[base..base + CLUSTER].fill(0xFF);
            if k == 0 {
                let take = (CLUSTER - 8).min(p.len());
                mem[base..base + take].copy_from_slice(&p[..take]);
                off = take;
            } else {
                mem[base..base + 4].copy_from_slice(&CONT_B.to_be_bytes());
                let prev = (clusters[k - 1] * CLUSTER) as u32;
                mem[base + 4..base + 8].copy_from_slice(&prev.to_be_bytes());
                let take = (CLUSTER - 16).min(p.len() - off);
                mem[base + 8..base + 8 + take].copy_from_slice(&p[off..off + take]);
                off += take;
            }
            if k + 1 < clusters.len() {
                mem[base + CLUSTER - 8..base + CLUSTER - 4].copy_from_slice(&CONT_A.to_be_bytes());
                let next = (clusters[k + 1] * CLUSTER) as u32;
                mem[base + CLUSTER - 4..base + CLUSTER].copy_from_slice(&next.to_be_bytes());
            }
        }
        assert_eq!(off, p.len(), "clusters 數量與 payload 長度不符");
    }
}

// 分區內完全沒有 v1 痕跡（逐 cluster 跳過 0xFF 前導後檢查標記）
fn no_v1_left(mem: &[u8]) -> bool {
    for c in 0..mem.len() / CLUSTER {
        let cl = &mem[c * CLUSTER..(c + 1) * CLUSTER];
        let mut i = 0;
        while i < cl.len() && cl[i] == 0xFF {
            i += 1;
        }
        if i + 4 <= cl.len() {
            let w = u32::from_be_bytes(cl[i..i + 4].try_into().unwrap());
            if w == 0x01311AAB || w == 0x01311AAD {
                return false;
            }
        }
    }
    true
}

#[test]
fn migrate_v1_basic() {
    let mut flash = RamFlash::new(FLASH_SIZE as usize);
    let big = pattern(12000);
    v1_writer::write_single(&mut flash.mem, 0, "/wifi/ssid.txt", b"MyHomeWiFi", 0);
    v1_writer::write_single(&mut flash.mem, 2, "/wifi/pass.txt", b"secret123", 37); // 有前導填充
    v1_writer::write_multi(&mut flash.mem, &[5, 6, 7], "/data/big.bin", &big);

    let mut s = Store::new(flash, 0, FLASH_SIZE);
    assert_eq!(s.read("/wifi/ssid.txt").unwrap(), "MyHomeWiFi");
    assert_eq!(s.read("/wifi/pass.txt").unwrap(), "secret123");
    assert_eq!(s.read_bytes("/data/big.bin").unwrap(), big);
    assert_eq!(s.files().len(), 3);
    assert!(no_v1_left(&s.storage_mut().mem));
    assert_eq!(s.used_space() + s.free_space(), s.capacity());

    // 再開一次也要穩定（遷移是冪等的）
    let mut s = reopen(s);
    assert_eq!(s.read("/wifi/ssid.txt").unwrap(), "MyHomeWiFi");
    assert_eq!(s.files().len(), 3);
}

#[test]
fn migrate_v1_rescues_sizes_v1_could_not_read() {
    // 這些尺寸在 0.2.x/0.3.x 下「寫得進、讀不回」（CRC 跨 cluster 邊界 → panic）
    // 遷移器要能救回來
    for d in [4054usize, 4055, 4056] {
        let mut flash = RamFlash::new(FLASH_SIZE as usize);
        let data = pattern(d);
        v1_writer::write_multi(&mut flash.mem, &[0, 1], "/data/x.bin", &data);
        let mut s = Store::new(flash, 0, FLASH_SIZE);
        assert_eq!(s.read_bytes("/data/x.bin").unwrap(), data, "尺寸 {} 應被救回", d);
        assert!(no_v1_left(&s.storage_mut().mem));
    }
}

#[test]
fn migrate_v1_resumes_after_interrupted_migration() {
    // 模擬上次遷移在「v2 複本已寫、v1 還沒擦」之間斷電：
    // v1 與 v2 同名並存 → 只補擦 v1，內容以 v2 為準
    let mut flash = RamFlash::new(FLASH_SIZE as usize);
    v1_writer::write_single(&mut flash.mem, 10, "/cfg", b"from-v1", 0);
    let mut s = Store::new(flash, 0, FLASH_SIZE); // 正常遷移完成
    assert_eq!(s.read("/cfg").unwrap(), "from-v1");

    v1_writer::write_single(&mut s.storage_mut().mem, 20, "/cfg", b"from-v1", 0);
    let mut s = reopen(s);
    assert_eq!(s.read("/cfg").unwrap(), "from-v1");
    assert_eq!(s.files(), vec![String::from("/cfg")]);
    assert!(no_v1_left(&s.storage_mut().mem));
}

#[test]
fn migrate_v1_corrupt_and_torn_are_reclaimed() {
    let mut flash = RamFlash::new(FLASH_SIZE as usize);
    // 資料 CRC 被破壞的單塊檔（0.2.3 下也讀不到）
    v1_writer::write_single(&mut flash.mem, 0, "/bad.bin", &pattern(100), 0);
    flash.mem[28 + "/bad.bin".len()] ^= 0x01; // 翻資料第一個 byte（v1 資料從 20+L 開始，這裡改壞它）
    // 斷鏈的多塊檔：只鋪首塊，接續塊缺失
    let p = v1_writer::payload("/torn.bin", &pattern(9000));
    let base = 3 * CLUSTER;
    flash.mem[base..base + 4088].copy_from_slice(&p[..4088]);
    flash.mem[base + 4088..base + 4092].copy_from_slice(&0x01311AACu32.to_be_bytes());
    flash.mem[base + 4092..base + 4096].copy_from_slice(&((5 * CLUSTER) as u32).to_be_bytes());
    // 孤兒接續塊（header 從未寫入）
    flash.mem[8 * CLUSTER..8 * CLUSTER + 4].copy_from_slice(&0x01311AADu32.to_be_bytes());

    let mut s = Store::new(flash, 0, FLASH_SIZE);
    assert!(s.files().is_empty());
    assert_eq!(s.used_space(), 0); // 全部回收
    assert!(no_v1_left(&s.storage_mut().mem));
}

#[test]
fn migrate_v1_duplicate_names_keep_one() {
    // v1 時代覆寫斷電可能留下同名兩份（v1 沒有世代序號，保留其中一份）
    let mut flash = RamFlash::new(FLASH_SIZE as usize);
    v1_writer::write_single(&mut flash.mem, 3, "/cfg", b"copy-A", 0);
    v1_writer::write_single(&mut flash.mem, 30, "/cfg", b"copy-B", 0);
    let mut s = Store::new(flash, 0, FLASH_SIZE);
    assert_eq!(s.files(), vec![String::from("/cfg")]);
    let content = s.read("/cfg").unwrap();
    assert!(content == "copy-A" || content == "copy-B");
    assert!(no_v1_left(&s.storage_mut().mem));
}

#[test]
fn migrate_v1_no_space_preserves_data_until_it_fits() {
    // v2 資料佔到只剩 2 個 free cluster，v1 檔需要 3 個 → 搬不動：
    // 必須原樣保留、不能被覆寫；騰出空間後的下一次開機要成功搬移
    let mut s = new_store();
    let big = pattern(4056 + 73 * 4080); // "/big"：74 clusters
    s.write_bytes("/big", &big).unwrap();
    s.write("/one", "x").unwrap(); // 第 75 個
    let mut flash = s.into_storage();

    let v1_data = pattern(9000); // v1 佔 3 塊，v2 複本也要 3 塊 > 剩餘 2 塊
    v1_writer::write_multi(&mut flash.mem, &[75, 76, 77], "/keep.bin", &v1_data);
    let snapshot = flash.mem[75 * CLUSTER..78 * CLUSTER].to_vec();

    let mut s = Store::new(flash, 0, FLASH_SIZE);
    assert!(!s.exists("/keep.bin")); // 還沒搬成，v2 看不到
    assert_eq!(s.free_space(), 2 * CLUSTER as u32); // v1 區塊視為已佔用
    // 寫滿剩餘空間也不會碰到 v1 的區塊
    s.write("/fill1", "a").unwrap();
    s.write("/fill2", "b").unwrap();
    assert_eq!(s.write("/fill3", "c"), Err(StoreError::NoSpace));
    assert_eq!(
        &s.storage_mut().mem[75 * CLUSTER..78 * CLUSTER],
        &snapshot[..],
        "v1 資料必須原封不動"
    );

    // 騰出空間 → 重新開機 → 自動搬移成功
    s.delete("/big").unwrap();
    let mut s = reopen(s);
    assert_eq!(s.read_bytes("/keep.bin").unwrap(), v1_data);
    assert!(no_v1_left(&s.storage_mut().mem));
    assert_eq!(s.read("/one").unwrap(), "x"); // 其他檔案不受影響
}

#[test]
fn migrate_v1_mixed_with_v2_files() {
    // v2 檔案已存在的分區裡混入 v1 檔案 → 兩邊都完好
    let mut s = new_store();
    s.write("/v2/a.txt", "already v2").unwrap();
    s.write_bytes("/v2/b.bin", &pattern(5000)).unwrap();
    let mut flash = s.into_storage();
    v1_writer::write_single(&mut flash.mem, 40, "/v1/c.txt", "舊格式檔案".as_bytes(), 5);

    let mut s = Store::new(flash, 0, FLASH_SIZE);
    assert_eq!(s.read("/v2/a.txt").unwrap(), "already v2");
    assert_eq!(s.read_bytes("/v2/b.bin").unwrap(), pattern(5000));
    assert_eq!(s.read("/v1/c.txt").unwrap(), "舊格式檔案");
    assert_eq!(s.files().len(), 3);
    assert!(no_v1_left(&s.storage_mut().mem));
}

#[test]
#[should_panic]
fn new_rejects_tiny_flash() {
    let _ = Store::new(RamFlash::new(2048), 0, 2048);
}

#[test]
#[should_panic]
fn new_rejects_unaligned_addr() {
    let _ = Store::new(RamFlash::new(0x3000), 0x100, 0x2000);
}
